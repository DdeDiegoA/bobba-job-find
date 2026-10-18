//! Status proposal use-case tests (S8.4): classification (AC1, AC4), the queue
//! and approval (AC2), the disclosure gate (AC3), auto-confirm and undo (AC5),
//! and the untrusted taint (AC6).

use std::sync::Arc;
use std::sync::mpsc::channel;

use boba_domain::application::{AppStatus, Application, StatusSource};
use boba_domain::mail::{AUTO_CONFIRM_CONFIDENCE, ClassificationMethod, ProposalState, ReplyClass};
use boba_domain::posting::{ApplyHints, Offer, Provenance, WorkMode};

use crate::applications::testing::FakeApplyRepo;
use crate::ingest::normalize::testing::FakeIngestionRepo;
use crate::ports::agent::{
    AgentEvent, AgentPort, AgentTask, CancelToken, FailureReason, RunHandle,
};
use crate::status_proposals::repo::StoredMessage;
use crate::system::FixedClock;
use crate::system::testing::FakeSystemRepo;

use super::repo::StatusProposalRepo;
use super::testing::{FakeClassifier, FakeStatusProposalRepo, matched};
use super::*;

const NOW: &str = "2026-10-17T09:00:00Z";

fn offer(id: &str, company: &str, email: &str) -> Offer {
    Offer {
        id: id.to_string(),
        source_id: "greenhouse".to_string(),
        external_id: format!("gh-{id}"),
        title: "Senior Data Engineer".to_string(),
        company: company.to_string(),
        locations: vec!["Remote".to_string()],
        work_mode: WorkMode::Remote,
        money: None,
        posted_at: None,
        deadline: None,
        first_seen: "2026-10-04T00:00:00Z".to_string(),
        last_seen: NOW.to_string(),
        lang: "en".to_string(),
        description_html: String::new(),
        description_text: String::new(),
        requirements: vec![],
        apply: ApplyHints {
            url: format!("https://{company}.example/jobs/{id}"),
            email: Some(email.to_string()),
        },
    }
}

fn provenance(offer: &Offer) -> Provenance {
    Provenance {
        posting_id: offer.id.clone(),
        source_id: offer.source_id.clone(),
        external_id: offer.external_id.clone(),
        url: offer.apply.url.clone(),
        raw: "{}".to_string(),
        fetched_at: NOW.to_string(),
    }
}

fn application(id: &str, posting_id: &str, status: AppStatus, company: &str) -> Application {
    Application {
        id: id.to_string(),
        posting_id: posting_id.to_string(),
        status,
        offer_snapshot_json: serde_json::json!({
            "title": "Senior Data Engineer",
            "company": company,
        })
        .to_string(),
        notes: None,
        next_action: None,
        created_at: NOW.to_string(),
        updated_at: NOW.to_string(),
    }
}

fn message(id: &str, from: &str, subject: &str, body: &str) -> StoredMessage {
    StoredMessage {
        id: id.to_string(),
        from_address: from.to_string(),
        from_name: None,
        subject: subject.to_string(),
        body_text: body.to_string(),
        received_at: NOW.to_string(),
    }
}

/// An agent that answers from a script and counts its runs.
#[derive(Default)]
struct FakeAgent {
    answers: Vec<AgentEvent>,
    runs: std::sync::Mutex<Vec<AgentTask>>,
}

