//! Triage use-case tests (S6.6): Save, Dismiss/Undo, the Excluded override and
//! the Preferences suggestion of FR-67 (AC1..AC3).

use std::sync::Arc;

use boba_domain::application::{AppStatus, Application};
use boba_domain::posting::lifecycle::PostingLifecycle;
use boba_domain::posting::{ApplyHints, Offer, Provenance, WorkMode};
use boba_domain::scoring::gates::{
    Confidence as GateConfidence, Evidence, GateKind, GateReason, GateReport, GateResult,
    Verdict as GateVerdict,
};
use boba_domain::scoring::rank::{CardVersions, ScoreCard, ScoreCardRow, Verdict as Band};
use boba_domain::triage::{DismissReason, SuggestionDecisionStatus};

use crate::applications::testing::FakeApplyRepo;
use crate::ingest::normalize::testing::FakeIngestionRepo;
use crate::scoring::testing::FakeScoringRepo;
use crate::system::FixedClock;

use super::testing::FakeTriageRepo;
use super::*;

const NOW: &str = "2026-10-17T09:00:00Z";

fn offer(id: &str, company: &str, work_mode: WorkMode) -> Offer {
    Offer {
        id: id.to_string(),
        source_id: "greenhouse".to_string(),
        external_id: format!("gh-{id}"),
        title: "Senior Data Engineer".to_string(),
        company: company.to_string(),
        locations: vec!["Remote, Spain".to_string()],
        work_mode,
        money: None,
        posted_at: None,
        deadline: None,
        first_seen: "2026-10-04T00:00:00Z".to_string(),
        last_seen: "2026-10-16T09:00:00Z".to_string(),
        lang: "en".to_string(),
        description_html: "<p>dbt</p>".to_string(),
        description_text: "dbt and Airflow".to_string(),
        requirements: vec![],
        apply: ApplyHints {
            url: "https://jobs.example.com/1".to_string(),
            email: None,
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
        fetched_at: offer.first_seen.clone(),
    }
}

fn veto_card(posting_id: &str) -> ScoreCardRow {
    ScoreCardRow::new(
        format!("sc_{posting_id}"),
        posting_id,
        CardVersions {
            profile_version: 1,
            prefs_version: 1,
            model_version: 1,
        },
        ScoreCard {
            model_version: 1,
            factors: Vec::new(),
            fit: Some(40.0),
            quality: Some(60.0),
            rank: 47.0,
            verdict: Band::Weak,
            confidence: GateConfidence::Medium,
        },
        GateReport {
            results: vec![GateResult {
                gate: GateKind::Eligibility,
                verdict: GateVerdict::Veto,
                reason: GateReason::RequirementNotMet,
                evidence: Some(Evidence::Field("work_authorization".to_string())),
                confidence: GateConfidence::High,
            }],
        },
        NOW,
    )
}

struct Fixture {
    service: TriageService,
    postings: Arc<FakeIngestionRepo>,
    scoring: Arc<FakeScoringRepo>,
    apply: Arc<FakeApplyRepo>,
    triage: Arc<FakeTriageRepo>,
}

fn fixture() -> Fixture {
    let postings = Arc::new(FakeIngestionRepo::default());
    let scoring = Arc::new(FakeScoringRepo::default());
    let apply = Arc::new(FakeApplyRepo::default());
    let triage = Arc::new(FakeTriageRepo::default());
    let service = TriageService::new(
        triage.clone(),
        postings.clone(),
        scoring.clone(),
        apply.clone(),
        Arc::new(FixedClock::new(NOW)),
    );
    Fixture {
        service,
        postings,
        scoring,
        apply,
        triage,
    }
}

impl Fixture {
    /// Store one Offer (with its lifecycle row, as ingestion writes them).
    fn seed(&self, offer: &Offer) {
        self.postings
            .insert_offer(offer, &provenance(offer))
            .expect("store the posting");
    }

    fn dismissed_at(&self, posting_id: &str) -> Option<String> {
        self.postings
            .lifecycle(posting_id)
            .expect("lifecycle read")
            .and_then(|row: PostingLifecycle| row.dismissed_at)
    }
}

// AC1: Save creates an Application in Saved, born with its first status event.
#[test]
fn save_creates_one_application_in_saved_with_its_birth_event() {
    let fixture = fixture();
    fixture.seed(&offer("pst_1", "Boba Labs", WorkMode::Remote));

    let saved = fixture.service.save("pst_1").expect("save");

    assert!(saved.created);
    let application: Application = fixture.apply.applications().unwrap().remove(0);
    assert_eq!(application.id, saved.application_id);
    assert_eq!(application.posting_id, "pst_1");
    assert_eq!(application.status, AppStatus::Saved);
    assert!(application.offer_snapshot_json.contains("Boba Labs"));

    let events = fixture.apply.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].from_status, None);
    assert_eq!(events[0].to_status, AppStatus::Saved);
    assert_eq!(
        events[0].source,
        boba_domain::application::StatusSource::User
    );
}

