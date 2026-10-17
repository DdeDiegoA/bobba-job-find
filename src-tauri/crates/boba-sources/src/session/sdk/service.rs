//! The Session Board opt-in use case (S11.1a, FR-30, AD-9, AD-12).
//!
//! Everything the UI and a future adapter can do with a Session Board Source
//! goes through here, so the core — never the webview — decides what is allowed:
//!
//! * `enable` refuses a stale warning version, and on Linux without a keyring it
//!   refuses to continue without the **separate** weak-cookie acknowledgement;
//!   both the opt-in row and the audit pair are written by the core;
//! * `disable` removes the opt-in, so the registry's gate closes;
//! * `sign_out` deletes that Source's browser profile and cookies;
//! * `report_challenge` pauses the Source and records why — the notification is
//!   the caller's to raise, the pause itself lives in the core;
//! * `resume` clears a challenge pause once the user has acted.

use std::sync::Arc;

use boba_app::ports::clock::ClockPort;
use boba_app::ports::notifier::{Notification, NotifierPort};
use boba_app::ports::repo::sources::{SourcesRepo, SourcesRepoError};
use boba_app::ports::repo::system::{NewIntent, SystemRepo, SystemRepoError};
use boba_app::ports::session::SessionStore;
use boba_domain::audit::AuditOutcome;
use boba_domain::session::{ClearReport, CookieProtection, SessionError};
use boba_domain::source::{AccessClass, OptInRecord, SourceRecord};
use serde::{Deserialize, Serialize};

use super::{ChallengeKind, PauseRecord, WARNING_VERSION, loads_key, pause_key, weak_ack_key};

/// The audit action the per-Source opt-in writes (AD-12: a settings change that
/// enables automation is audited).
pub const OPT_IN_ACTION: &str = "settings.session.opt_in";
/// The audit action "Sign out / clear session" writes.
pub const SIGN_OUT_ACTION: &str = "session.signed_out";
/// The audit action a detected challenge writes.
pub const CHALLENGE_ACTION: &str = "session.challenge_paused";
/// The audit action clearing a challenge pause writes.
pub const RESUME_ACTION: &str = "session.resumed";
/// The audit action one page load of a session run writes (S11.1b).
pub const PAGE_LOAD_ACTION: &str = "session.run";

/// Why a Session Board action was refused.
#[derive(Debug, thiserror::Error)]
pub enum SessionBoardError {
    /// The id names no configured source.
    #[error("unknown source '{0}'")]
    UnknownSource(String),
    /// The source exists but is not browsed in the user's session.
    #[error("source '{0}' is not a Session Board Source")]
    NotSessionBoard(String),
    /// The UI showed a warning copy the core no longer accepts.
    #[error("the warning changed; re-read version {current}")]
    WarningChanged { current: String },
    /// Linux without a keyring: the weaker cookie protection needs its own ack.
    #[error("the weaker cookie protection needs its own acknowledgement")]
    WeakAckRequired,
    #[error(transparent)]
    Sources(#[from] SourcesRepoError),
    #[error(transparent)]
    System(#[from] SystemRepoError),
    #[error(transparent)]
    Sessions(#[from] SessionError),
}

impl SessionBoardError {
    /// The translated message key the UI shows (CH7: `errors.session.*`).
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::UnknownSource(_) => "errors.session.unknown_source",
            Self::NotSessionBoard(_) => "errors.session.not_session_board",
            Self::WarningChanged { .. } => "errors.session.warning_changed",
            Self::WeakAckRequired => "errors.session.weak_ack",
            // A profile a browser still holds open is a state conflict, not a
            // store failure (S11.1b R2).
            Self::Sessions(SessionError::InUse { .. }) => "errors.session.in_use",
            Self::Sources(_) | Self::System(_) | Self::Sessions(_) => "errors.session.store",
        }
    }
}

/// One Session Board Source, as the UI and adapters see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardView {
    pub source_id: String,
    pub name: String,
    /// Whether a run may use the Source (a pause turns this off).
    pub enabled: bool,
    /// Whether a current opt-in row exists.
    pub opted_in: bool,
    /// When the opt-in was accepted (the latest acceptance).
    pub accepted_at: Option<String>,
    /// The warning version that was accepted.
    pub accepted_warning_version: Option<String>,
    /// Set while a challenge keeps the Source paused (FR-30, AD-10).
    pub pause: Option<PauseRecord>,
    /// Whether the weak-cookie acknowledgement covers the current warning.
    pub weak_ack: bool,
}

