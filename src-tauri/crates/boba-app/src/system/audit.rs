//! The audit use case (AD-12, FR-89): write an `intent` before an outbound
//! action and an `outcome` after it, verify the chain at startup, and resolve
//! crashed intents. All IO goes through [`SystemRepo`].

use crate::ports::clock::ClockPort;
use crate::ports::repo::system::{NewIntent, SystemRepo, SystemRepoError};
use boba_domain::audit::{AuditOutcome, AuditRow, ChainStatus};

/// A use-case failure.
#[derive(Debug, thiserror::Error)]
pub enum SystemError {
    #[error(transparent)]
    Repo(#[from] SystemRepoError),
    #[error("invalid value for setting {key}: {value}")]
    InvalidSetting { key: String, value: String },
}

/// What an `intent` row records about the action it precedes.
#[derive(Debug, Clone)]
pub struct IntentRequest {
    /// Stable action name, e.g. `apply.submitted`, `email.sent`.
    pub action: String,
    /// Groups the `intent`/`outcome` pair for one attempt.
    pub correlation_id: String,
    /// `core`, `user` or `auto`.
    pub actor: String,
    /// Kind of target, e.g. `source`, `email`, `application`.
    pub target_kind: String,
    pub target_id: Option<String>,
    /// Metadata JSON (IDs, counts, reasons) — never a payload.
    pub detail_json: String,
    /// blake3 of the payload, never the payload (AD-12).
    pub payload_blake3: Option<String>,
}

impl IntentRequest {
    pub fn new(
        action: impl Into<String>,
        correlation_id: impl Into<String>,
        actor: impl Into<String>,
        target_kind: impl Into<String>,
    ) -> Self {
        Self {
            action: action.into(),
            correlation_id: correlation_id.into(),
            actor: actor.into(),
            target_kind: target_kind.into(),
            target_id: None,
            detail_json: "{}".to_string(),
            payload_blake3: None,
        }
    }
}

/// Why the chain did not match at startup (AC-3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TamperWarning {
    /// A row failed re-hashing or linking, at this `seq`.
    ChainBroken { seq: Option<i64> },
    /// The keychain `audit-head` anchor does not match the chain head.
    AnchorMismatch,
}

/// What the startup check did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupReport {
    /// `Some` when the log failed verification; the UI shows a tamper warning.
    pub tamper_warning: Option<TamperWarning>,
    /// Crashed intents resolved with `unknown_after_crash`.
    pub recovered_intents: usize,
    /// Whether the keychain anchor was refreshed.
    pub anchor_updated: bool,
}

/// Record outbound actions and check the chain (AD-12).
pub struct AuditService<R, C> {
    repo: R,
    clock: C,
}

impl<R: SystemRepo, C: ClockPort> AuditService<R, C> {
    pub fn new(repo: R, clock: C) -> Self {
        Self { repo, clock }
    }

    /// Append the `intent` row before an outbound action runs.
    pub fn record_intent(&self, req: &IntentRequest) -> Result<AuditRow, SystemError> {
        let intent = NewIntent {
            action: req.action.clone(),
            correlation_id: req.correlation_id.clone(),
            actor: req.actor.clone(),
            target_kind: req.target_kind.clone(),
            target_id: req.target_id.clone(),
            detail_json: req.detail_json.clone(),
            payload_blake3: req.payload_blake3.clone(),
            created_at: self.clock.now_rfc3339(),
        };
        Ok(self.repo.audit_append_intent(&intent)?)
    }

    /// Append the `outcome` row after the action runs.
    pub fn record_outcome(
        &self,
        intent: &AuditRow,
        outcome: AuditOutcome,
        detail_json: &str,
    ) -> Result<AuditRow, SystemError> {
        Ok(self.repo.audit_append_outcome(
            intent,
            outcome,
            detail_json,
            &self.clock.now_rfc3339(),
        )?)
    }

    /// Walk the whole chain.
    pub fn verify(&self) -> Result<ChainStatus, SystemError> {
        Ok(self.repo.audit_verify_chain()?)
    }

    /// Resolve every crashed `intent` with `unknown_after_crash` (FR-89). The
    /// appended outcome is the marker the Tracker reads as Needs attention.
    pub fn resolve_crashed_intents(&self) -> Result<Vec<AuditRow>, SystemError> {
        let mut appended = Vec::new();
        for intent in self.repo.audit_unresolved_intents()? {
            appended.push(self.record_outcome(
                &intent,
                AuditOutcome::UnknownAfterCrash,
                "{\"reason\":\"no_outcome_at_startup\"}",
            )?);
        }
        Ok(appended)
    }