impl FakeAgent {
    fn with(answers: Vec<AgentEvent>) -> Self {
        Self {
            answers,
            runs: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn runs(&self) -> usize {
        self.runs.lock().unwrap().len()
    }
}

impl AgentPort for FakeAgent {
    fn run(&self, task: AgentTask) -> RunHandle {
        self.runs.lock().unwrap().push(task);
        let (sender, events) = channel();
        for event in &self.answers {
            let _ = sender.send(event.clone());
        }
        drop(sender);
        RunHandle {
            run_id: "run_classify".to_string(),
            events,
            cancel: CancelToken::new(),
        }
    }
}

struct Harness {
    service: StatusProposalsService,
    repo: Arc<FakeStatusProposalRepo>,
    apply: Arc<FakeApplyRepo>,
    postings: Arc<FakeIngestionRepo>,
    agents: Arc<FakeAgent>,
    classifier: FakeClassifier,
}

fn harness_with(
    classifier: FakeClassifier,
    agents: Arc<FakeAgent>,
    settings: &[(&str, &str)],
) -> Harness {
    let repo = Arc::new(FakeStatusProposalRepo::default());
    let apply = Arc::new(FakeApplyRepo::default());
    let postings = Arc::new(FakeIngestionRepo::default());
    let mut system = FakeSystemRepo::default();
    for (key, value) in settings {
        system.seed_setting(key, value);
    }
    let service = StatusProposalsService::new(
        repo.clone(),
        apply.clone(),
        postings.clone(),
        Arc::new(system),
        agents.clone(),
        Arc::new(classifier.clone()) as Arc<dyn ReplyClassifier>,
        Arc::new(FixedClock::new(NOW)),
    );
    Harness {
        service,
        repo,
        apply,
        postings,
        agents,
        classifier,
    }
}

/// The default: the user accepted the disclosure and kept the LLM mode, so the
/// rules run first and the agent is only asked for what they miss.
fn harness(classifier: FakeClassifier) -> Harness {
    harness_with(
        classifier,
        Arc::new(FakeAgent::default()),
        &[(SETTING_MODE, "llm"), (SETTING_DISCLOSURE_ACK, "true")],
    )
}

fn seed_offer(harness: &Harness, id: &str, company: &str, email: &str) {
    let offer = offer(id, company, email);
    harness
        .postings
        .insert_offer(&offer, &provenance(&offer))
        .unwrap();
}

/// Seed the Acme Offer, its Application and the reply Acme sent.
fn seed_application(harness: &Harness, status: AppStatus) {
    seed_offer(harness, "pst_1", "acme", "jobs@acme.com");
    harness
        .apply
        .seed_application(application("app_1", "pst_1", status, "Acme"));
    harness.repo.seed_message(message(
        "msg_1",
        "recruiting@acme.com",
        "Your application",
        "Thank you for applying to Acme.",
    ));
}

// ---- AC1: classification --------------------------------------------------

#[test]
fn a_deterministic_match_stores_the_class_confidence_and_quote() {
    let harness = harness(FakeClassifier::scripted(vec![matched(
        ReplyClass::Acknowledgement,
        0.95,
        "Thank you for applying to Acme.",
    )]));
    seed_application(&harness, AppStatus::Saved);

    let outcome = harness.service.classify(None, None).unwrap();

    assert_eq!(outcome.classified, 1);
    assert!(!outcome.llm_asked);
    let stored = harness.repo.classifications();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].class, ReplyClass::Acknowledgement);
    assert_eq!(stored[0].confidence, 0.95);
    assert_eq!(stored[0].quoted_line, "Thank you for applying to Acme.");
    assert_eq!(stored[0].method, ClassificationMethod::Deterministic);
    // A rule quotes the message it read: it is not agent text (ADR-010).
    assert!(!stored[0].untrusted_origin);
}

#[test]
fn a_class_that_implies_a_stage_change_becomes_a_proposal() {
    let harness = harness(FakeClassifier::scripted(vec![matched(
        ReplyClass::Rejection,
        0.97,
        "Unfortunately we cannot proceed.",
    )]));
    seed_application(&harness, AppStatus::Applied);

    let outcome = harness.service.classify(None, None).unwrap();
    assert_eq!(outcome.proposals, 1);

    let proposals = harness.service.proposals().unwrap();
    assert_eq!(proposals.len(), 1);
    assert_eq!(proposals[0].class, "rejection");
    assert_eq!(proposals[0].to_status, "rejected");
    assert!(!proposals[0].needs_pick);
    assert_eq!(
        proposals[0]
            .application
            .as_ref()
            .map(|app| app.application_id.as_str()),
        Some("app_1")
    );
    // Nothing moved yet: the proposal waits for the user (AC2).
    assert_eq!(
        harness.apply.stored_status("app_1"),
        Some(AppStatus::Applied)
    );
}

