//! Kill Switch tests (S9.8, FR-86).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use boba_domain::apply::outbox::{OutboxItem, OutboxState};
use boba_domain::apply::policy::{AutoAttemptEntry, AutoRule, PolicyCandidate, RuleChannel};
use boba_domain::audit::{AuditOutcome, AuditPhase};
use boba_domain::scoring::gates::{Confidence, Verdict as GateVerdict};
use boba_domain::scoring::rank::Verdict;

use super::*;
use crate::apply::outbox::{OutboxError, OutboxService};
use crate::apply::rules::AutoRulesService;
use crate::ports::agent::{
    AgentEvent, AgentPort, AgentTask, AgentTaskKind, InputTrust, RunBudget, RunHandle,
};
use crate::ports::browser::{BrowserError, FormSnapshot, Screenshot, Upload};
use crate::ports::repo::auto_rules::{AutoRulesRepo, AutoRulesRepoError};
use crate::system::FixedClock;
use crate::system::testing::FakeSystemRepo;

const NOW: &str = "2026-10-17T10:00:00Z";

// ---- fixtures -------------------------------------------------------------

fn item(id: &str, state: OutboxState, note: Option<&str>) -> OutboxItem {
    OutboxItem {
        id: id.to_string(),
        application_id: format!("app_{id}"),
        rule_id: "rule_1".to_string(),
        source_id: "remotive".to_string(),
        channel: "email".to_string(),
        company_key: "acme".to_string(),
        title: "Data Engineer".to_string(),
        company: "Tapi Tea Co".to_string(),
        state,
        hold_minutes: 30,
        release_at: NOW.to_string(),
        subject_override: None,
        body_override: None,
        note: note.map(str::to_string),
        created_at: NOW.to_string(),
        updated_at: NOW.to_string(),
    }
}

/// An in-memory Outbox holding the rows the switch must park.
#[derive(Default)]
struct FakeOutbox {
    items: Mutex<Vec<OutboxItem>>,
}

impl FakeOutbox {
    fn with(items: Vec<OutboxItem>) -> Arc<Self> {
        Arc::new(Self {
            items: Mutex::new(items),
        })
    }

    fn state_of(&self, id: &str) -> OutboxState {
        self.items
            .lock()
            .unwrap()
            .iter()
            .find(|item| item.id == id)
            .expect("stored item")
            .state
    }

    fn note_of(&self, id: &str) -> Option<String> {
        self.items
            .lock()
            .unwrap()
            .iter()
            .find(|item| item.id == id)
            .and_then(|item| item.note.clone())
    }
}

impl OutboxHoldRepo for FakeOutbox {
    fn outbox_insert(
        &self,
        _item: &boba_domain::apply::outbox::NewOutboxItem,
        _now: &str,
    ) -> Result<OutboxItem, OutboxHoldRepoError> {
        Err(OutboxHoldRepoError::new("unused"))
    }

    fn outbox_get(&self, id: &str) -> Result<Option<OutboxItem>, OutboxHoldRepoError> {
        Ok(self
            .items
            .lock()
            .unwrap()
            .iter()
            .find(|item| item.id == id)
            .cloned())
    }

    fn outbox_live_for(
        &self,
        application_id: &str,
    ) -> Result<Option<OutboxItem>, OutboxHoldRepoError> {
        Ok(self
            .items
            .lock()
            .unwrap()
            .iter()
            .find(|item| item.application_id == application_id && item.state.is_live())
            .cloned())
    }

    fn outbox_list(&self, _limit: usize) -> Result<Vec<OutboxItem>, OutboxHoldRepoError> {
        Ok(self.items.lock().unwrap().clone())
    }

    fn outbox_due(&self, _now: &str) -> Result<Vec<OutboxItem>, OutboxHoldRepoError> {
        Ok(Vec::new())
    }

    fn outbox_releasing(&self) -> Result<Vec<OutboxItem>, OutboxHoldRepoError> {
        Ok(Vec::new())
    }

    fn outbox_claim(&self, _id: &str, _now: &str) -> Result<bool, OutboxHoldRepoError> {
        Ok(false)
    }