    /// Startup check (AC-2, AC-3): verify the chain and the keychain anchor,
    /// then resolve crashed intents and refresh the anchor.
    ///
    /// The anchor names the head after every append, so on a clean run it equals
    /// the head. After a crash between an append and its anchor write it may lag
    /// to an earlier row; such an anchor names a row that is still in the chain,
    /// so it is accepted and re-synced. An anchor that names **no** row means the
    /// tail it pinned was removed — truncation or a re-minted log — and is
    /// reported as tamper. On tamper nothing is appended and the anchor is left
    /// untouched, so the evidence survives for inspection.
    pub fn startup(&self) -> Result<StartupReport, SystemError> {
        let status = self.repo.audit_verify_chain()?;

        if !status.is_intact() {
            return Ok(StartupReport {
                tamper_warning: Some(TamperWarning::ChainBroken {
                    seq: status.broken_at,
                }),
                recovered_intents: 0,
                anchor_updated: false,
            });
        }

        if let Some(stored) = self.repo.audit_anchor_get()?
            && !self.repo.audit_anchor_is_known(&stored)?
        {
            return Ok(StartupReport {
                tamper_warning: Some(TamperWarning::AnchorMismatch),
                recovered_intents: 0,
                anchor_updated: false,
            });
        }

        let recovered_intents = self.resolve_crashed_intents()?.len();
        let anchor_updated = match self.repo.audit_head_hash()? {
            Some(head) => {
                self.repo.audit_anchor_set(&head)?;
                true
            }
            None => false,
        };
        Ok(StartupReport {
            tamper_warning: None,
            recovered_intents,
            anchor_updated,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::{FixedClock, testing::FakeSystemRepo};

    fn service(repo: FakeSystemRepo) -> AuditService<FakeSystemRepo, FixedClock> {
        AuditService::new(repo, FixedClock::new("2026-10-05T01:00:00.000Z"))
    }

    fn request() -> IntentRequest {
        IntentRequest::new("apply.submitted", "att_1", "core", "source")
    }

    // AC-2, AC-3: a clean chain is reported intact and the anchor is written.
    #[test]
    fn startup_on_clean_chain_writes_anchor() {
        let svc = service(FakeSystemRepo::default());
        let intent = svc.record_intent(&request()).unwrap();
        svc.record_outcome(&intent, AuditOutcome::Ok, "{}").unwrap();

        let report = svc.startup().unwrap();
        assert_eq!(report.tamper_warning, None);
        assert_eq!(report.recovered_intents, 0);
        assert!(report.anchor_updated);
    }

    // AC-2: a crashed intent is resolved and the attempt is marked.
    #[test]
    fn startup_resolves_crashed_intent() {
        let svc = service(FakeSystemRepo::default());
        svc.record_intent(&request()).unwrap();

        let report = svc.startup().unwrap();
        assert_eq!(report.tamper_warning, None);
        assert_eq!(report.recovered_intents, 1);
        assert!(svc.verify().unwrap().is_intact());
    }

    // AC-3: a mismatched keychain anchor raises a tamper warning.
    #[test]
    fn startup_flags_anchor_mismatch() {
        let svc = service(FakeSystemRepo::default());
        let intent = svc.record_intent(&request()).unwrap();
        svc.record_outcome(&intent, AuditOutcome::Ok, "{}").unwrap();
        svc.repo.set_anchor("some-other-hash");

        let report = svc.startup().unwrap();
        assert_eq!(report.tamper_warning, Some(TamperWarning::AnchorMismatch));
        assert_eq!(report.recovered_intents, 0);
        assert!(!report.anchor_updated, "evidence must not be overwritten");
    }

    // AC-3: a broken chain raises a tamper warning and writes nothing.
    #[test]
    fn startup_flags_broken_chain_without_writing() {
        let svc = service(FakeSystemRepo::default());
        let intent = svc.record_intent(&request()).unwrap();
        svc.record_outcome(&intent, AuditOutcome::Ok, "{}").unwrap();
        svc.repo.corrupt_last();

        let report = svc.startup().unwrap();
        assert_eq!(
            report.tamper_warning,
            Some(TamperWarning::ChainBroken { seq: Some(2) })
        );
        assert!(!report.anchor_updated);
    }

    // P1 regression: an append after startup must not make the next startup a
    // false AnchorMismatch — the anchor advances with the head on every append.
    #[test]
    fn restart_after_appends_reports_no_anchor_mismatch() {
        let svc = service(FakeSystemRepo::default());
        let i = svc.record_intent(&request()).unwrap();
        svc.record_outcome(&i, AuditOutcome::Ok, "{}").unwrap();
        assert_eq!(svc.startup().unwrap().tamper_warning, None);

        // Normal activity after the first startup.
        let i2 = svc.record_intent(&request()).unwrap();
        svc.record_outcome(&i2, AuditOutcome::Ok, "{}").unwrap();

        let report = svc.startup().unwrap();
        assert_eq!(report.tamper_warning, None);
        assert_eq!(report.recovered_intents, 0);
        assert!(report.anchor_updated);
    }

    // P1 regression: truncating rows appended after the anchor is detected.
    #[test]
    fn truncating_rows_appended_after_the_anchor_is_detected() {
        let svc = service(FakeSystemRepo::default());
        let i = svc.record_intent(&request()).unwrap();
        svc.record_outcome(&i, AuditOutcome::Ok, "{}").unwrap();
        svc.startup().unwrap();

        let i2 = svc.record_intent(&request()).unwrap();
        svc.record_outcome(&i2, AuditOutcome::Ok, "{}").unwrap();
        svc.repo.truncate_last();

        let report = svc.startup().unwrap();
        assert_eq!(report.tamper_warning, Some(TamperWarning::AnchorMismatch));
        assert!(!report.anchor_updated, "evidence must not be overwritten");
    }

    // P1 crash-safety: an anchor left lagging behind the head (as after a crash
    // between the append and its anchor write) is a known prefix, not tamper,
    // and startup re-syncs it to the head.
    #[test]
    fn lagging_anchor_that_is_a_known_prefix_is_resynced_not_flagged() {
        let svc = service(FakeSystemRepo::default());
        let i = svc.record_intent(&request()).unwrap();
        let o = svc.record_outcome(&i, AuditOutcome::Ok, "{}").unwrap();
        svc.repo.set_anchor(&i.hash); // crash: anchor lags at the intent row

        let report = svc.startup().unwrap();
        assert_eq!(report.tamper_warning, None);
        assert!(report.anchor_updated);
        assert_eq!(
            svc.repo.audit_anchor_get().unwrap().as_deref(),
            Some(o.hash.as_str())
        );
    }
}