#[test]
fn a_class_with_no_stage_change_creates_no_proposal() {
    let harness = harness(FakeClassifier::scripted(vec![matched(
        ReplyClass::RequestForInfo,
        0.9,
        "Please send your notice period.",
    )]));
    seed_application(&harness, AppStatus::Applied);

    let outcome = harness.service.classify(None, None).unwrap();
    assert_eq!(outcome.classified, 1);
    assert_eq!(outcome.proposals, 0);
    assert!(harness.service.proposals().unwrap().is_empty());
}

// ---- AC2: approval --------------------------------------------------------

#[test]
fn approving_a_proposal_calls_the_tracker_transition() {
    let harness = harness(FakeClassifier::scripted(vec![matched(
        ReplyClass::InterviewInvite,
        0.97,
        "We would like to invite you to an interview.",
    )]));
    seed_application(&harness, AppStatus::Applied);
    harness.service.classify(None, None).unwrap();
    let id = harness.service.proposals().unwrap()[0].id.clone();

    let outcome = harness.service.approve(&id, None).unwrap();

    assert_eq!(outcome.from_status, "applied");
    assert_eq!(outcome.to_status, "interview");
    assert_eq!(
        harness.apply.stored_status("app_1"),
        Some(AppStatus::Interview)
    );
    let events = harness.apply.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].source, StatusSource::StatusProposal);
    assert_eq!(events[0].reason.as_deref(), Some("reply:interview_invite"));
    // The queue no longer holds it.
    assert!(harness.service.proposals().unwrap().is_empty());
}

#[test]
fn a_batch_approves_what_it_can_and_reports_the_rest() {
    let harness = harness(FakeClassifier::scripted(vec![
        matched(ReplyClass::InterviewInvite, 0.97, "Interview next week."),
        matched(ReplyClass::Rejection, 0.97, "Unfortunately, no."),
    ]));
    seed_application(&harness, AppStatus::Applied);
    harness.repo.seed_message(message(
        "msg_2",
        "recruiting@acme.com",
        "Update",
        "Unfortunately we cannot proceed.",
    ));
    harness.service.classify(None, None).unwrap();
    let ids: Vec<String> = harness
        .service
        .proposals()
        .unwrap()
        .into_iter()
        .map(|proposal| proposal.id)
        .collect();
    assert_eq!(ids.len(), 2);

    let mut ids = ids;
    ids.push("sp_ghost".to_string());
    let outcome = harness.service.approve_many(&ids).unwrap();

    assert_eq!(outcome.approved.len(), 2);
    assert_eq!(
        outcome.failed,
        vec!["sp_ghost".to_string()],
        "a stale id in a batch is reported, never fatal"
    );
    // Both moves landed, in the order the batch was given.
    assert_eq!(
        harness.apply.stored_status("app_1"),
        Some(AppStatus::Rejected)
    );
}

#[test]
fn an_ambiguous_match_asks_the_user_to_pick_the_application() {
    // Two Applications at Acme: the message names no single one.
    let offer_a = offer("pst_a", "acme", "jobs@acme.com");
    let offer_b = offer("pst_b", "acme", "jobs@acme.com");
    let harness = harness(FakeClassifier::scripted(vec![matched(
        ReplyClass::InterviewInvite,
        0.97,
        "Interview next week.",
    )]));
    harness
        .postings
        .insert_offer(&offer_a, &provenance(&offer_a))
        .unwrap();
    harness
        .postings
        .insert_offer(&offer_b, &provenance(&offer_b))
        .unwrap();
    harness
        .apply
        .seed_application(application("app_a", "pst_a", AppStatus::Applied, "Acme"));
    harness
        .apply
        .seed_application(application("app_b", "pst_b", AppStatus::Applied, "Acme"));
    harness.repo.seed_message(message(
        "msg_1",
        "recruiting@acme.com",
        "Interview",
        "We would like to invite you to an interview.",
    ));

    harness.service.classify(None, None).unwrap();
    let proposal = harness.service.proposals().unwrap().remove(0);
    assert!(proposal.needs_pick);
    assert_eq!(proposal.candidates.len(), 2);

    let error = harness.service.approve(&proposal.id, None).unwrap_err();
    assert!(matches!(error, StatusProposalsError::NeedsApplication(_)));

    let outcome = harness
        .service
        .approve(&proposal.id, Some("app_b"))
        .unwrap();
    assert_eq!(outcome.application_id, "app_b");
    assert_eq!(
        harness.apply.stored_status("app_b"),
        Some(AppStatus::Interview)
    );
    assert_eq!(
        harness.apply.stored_status("app_a"),
        Some(AppStatus::Applied)
    );
}

