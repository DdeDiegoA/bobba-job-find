//! The Audit Log viewer's read side (S9.9, FR-89, SM-7): every send, Auto
//! decision, Kill Switch event, opt-in and Stage change in one timeline, plus
//! the hash-chain verdict and a CSV export.
//!
//! It reads, it never writes. Each entry comes from the area that owns it
//! (AD-16): `audit_event` from [`SystemRepo`], `status_event` from [`ApplyRepo`],
//! `apply_outbox` from [`OutboxHoldRepo`], the rule names from
//! [`AutoRulesRepo`] and the opt-ins from [`SourcesRepo`].
//!
//! A send's Package is named by the `payload_blake3` its audit row carries
//! (AD-12: the hash, never the payload). The Application id is the link the UI
//! follows to the archived Package.

use std::collections::BTreeMap;
use std::sync::Arc;

use boba_domain::apply::outbox::OutboxItem;
use boba_domain::audit::AuditRow;
use serde::{Deserialize, Serialize};
use specta::Type;

use super::StatsError;
use crate::ports::repo::apply::ApplyRepo;
use crate::ports::repo::auto_rules::AutoRulesRepo;
use crate::ports::repo::outbox_hold::OutboxHoldRepo;
use crate::ports::repo::sources::SourcesRepo;
use crate::ports::repo::system::SystemRepo;

/// `outbox_list`'s "no cap": the adapter converts the limit to `i64::MAX`, so
/// every stored item comes back (AC-1: no decision may silently fall off the
/// viewer or the CSV).
const ALL_OUTBOX_ITEMS: usize = usize::MAX;

/// What one timeline row is (the UI picks its icon and label from this).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct AuditEntry {
    /// A stable id: the row's own ULID, a `status_event` id, or a derived id
    /// for an outbox item / opt-in.
    pub id: String,
    /// `send`, `decision`, `kill_switch`, `optin`, `stage_change` or `other`.
    pub kind: String,
    /// UTC RFC 3339 (AD-17); the UI converts to local time.
    pub at: String,
    /// The action name (`apply.submitted`, `stage.applied`, `auto.decision`, …).
    pub action: String,
    /// `core`, `user`, `auto` or a Status source (`status_proposal`, …).
    pub actor: String,
    /// `intent` / `outcome` on an audit row, `None` elsewhere.
    pub phase: Option<String>,
    /// `ok`, `failed`, `unknown_after_crash`, or the outbox state.
    pub outcome: Option<String>,
    pub target_kind: String,
    pub target_id: Option<String>,
    /// The Application the row is about, when there is one: the link to its
    /// archived Package.
    pub application_id: Option<String>,
    /// The row's metadata as stored (rule, reasons, from/to).
    pub detail_json: String,
    /// blake3 of the Package/message that was sent (AD-12).
    pub payload_blake3: Option<String>,
    /// The chain hash of an `audit_event` row; `None` for the merged sources.
    pub hash: Option<String>,
}

/// The Audit Log as the viewer renders it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct AuditLogView {
    /// Newest first.
    pub entries: Vec<AuditEntry>,
    /// Whether every `audit_event` row re-hashes and links (AD-12).
    pub chain_intact: bool,
    /// How many `audit_event` rows the check walked.
    pub chain_rows: u32,
    /// `seq` of the first broken row, when the chain does not verify. The chain
    /// is `i64`; a position that large is reported as unknown, never truncated
    /// silently (specta cannot export `i64` to TypeScript).
    pub broken_at: Option<i32>,
}

/// Read the Audit Log (AC-1).
pub struct AuditLogService {
    system: Arc<dyn SystemRepo>,
    applications: Arc<dyn ApplyRepo>,
    outbox: Arc<dyn OutboxHoldRepo>,
    rules: Arc<dyn AutoRulesRepo>,
    sources: Arc<dyn SourcesRepo>,
}

