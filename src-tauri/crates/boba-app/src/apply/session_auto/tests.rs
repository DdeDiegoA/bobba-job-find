//! Session Board Auto use cases (S11.3). The store's SQL is tested in
//! `boba-store`; here the decision, the audit pair and the state are.

use std::sync::Arc;
use std::sync::Mutex;

use boba_domain::apply::policy::AutoAttemptEntry;
use boba_domain::apply::policy::AutoRule;
use boba_domain::source::{
    AccessClass, ApplyCapability, LegalTier, OptInRecord, RunStatus, SourceRecord, SourceRunRecord,
};

use super::*;
use crate::ports::repo::auto_rules::AutoRulesRepo;
use crate::system::FixedClock;
use crate::system::testing::FakeSystemRepo;

const NOW: &str = "2026-10-17T09:00:00Z";

/// The in-memory sources the use case needs (source, opt-in, Auto opt-in,
/// pause). The other methods are outside this slice's read set.
#[derive(Default)]
struct FakeSources {
    records: Mutex<Vec<SourceRecord>>,
    optins: Mutex<Vec<OptInRecord>>,
    auto: Mutex<Vec<SessionAutoOptIn>>,
    paused: Mutex<Vec<(String, String)>>,
}

impl FakeSources {
    fn board(source_id: &str, reading_opted_in: bool) -> Self {
        let sources = Self::default();
        sources
            .records
            .lock()
            .unwrap()
            .push(source(source_id, AccessClass::SessionBrowser));
        if reading_opted_in {
            sources.optins.lock().unwrap().push(OptInRecord {
                source_id: source_id.to_string(),
                warning_version: "v1".to_string(),
                accepted_at: NOW.to_string(),
            });
        }
        sources
    }

    fn seed_auto(&self, optin: SessionAutoOptIn) {
        self.auto.lock().unwrap().push(optin);
    }

    fn seed_pause(&self, source_id: &str, until: &str) {
        self.paused
            .lock()
            .unwrap()
            .push((source_id.to_string(), until.to_string()));
    }
}