    fn outbox_set_state(
        &self,
        id: &str,
        state: OutboxState,
        note: Option<&str>,
        now: &str,
    ) -> Result<OutboxItem, OutboxHoldRepoError> {
        let mut items = self.items.lock().unwrap();
        let item = items
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or_else(|| OutboxHoldRepoError::new("unknown item"))?;
        item.state = state;
        item.note = note.map(str::to_string);
        item.updated_at = now.to_string();
        Ok(item.clone())
    }

    fn outbox_set_overrides(
        &self,
        _id: &str,
        _subject: Option<&str>,
        _body: Option<&str>,
        _now: &str,
    ) -> Result<OutboxItem, OutboxHoldRepoError> {
        Err(OutboxHoldRepoError::new("unused"))
    }

    fn dry_run_get(
        &self,
        _rule_id: &str,
    ) -> Result<Option<boba_domain::apply::outbox::DryRunState>, OutboxHoldRepoError> {
        Ok(None)
    }

    fn dry_run_list(
        &self,
    ) -> Result<Vec<boba_domain::apply::outbox::DryRunState>, OutboxHoldRepoError> {
        Ok(Vec::new())
    }

    fn dry_run_upsert(
        &self,
        _state: &boba_domain::apply::outbox::DryRunState,
        _now: &str,
    ) -> Result<(), OutboxHoldRepoError> {
        Ok(())
    }
}

/// A browser that only counts being closed: the switch never drives a page.
#[derive(Default)]
struct FakeBrowser {
    closes: AtomicUsize,
}

impl BrowserPort for FakeBrowser {
    fn open(
        &self,
        _provider: boba_domain::apply::channel::AtsProvider,
        _url: &str,
    ) -> Result<FormSnapshot, BrowserError> {
        Err(BrowserError::NotRunning)
    }

    fn snapshot(&self) -> Result<FormSnapshot, BrowserError> {
        Err(BrowserError::NotRunning)
    }

    fn fill_text(&self, _handle: &str, _value: &str) -> Result<(), BrowserError> {
        Err(BrowserError::NotRunning)
    }

    fn choose(&self, _handle: &str) -> Result<(), BrowserError> {
        Err(BrowserError::NotRunning)
    }

    fn attach(&self, _handle: &str, _file: &Upload) -> Result<(), BrowserError> {
        Err(BrowserError::NotRunning)
    }

    fn screenshot(&self) -> Result<Screenshot, BrowserError> {
        Err(BrowserError::NotRunning)
    }