impl AuditLogService {
    pub fn new(
        system: Arc<dyn SystemRepo>,
        applications: Arc<dyn ApplyRepo>,
        outbox: Arc<dyn OutboxHoldRepo>,
        rules: Arc<dyn AutoRulesRepo>,
        sources: Arc<dyn SourcesRepo>,
    ) -> Self {
        Self {
            system,
            applications,
            outbox,
            rules,
            sources,
        }
    }

    /// Every entry, newest first, with the chain verdict.
    pub fn entries(&self) -> Result<AuditLogView, StatsError> {
        let rows = self.system.audit_rows()?;
        let chain = self.system.audit_verify_chain()?;

        let mut entries: Vec<AuditEntry> = rows.iter().map(audit_entry).collect();
        entries.extend(self.stage_entries()?);
        entries.extend(self.outbox_entries()?);
        entries.extend(self.opt_in_entries()?);

        entries.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| b.id.cmp(&a.id)));
        Ok(AuditLogView {
            entries,
            chain_intact: chain.is_intact(),
            chain_rows: u32::try_from(chain.rows).unwrap_or(u32::MAX),
            broken_at: chain.broken_at.and_then(|seq| i32::try_from(seq).ok()),
        })
    }

    /// The whole log as CSV (AC-1: "I can export it as CSV").
    pub fn csv(&self) -> Result<String, StatsError> {
        Ok(to_csv(&self.entries()?.entries))
    }

    /// Every Stage change: the Tracker's own history (S9.1, FR-89).
    fn stage_entries(&self) -> Result<Vec<AuditEntry>, StatsError> {
        let mut entries = Vec::new();
        for application in self.applications.applications()? {
            for event in self.applications.status_events(&application.id)? {
                let detail = serde_json::json!({
                    "from": event.from_status.map(|status| status.as_str()),
                    "to": event.to_status.as_str(),
                    "reason": event.reason,
                })
                .to_string();
                entries.push(AuditEntry {
                    id: format!("status:{}", event.id),
                    kind: "stage_change".to_string(),
                    at: event.created_at,
                    action: format!("stage.{}", event.to_status.as_str()),
                    actor: event.source.as_str().to_string(),
                    phase: None,
                    outcome: None,
                    target_kind: "application".to_string(),
                    target_id: Some(application.id.clone()),
                    application_id: Some(application.id.clone()),
                    detail_json: detail,
                    payload_blake3: None,
                    hash: None,
                });
            }
        }
        Ok(entries)
    }

    /// Auto decisions that produced (or held back) an Outbox item, with the rule
    /// and the reason the item left the automatic path (FR-83/FR-85).
    fn outbox_entries(&self) -> Result<Vec<AuditEntry>, StatsError> {
        let items = self.outbox.outbox_list(ALL_OUTBOX_ITEMS)?;
        if items.is_empty() {
            return Ok(Vec::new());
        }
        let rule_names: BTreeMap<String, String> = self
            .rules
            .rules()?
            .into_iter()
            .map(|rule| (rule.id, rule.name))
            .collect();
        Ok(items
            .into_iter()
            .map(|item| outbox_entry(&item, &rule_names))
            .collect())
    }

    /// The user's opt-ins (AD-9/FR-91): accepted at, and the warning version
    /// they were shown.
    fn opt_in_entries(&self) -> Result<Vec<AuditEntry>, StatsError> {
        let mut entries = Vec::new();
        for source in self.sources.list_sources()? {
            let Some(optin) = self.sources.get_optin(&source.id)? else {
                continue;
            };
            let detail = serde_json::json!({
                "source_id": source.id,
                "source_name": source.name,
                "warning_version": optin.warning_version,
            })
            .to_string();
            entries.push(AuditEntry {
                id: format!("optin:{}", source.id),
                kind: "optin".to_string(),
                at: optin.accepted_at,
                action: "source.optin".to_string(),
                actor: "user".to_string(),
                phase: None,
                outcome: None,
                target_kind: "source".to_string(),
                target_id: Some(source.id),
                application_id: None,
                detail_json: detail,
                payload_blake3: None,
                hash: None,
            });
        }
        Ok(entries)
    }
}