// ---- AC3/AC4: disclosure and deterministic-only ---------------------------

#[test]
fn llm_mode_without_the_disclosure_is_refused_before_anything_is_read() {
    let harness = harness_with(
        FakeClassifier::scripted(vec![None]),
        Arc::new(FakeAgent::default()),
        &[],
    );
    seed_application(&harness, AppStatus::Applied);

    let error = harness
        .service
        .classify(Some("claude_code"), None)
        .unwrap_err();

    assert!(matches!(error, StatusProposalsError::DisclosureRequired));
    assert_eq!(harness.classifier.calls(), 0);
    assert_eq!(harness.agents.runs(), 0, "nothing left the machine");
}

#[test]
fn accepting_the_disclosure_keeps_the_chosen_mode() {
    let harness = harness(FakeClassifier::default());

    let view = harness
        .service
        .save_settings(ClassificationMode::Deterministic, false, true)
        .unwrap();

    assert_eq!(view.mode, ClassificationMode::Deterministic);
    assert!(view.acknowledged);
    assert!(!view.needs_disclosure);
    assert!(!view.auto_confirm);
    assert_eq!(
        harness.service.disclosure().unwrap().mode,
        ClassificationMode::Deterministic
    );
}

#[test]
fn deterministic_only_mode_never_asks_the_agent() {
    let harness = harness_with(
        FakeClassifier::scripted(vec![None]),
        Arc::new(FakeAgent::default()),
        &[(SETTING_MODE, "deterministic")],
    );
    seed_application(&harness, AppStatus::Applied);

    let outcome = harness.service.classify(Some("claude_code"), None).unwrap();

    assert!(!outcome.llm_asked);
    assert_eq!(outcome.uncertain, 1);
    assert_eq!(harness.agents.runs(), 0);
    assert!(harness.repo.classifications().is_empty());
    assert!(
        harness.repo.messages_unclassified().unwrap().len() == 1,
        "the message waits for manual triage (AC4)"
    );
}

// ---- AC1/AC6: the LLM stage ----------------------------------------------

fn llm_harness(answers: Vec<AgentEvent>) -> Harness {
    harness_with(
        FakeClassifier::scripted(vec![None]),
        Arc::new(FakeAgent::with(answers)),
        &[(SETTING_MODE, "llm"), (SETTING_DISCLOSURE_ACK, "true")],
    )
}

fn propose(email_id: &str, app_id: &str, status: &str, quote: &str) -> AgentEvent {
    AgentEvent::ToolCall {
        name: PROPOSE_TOOL.to_string(),
        input: serde_json::json!({
            "email_id": email_id,
            "app_id": app_id,
            "status": status,
            "quote": quote,
        }),
    }
}

#[test]
fn an_llm_proposal_is_verified_and_stored_tainted() {
    let harness = llm_harness(vec![propose(
        "msg_1",
        "app_1",
        "interview",
        "would love to set up a chat",
    )]);
    seed_offer(&harness, "pst_1", "acme", "jobs@acme.com");
    harness
        .apply
        .seed_application(application("app_1", "pst_1", AppStatus::Applied, "Acme"));
    harness.repo.seed_message(message(
        "msg_1",
        "recruiting@acme.com",
        "Hello",
        "We would love to set up a chat about the role.",
    ));

    let outcome = harness.service.classify(Some("claude_code"), None).unwrap();

    assert!(outcome.llm_asked);
    assert_eq!(outcome.classified, 1);
    assert_eq!(harness.agents.runs(), 1);
    let stored = harness.repo.classifications();
    assert_eq!(stored[0].method, ClassificationMethod::Llm);
    assert_eq!(stored[0].class, ReplyClass::InterviewInvite);
    assert!(stored[0].untrusted_origin, "AC6: LLM text is tainted");
    // The quote comes from the message itself, not from the run's string.
    assert_eq!(
        stored[0].quoted_line,
        "We would love to set up a chat about the role."
    );
    let proposal = &harness.repo.proposals()[0];
    assert!(proposal.untrusted_origin);
    // An LLM proposal can never clear the auto-confirm threshold (AC5/AC6).
    assert!(proposal.confidence < AUTO_CONFIRM_CONFIDENCE);
}