/// Everything the opt-in and privacy screens need at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSnapshot {
    /// The warning copy version this build accepts.
    pub warning_version: String,
    /// How Chrome protects this platform's cookies (AC2).
    pub protection: CookieProtection,
    /// Every configured Session Board Source, by name.
    pub boards: Vec<BoardView>,
}

/// The page a load opened, without its query or fragment: the audit row names
/// the board page, never the user's search terms or location (S11.1b R-b).
fn page_url(url: &str) -> &str {
    url.split(['?', '#']).next().unwrap_or(url)
}

/// The Session Board opt-in, sign-out and pause use case.
pub struct SessionBoardService<C> {
    sources: Arc<dyn SourcesRepo>,
    system: Arc<dyn SystemRepo>,
    clock: C,
    sessions: Arc<dyn SessionStore>,
    protection: CookieProtection,
    notifier: Option<Arc<dyn NotifierPort>>,
}

impl<C: ClockPort> SessionBoardService<C> {
    pub fn new(
        sources: Arc<dyn SourcesRepo>,
        system: Arc<dyn SystemRepo>,
        clock: C,
        sessions: Arc<dyn SessionStore>,
        protection: CookieProtection,
        notifier: Option<Arc<dyn NotifierPort>>,
    ) -> Self {
        Self {
            sources,
            system,
            clock,
            sessions,
            protection,
            notifier,
        }
    }

    /// Every Session Board Source with its opt-in, pause and acknowledgement.
    pub fn snapshot(&self) -> Result<SessionSnapshot, SessionBoardError> {
        let mut boards = Vec::new();
        for source in self.sources.list_sources()? {
            if source.access_class == AccessClass::SessionBrowser {
                boards.push(self.view(&source)?);
            }
        }
        boards.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(SessionSnapshot {
            warning_version: WARNING_VERSION.to_string(),
            protection: self.protection,
            boards,
        })
    }

    /// Record the per-Source opt-in after the user read `warning_version` (AC1).
    pub fn enable(
        &self,
        source_id: &str,
        warning_version: &str,
        weak_ack: bool,
    ) -> Result<BoardView, SessionBoardError> {
        self.session_source(source_id)?;
        // AC1: the copy the user read must be the one this build ships.
        if warning_version != WARNING_VERSION {
            return Err(SessionBoardError::WarningChanged {
                current: WARNING_VERSION.to_string(),
            });
        }
        // AC2: on Linux without a keyring the weaker cookie protection needs a
        // second, separate acknowledgement — the core refuses without it.
        if self.protection.requires_extra_ack() && !weak_ack {
            return Err(SessionBoardError::WeakAckRequired);
        }
        let detail = serde_json::json!({
            "enabled": true,
            "source_id": source_id,
            "warning_version": WARNING_VERSION,
            "weak_cookie_ack": weak_ack,
            "cookie_protection": self.protection.as_str(),
        });
        self.audited(
            OPT_IN_ACTION,
            source_id,
            detail,
            || {
                let now = self.clock.now_rfc3339();
                self.sources.record_optin(&OptInRecord {
                    source_id: source_id.to_string(),
                    warning_version: WARNING_VERSION.to_string(),
                    accepted_at: now.clone(),
                })?;
                self.sources.set_source_enabled(source_id, true, &now)?;
                if self.protection.requires_extra_ack() {
                    self.system
                        .setting_put(&weak_ack_key(source_id), WARNING_VERSION, &now)?;
                }
                Ok(())
            },
            |_| serde_json::json!({}),
        )?;
        self.view(&self.session_source(source_id)?)
    }

    /// Remove the opt-in and disable the Source; cookies stay until sign-out.
    pub fn disable(&self, source_id: &str) -> Result<BoardView, SessionBoardError> {
        self.session_source(source_id)?;
        let detail = serde_json::json!({ "enabled": false, "source_id": source_id });
        self.audited(
            OPT_IN_ACTION,
            source_id,
            detail,
            || {
                let now = self.clock.now_rfc3339();
                self.sources.set_source_enabled(source_id, false, &now)?;
                self.sources.remove_optin(source_id)?;
                // The acknowledgements only mean something with an opt-in.
                self.system.setting_delete(&weak_ack_key(source_id))?;
                self.system.setting_delete(&pause_key(source_id))?;
                Ok(())
            },
            |_| serde_json::json!({}),
        )?;
        self.view(&self.session_source(source_id)?)
    }