fn audit_entry(row: &AuditRow) -> AuditEntry {
    AuditEntry {
        id: row.id.clone(),
        kind: kind_of(&row.action).to_string(),
        at: row.created_at.clone(),
        action: row.action.clone(),
        actor: row.actor.clone(),
        phase: Some(row.phase.as_str().to_string()),
        outcome: row.outcome.map(|outcome| outcome.as_str().to_string()),
        target_kind: row.target_kind.clone(),
        target_id: row.target_id.clone(),
        application_id: (row.target_kind == "application")
            .then(|| row.target_id.clone())
            .flatten(),
        detail_json: row.detail_json.clone(),
        payload_blake3: row.payload_blake3.clone(),
        hash: Some(row.hash.clone()),
    }
}

fn outbox_entry(item: &OutboxItem, rule_names: &BTreeMap<String, String>) -> AuditEntry {
    let detail = serde_json::json!({
        "rule_id": item.rule_id,
        "rule": rule_names.get(&item.rule_id),
        "channel": item.channel,
        "state": item.state.as_str(),
        "note": item.note,
    })
    .to_string();
    AuditEntry {
        id: format!("outbox:{}", item.id),
        kind: "decision".to_string(),
        at: item.created_at.clone(),
        action: "auto.decision".to_string(),
        actor: "auto".to_string(),
        phase: None,
        outcome: Some(item.state.as_str().to_string()),
        target_kind: "application".to_string(),
        target_id: Some(item.application_id.clone()),
        application_id: Some(item.application_id.clone()),
        detail_json: detail,
        payload_blake3: None,
        hash: None,
    }
}

/// What an audit action is, from its name (`apply.kill_switch`,
/// `apply.submitted`, `auto.decision`, …).
fn kind_of(action: &str) -> &'static str {
    if action == "apply.kill_switch" {
        "kill_switch"
    } else if action.starts_with("auto.") || action == "apply.decision" {
        "decision"
    } else if action.starts_with("apply.") {
        "send"
    } else {
        "other"
    }
}

/// CSV with an RFC 4180 header; every field is quoted, quotes doubled (AC-1).
pub fn to_csv(entries: &[AuditEntry]) -> String {
    let mut out = String::from(
        "timestamp,event,kind,actor,phase,outcome,target_kind,target,application,package_hash,hash,details\n",
    );
    for entry in entries {
        let fields = [
            entry.at.as_str(),
            entry.action.as_str(),
            entry.kind.as_str(),
            entry.actor.as_str(),
            entry.phase.as_deref().unwrap_or(""),
            entry.outcome.as_deref().unwrap_or(""),
            entry.target_kind.as_str(),
            entry.target_id.as_deref().unwrap_or(""),
            entry.application_id.as_deref().unwrap_or(""),
            entry.payload_blake3.as_deref().unwrap_or(""),
            entry.hash.as_deref().unwrap_or(""),
            entry.detail_json.as_str(),
        ];
        let line: Vec<String> = fields.iter().map(|field| csv_field(field)).collect();
        out.push_str(&line.join(","));
        out.push('\n');
    }
    out
}