/// FX16: the CLI's own stream names an MCP tool `mcp__boba__<tool>`. Matching
/// only the bare name dropped every real proposal before this.
#[test]
fn a_proposal_named_the_way_the_cli_names_it_is_stored() {
    let mut event = propose("msg_1", "app_1", "interview", "would love to set up a chat");
    if let AgentEvent::ToolCall { name, .. } = &mut event {
        *name = format!("mcp__boba__{PROPOSE_TOOL}");
    }
    let harness = llm_harness(vec![event]);
    seed_offer(&harness, "pst_1", "acme", "jobs@acme.com");
    harness
        .apply
        .seed_application(application("app_1", "pst_1", AppStatus::Applied, "Acme"));
    harness.repo.seed_message(message(
        "msg_1",
        "recruiting@acme.com",
        "Hello",
        "We would love to set up a chat about the role.",
    ));

    let outcome = harness.service.classify(Some("claude_code"), None).unwrap();

    assert_eq!(outcome.classified, 1);
    assert_eq!(harness.repo.proposals().len(), 1);
}

/// FX22 (L-1): the stream's name is `mcp__boba__<tool>` — this app's server,
/// with the trailing `__` as the boundary. A name from a server that only
/// starts with `mcp__boba` is another server's tool, not the proposal tool.
#[test]
fn a_proposal_under_another_server_name_is_not_the_proposal_tool() {
    assert!(is_proposal(PROPOSE_TOOL));
    assert!(is_proposal(&format!("mcp__boba__{PROPOSE_TOOL}")));
    for near_miss in [
        format!("mcp__boba_evil__{PROPOSE_TOOL}"),
        format!("mcp__bobax__{PROPOSE_TOOL}"),
        format!("evil__{PROPOSE_TOOL}"),
        format!("mcp__boba__{PROPOSE_TOOL}_evil"),
        "mcp__boba__get_task_input".to_string(),
    ] {
        assert!(!is_proposal(&near_miss), "{near_miss} must not match");
    }
}

#[test]
fn a_proposal_whose_quote_is_not_in_the_message_is_dropped() {
    let harness = llm_harness(vec![propose(
        "msg_1",
        "app_1",
        "offer",
        "we offer 1M salary",
    )]);
    seed_offer(&harness, "pst_1", "acme", "jobs@acme.com");
    harness
        .apply
        .seed_application(application("app_1", "pst_1", AppStatus::Applied, "Acme"));
    harness.repo.seed_message(message(
        "msg_1",
        "recruiting@acme.com",
        "Hello",
        "We would love to set up a chat.",
    ));

    let outcome = harness.service.classify(Some("claude_code"), None).unwrap();

    assert_eq!(outcome.dropped, 1);
    assert!(harness.repo.classifications().is_empty());
    assert!(harness.repo.proposals().is_empty());
}

/// FX22 (L-3): `ok` or `chat` is not evidence. A quote shorter than
/// [`MIN_QUOTE_CHARS`] is refused even when the message contains it, so the
/// user never sees a proposal whose quote shows nothing.
#[test]
fn a_quote_shorter_than_the_minimum_is_refused() {
    let stored = message(
        "msg_1",
        "recruiting@acme.com",
        "Hello",
        "We would love to set up a chat about the role.",
    );
    assert_eq!(
        verified_quote(&stored, "would love t").as_deref(),
        Some("We would love to set up a chat about the role.")
    );
    for short in ["chat", "would love", "ok"] {
        assert!(
            verified_quote(&stored, short).is_none(),
            "`{short}` must be too short to be evidence"
        );
    }
}