impl SourcesRepo for FakeSources {
    fn upsert_source(&self, record: &SourceRecord) -> Result<(), SourcesRepoError> {
        self.records.lock().unwrap().push(record.clone());
        Ok(())
    }
    fn get_source(&self, id: &str) -> Result<Option<SourceRecord>, SourcesRepoError> {
        Ok(self
            .records
            .lock()
            .unwrap()
            .iter()
            .find(|row| row.id == id)
            .cloned())
    }
    fn list_sources(&self) -> Result<Vec<SourceRecord>, SourcesRepoError> {
        Ok(self.records.lock().unwrap().clone())
    }
    fn set_source_enabled(
        &self,
        _id: &str,
        _enabled: bool,
        _updated_at: &str,
    ) -> Result<(), SourcesRepoError> {
        Ok(())
    }
    fn record_optin(&self, optin: &OptInRecord) -> Result<(), SourcesRepoError> {
        self.optins.lock().unwrap().push(optin.clone());
        Ok(())
    }
    fn get_optin(&self, source_id: &str) -> Result<Option<OptInRecord>, SourcesRepoError> {
        Ok(self
            .optins
            .lock()
            .unwrap()
            .iter()
            .find(|row| row.source_id == source_id)
            .cloned())
    }
    fn remove_optin(&self, _source_id: &str) -> Result<(), SourcesRepoError> {
        Ok(())
    }
    fn opted_in_ids(&self) -> Result<Vec<String>, SourcesRepoError> {
        Ok(self
            .optins
            .lock()
            .unwrap()
            .iter()
            .map(|row| row.source_id.clone())
            .collect())
    }
    fn auto_opt_in(&self, source_id: &str) -> Result<Option<SessionAutoOptIn>, SourcesRepoError> {
        Ok(self
            .auto
            .lock()
            .unwrap()
            .iter()
            .find(|row| row.source_id == source_id)
            .cloned())
    }
    fn record_auto_opt_in(&self, optin: &SessionAutoOptIn) -> Result<(), SourcesRepoError> {
        let mut rows = self.auto.lock().unwrap();
        rows.retain(|row| row.source_id != optin.source_id);
        rows.push(optin.clone());
        Ok(())
    }
    fn remove_auto_opt_in(&self, source_id: &str) -> Result<(), SourcesRepoError> {
        self.auto
            .lock()
            .unwrap()
            .retain(|row| row.source_id != source_id);
        Ok(())
    }
    fn auto_paused_until(&self, source_id: &str) -> Result<Option<String>, SourcesRepoError> {
        Ok(self
            .paused
            .lock()
            .unwrap()
            .iter()
            .find(|(id, _)| id == source_id)
            .map(|(_, until)| until.clone()))
    }
    fn set_auto_paused_until(
        &self,
        source_id: &str,
        until: Option<&str>,
        _updated_at: &str,
    ) -> Result<(), SourcesRepoError> {
        let mut rows = self.paused.lock().unwrap();
        rows.retain(|(id, _)| id != source_id);
        if let Some(until) = until {
            rows.push((source_id.to_string(), until.to_string()));
        }
        Ok(())
    }
    fn start_run(&self, _run: &SourceRunRecord) -> Result<(), SourcesRepoError> {
        Ok(())
    }
    fn recent_runs(
        &self,
        _source_id: &str,
        _limit: usize,
    ) -> Result<Vec<SourceRunRecord>, SourcesRepoError> {
        Ok(vec![])
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

/// The ledger count the cap reads; every other method is outside this slice.
#[derive(Default)]
struct FakeRules {
    session_today: Mutex<u32>,
}

impl AutoRulesRepo for FakeRules {
    fn rules(&self) -> Result<Vec<AutoRule>, AutoRulesRepoError> {
        Ok(vec![])
    }
    fn rule_upsert(&self, _rule: &AutoRule) -> Result<(), AutoRulesRepoError> {
        Ok(())
    }
    fn rule_delete(&self, _id: &str) -> Result<bool, AutoRulesRepoError> {
        Ok(false)
    }
    fn ledger_insert(&self, _entry: &AutoAttemptEntry) -> Result<(), AutoRulesRepoError> {
        Ok(())
    }
    fn ledger_global_since(&self, _since: &str) -> Result<u32, AutoRulesRepoError> {
        Ok(0)
    }
    fn ledger_rule_since(&self, _rule_id: &str, _since: &str) -> Result<u32, AutoRulesRepoError> {
        Ok(0)
    }
    fn ledger_company_since(
        &self,
        _company_key: &str,
        _since: &str,
    ) -> Result<u32, AutoRulesRepoError> {
        Ok(0)
    }
    fn ledger_last_at(&self) -> Result<Option<String>, AutoRulesRepoError> {
        Ok(None)
    }
    fn ledger_last_at_for_source(
        &self,
        _source_id: &str,
    ) -> Result<Option<String>, AutoRulesRepoError> {
        Ok(None)
    }
    fn ledger_source_since(
        &self,
        _source_id: &str,
        _channel: &str,
        _since: &str,
    ) -> Result<u32, AutoRulesRepoError> {
        Ok(*self.session_today.lock().unwrap())
    }
}

fn source(id: &str, class: AccessClass) -> SourceRecord {
    SourceRecord {
        id: id.to_string(),
        name: "LinkedIn".to_string(),
        access_class: class,
        legal_tier: LegalTier::ProhibitedAutomation,
        apply_capability: ApplyCapability::SessionForm,
        attribution: None,
        min_interval_ms: 30_000,
        daily_cap: Some(60),
        enabled: true,
        added_at: NOW.to_string(),
        updated_at: NOW.to_string(),
    }
}

struct Harness {
    service: SessionAutoService,
    sources: Arc<FakeSources>,
    rules: Arc<FakeRules>,
    system: Arc<FakeSystemRepo>,
}

fn harness(at: &str, sources: FakeSources) -> Harness {
    let sources = Arc::new(sources);
    let rules = Arc::new(FakeRules::default());
    let system = Arc::new(FakeSystemRepo::default());
    let service = SessionAutoService::new(
        sources.clone(),
        rules.clone(),
        system.clone(),
        Arc::new(FixedClock::new(at)),
    );
    Harness {
        service,
        sources,
        rules,
        system,
    }
}

#[test]
fn auto_needs_the_reading_opt_in_and_the_typed_name() {
    // No reading opt-in: the second opt-in cannot exist alone (FR-91 (c)).
    let h = harness(NOW, FakeSources::board("linkedin", false));
    assert!(matches!(
        h.service.opt_in("linkedin", "LinkedIn", "v1"),
        Err(SessionAutoError::ReadingOptInRequired(_))
    ));

    let h = harness(NOW, FakeSources::board("linkedin", true));
    assert_eq!(
        h.service
            .opt_in("linkedin", "Linkedin", "v1")
            .unwrap_err()
            .message_key(),
        "errors.session.auto_name_mismatch",
        "the typed name must be the Source's own"
    );
    assert_eq!(
        h.service
            .opt_in("linkedin", "LinkedIn", "v0")
            .unwrap_err()
            .message_key(),
        "errors.session.auto_warning_changed"
    );
    assert!(
        h.sources.auto_opt_in("linkedin").unwrap().is_none(),
        "a refused opt-in writes nothing"
    );
    assert!(
        h.system.rows().is_empty(),
        "a refused opt-in is not an event"
    );

    let view = h.service.opt_in("linkedin", "  LinkedIn ", "v1").unwrap();
    assert_eq!(view.state, "active");
    assert_eq!(view.confirmed_name.as_deref(), Some("LinkedIn"));
    assert_eq!(view.warning_version.as_deref(), Some("v1"));
    assert_eq!(view.accepted_at.as_deref(), Some(NOW));
    assert_eq!(
        view.expires_at.as_deref(),
        Some("2026-11-16T09:00:00Z"),
        "30 days from acceptance"
    );
    assert_eq!(view.daily_cap, 3, "three a day by default");
    assert_eq!(view.max_daily_cap, 10);
    assert!(view.reading_opted_in);
}

#[test]
fn the_opt_in_is_audited_with_the_typed_name_and_the_warning() {
    let h = harness(NOW, FakeSources::board("linkedin", true));
    h.service.opt_in("linkedin", "LinkedIn", "v1").unwrap();

    let rows = h.system.rows();
    assert_eq!(rows.len(), 2, "intent, then outcome");
    assert_eq!(rows[0].action, OPT_IN_ACTION);
    assert_eq!(rows[0].target_id.as_deref(), Some("linkedin"));
    let detail: serde_json::Value = serde_json::from_str(&rows[0].detail_json).unwrap();
    assert_eq!(detail["confirmed_name"], "LinkedIn");
    assert_eq!(detail["warning_version"], "v1");
    assert_eq!(rows[1].outcome, Some(boba_domain::audit::AuditOutcome::Ok));

    // An unavailable audit log means the opt-in never happened (fail closed).
    let h = harness(NOW, FakeSources::board("linkedin", true));
    h.system.fail_intent_for(OPT_IN_ACTION);
    assert!(h.service.opt_in("linkedin", "LinkedIn", "v1").is_err());
    assert!(h.sources.auto_opt_in("linkedin").unwrap().is_none());
}

#[test]
fn an_opt_in_lapses_after_thirty_days_and_falls_back_to_review() {
    let sources = FakeSources::board("linkedin", true);
    let seeded = SessionAutoOptIn {
        source_id: "linkedin".into(),
        confirmed_name: "LinkedIn".into(),
        warning_version: "v1".into(),
        accepted_at: "2026-09-17T09:00:00Z".into(),
    };
    sources.seed_auto(seeded);
    let h = harness(NOW, sources);

    let view = h.service.view("linkedin").unwrap();
    assert_eq!(view.state, "expired");
    assert_eq!(
        view.expires_at.as_deref(),
        Some("2026-10-17T09:00:00Z"),
        "exactly thirty days later"
    );

    // Re-confirming is the same command: it replaces the stale row.
    let view = h.service.opt_in("linkedin", "LinkedIn", "v1").unwrap();
    assert_eq!(view.state, "active");
}

#[test]
fn a_challenge_pauses_auto_for_seven_days() {
    let sources = FakeSources::board("linkedin", true);
    sources.seed_auto(SessionAutoOptIn {
        source_id: "linkedin".into(),
        confirmed_name: "LinkedIn".into(),
        warning_version: "v1".into(),
        accepted_at: NOW.into(),
    });
    sources.seed_pause("linkedin", "2026-10-24T09:00:00Z");
    let h = harness(NOW, sources);

    let view = h.service.view("linkedin").unwrap();
    assert_eq!(view.state, "paused");
    assert_eq!(view.paused_until.as_deref(), Some("2026-10-24T09:00:00Z"));

    // Seven days later the pause is over and the opt-in still covers Auto.
    let later = harness("2026-10-24T09:00:00Z", FakeSources::board("linkedin", true));
    later.sources.seed_auto(SessionAutoOptIn {
        source_id: "linkedin".into(),
        confirmed_name: "LinkedIn".into(),
        warning_version: "v1".into(),
        accepted_at: NOW.into(),
    });
    later.sources.seed_pause("linkedin", "2026-10-24T09:00:00Z");
    assert_eq!(later.service.view("linkedin").unwrap().state, "active");
}

#[test]
fn the_cap_is_three_by_default_ten_is_the_maximum_and_it_is_audited() {
    let h = harness(NOW, FakeSources::board("linkedin", true));
    assert_eq!(h.service.view("linkedin").unwrap().daily_cap, 3);

    assert_eq!(
        h.service.set_cap("linkedin", 11).unwrap_err().message_key(),
        "errors.session.auto_cap_invalid",
        "no UI raises a Source past ten a day"
    );
    assert_eq!(
        h.service.set_cap("linkedin", 0).unwrap_err().message_key(),
        "errors.session.auto_cap_invalid"
    );

    let view = h.service.set_cap("linkedin", 7).unwrap();
    assert_eq!(view.daily_cap, 7);
    assert_eq!(view.used_today, 0);
    assert_eq!(
        h.system
            .setting_get(&boba_domain::apply::session_policy::cap_key("linkedin"))
            .unwrap()
            .as_deref(),
        Some("7")
    );

    let actions: Vec<String> = h.system.rows().into_iter().map(|row| row.action).collect();
    assert_eq!(
        actions,
        vec![CAP_ACTION.to_string(), CAP_ACTION.to_string()]
    );

    // The ledger count the screen shows is today's, channel-scoped.
    *h.rules.session_today.lock().unwrap() = 2;
    assert_eq!(h.service.view("linkedin").unwrap().used_today, 2);
    assert_eq!(h.service.set_cap("linkedin", 10).unwrap().daily_cap, 10);
}

#[test]
fn revoking_drops_the_opt_in_and_audits_it() {
    let h = harness(NOW, FakeSources::board("linkedin", true));
    h.service.opt_in("linkedin", "LinkedIn", "v1").unwrap();

    let view = h.service.revoke("linkedin").unwrap();
    assert_eq!(view.state, "off");
    assert_eq!(view.accepted_at, None);
    assert!(h.sources.auto_opt_in("linkedin").unwrap().is_none());
    assert!(
        h.sources.get_optin("linkedin").unwrap().is_some(),
        "revoking Auto never turns the reading session off"
    );
    assert!(
        h.system
            .rows()
            .iter()
            .any(|row| row.action == REVOKE_ACTION),
        "the revocation is audited"
    );
}

#[test]
fn only_a_session_board_source_takes_an_auto_opt_in() {
    let h = harness(NOW, FakeSources::board("remotive", true));
    h.sources.records.lock().unwrap()[0].access_class = AccessClass::PublicJson;
    assert!(matches!(
        h.service.view("remotive"),
        Err(SessionAutoError::NotSessionBoard(_))
    ));
    assert!(matches!(
        h.service.opt_in("ghost", "Ghost", "v1"),
        Err(SessionAutoError::UnknownSource(_))
    ));
    assert_eq!(
        h.service.view("remotive").unwrap_err().message_key(),
        "errors.session.not_session_board"
    );
}

#[test]
fn the_auto_state_the_policy_reads_is_the_same_one_the_screen_shows() {
    let h = harness(NOW, FakeSources::board("linkedin", true));
    assert_eq!(
        auto_state(
            h.sources.as_ref(),
            "linkedin",
            session_policy::parse_time(NOW).unwrap()
        )
        .unwrap(),
        SessionAutoStatus::Off
    );

    h.service.opt_in("linkedin", "LinkedIn", "v1").unwrap();
    assert_eq!(
        auto_state(
            h.sources.as_ref(),
            "linkedin",
            session_policy::parse_time(NOW).unwrap()
        )
        .unwrap(),
        SessionAutoStatus::Active
    );

    h.sources.seed_pause("linkedin", "2026-10-24T09:00:00Z");
    assert!(matches!(
        auto_state(
            h.sources.as_ref(),
            "linkedin",
            session_policy::parse_time(NOW).unwrap()
        )
        .unwrap(),
        SessionAutoStatus::Paused { .. }
    ));

    // Turning the reading session off ends Auto too (FR-91 (c)).
    h.sources.optins.lock().unwrap().clear();
    assert_eq!(
        auto_state(
            h.sources.as_ref(),
            "linkedin",
            session_policy::parse_time(NOW).unwrap()
        )
        .unwrap(),
        SessionAutoStatus::Off
    );
    assert_eq!(h.service.view("linkedin").unwrap().state, "off");
}