// AC1: Save is idempotent — the same Offer saved twice is one Application.
#[test]
fn saving_the_same_offer_twice_keeps_one_application() {
    let fixture = fixture();
    fixture.seed(&offer("pst_1", "Boba Labs", WorkMode::Remote));

    let first = fixture.service.save("pst_1").expect("first save");
    let second = fixture.service.save("pst_1").expect("second save");

    assert!(!second.created);
    assert_eq!(first.application_id, second.application_id);
    assert_eq!(fixture.apply.applications().unwrap().len(), 1);
}

#[test]
fn saving_an_offer_the_store_does_not_hold_is_unknown() {
    let fixture = fixture();
    let error = fixture.service.save("pst_gone").expect_err("unknown");
    assert_eq!(error.message_key(), "errors.inbox_actions.unknown_offer");
}

// AC1: Dismiss moves the lifecycle and records the reason (FR-66/FR-67).
#[test]
fn dismiss_records_the_reason_and_the_lifecycle() {
    let fixture = fixture();
    fixture.seed(&offer("pst_1", "Boba Labs", WorkMode::Remote));

    let outcome = fixture
        .service
        .dismiss(&["pst_1".to_string()], Some(DismissReason::Salary))
        .expect("dismiss");

    assert_eq!(outcome.dismissed, vec!["pst_1".to_string()]);
    assert!(outcome.failed.is_empty());
    assert_eq!(fixture.dismissed_at("pst_1").as_deref(), Some(NOW));
    let rows = fixture.triage.dismissals();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].posting_id, "pst_1");
    assert_eq!(rows[0].reason, DismissReason::Salary);
}

// AC1: the reason is optional; dismissing without one still dismisses.
#[test]
fn dismiss_without_a_reason_is_still_a_dismissal() {
    let fixture = fixture();
    fixture.seed(&offer("pst_1", "Boba Labs", WorkMode::Remote));

    fixture
        .service
        .dismiss(&["pst_1".to_string()], None)
        .expect("dismiss");

    assert_eq!(fixture.dismissed_at("pst_1").as_deref(), Some(NOW));
    assert!(fixture.triage.dismissals().is_empty());
}

// AC1: bulk dismiss dismisses what it can and names the stale card.
#[test]
fn bulk_dismiss_reports_a_stale_card_instead_of_failing_them_all() {
    let fixture = fixture();
    fixture.seed(&offer("pst_1", "Boba Labs", WorkMode::Remote));
    fixture.seed(&offer("pst_2", "Boba Labs", WorkMode::Remote));

    let outcome = fixture
        .service
        .dismiss(
            &[
                "pst_1".to_string(),
                "pst_gone".to_string(),
                "pst_2".to_string(),
            ],
            Some(DismissReason::NotInterested),
        )
        .expect("bulk dismiss");

    assert_eq!(
        outcome.dismissed,
        vec!["pst_1".to_string(), "pst_2".to_string()]
    );
    assert_eq!(outcome.failed, vec!["pst_gone".to_string()]);
    assert_eq!(fixture.dismissed_at("pst_1").as_deref(), Some(NOW));
    assert_eq!(fixture.dismissed_at("pst_2").as_deref(), Some(NOW));
    assert_eq!(fixture.triage.dismissals().len(), 2);
}

// AC1: Undo puts the Offer back exactly as it was.
#[test]
fn undo_clears_the_dismissal_and_its_reason() {
    let fixture = fixture();
    fixture.seed(&offer("pst_1", "Boba Labs", WorkMode::Remote));
    fixture
        .service
        .dismiss(&["pst_1".to_string()], Some(DismissReason::Company))
        .expect("dismiss");

    let undone = fixture.service.undo_dismiss("pst_1").expect("undo");

    assert!(undone);
    assert_eq!(fixture.dismissed_at("pst_1"), None);
    assert!(fixture.triage.dismissals().is_empty());
    // A second Undo has nothing to do and says so.
    assert!(!fixture.service.undo_dismiss("pst_1").expect("undo again"));
}

// AC2: overriding a VETO records the Gate the user overruled.
#[test]
fn override_records_the_vetoed_gate() {
    let fixture = fixture();
    fixture.seed(&offer("pst_1", "Boba Labs", WorkMode::Remote));
    fixture
        .scoring
        .insert_card(&veto_card("pst_1"))
        .expect("store the card");

    let outcome = fixture.service.override_veto("pst_1").expect("override");

    assert_eq!(outcome.gate, "eligibility");
    let overrides = fixture.triage.overrides();
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0].posting_id, "pst_1");
    assert_eq!(overrides[0].gate, "eligibility");
}

// AC2: an Offer no Gate vetoed is not an override, it is a stale card.
#[test]
fn overriding_an_offer_that_is_not_excluded_is_refused() {
    let fixture = fixture();
    fixture.seed(&offer("pst_1", "Boba Labs", WorkMode::Remote));

    let error = fixture.service.override_veto("pst_1").expect_err("no veto");
    assert_eq!(error.message_key(), "errors.inbox_actions.not_excluded");
    assert!(fixture.triage.overrides().is_empty());
}