/// FX22 (L-3): and the run's proposal is dropped, not stored, on a short
/// quote the message really contains.
#[test]
fn a_proposal_quoted_too_short_is_dropped() {
    let harness = llm_harness(vec![propose("msg_1", "app_1", "interview", "chat")]);
    seed_offer(&harness, "pst_1", "acme", "jobs@acme.com");
    harness
        .apply
        .seed_application(application("app_1", "pst_1", AppStatus::Applied, "Acme"));
    harness.repo.seed_message(message(
        "msg_1",
        "recruiting@acme.com",
        "Hello",
        "We would love to set up a chat about the role.",
    ));

    let outcome = harness.service.classify(Some("claude_code"), None).unwrap();

    assert_eq!(outcome.dropped, 1);
    assert!(harness.repo.classifications().is_empty());
    assert!(harness.repo.proposals().is_empty());
}

#[test]
fn a_proposal_for_an_unrelated_application_is_dropped() {
    let harness = llm_harness(vec![propose(
        "msg_1",
        "app_evil",
        "rejected",
        "set up a chat",
    )]);
    seed_offer(&harness, "pst_1", "acme", "jobs@acme.com");
    harness
        .apply
        .seed_application(application("app_1", "pst_1", AppStatus::Applied, "Acme"));
    harness.repo.seed_message(message(
        "msg_1",
        "recruiting@acme.com",
        "Hello",
        "We would love to set up a chat.",
    ));

    let outcome = harness.service.classify(Some("claude_code"), None).unwrap();

    assert_eq!(outcome.dropped, 1);
    assert!(harness.repo.proposals().is_empty());
}

#[test]
fn a_failed_run_is_reported_and_the_rules_still_count() {
    let harness = llm_harness(vec![AgentEvent::Failed {
        reason: FailureReason::AuthExpired,
        message_key: "errors.agent.auth_expired".to_string(),
    }]);
    harness.repo.seed_message(message(
        "msg_1",
        "recruiting@acme.com",
        "Hello",
        "Hello there.",
    ));

    let error = harness
        .service
        .classify(Some("claude_code"), None)
        .unwrap_err();

    assert!(matches!(
        error,
        StatusProposalsError::Run(FailureReason::AuthExpired)
    ));
}

// ---- AC5: auto-confirm and undo ------------------------------------------

#[test]
fn auto_confirm_applies_a_high_confidence_rejection_and_undo_reverses_it() {
    let harness = harness_with(
        FakeClassifier::scripted(vec![matched(
            ReplyClass::Rejection,
            0.97,
            "Unfortunately, no.",
        )]),
        Arc::new(FakeAgent::default()),
        &[
            (SETTING_MODE, "llm"),
            (SETTING_DISCLOSURE_ACK, "true"),
            (SETTING_AUTO_CONFIRM, "true"),
        ],
    );
    seed_application(&harness, AppStatus::Applied);

    let outcome = harness.service.classify(None, None).unwrap();
    assert_eq!(outcome.auto_applied, 1);
    assert_eq!(outcome.auto_applied_ids.len(), 1, "the screen offers Undo");
    assert_eq!(
        harness.apply.stored_status("app_1"),
        Some(AppStatus::Rejected),
        "a high-confidence rejection is applied at once (AC5)"
    );
    // It is logged, and out of the pending queue.
    assert_eq!(harness.apply.events().len(), 1);
    assert!(harness.service.proposals().unwrap().is_empty());

    let proposal = harness.repo.proposals().remove(0);
    harness.service.undo(&proposal.id).unwrap();

    assert_eq!(
        harness.apply.stored_status("app_1"),
        Some(AppStatus::Applied)
    );
    assert_eq!(harness.apply.events().len(), 2);
    assert_eq!(
        harness
            .repo
            .proposal_get(&proposal.id)
            .unwrap()
            .unwrap()
            .state,
        ProposalState::Undone
    );
}

#[test]
fn auto_confirm_off_leaves_the_proposal_waiting() {
    let harness = harness(FakeClassifier::scripted(vec![matched(
        ReplyClass::Rejection,
        0.97,
        "Unfortunately, no.",
    )]));
    seed_application(&harness, AppStatus::Applied);

    let outcome = harness.service.classify(None, None).unwrap();

    assert_eq!(outcome.auto_applied, 0);
    assert_eq!(
        harness.apply.stored_status("app_1"),
        Some(AppStatus::Applied)
    );
    assert_eq!(harness.service.proposals().unwrap().len(), 1);
}