    fn close(&self) -> Result<(), BrowserError> {
        self.closes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// An agent port that hands back a live run handle for every task.
#[derive(Default)]
struct FakeAgent {
    runs: AtomicUsize,
}

impl AgentPort for FakeAgent {
    fn run(&self, _task: AgentTask) -> RunHandle {
        let seq = self.runs.fetch_add(1, Ordering::SeqCst) + 1;
        let (_sender, events) = mpsc::channel::<AgentEvent>();
        RunHandle {
            run_id: format!("run_{seq}"),
            events,
            cancel: CancelToken::new(),
        }
    }
}

fn task() -> AgentTask {
    AgentTask {
        kind: AgentTaskKind::TailorApplication,
        input_trust: InputTrust::Untrusted,
        cli: "claude_code".to_string(),
        model: None,
        prompt: String::new(),
        target_language: "en".to_string(),
        budget: RunBudget::default(),
        env: Vec::new(),
    }
}

/// The rules repo the pre-submit check would read; the switch stops before it.
struct FakeAutoRules;

impl AutoRulesRepo for FakeAutoRules {
    fn rules(&self) -> Result<Vec<AutoRule>, AutoRulesRepoError> {
        Ok(Vec::new())
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
}

fn candidate() -> PolicyCandidate {
    PolicyCandidate {
        source_id: "remotive".to_string(),
        channel: RuleChannel::Email,
        company_key: "acme".to_string(),
        rank: Some(85.0),
        fit: Some(82.0),
        verdict: Verdict::Good,
        confidence: Confidence::Medium,
        gates: GateVerdict::Pass,
        fraud_badge: false,
        documents_verified: true,
        language_variant_confirmed: true,
        duplicate: false,
        agent_used: false,
        cli_tool_lockdown: true,
        source_auto_opt_in: true,
        local_minute: 10 * 60,
        utc_offset_minutes: 0,
    }
}

fn service(
    system: Arc<FakeSystemRepo>,
    outbox: Arc<FakeOutbox>,
    browser: Option<Arc<FakeBrowser>>,
    runs: Arc<RunCancellations>,
) -> KillSwitchService {
    KillSwitchService::new(
        system,
        outbox,
        Arc::new(FixedClock::new(NOW)),
        browser.map(|browser| browser as Arc<dyn BrowserPort>),
        runs,
    )
}

// ---- the flag -------------------------------------------------------------

#[test]
fn a_fresh_install_starts_with_the_switch_off() {
    let service = service(Arc::default(), Arc::default(), None, Arc::default());
    assert!(!service.engaged());
    assert_eq!(
        service.view(),
        KillSwitchView {
            engaged: false,
            since: None,
        }
    );
}

#[test]
fn engaging_persists_the_flag_and_when_it_happened() {
    let system = Arc::new(FakeSystemRepo::default());
    let service = service(Arc::clone(&system), Arc::default(), None, Arc::default());

    let engagement = service.set(true).expect("engage");

    assert!(engagement.view.engaged);
    assert_eq!(engagement.view.since.as_deref(), Some(NOW));
    assert_eq!(
        system.setting_get(KILL_SWITCH_KEY).unwrap().as_deref(),
        Some(ENGAGED)
    );
    assert_eq!(system.setting_get(SINCE_KEY).unwrap().as_deref(), Some(NOW));
}

#[test]
fn an_engaged_switch_comes_back_engaged_after_a_restart() {
    let mut system = FakeSystemRepo::default();
    system.seed_setting(KILL_SWITCH_KEY, ENGAGED);
    system.seed_setting(SINCE_KEY, NOW);
    let runs = Arc::new(RunCancellations::default());

    let service = service(Arc::new(system), Arc::default(), None, Arc::clone(&runs));

    assert!(service.engaged());
    assert_eq!(service.view().since.as_deref(), Some(NOW));
    // Loading only reads the persisted state: it cancels nothing.
    assert!(runs.is_empty());
}

#[test]
fn turning_it_off_clears_the_flag_and_the_stamp() {
    let system = Arc::new(FakeSystemRepo::default());
    let service = service(Arc::clone(&system), Arc::default(), None, Arc::default());
    service.set(true).expect("engage");

    let engagement = service.set(false).expect("disengage");

    assert!(!engagement.view.engaged);
    assert_eq!(engagement.view.since, None);
    assert_eq!(
        system.setting_get(KILL_SWITCH_KEY).unwrap().as_deref(),
        Some(OFF)
    );
    assert_eq!(system.setting_get(SINCE_KEY).unwrap(), None);
}

#[test]
fn engaging_twice_is_a_no_op() {
    let system = Arc::new(FakeSystemRepo::default());
    let service = service(Arc::clone(&system), Arc::default(), None, Arc::default());
    service.set(true).expect("engage");

    let again = service.set(true).expect("engage again");

    // One intent and one outcome for the one real change, not two.
    assert_eq!(system.audit_verify_chain().unwrap().rows, 2);
    assert_eq!(again.parked, 0);
    assert_eq!(again.cancelled_runs, 0);
    assert!(again.view.engaged);
}

// ---- the Outbox -----------------------------------------------------------

#[test]
fn engaging_moves_every_live_item_to_the_review_queue() {
    let outbox = FakeOutbox::with(vec![
        item("held", OutboxState::Held, None),
        item("would", OutboxState::WouldSend, None),
        item("releasing", OutboxState::Releasing, None),
        item("sent", OutboxState::Sent, None),
        item("cancelled", OutboxState::Cancelled, Some("user")),
        item("review", OutboxState::NeedsReview, Some("dry_run_ended")),
    ]);
    let service = service(Arc::default(), Arc::clone(&outbox), None, Arc::default());

    let engagement = service.set(true).expect("engage");

    assert_eq!(engagement.parked, 3);
    for id in ["held", "would", "releasing"] {
        assert_eq!(outbox.state_of(id), OutboxState::NeedsReview, "{id}");
        assert_eq!(outbox.note_of(id).as_deref(), Some(NOTE), "{id}");
    }
    // The rows that were already out of the automatic path keep their story.
    assert_eq!(outbox.state_of("sent"), OutboxState::Sent);
    assert_eq!(outbox.state_of("cancelled"), OutboxState::Cancelled);
    assert_eq!(outbox.note_of("review").as_deref(), Some("dry_run_ended"));
}

#[test]
fn turning_it_off_leaves_the_review_queue_alone() {
    let outbox = FakeOutbox::with(vec![item("held", OutboxState::Held, None)]);
    let service = service(Arc::default(), Arc::clone(&outbox), None, Arc::default());

    service.set(false).expect("disengage");

    assert_eq!(outbox.state_of("held"), OutboxState::Held);
}

// ---- in-flight work -------------------------------------------------------

#[test]
fn engaging_cancels_the_runs_in_flight() {
    let runs = Arc::new(RunCancellations::default());
    let first = CancelToken::new();
    let second = CancelToken::new();
    runs.track("run_1", first.clone());
    runs.track("run_2", second.clone());
    let service = service(Arc::default(), Arc::default(), None, Arc::clone(&runs));

    let engagement = service.set(true).expect("engage");

    assert!(first.is_cancelled());
    assert!(second.is_cancelled());
    assert_eq!(engagement.cancelled_runs, 2);

    // A run started after the switch stays live: the switch stops what is
    // running, it does not forbid the user's own AI work.
    let later = CancelToken::new();
    runs.track("run_3", later.clone());
    assert!(!later.is_cancelled());
}

#[test]
fn engaging_closes_the_browser_session() {
    let browser = Arc::new(FakeBrowser::default());
    let service = service(
        Arc::default(),
        Arc::default(),
        Some(Arc::clone(&browser)),
        Arc::default(),
    );

    let engagement = service.set(true).expect("engage");

    assert!(engagement.browser_closed);
    assert_eq!(browser.closes.load(Ordering::SeqCst), 1);
}

#[test]
fn a_launch_without_a_browser_reports_nothing_closed() {
    let service = service(Arc::default(), Arc::default(), None, Arc::default());
    assert!(!service.set(true).expect("engage").browser_closed);
}

#[test]
fn tracking_more_runs_than_the_cap_keeps_the_registry_bounded() {
    let runs = RunCancellations::default();
    for index in 0..(MAX_TRACKED_RUNS + 10) {
        runs.track(format!("run_{index}"), CancelToken::new());
    }
    assert_eq!(runs.len(), MAX_TRACKED_RUNS);
}

#[test]
fn a_tracked_run_keeps_one_entry_per_run_id() {
    let runs = RunCancellations::default();
    runs.track("run_1", CancelToken::new());
    runs.track("run_1", CancelToken::new());
    assert_eq!(runs.len(), 1);
}

// ---- the engine re-checks it before the irreversible step ------------------

#[test]
fn the_port_flips_immediately_for_the_pre_submit_check() {
    let service = service(Arc::default(), Arc::default(), None, Arc::default());
    let port: &dyn KillSwitchPort = &service;

    assert!(!port.engaged());
    service.set(true).expect("engage");
    assert!(port.engaged());
    service.set(false).expect("disengage");
    assert!(!port.engaged());
}

#[test]
fn a_send_stops_mid_flow_when_the_switch_engages_before_the_submit() {
    let outbox = FakeOutbox::with(vec![item("ob_1", OutboxState::Releasing, None)]);
    let system = Arc::new(FakeSystemRepo::default());
    let service = Arc::new(service(
        Arc::clone(&system),
        Arc::clone(&outbox),
        None,
        Arc::default(),
    ));
    let rules = Arc::new(AutoRulesService::new(
        Arc::new(FakeAutoRules),
        Arc::clone(&system) as Arc<dyn SystemRepo>,
        Arc::new(FixedClock::new(NOW)),
    ));
    let outbox_service = OutboxService::new(
        Arc::clone(&outbox) as Arc<dyn OutboxHoldRepo>,
        rules,
        Arc::clone(&system) as Arc<dyn SystemRepo>,
        Arc::new(FixedClock::new(NOW)),
        None,
        Some(Arc::clone(&service) as Arc<dyn KillSwitchPort>),
    );

    // The release already claimed the item and the user engages the switch
    // while it waits for the final submit.
    service.set(true).expect("engage");
    let submit = outbox_service.confirm_before_submit("ob_1", &candidate(), 7);

    // The switch's engage pass moved the claimed item out of the automatic
    // path, so the final check can never hand it a Go (and the check itself
    // would answer Stop if the item were still `releasing`: S9.7 tests that
    // branch against the port).
    assert!(
        matches!(&submit, Err(OutboxError::NotWaiting(id)) if id == "ob_1"),
        "the send must not pass the final check: {submit:?}"
    );
    assert_eq!(outbox.state_of("ob_1"), OutboxState::NeedsReview);
    assert_eq!(outbox.note_of("ob_1").as_deref(), Some(NOTE));
}

// ---- the audit ------------------------------------------------------------

/// H-1: the switch is a panic control, so a broken audit log must never keep
/// automation running. Engaging still flips the flag, persists it, parks the
/// live items and cancels the runs; the audit failure is only reported.
#[test]
fn an_audit_failure_never_keeps_automation_running() {
    let system = Arc::new(FakeSystemRepo::default());
    system.fail_audit();
    let outbox = FakeOutbox::with(vec![item("held", OutboxState::Held, None)]);
    let runs = Arc::new(RunCancellations::default());
    let run = CancelToken::new();
    runs.track("run_1", run.clone());
    let switch = service(
        Arc::clone(&system),
        Arc::clone(&outbox),
        None,
        Arc::clone(&runs),
    );

    let engagement = switch
        .set(true)
        .expect("engaging must not depend on the log");

    assert!(switch.engaged());
    assert_eq!(
        system.setting_get(KILL_SWITCH_KEY).unwrap().as_deref(),
        Some(ENGAGED),
        "the flag is persisted even with the log down"
    );
    assert_eq!(outbox.state_of("held"), OutboxState::NeedsReview);
    assert_eq!(outbox.note_of("held").as_deref(), Some(NOTE));
    assert!(run.is_cancelled());
    assert!(engagement.view.engaged);
    assert_eq!(system.rows().len(), 0, "nothing reached the log");
}

#[test]
fn every_change_is_audited_as_an_intent_and_an_outcome() {
    let system = Arc::new(FakeSystemRepo::default());
    let service = service(Arc::clone(&system), Arc::default(), None, Arc::default());

    service.set(true).expect("engage");
    service.set(false).expect("disengage");

    let rows = system.rows();
    assert_eq!(rows.len(), 4, "one intent + one outcome per change");
    for pair in rows.chunks(2) {
        assert_eq!(pair[0].phase, AuditPhase::Intent);
        assert_eq!(pair[1].phase, AuditPhase::Outcome);
        for row in pair {
            assert_eq!(row.action, AUDIT_ACTION);
            assert_eq!(row.actor, "user");
            assert_eq!(row.target_id.as_deref(), Some(KILL_SWITCH_KEY));
        }
        assert_eq!(pair[1].outcome, Some(AuditOutcome::Ok));
    }
    assert!(rows[0].detail_json.contains("\"engaged\":true"));
    assert!(rows[2].detail_json.contains("\"engaged\":false"));
    assert!(system.audit_unresolved_intents().unwrap().is_empty());
}

// ---- the agent decorator --------------------------------------------------

#[test]
fn the_cancelling_agent_tracks_every_run_it_starts() {
    let runs = Arc::new(RunCancellations::default());
    let agent = CancellingAgent::new(Arc::new(FakeAgent::default()), Arc::clone(&runs));
    let service = service(Arc::default(), Arc::default(), None, Arc::clone(&runs));

    let first = agent.run(task());
    let second = agent.run(task());
    assert_eq!(runs.len(), 2);

    service.set(true).expect("engage");

    assert!(first.cancel.is_cancelled());
    assert!(second.cancel.is_cancelled());
}

#[test]
fn a_cancelled_run_token_is_not_counted_twice() {
    let runs = RunCancellations::default();
    let token = CancelToken::new();
    token.cancel();
    runs.track("run_1", token);

    assert_eq!(runs.cancel_all(), 0);
}