/// One RFC 4180 field, with a leading `'` on a value a spreadsheet would run
/// as a formula (`=`, `+`, `-`, `@`, tab, CR): CSV injection is a real hole
/// when a Source name or an outbox note can start with `=HYPERLINK(...)`.
fn csv_field(value: &str) -> String {
    let guarded = if matches!(
        value.chars().next(),
        Some('=' | '+' | '-' | '@' | '\t' | '\r')
    ) {
        format!("'{value}")
    } else {
        value.to_string()
    };
    format!("\"{}\"", guarded.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use boba_domain::application::{
        AppStatus, Application, StatusEvent, StatusSource, new_status_event_id,
    };
    use boba_domain::apply::outbox::{NewOutboxItem, OutboxItem, OutboxState};
    use boba_domain::apply::policy::AutoAttemptEntry;
    use boba_domain::audit::{AuditOutcome, AuditPhase, GENESIS_HASH};
    use boba_domain::source::OptInRecord;

    use crate::applications::testing::FakeApplyRepo;
    use crate::ports::repo::apply::ApplyRepo;
    use crate::ports::repo::system::NewIntent;
    use crate::stats::testing::OptInSources;
    use crate::system::testing::FakeSystemRepo;

    const NOW: &str = "2026-10-17T08:00:00.000Z";

    #[derive(Default)]
    struct FakeHold(Vec<OutboxItem>);

    impl OutboxHoldRepo for FakeHold {
        fn outbox_insert(
            &self,
            _: &NewOutboxItem,
            _: &str,
        ) -> Result<OutboxItem, crate::ports::repo::outbox_hold::OutboxHoldRepoError> {
            Err(crate::ports::repo::outbox_hold::OutboxHoldRepoError::new(
                "read-only fake",
            ))
        }

        fn outbox_get(
            &self,
            _: &str,
        ) -> Result<Option<OutboxItem>, crate::ports::repo::outbox_hold::OutboxHoldRepoError>
        {
            Ok(None)
        }

        fn outbox_live_for(
            &self,
            _: &str,
        ) -> Result<Option<OutboxItem>, crate::ports::repo::outbox_hold::OutboxHoldRepoError>
        {
            Ok(None)
        }

        fn outbox_list(
            &self,
            limit: usize,
        ) -> Result<Vec<OutboxItem>, crate::ports::repo::outbox_hold::OutboxHoldRepoError> {
            Ok(self.0.iter().take(limit).cloned().collect())
        }

        fn outbox_due(
            &self,
            _: &str,
        ) -> Result<Vec<OutboxItem>, crate::ports::repo::outbox_hold::OutboxHoldRepoError> {
            Ok(Vec::new())
        }

        fn outbox_releasing(
            &self,
        ) -> Result<Vec<OutboxItem>, crate::ports::repo::outbox_hold::OutboxHoldRepoError> {
            Ok(Vec::new())
        }

        fn outbox_claim(
            &self,
            _: &str,
            _: &str,
        ) -> Result<bool, crate::ports::repo::outbox_hold::OutboxHoldRepoError> {
            Ok(false)
        }

        fn outbox_set_state(
            &self,
            _: &str,
            _: OutboxState,
            _: Option<&str>,
            _: &str,
        ) -> Result<OutboxItem, crate::ports::repo::outbox_hold::OutboxHoldRepoError> {
            Err(crate::ports::repo::outbox_hold::OutboxHoldRepoError::new(
                "read-only fake",
            ))
        }

        fn outbox_set_overrides(
            &self,
            _: &str,
            _: Option<&str>,
            _: Option<&str>,
            _: &str,
        ) -> Result<OutboxItem, crate::ports::repo::outbox_hold::OutboxHoldRepoError> {
            Err(crate::ports::repo::outbox_hold::OutboxHoldRepoError::new(
                "read-only fake",
            ))
        }

        fn dry_run_get(
            &self,
            _: &str,
        ) -> Result<
            Option<boba_domain::apply::outbox::DryRunState>,
            crate::ports::repo::outbox_hold::OutboxHoldRepoError,
        > {
            Ok(None)
        }

        fn dry_run_list(
            &self,
        ) -> Result<
            Vec<boba_domain::apply::outbox::DryRunState>,
            crate::ports::repo::outbox_hold::OutboxHoldRepoError,
        > {
            Ok(Vec::new())
        }

        fn dry_run_upsert(
            &self,
            _: &boba_domain::apply::outbox::DryRunState,
            _: &str,
        ) -> Result<(), crate::ports::repo::outbox_hold::OutboxHoldRepoError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeRules(Vec<boba_domain::apply::policy::AutoRule>);

    impl AutoRulesRepo for FakeRules {
        fn rules(
            &self,
        ) -> Result<
            Vec<boba_domain::apply::policy::AutoRule>,
            crate::ports::repo::auto_rules::AutoRulesRepoError,
        > {
            Ok(self.0.clone())
        }

        fn rule_upsert(
            &self,
            _: &boba_domain::apply::policy::AutoRule,
        ) -> Result<(), crate::ports::repo::auto_rules::AutoRulesRepoError> {
            Ok(())
        }

        fn rule_delete(
            &self,
            _: &str,
        ) -> Result<bool, crate::ports::repo::auto_rules::AutoRulesRepoError> {
            Ok(false)
        }

        fn ledger_insert(
            &self,
            _: &AutoAttemptEntry,
        ) -> Result<(), crate::ports::repo::auto_rules::AutoRulesRepoError> {
            Ok(())
        }

        fn ledger_global_since(
            &self,
            _: &str,
        ) -> Result<u32, crate::ports::repo::auto_rules::AutoRulesRepoError> {
            Ok(0)
        }

        fn ledger_rule_since(
            &self,
            _: &str,
            _: &str,
        ) -> Result<u32, crate::ports::repo::auto_rules::AutoRulesRepoError> {
            Ok(0)
        }

        fn ledger_company_since(
            &self,
            _: &str,
            _: &str,
        ) -> Result<u32, crate::ports::repo::auto_rules::AutoRulesRepoError> {
            Ok(0)
        }

        fn ledger_last_at(
            &self,
        ) -> Result<Option<String>, crate::ports::repo::auto_rules::AutoRulesRepoError> {
            Ok(None)
        }

        fn ledger_last_at_for_source(
            &self,
            _: &str,
        ) -> Result<Option<String>, crate::ports::repo::auto_rules::AutoRulesRepoError> {
            Ok(None)
        }
    }

    fn intent(action: &str, at: &str) -> NewIntent {
        NewIntent {
            action: action.to_string(),
            correlation_id: "att_1".to_string(),
            actor: "user".to_string(),
            target_kind: "application".to_string(),
            target_id: Some("app_1".to_string()),
            detail_json: "{\"rule\":\"Email x Remotive\"}".to_string(),
            payload_blake3: Some("abc123".to_string()),
            created_at: at.to_string(),
        }
    }

    fn application(id: &str, posting_id: &str, status: AppStatus) -> Application {
        Application {
            id: id.to_string(),
            posting_id: posting_id.to_string(),
            status,
            offer_snapshot_json: "{}".to_string(),
            notes: None,
            next_action: None,
            created_at: NOW.to_string(),
            updated_at: NOW.to_string(),
        }
    }

    fn service(
        system: FakeSystemRepo,
        apply: FakeApplyRepo,
        hold: FakeHold,
        rules: FakeRules,
        sources: OptInSources,
    ) -> AuditLogService {
        AuditLogService::new(
            Arc::new(system),
            Arc::new(apply),
            Arc::new(hold),
            Arc::new(rules),
            Arc::new(sources),
        )
    }

    // AC-1: a send appears with its Package hash and the Application to link to.
    #[test]
    fn a_send_is_listed_with_its_package_hash_and_application() {
        let system = FakeSystemRepo::default();
        system
            .audit_append_intent(&intent("apply.submitted", NOW))
            .unwrap();

        let view = service(
            system,
            FakeApplyRepo::default(),
            FakeHold::default(),
            FakeRules::default(),
            OptInSources::default(),
        )
        .entries()
        .unwrap();

        assert_eq!(view.entries.len(), 1);
        let entry = &view.entries[0];
        assert_eq!(entry.kind, "send");
        assert_eq!(entry.application_id.as_deref(), Some("app_1"));
        assert_eq!(entry.payload_blake3.as_deref(), Some("abc123"));
        assert!(entry.hash.is_some());
        assert!(view.chain_intact);
    }

    // AC-1: a Kill Switch event is its own kind.
    #[test]
    fn kill_switch_events_are_classified() {
        let system = FakeSystemRepo::default();
        system
            .audit_append_intent(&intent("apply.kill_switch", NOW))
            .unwrap();

        let view = service(
            system,
            FakeApplyRepo::default(),
            FakeHold::default(),
            FakeRules::default(),
            OptInSources::default(),
        )
        .entries()
        .unwrap();
        assert_eq!(view.entries[0].kind, "kill_switch");
    }

    // AC-1: a Stage change comes from the Tracker's history, with from → to.
    #[test]
    fn stage_changes_are_merged_from_the_history() {
        let apply = FakeApplyRepo::default();
        apply.seed_application(application("app_1", "pst_1", AppStatus::Applied));
        apply
            .append_status_event(&StatusEvent {
                id: new_status_event_id(),
                application_id: "app_1".to_string(),
                from_status: None,
                to_status: AppStatus::Applied,
                source: StatusSource::ApplyOutcome,
                reason: None,
                created_at: "2026-10-17T09:00:00.000Z".to_string(),
            })
            .unwrap();

        let view = service(
            FakeSystemRepo::default(),
            apply,
            FakeHold::default(),
            FakeRules::default(),
            OptInSources::default(),
        )
        .entries()
        .unwrap();

        let entry = view
            .entries
            .iter()
            .find(|entry| entry.kind == "stage_change")
            .expect("a stage change entry");
        assert_eq!(entry.action, "stage.applied");
        assert_eq!(entry.actor, "apply_outcome");
        assert!(entry.detail_json.contains("\"to\":\"applied\""));
    }

    // AC-1: an Auto decision carries the rule and the reason it left the
    // automatic path.
    #[test]
    fn auto_decisions_carry_the_rule_and_reason() {
        let item = OutboxItem {
            id: "obx_1".to_string(),
            application_id: "app_1".to_string(),
            rule_id: "rule_1".to_string(),
            source_id: "src_1".to_string(),
            channel: "email".to_string(),
            company_key: "nimbus".to_string(),
            title: "Backend Engineer".to_string(),
            company: "Nimbus".to_string(),
            state: OutboxState::NeedsReview,
            hold_minutes: 30,
            release_at: NOW.to_string(),
            subject_override: None,
            body_override: None,
            note: Some("Rank 61 < 80".to_string()),
            created_at: "2026-10-17T10:00:00.000Z".to_string(),
            updated_at: NOW.to_string(),
        };
        let view = service(
            FakeSystemRepo::default(),
            FakeApplyRepo::default(),
            FakeHold(vec![item]),
            FakeRules(vec![rule("rule_1", "Email x Remotive")]),
            OptInSources::default(),
        )
        .entries()
        .unwrap();

        let entry = &view.entries[0];
        assert_eq!(entry.kind, "decision");
        assert_eq!(entry.outcome.as_deref(), Some("needs_review"));
        assert!(entry.detail_json.contains("Email x Remotive"));
        assert!(entry.detail_json.contains("Rank 61 < 80"));
    }

    // AC-1: an opt-in is listed with the warning version the user saw.
    #[test]
    fn opt_ins_show_the_warning_version() {
        let sources = OptInSources::default();
        sources.upsert_source(&source("src_1", "LinkedIn")).unwrap();
        sources
            .record_optin(&OptInRecord {
                source_id: "src_1".to_string(),
                warning_version: "v3".to_string(),
                accepted_at: "2026-10-17T07:00:00.000Z".to_string(),
            })
            .unwrap();

        let view = service(
            FakeSystemRepo::default(),
            FakeApplyRepo::default(),
            FakeHold::default(),
            FakeRules::default(),
            sources,
        )
        .entries()
        .unwrap();

        let entry = &view.entries[0];
        assert_eq!(entry.kind, "optin");
        assert_eq!(entry.at, "2026-10-17T07:00:00.000Z");
        assert!(entry.detail_json.contains("\"warning_version\":\"v3\""));
    }

    // AC-1: newest first, whatever the source.
    #[test]
    fn entries_are_newest_first() {
        let system = FakeSystemRepo::default();
        system
            .audit_append_intent(&intent("apply.submitted", "2026-10-17T06:00:00.000Z"))
            .unwrap();
        let apply = FakeApplyRepo::default();
        apply.seed_application(application("app_1", "pst_1", AppStatus::Applied));
        apply
            .append_status_event(&StatusEvent {
                id: new_status_event_id(),
                application_id: "app_1".to_string(),
                from_status: None,
                to_status: AppStatus::Applied,
                source: StatusSource::User,
                reason: None,
                created_at: "2026-10-17T09:00:00.000Z".to_string(),
            })
            .unwrap();

        let view = service(
            system,
            apply,
            FakeHold::default(),
            FakeRules::default(),
            OptInSources::default(),
        )
        .entries()
        .unwrap();
        assert_eq!(view.entries[0].kind, "stage_change");
        assert_eq!(view.entries[1].kind, "send");
    }

    // AC-1: a tampered chain is reported, and the rows are still listed.
    #[test]
    fn a_broken_chain_is_reported_with_the_rows_still_listed() {
        let system = FakeSystemRepo::default();
        system
            .audit_append_intent(&intent("apply.submitted", NOW))
            .unwrap();
        system.corrupt_last();

        let view = service(
            system,
            FakeApplyRepo::default(),
            FakeHold::default(),
            FakeRules::default(),
            OptInSources::default(),
        )
        .entries()
        .unwrap();
        assert!(!view.chain_intact);
        assert_eq!(view.broken_at, Some(1));
        assert_eq!(view.entries.len(), 1);
    }

    // AC-1: the CSV quotes every field and doubles inner quotes (RFC 4180).
    #[test]
    fn csv_escapes_commas_and_quotes() {
        let entry = AuditEntry {
            id: "row".to_string(),
            kind: "send".to_string(),
            at: NOW.to_string(),
            action: "apply.submitted".to_string(),
            actor: "user".to_string(),
            phase: Some(AuditPhase::Intent.as_str().to_string()),
            outcome: Some(AuditOutcome::Ok.as_str().to_string()),
            target_kind: "application".to_string(),
            target_id: Some("app_1".to_string()),
            application_id: Some("app_1".to_string()),
            detail_json: "{\"note\":\"a, quoted reason\"}".to_string(),
            payload_blake3: Some("abc".to_string()),
            hash: Some(GENESIS_HASH.to_string()),
        };
        let csv = to_csv(&[entry]);
        let line = csv.lines().nth(1).expect("a data row");
        assert!(line.contains("\"{\"\"note\"\":\"\"a, quoted reason\"\"}\""));
        assert!(line.starts_with(&format!("\"{NOW}\",\"apply.submitted\"")));
    }

    fn source(id: &str, name: &str) -> boba_domain::source::SourceRecord {
        boba_domain::source::SourceRecord {
            id: id.to_string(),
            name: name.to_string(),
            access_class: boba_domain::source::AccessClass::SessionBrowser,
            legal_tier: boba_domain::source::LegalTier::ProhibitedAutomation,
            apply_capability: boba_domain::source::ApplyCapability::SessionForm,
            attribution: None,
            min_interval_ms: 1_000,
            daily_cap: None,
            enabled: true,
            added_at: NOW.to_string(),
            updated_at: NOW.to_string(),
        }
    }

    // M-2: a spreadsheet must never run a cell from a Source name or a note.
    #[test]
    fn csv_neutralises_formula_prefixes() {
        for value in ["=HYPERLINK(\"x\")", "+1+1", "-2", "@cmd", "\tx", "\rx"] {
            let field = csv_field(value);
            assert!(field.starts_with("\"'"), "{value} → {field}");
        }
        assert_eq!(csv_field("plain"), "\"plain\"");
        // A quote inside a guarded field is still doubled.
        assert_eq!(csv_field("=A\"B"), "\"'=A\"\"B\"");
    }

    fn rule(id: &str, name: &str) -> boba_domain::apply::policy::AutoRule {
        boba_domain::apply::policy::AutoRule::defaults(id, name, NOW)
    }
}