#[test]
fn auto_confirm_never_touches_an_ambiguous_match() {
    let harness = harness_with(
        FakeClassifier::scripted(vec![matched(
            ReplyClass::Rejection,
            0.97,
            "Unfortunately, no.",
        )]),
        Arc::new(FakeAgent::default()),
        &[
            (SETTING_MODE, "llm"),
            (SETTING_DISCLOSURE_ACK, "true"),
            (SETTING_AUTO_CONFIRM, "true"),
        ],
    );
    seed_application(&harness, AppStatus::Applied);
    // A second Acme Application makes the match ambiguous.
    harness
        .apply
        .seed_application(application("app_2", "pst_1", AppStatus::Applied, "Acme"));

    let outcome = harness.service.classify(None, None).unwrap();

    assert_eq!(outcome.auto_applied, 0);
    assert!(harness.service.proposals().unwrap()[0].needs_pick);
}

#[test]
fn undo_refuses_when_the_card_moved_after_the_proposal() {
    let harness = harness_with(
        FakeClassifier::scripted(vec![matched(
            ReplyClass::Rejection,
            0.97,
            "Unfortunately, no.",
        )]),
        Arc::new(FakeAgent::default()),
        &[
            (SETTING_MODE, "llm"),
            (SETTING_DISCLOSURE_ACK, "true"),
            (SETTING_AUTO_CONFIRM, "true"),
        ],
    );
    seed_application(&harness, AppStatus::Applied);
    harness.service.classify(None, None).unwrap();
    let proposal = harness.repo.proposals().remove(0);

    // The user moved the card on by hand: undo would be a guess.
    harness.apply.set_status("app_1", AppStatus::Interview);

    let error = harness.service.undo(&proposal.id).unwrap_err();
    assert!(matches!(error, StatusProposalsError::StaleCard));
}

#[test]
fn a_dismissed_proposal_moves_nothing() {
    let harness = harness(FakeClassifier::scripted(vec![matched(
        ReplyClass::InterviewInvite,
        0.97,
        "Interview next week.",
    )]));
    seed_application(&harness, AppStatus::Applied);
    harness.service.classify(None, None).unwrap();
    let id = harness.service.proposals().unwrap()[0].id.clone();

    harness.service.dismiss(&id).unwrap();

    assert!(harness.service.proposals().unwrap().is_empty());
    assert_eq!(
        harness.apply.stored_status("app_1"),
        Some(AppStatus::Applied)
    );
    assert!(harness.apply.events().is_empty());
    // The decision is final for this build.
    let error = harness.service.approve(&id, None).unwrap_err();
    assert!(matches!(error, StatusProposalsError::NotPending(_)));
}

#[test]
fn a_batch_keeps_the_moves_that_succeeded_when_one_fails() {
    let harness = harness(FakeClassifier::scripted(vec![matched(
        ReplyClass::InterviewInvite,
        0.97,
        "Interview next week.",
    )]));
    seed_application(&harness, AppStatus::Applied);
    harness.service.classify(None, None).unwrap();
    let valid = harness.service.proposals().unwrap()[0].id.clone();

    // F1: the second item's row cannot be read (a store error). The old code
    // returned that error and dropped the move that had already landed.
    harness
        .repo
        .seed_message(message("msg_2", "recruiting@acme.com", "Update", "Text."));
    harness
        .repo
        .proposal_insert(&StatusProposal {
            id: "sp_broken".to_string(),
            message_id: "msg_2".to_string(),
            application_id: Some("app_1".to_string()),
            from_status: Some("applied".to_string()),
            to_status: "rejected".to_string(),
            class: ReplyClass::Rejection,
            confidence: 0.97,
            quoted_line: "Text.".to_string(),
            untrusted_origin: false,
            state: ProposalState::Pending,
            created_at: NOW.to_string(),
            decided_at: None,
        })
        .unwrap();
    harness.repo.fail_get_for("sp_broken");

    let outcome = harness
        .service
        .approve_many(&[valid.clone(), "sp_broken".to_string()])
        .unwrap();

    // A broken item never hides the moves that did land.
    assert_eq!(outcome.approved, vec![valid]);
    assert_eq!(outcome.failed, vec!["sp_broken".to_string()]);
    assert_eq!(
        harness.apply.stored_status("app_1"),
        Some(AppStatus::Interview)
    );
}