// AC3: three dismissals for one company suggest excluding it.
#[test]
fn three_dismissals_of_one_company_suggest_excluding_it() {
    let fixture = fixture();
    for id in ["pst_1", "pst_2", "pst_3"] {
        fixture.seed(&offer(id, "Boba Labs", WorkMode::Remote));
    }

    let mut suggestion = None;
    for id in ["pst_1", "pst_2", "pst_3"] {
        suggestion = fixture
            .service
            .dismiss(&[id.to_string()], Some(DismissReason::Company))
            .expect("dismiss")
            .suggestion;
    }

    let suggestion = suggestion.expect("the third dismissal suggests the change");
    assert_eq!(suggestion.id, "exclude_company:boba labs");
    assert_eq!(suggestion.kind, "exclude_company");
    assert_eq!(suggestion.company.as_deref(), Some("Boba Labs"));
    assert_eq!(suggestion.count, 3);
}

// AC3: two dismissals are not yet a pattern.
#[test]
fn two_dismissals_of_one_company_are_not_a_suggestion() {
    let fixture = fixture();
    for id in ["pst_1", "pst_2"] {
        fixture.seed(&offer(id, "Boba Labs", WorkMode::Remote));
    }

    for id in ["pst_1", "pst_2"] {
        let outcome = fixture
            .service
            .dismiss(&[id.to_string()], Some(DismissReason::Company))
            .expect("dismiss");
        assert!(outcome.suggestion.is_none());
    }
}

// AC3: the onsite suggestion counts only the onsite dismissals.
#[test]
fn three_onsite_dismissals_suggest_excluding_onsite() {
    let fixture = fixture();
    fixture.seed(&offer("pst_1", "A", WorkMode::Onsite));
    fixture.seed(&offer("pst_2", "B", WorkMode::Onsite));
    fixture.seed(&offer("pst_3", "C", WorkMode::Remote));
    fixture.seed(&offer("pst_4", "D", WorkMode::Onsite));

    let mut suggestion = None;
    for id in ["pst_1", "pst_2", "pst_3"] {
        suggestion = fixture
            .service
            .dismiss(&[id.to_string()], Some(DismissReason::Location))
            .expect("dismiss")
            .suggestion;
    }
    // Two onsite ones so far: not yet.
    assert!(suggestion.is_none());

    let suggestion = fixture
        .service
        .dismiss(&["pst_4".to_string()], Some(DismissReason::Location))
        .expect("dismiss")
        .suggestion
        .expect("three onsite dismissals");
    assert_eq!(suggestion.id, "exclude_onsite");
    assert_eq!(suggestion.kind, "exclude_onsite");
    assert_eq!(suggestion.company, None);
    assert_eq!(suggestion.count, 3);
}

// AC3: a reason no preference can express never suggests a change.
#[test]
fn a_reason_preferences_cannot_capture_never_suggests() {
    let fixture = fixture();
    for id in ["pst_1", "pst_2", "pst_3"] {
        fixture.seed(&offer(id, "Boba Labs", WorkMode::Remote));
    }
    for id in ["pst_1", "pst_2", "pst_3"] {
        let outcome = fixture
            .service
            .dismiss(&[id.to_string()], Some(DismissReason::Salary))
            .expect("dismiss");
        assert!(outcome.suggestion.is_none());
    }
}

// AC3: an answered suggestion is not asked again.
#[test]
fn a_decided_suggestion_is_not_offered_again() {
    let fixture = fixture();
    for id in ["pst_1", "pst_2", "pst_3"] {
        fixture.seed(&offer(id, "Boba Labs", WorkMode::Remote));
    }
    for id in ["pst_1", "pst_2"] {
        fixture
            .service
            .dismiss(&[id.to_string()], Some(DismissReason::Company))
            .expect("dismiss");
    }
    let suggestion = fixture
        .service
        .dismiss(&["pst_3".to_string()], Some(DismissReason::Company))
        .expect("dismiss")
        .suggestion
        .expect("suggested once");

    fixture
        .service
        .decide_suggestion(&suggestion.id, SuggestionDecisionStatus::Ignored)
        .expect("ignore");

    fixture.seed(&offer("pst_4", "Boba Labs", WorkMode::Remote));
    let outcome = fixture
        .service
        .dismiss(&["pst_4".to_string()], Some(DismissReason::Company))
        .expect("dismiss");
    assert!(outcome.suggestion.is_none());
    assert_eq!(fixture.triage.decisions().len(), 1);
}

#[test]
fn deciding_an_unknown_suggestion_is_refused() {
    let fixture = fixture();
    let error = fixture
        .service
        .decide_suggestion("delete_everything", SuggestionDecisionStatus::Accepted)
        .expect_err("unknown");
    assert_eq!(
        error.message_key(),
        "errors.inbox_actions.unknown_suggestion"
    );
}

/// FR-54's override is visible to the Inbox read model.
#[test]
fn overrides_are_readable_for_the_inbox() {
    let fixture = fixture();
    fixture.seed(&offer("pst_1", "Boba Labs", WorkMode::Remote));
    fixture
        .scoring
        .insert_card(&veto_card("pst_1"))
        .expect("store the card");
    fixture.service.override_veto("pst_1").expect("override");

    let overrides = fixture.service.overrides().expect("overrides");
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0].posting_id, "pst_1");
}