    /// Delete that Source's browser profile and cookies (AC3).
    pub fn sign_out(&self, source_id: &str) -> Result<BoardView, SessionBoardError> {
        self.session_source(source_id)?;
        let detail = serde_json::json!({ "source_id": source_id });
        self.audited(
            SIGN_OUT_ACTION,
            source_id,
            detail,
            || self.clear(source_id),
            |report: &ClearReport| serde_json::json!({ "removed": report.removed }),
        )?;
        self.view(&self.session_source(source_id)?)
    }

    /// Pause the Source after a challenge; the caller notifies the user.
    pub fn report_challenge(
        &self,
        source_id: &str,
        kind: ChallengeKind,
    ) -> Result<BoardView, SessionBoardError> {
        self.session_source(source_id)?;
        let pause = PauseRecord::new(kind, self.clock.now_rfc3339());
        let detail = serde_json::json!({ "source_id": source_id, "kind": kind.as_str() });
        self.audited(
            CHALLENGE_ACTION,
            source_id,
            detail,
            || {
                let now = self.clock.now_rfc3339();
                self.system
                    .setting_put(&pause_key(source_id), &pause.to_json(), &now)?;
                // AD-10: the Source stops; the user acts, never Boba.
                self.sources.set_source_enabled(source_id, false, &now)?;
                Ok(())
            },
            |_| serde_json::json!({}),
        )?;
        // AD-21: a missing toast never undoes the pause, which is already
        // recorded above — the user still sees it in Settings → Privacy.
        if let Some(notifier) = &self.notifier {
            let _ = notifier.notify(Notification::SessionChallengePaused);
        }
        self.view(&self.session_source(source_id)?)
    }

    /// Record one page load of a run: the local-day counter and the audit row
    /// (S11.1b R1, AD-12).
    ///
    /// The counter only ever grows for the day: two overlapping runs each read
    /// it when they opened, so the second write must never lower the total
    /// (S11.1b review R-a). The detail carries the Source and the board page it
    /// opened — never a cookie, a URL query that could hold a token, or a page
    /// body.
    pub fn record_page_load(
        &self,
        source_id: &str,
        url: &str,
        loads_today: u32,
    ) -> Result<(), SessionBoardError> {
        self.session_source(source_id)?;
        let now = self.clock.now_rfc3339();
        let key = loads_key(source_id, &now);
        let stored = self
            .system
            .setting_get(&key)?
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0);
        let total = stored.max(loads_today);
        let detail = serde_json::json!({
            "source_id": source_id,
            // R-b (S11.1b review): search keywords and locations stay out of
            // the audit log; the page itself is what the row is about.
            "url": page_url(url),
            "loads_today": total,
        });
        self.audited(
            PAGE_LOAD_ACTION,
            source_id,
            detail,
            || {
                self.system.setting_put(&key, &total.to_string(), &now)?;
                Ok(())
            },
            |_| serde_json::json!({ "loads_today": total }),
        )?;
        Ok(())
    }

    /// Clear a challenge pause after the user acted on it.
    pub fn resume(&self, source_id: &str) -> Result<BoardView, SessionBoardError> {
        self.session_source(source_id)?;
        let detail = serde_json::json!({ "source_id": source_id });
        self.audited(
            RESUME_ACTION,
            source_id,
            detail,
            || {
                let now = self.clock.now_rfc3339();
                self.system.setting_delete(&pause_key(source_id))?;
                // Only a current opt-in turns the Source back on.
                let opted_in = self.sources.get_optin(source_id)?.is_some();
                self.sources.set_source_enabled(source_id, opted_in, &now)?;
                Ok(())
            },
            |_| serde_json::json!({}),
        )?;
        self.view(&self.session_source(source_id)?)
    }

    /// The newest view of one source (after a mutation).
    fn view(&self, source: &SourceRecord) -> Result<BoardView, SessionBoardError> {
        let optin = self.sources.get_optin(&source.id)?;
        let pause = self
            .system
            .setting_get(&pause_key(&source.id))?
            .and_then(|value| PauseRecord::parse(&value));
        let weak_ack = self
            .system
            .setting_get(&weak_ack_key(&source.id))?
            .as_deref()
            == Some(WARNING_VERSION);
        Ok(BoardView {
            source_id: source.id.clone(),
            name: source.name.clone(),
            enabled: source.enabled,
            opted_in: optin.is_some(),
            accepted_at: optin.as_ref().map(|row| row.accepted_at.clone()),
            accepted_warning_version: optin.as_ref().map(|row| row.warning_version.clone()),
            pause,
            weak_ack,
        })
    }

    /// Load `source_id`, refusing an id that is not a Session Board Source.
    fn session_source(&self, source_id: &str) -> Result<SourceRecord, SessionBoardError> {
        let source = self
            .sources
            .get_source(source_id)?
            .ok_or_else(|| SessionBoardError::UnknownSource(source_id.to_string()))?;
        if source.access_class != AccessClass::SessionBrowser {
            return Err(SessionBoardError::NotSessionBoard(source_id.to_string()));
        }
        Ok(source)
    }

    /// Append the `intent` row, run `action`, then complete it with its outcome.
    fn audited<T>(
        &self,
        action: &str,
        source_id: &str,
        detail: serde_json::Value,
        run: impl FnOnce() -> Result<T, SessionBoardError>,
        describe: impl FnOnce(&T) -> serde_json::Value,
    ) -> Result<T, SessionBoardError> {
        let now = self.clock.now_rfc3339();
        let intent = self.system.audit_append_intent(&NewIntent {
            action: action.to_string(),
            correlation_id: format!("{action}_{source_id}_{now}"),
            actor: "user".to_string(),
            target_kind: "source".to_string(),
            target_id: Some(source_id.to_string()),
            detail_json: detail.to_string(),
            payload_blake3: None,
            created_at: now.clone(),
        })?;
        match run() {
            Ok(value) => {
                let outcome = describe(&value);
                self.system.audit_append_outcome(
                    &intent,
                    AuditOutcome::Ok,
                    &outcome.to_string(),
                    &now,
                )?;
                Ok(value)
            }
            Err(error) => {
                let outcome = serde_json::json!({ "failed": error.to_string() });
                let _ = self.system.audit_append_outcome(
                    &intent,
                    AuditOutcome::Failed,
                    &outcome.to_string(),
                    &now,
                );
                Err(error)
            }
        }
    }

    /// The signed-out report the audit outcome records.
    fn clear(&self, source_id: &str) -> Result<ClearReport, SessionBoardError> {
        Ok(self.sessions.clear(source_id)?)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use boba_app::ports::notifier::NotifierError;
    use boba_app::ports::repo::system::SettingRecord;
    use boba_app::system::FixedClock;
    use boba_domain::audit::{AuditPhase, AuditRow, ChainStatus, GENESIS_HASH};
    use boba_domain::source::{ApplyCapability, LegalTier, RunStatus, SourceRunRecord};

    use super::*;
    use crate::session::sdk::DEFAULT_PACING;

    const NOW: &str = "2026-10-17T06:00:00.000Z";

    fn source(id: &str, class: AccessClass) -> SourceRecord {
        SourceRecord {
            id: id.to_string(),
            name: id.to_uppercase(),
            access_class: class,
            legal_tier: LegalTier::ProhibitedAutomation,
            apply_capability: ApplyCapability::SessionForm,
            attribution: None,
            min_interval_ms: DEFAULT_PACING.min_interval_ms,
            daily_cap: Some(120),
            enabled: true,
            added_at: NOW.to_string(),
            updated_at: NOW.to_string(),
        }
    }

    #[derive(Default)]
    struct FakeSources {
        sources: Mutex<BTreeMap<String, SourceRecord>>,
        optins: Mutex<BTreeMap<String, OptInRecord>>,
    }

    impl FakeSources {
        fn with(mut self, record: SourceRecord) -> Self {
            self.sources
                .get_mut()
                .unwrap()
                .insert(record.id.clone(), record);
            self
        }
    }

    impl SourcesRepo for FakeSources {
        fn upsert_source(&self, record: &SourceRecord) -> Result<(), SourcesRepoError> {
            self.sources
                .lock()
                .unwrap()
                .insert(record.id.clone(), record.clone());
            Ok(())
        }

        fn get_source(&self, id: &str) -> Result<Option<SourceRecord>, SourcesRepoError> {
            Ok(self.sources.lock().unwrap().get(id).cloned())
        }

        fn list_sources(&self) -> Result<Vec<SourceRecord>, SourcesRepoError> {
            Ok(self.sources.lock().unwrap().values().cloned().collect())
        }

        fn set_source_enabled(
            &self,
            id: &str,
            enabled: bool,
            updated_at: &str,
        ) -> Result<(), SourcesRepoError> {
            if let Some(record) = self.sources.lock().unwrap().get_mut(id) {
                record.enabled = enabled;
                record.updated_at = updated_at.to_string();
            }
            Ok(())
        }

        fn record_optin(&self, optin: &OptInRecord) -> Result<(), SourcesRepoError> {
            self.optins
                .lock()
                .unwrap()
                .insert(optin.source_id.clone(), optin.clone());
            Ok(())
        }

        fn get_optin(&self, source_id: &str) -> Result<Option<OptInRecord>, SourcesRepoError> {
            Ok(self.optins.lock().unwrap().get(source_id).cloned())
        }

        fn remove_optin(&self, source_id: &str) -> Result<(), SourcesRepoError> {
            self.optins.lock().unwrap().remove(source_id);
            Ok(())
        }

        fn opted_in_ids(&self) -> Result<Vec<String>, SourcesRepoError> {
            Ok(self.optins.lock().unwrap().keys().cloned().collect())
        }

        fn start_run(&self, _run: &SourceRunRecord) -> Result<(), SourcesRepoError> {
            Ok(())
        }

        fn recent_runs(
            &self,
            _source_id: &str,
            _limit: usize,
        ) -> Result<Vec<SourceRunRecord>, SourcesRepoError> {
            Ok(Vec::new())
        }

        fn finish_run(
            &self,
            _id: &str,
            _finished_at: &str,
            _status: RunStatus,
            _postings_found: i64,
            _error: Option<&str>,
        ) -> Result<(), SourcesRepoError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeSystem {
        settings: Mutex<BTreeMap<String, String>>,
        rows: Mutex<Vec<AuditRow>>,
    }

    impl FakeSystem {
        fn actions(&self) -> Vec<String> {
            self.rows
                .lock()
                .unwrap()
                .iter()
                .map(|row| row.action.clone())
                .collect()
        }

        fn detail(&self, action: &str) -> String {
            self.rows
                .lock()
                .unwrap()
                .iter()
                .find(|row| row.action == action)
                .map(|row| row.detail_json.clone())
                .unwrap_or_default()
        }

        fn setting(&self, key: &str) -> Option<String> {
            self.settings.lock().unwrap().get(key).cloned()
        }

        fn append(&self, mut row: AuditRow) -> AuditRow {
            let mut rows = self.rows.lock().unwrap();
            row.seq = rows.len() as i64 + 1;
            row.prev_hash = rows
                .last()
                .map(|prev| prev.hash.clone())
                .unwrap_or_else(|| GENESIS_HASH.to_string());
            row.hash = format!("hash-{}", row.seq);
            rows.push(row.clone());
            row
        }
    }

    impl SystemRepo for FakeSystem {
        fn setting_get(&self, key: &str) -> Result<Option<String>, SystemRepoError> {
            Ok(self.settings.lock().unwrap().get(key).cloned())
        }

        fn setting_put(
            &self,
            key: &str,
            value: &str,
            _updated_at: &str,
        ) -> Result<(), SystemRepoError> {
            self.settings
                .lock()
                .unwrap()
                .insert(key.to_string(), value.to_string());
            Ok(())
        }

        fn setting_list(&self) -> Result<Vec<SettingRecord>, SystemRepoError> {
            Ok(self
                .settings
                .lock()
                .unwrap()
                .iter()
                .map(|(key, value)| SettingRecord {
                    key: key.clone(),
                    value: value.clone(),
                    updated_at: NOW.to_string(),
                })
                .collect())
        }

        fn setting_delete(&self, key: &str) -> Result<bool, SystemRepoError> {
            Ok(self.settings.lock().unwrap().remove(key).is_some())
        }

        fn audit_append_intent(&self, intent: &NewIntent) -> Result<AuditRow, SystemRepoError> {
            Ok(self.append(AuditRow {
                seq: 0,
                id: format!("intent-{}", self.rows.lock().unwrap().len() + 1),
                created_at: intent.created_at.clone(),
                phase: AuditPhase::Intent,
                action: intent.action.clone(),
                outcome: None,
                correlation_id: intent.correlation_id.clone(),
                intent_id: None,
                actor: intent.actor.clone(),
                target_kind: intent.target_kind.clone(),
                target_id: intent.target_id.clone(),
                detail_json: intent.detail_json.clone(),
                payload_blake3: intent.payload_blake3.clone(),
                prev_hash: String::new(),
                hash: String::new(),
            }))
        }

        fn audit_append_outcome(
            &self,
            intent: &AuditRow,
            outcome: AuditOutcome,
            detail_json: &str,
            created_at: &str,
        ) -> Result<AuditRow, SystemRepoError> {
            Ok(self.append(AuditRow {
                seq: 0,
                id: format!("outcome-{}", self.rows.lock().unwrap().len() + 1),
                created_at: created_at.to_string(),
                phase: AuditPhase::Outcome,
                action: intent.action.clone(),
                outcome: Some(outcome),
                correlation_id: intent.correlation_id.clone(),
                intent_id: Some(intent.id.clone()),
                actor: intent.actor.clone(),
                target_kind: intent.target_kind.clone(),
                target_id: intent.target_id.clone(),
                detail_json: detail_json.to_string(),
                payload_blake3: None,
                prev_hash: String::new(),
                hash: String::new(),
            }))
        }

        fn audit_head_hash(&self) -> Result<Option<String>, SystemRepoError> {
            Ok(self.rows.lock().unwrap().last().map(|row| row.hash.clone()))
        }

        fn audit_verify_chain(&self) -> Result<ChainStatus, SystemRepoError> {
            Ok(ChainStatus {
                rows: 0,
                head: None,
                broken_at: None,
            })
        }

        fn audit_unresolved_intents(&self) -> Result<Vec<AuditRow>, SystemRepoError> {
            Ok(Vec::new())
        }

        fn audit_anchor_get(&self) -> Result<Option<String>, SystemRepoError> {
            Ok(None)
        }

        fn audit_anchor_is_known(&self, _hash: &str) -> Result<bool, SystemRepoError> {
            Ok(false)
        }

        fn audit_anchor_set(&self, _hash: &str) -> Result<(), SystemRepoError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeNotifier {
        calls: Mutex<Vec<Notification>>,
    }

    impl NotifierPort for FakeNotifier {
        fn notify(&self, notification: Notification) -> Result<(), NotifierError> {
            self.calls.lock().unwrap().push(notification);
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeStore {
        cleared: Mutex<Vec<String>>,
    }

    impl SessionStore for FakeStore {
        fn clear(&self, source_id: &str) -> Result<ClearReport, SessionError> {
            self.cleared.lock().unwrap().push(source_id.to_string());
            Ok(ClearReport {
                source_id: source_id.to_string(),
                removed: true,
            })
        }
    }

    fn boards() -> FakeSources {
        FakeSources::default()
            .with(source("linkedin", AccessClass::SessionBrowser))
            .with(source("indeed", AccessClass::SessionBrowser))
            .with(source("remotive", AccessClass::PublicJson))
    }

    /// The service plus the handles its fakes are inspected through.
    struct Harness {
        service: SessionBoardService<FixedClock>,
        sources: Arc<FakeSources>,
        system: Arc<FakeSystem>,
        store: Arc<FakeStore>,
    }

    fn harness(protection: CookieProtection) -> Harness {
        let sources = Arc::new(boards());
        let system = Arc::new(FakeSystem::default());
        let store = Arc::new(FakeStore::default());
        let service = SessionBoardService::new(
            sources.clone(),
            system.clone(),
            FixedClock::new(NOW),
            store.clone(),
            protection,
            None,
        );
        Harness {
            service,
            sources,
            system,
            store,
        }
    }

    #[test]
    fn only_session_board_sources_are_listed() {
        let h = harness(CookieProtection::MacosKeychain);
        let snapshot = h.service.snapshot().unwrap();
        assert_eq!(
            snapshot
                .boards
                .iter()
                .map(|board| board.source_id.as_str())
                .collect::<Vec<_>>(),
            vec!["indeed", "linkedin"]
        );
        assert_eq!(snapshot.warning_version, WARNING_VERSION);
        assert_eq!(snapshot.protection, CookieProtection::MacosKeychain);
        assert!(snapshot.boards.iter().all(|board| !board.opted_in));
    }

    #[test]
    fn enabling_records_the_optin_with_its_timestamp_and_version_and_audits_it() {
        let h = harness(CookieProtection::MacosKeychain);
        let view = h
            .service
            .enable("linkedin", WARNING_VERSION, false)
            .unwrap();

        assert!(view.opted_in);
        assert!(view.enabled);
        assert_eq!(view.accepted_at.as_deref(), Some(NOW));
        assert_eq!(
            view.accepted_warning_version.as_deref(),
            Some(WARNING_VERSION)
        );

        let stored = h.sources.get_optin("linkedin").unwrap().unwrap();
        assert_eq!(stored.warning_version, WARNING_VERSION);
        assert_eq!(stored.accepted_at, NOW);
        assert!(h.sources.get_source("linkedin").unwrap().unwrap().enabled);

        let actions = h.system.actions();
        assert_eq!(actions, vec![OPT_IN_ACTION, OPT_IN_ACTION]);
        assert!(h.system.detail(OPT_IN_ACTION).contains("linkedin"));
        assert!(h.system.detail(OPT_IN_ACTION).contains(WARNING_VERSION));
    }

    #[test]
    fn a_page_load_is_audited_without_the_query() {
        let h = harness(CookieProtection::MacosKeychain);
        h.service
            .record_page_load(
                "linkedin",
                "https://www.linkedin.com/jobs/search/?keywords=rust&location=Madrid",
                3,
            )
            .unwrap();
        let detail = h.system.detail(PAGE_LOAD_ACTION);
        assert!(
            detail.contains("https://www.linkedin.com/jobs/search/"),
            "{detail}"
        );
        assert!(!detail.contains("keywords"), "{detail}");
        assert!(!detail.contains("Madrid"), "{detail}");
        assert!(detail.contains("\"loads_today\":3"), "{detail}");
        assert_eq!(
            h.system.setting_get(&loads_key("linkedin", NOW)).unwrap(),
            Some("3".to_string()),
            "the day's counter is what the next run reads"
        );
    }

    #[test]
    fn an_overlapping_run_never_lowers_the_days_total() {
        // R-a (S11.1b review): both runs read the counter when they opened; a
        // stale total must not overwrite the newer one.
        let h = harness(CookieProtection::MacosKeychain);
        let key = loads_key("linkedin", NOW);
        h.system
            .setting_put(&key, "5", NOW)
            .expect("the day already spent 5");
        h.service
            .record_page_load("linkedin", "https://www.linkedin.com/jobs/search/", 3)
            .unwrap();
        assert_eq!(
            h.system.setting_get(&key).unwrap(),
            Some("5".to_string()),
            "the stale write did not lower the day's total"
        );
    }

    #[test]
    fn a_page_load_for_a_source_that_is_not_a_board_is_refused() {
        let h = harness(CookieProtection::MacosKeychain);
        assert!(matches!(
            h.service
                .record_page_load("remotive", "https://remotive.com/api/remote-jobs", 1),
            Err(SessionBoardError::NotSessionBoard(_))
        ));
        assert!(matches!(
            h.service
                .record_page_load("telepathy", "https://example.com/", 1),
            Err(SessionBoardError::UnknownSource(_))
        ));
    }

    #[test]
    fn a_stale_warning_version_is_refused_and_changes_nothing() {
        let h = harness(CookieProtection::MacosKeychain);
        let error = h.service.enable("linkedin", "v0", false).unwrap_err();
        assert!(matches!(
            error,
            SessionBoardError::WarningChanged { current } if current == WARNING_VERSION
        ));
        assert!(h.sources.get_optin("linkedin").unwrap().is_none());
        assert!(h.system.actions().is_empty());
    }

    #[test]
    fn a_source_that_is_not_a_session_board_cannot_be_enabled() {
        let h = harness(CookieProtection::MacosKeychain);
        assert!(matches!(
            h.service.enable("remotive", WARNING_VERSION, false),
            Err(SessionBoardError::NotSessionBoard(_))
        ));
        assert!(matches!(
            h.service.enable("telepathy", WARNING_VERSION, false),
            Err(SessionBoardError::UnknownSource(_))
        ));
    }

    #[test]
    fn linux_without_a_keyring_needs_the_separate_acknowledgement() {
        let h = harness(CookieProtection::LinuxWeak);
        assert!(matches!(
            h.service.enable("linkedin", WARNING_VERSION, false),
            Err(SessionBoardError::WeakAckRequired)
        ));
        assert!(h.sources.get_optin("linkedin").unwrap().is_none());

        let view = h.service.enable("linkedin", WARNING_VERSION, true).unwrap();
        assert!(view.weak_ack);
        assert_eq!(
            h.system.setting(&weak_ack_key("linkedin")),
            Some(WARNING_VERSION.to_string())
        );
        let detail = h.system.detail(OPT_IN_ACTION);
        assert!(
            detail.contains("weak_cookie_ack"),
            "the extra ack is audited: {detail}"
        );
    }

    #[test]
    fn a_keyring_platform_needs_no_extra_acknowledgement() {
        let h = harness(CookieProtection::MacosKeychain);
        let view = h
            .service
            .enable("linkedin", WARNING_VERSION, false)
            .unwrap();
        assert!(!view.weak_ack);
        assert_eq!(h.system.setting(&weak_ack_key("linkedin")), None);
    }

    #[test]
    fn disabling_removes_the_optin_and_its_acknowledgements() {
        let h = harness(CookieProtection::LinuxWeak);
        h.service.enable("linkedin", WARNING_VERSION, true).unwrap();
        h.service
            .report_challenge("linkedin", ChallengeKind::Captcha)
            .unwrap();

        let view = h.service.disable("linkedin").unwrap();
        assert!(!view.opted_in);
        assert!(!view.enabled);
        assert!(view.pause.is_none());
        assert!(h.sources.get_optin("linkedin").unwrap().is_none());
        assert_eq!(h.system.setting(&weak_ack_key("linkedin")), None);
        assert_eq!(h.system.setting(&pause_key("linkedin")), None);
    }

    #[test]
    fn signing_out_clears_that_sessions_cookies_and_audits_it() {
        let h = harness(CookieProtection::MacosKeychain);
        h.service
            .enable("linkedin", WARNING_VERSION, false)
            .unwrap();
        h.service.sign_out("linkedin").unwrap();

        assert_eq!(*h.store.cleared.lock().unwrap(), vec!["linkedin"]);
        assert!(h.system.actions().contains(&SIGN_OUT_ACTION.to_string()));
        // Cookies are kept until sign-out or a reset; the opt-in survives it.
        assert!(
            h.service
                .snapshot()
                .unwrap()
                .boards
                .iter()
                .any(|board| board.source_id == "linkedin" && board.opted_in)
        );
    }

    #[test]
    fn signing_out_of_an_unknown_source_is_refused() {
        let h = harness(CookieProtection::MacosKeychain);
        assert!(matches!(
            h.service.sign_out("telepathy"),
            Err(SessionBoardError::UnknownSource(_))
        ));
        assert!(h.system.actions().is_empty());
    }

    #[test]
    fn a_challenge_pauses_the_source_and_records_why() {
        let h = harness(CookieProtection::MacosKeychain);
        h.service
            .enable("linkedin", WARNING_VERSION, false)
            .unwrap();
        let view = h
            .service
            .report_challenge("linkedin", ChallengeKind::RateLimited)
            .unwrap();

        assert!(!view.enabled, "a challenge pauses the Source");
        assert_eq!(
            view.pause,
            Some(PauseRecord::new(ChallengeKind::RateLimited, NOW))
        );
        assert!(h.system.actions().contains(&CHALLENGE_ACTION.to_string()));

        // The pause survives a reload: it lives in the core, not the UI.
        let snapshot = h.service.snapshot().unwrap();
        let linkedin = snapshot
            .boards
            .iter()
            .find(|board| board.source_id == "linkedin")
            .unwrap();
        assert_eq!(
            linkedin.pause.as_ref().map(|pause| pause.kind),
            Some(ChallengeKind::RateLimited)
        );
    }

    #[test]
    fn a_challenge_notifies_the_user() {
        let notifier = Arc::new(FakeNotifier::default());
        let port: Arc<dyn NotifierPort> = notifier.clone();
        let service = SessionBoardService::new(
            Arc::new(boards()),
            Arc::new(FakeSystem::default()),
            FixedClock::new(NOW),
            Arc::new(FakeStore::default()),
            CookieProtection::MacosKeychain,
            Some(port),
        );
        service.enable("linkedin", WARNING_VERSION, false).unwrap();
        service
            .report_challenge("linkedin", ChallengeKind::Captcha)
            .unwrap();

        assert_eq!(
            *notifier.calls.lock().unwrap(),
            vec![Notification::SessionChallengePaused],
            "one notice per pause, never per retry"
        );
    }

    #[test]
    fn resuming_clears_the_pause_and_restores_the_optin_state() {
        let h = harness(CookieProtection::MacosKeychain);
        h.service
            .enable("linkedin", WARNING_VERSION, false)
            .unwrap();
        h.service
            .report_challenge("linkedin", ChallengeKind::LoginWall)
            .unwrap();

        let view = h.service.resume("linkedin").unwrap();
        assert!(view.pause.is_none());
        assert!(view.enabled, "the opt-in is still current");
        assert!(view.opted_in);
        assert!(h.system.actions().contains(&RESUME_ACTION.to_string()));
        assert_eq!(h.system.setting(&pause_key("linkedin")), None);
    }

    #[test]
    fn resuming_a_source_without_an_optin_leaves_it_disabled() {
        let h = harness(CookieProtection::MacosKeychain);
        let view = h.service.resume("linkedin").unwrap();
        assert!(!view.enabled);
        assert!(!view.opted_in);
    }
}
