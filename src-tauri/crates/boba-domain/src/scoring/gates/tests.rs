//! Golden and unit tests for the Gates (S6.1 / FR-53 / AD-8).
//!
//! AC3 asks for golden coverage of "US only", "EU work permit required", an
//! undeclared required language and a salary below a hard floor; AC1 and AC2 are
//! covered by the tests around them.

use super::*;
use crate::posting::{ApplyHints, Money, PayPeriod, WorkMode};

/// The posting every test starts from: remote, USD 6,000 a month, no deadline.
fn offer(description: &str) -> Offer {
    Offer {
        id: "pst_01".to_string(),
        source_id: "src_01".to_string(),
        external_id: "ext-1".to_string(),
        title: "Senior Backend Engineer".to_string(),
        company: "Nimbus".to_string(),
        locations: vec!["Remote".to_string()],
        work_mode: WorkMode::Remote,
        money: Some(money(600_000, "USD", PayPeriod::Month)),
        posted_at: Some("2026-10-10T00:00:00Z".to_string()),
        deadline: None,
        first_seen: "2026-10-10T00:00:00Z".to_string(),
        last_seen: "2026-10-15T00:00:00Z".to_string(),
        lang: "en".to_string(),
        description_html: String::new(),
        description_text: description.to_string(),
        requirements: Vec::new(),
        apply: ApplyHints {
            url: "https://example.com/job".to_string(),
            email: None,
        },
    }
}

fn money(amount_minor: i64, currency: &str, period: PayPeriod) -> Money {
    Money {
        amount_minor,
        currency: currency.to_string(),
        period,
    }
}

/// The profile + preferences a gate run reads (FR-7, FR-11..13).
fn candidate() -> GateInputs {
    GateInputs {
        work_authorizations: vec![WorkAuthorization {
            region: "eu".to_string(),
            permit: PermitKind::WorkPermit,
        }],
        languages: vec![
            DeclaredLanguage {
                language: "en".to_string(),
                level: Cefr::B2,
            },
            DeclaredLanguage {
                language: "es".to_string(),
                level: Cefr::C2,
            },
        ],
        salary_floor: None,
        requires_known_compensation: false,
        work_modes_allowed: vec![WorkMode::Remote, WorkMode::Hybrid, WorkMode::Onsite],
        locations: vec!["madrid".to_string()],
        relocate_ok: false,
        closing_soon_days: 7,
        now: "2026-10-16T12:00:00Z".to_string(),
    }
}

/// A hard Salary Floor of USD 4,000 a month (FR-7).
fn hard_floor(amount_minor: i64, currency: &str, period: PayPeriod) -> SalaryFloor {
    SalaryFloor {
        money: money(amount_minor, currency, period),
        hardness: Hardness::Hard,
    }
}

fn gate(report: &GateReport, kind: GateKind) -> &GateResult {
    report
        .gate(kind)
        .unwrap_or_else(|| panic!("{kind:?} is always reported"))
}

fn quote(result: &GateResult) -> &str {
    match &result.evidence {
        Some(Evidence::Quoted(span)) => span.text.as_str(),
        other => panic!("expected a quoted span, got {other:?}"),
    }
}

// --- AC1: five gates, each with a verdict, a reason and a Confidence --------

#[test]
fn every_gate_reports_a_verdict_and_a_confidence_in_a_stable_order() {
    let report = run(&offer("A calm posting with no requirements."), &candidate());

    assert_eq!(
        report.results.iter().map(|r| r.gate).collect::<Vec<_>>(),
        GateKind::ALL.to_vec(),
        "one result per gate, in a stable order"
    );
    assert_eq!(report.results.len(), 5);
    for kind in [
        GateKind::Language,
        GateKind::Location,
        GateKind::Compensation,
        GateKind::Deadline,
    ] {
        assert_eq!(
            gate(&report, kind).verdict,
            Verdict::Pass,
            "{kind:?} should pass on a clean posting"
        );
    }
    // AC2: eligibility was never stated, so it is flagged, never passed.
    let eligibility = gate(&report, GateKind::Eligibility);
    assert_eq!(
        (
            eligibility.verdict,
            eligibility.reason,
            eligibility.confidence
        ),
        (Verdict::Flag, GateReason::Unverified, Confidence::Low)
    );
}

#[test]
fn verdicts_order_pass_below_flag_below_veto() {
    assert!(Verdict::Pass < Verdict::Flag);
    assert!(Verdict::Flag < Verdict::Veto);
    assert_eq!(
        [Verdict::Veto, Verdict::Pass, Verdict::Flag]
            .into_iter()
            .max(),
        Some(Verdict::Veto)
    );
}

#[test]
fn the_report_verdict_is_the_worst_of_its_gates() {
    let report = run(
        &offer("Must be authorized to work in the US."),
        &candidate(),
    );
    assert_eq!(gate(&report, GateKind::Eligibility).verdict, Verdict::Veto);
    assert_eq!(report.verdict(), Verdict::Veto);
}

#[test]
fn the_vocabulary_round_trips_through_its_stored_text() {
    assert_eq!(
        GateKind::ALL.map(GateKind::as_str),
        [
            "eligibility",
            "language",
            "location",
            "compensation",
            "deadline"
        ]
    );
    assert_eq!(
        Cefr::ALL.map(Cefr::as_str),
        ["a1", "a2", "b1", "b2", "c1", "c2"]
    );
    assert_eq!(Cefr::parse("C1"), Some(Cefr::C1));
    assert_eq!(Cefr::parse("b2"), Some(Cefr::B2));
    assert_eq!(Cefr::parse("d1"), None);
    assert_eq!(PermitKind::WorkPermit.as_str(), "work_permit");
    assert_eq!(PermitKind::Clearance.as_str(), "clearance");
}

// --- AC2: silence is never PASS for eligibility ----------------------------

#[test]
fn silent_eligibility_is_unverified_and_never_pass() {
    let report = run(
        &offer("We are a friendly, remote-first team."),
        &candidate(),
    );
    let eligibility = gate(&report, GateKind::Eligibility);

    assert_eq!(eligibility.verdict, Verdict::Flag, "{eligibility:?}");
    assert_eq!(eligibility.reason, GateReason::Unverified);
    assert!(eligibility.evidence.is_none(), "nothing was quoted");
    assert_eq!(eligibility.confidence, Confidence::Low);
}

#[test]
fn a_declared_permit_satisfies_the_stated_requirement() {
    let report = run(
        &offer("Must be authorized to work in the EU. Hybrid in Madrid."),
        &candidate(),
    );
    let eligibility = gate(&report, GateKind::Eligibility);

    assert_eq!(eligibility.verdict, Verdict::Pass, "{eligibility:?}");
    assert_eq!(quote(eligibility), "Must be authorized to work in the EU");
    assert_eq!(eligibility.confidence, Confidence::High);
}

// --- AC3: the four golden postings ----------------------------------------

// Golden 1: "US only" is a requirement the candidate has not declared.
#[test]
fn golden_us_only_vetoes_a_candidate_without_us_authorization() {
    let report = run(
        &offer("US only. Senior backend role, remote."),
        &candidate(),
    );
    let eligibility = gate(&report, GateKind::Eligibility);

    assert_eq!(eligibility.verdict, Verdict::Veto, "{eligibility:?}");
    assert_eq!(eligibility.reason, GateReason::RequirementNotMet);
    assert_eq!(quote(eligibility), "US only");
    assert_eq!(eligibility.confidence, Confidence::High);
}

// Golden 2: the same demand passes when the candidate declares it.
#[test]
fn golden_eu_work_permit_required_passes_with_a_declared_permit() {
    let report = run(&offer("EU work permit required."), &candidate());
    let eligibility = gate(&report, GateKind::Eligibility);

    assert_eq!(eligibility.verdict, Verdict::Pass, "{eligibility:?}");
    assert_eq!(quote(eligibility), "EU work permit required");
}

#[test]
fn a_citizenship_covers_a_plain_work_permit_demand() {
    let mut inputs = candidate();
    inputs.work_authorizations = vec![WorkAuthorization {
        region: "us".to_string(),
        permit: PermitKind::Citizenship,
    }];
    let report = run(&offer("Must be authorized to work in the US."), &inputs);

    assert_eq!(gate(&report, GateKind::Eligibility).verdict, Verdict::Pass);
}

#[test]
fn a_clearance_demand_is_not_covered_by_a_work_permit() {
    let report = run(
        &offer("An active security clearance is required."),
        &candidate(),
    );
    assert_eq!(gate(&report, GateKind::Eligibility).verdict, Verdict::Veto);
}

// Golden 3: a required language the candidate never declared.
#[test]
fn golden_undeclared_required_language_vetoes() {
    let report = run(
        &offer("Fluent German required for our Berlin team."),
        &candidate(),
    );
    let language = gate(&report, GateKind::Language);

    assert_eq!(language.verdict, Verdict::Veto, "{language:?}");
    assert_eq!(language.reason, GateReason::RequirementNotMet);
    assert_eq!(
        quote(language),
        "Fluent German required for our Berlin team"
    );
}

#[test]
fn a_declared_level_below_the_stated_bar_flags() {
    let report = run(&offer("English (C1) is required."), &candidate());
    let language = gate(&report, GateKind::Language);

    assert_eq!(language.verdict, Verdict::Flag, "{language:?}");
    assert_eq!(language.reason, GateReason::BelowRequiredLevel);
    assert_eq!(quote(language), "English (C1) is required");
}

#[test]
fn a_language_at_the_stated_bar_passes() {
    let report = run(
        &offer("Working knowledge of English is required."),
        &candidate(),
    );
    let language = gate(&report, GateKind::Language);

    assert_eq!(language.verdict, Verdict::Pass, "{language:?}");
    assert_eq!(language.confidence, Confidence::Medium, "levels are fuzzy");
}

#[test]
fn a_working_language_named_without_a_requirement_is_not_gated() {
    // "require" and "English" are in different sentences: not a demand.
    let report = run(
        &offer("We require 3 years of backend experience. We speak English daily."),
        &candidate(),
    );
    assert_eq!(gate(&report, GateKind::Language).verdict, Verdict::Pass);
}

// Golden 4: pay under a hard Salary Floor.
#[test]
fn golden_salary_below_a_hard_floor_vetoes() {
    let mut inputs = candidate();
    inputs.salary_floor = Some(hard_floor(400_000, "USD", PayPeriod::Month));
    let mut posting = offer("Competitive salary.");
    posting.money = Some(money(350_000, "USD", PayPeriod::Month));

    let report = run(&posting, &inputs);
    let pay = gate(&report, GateKind::Compensation);

    assert_eq!(pay.verdict, Verdict::Veto, "{pay:?}");
    assert_eq!(pay.reason, GateReason::BelowFloor);
    assert_eq!(pay.confidence, Confidence::High);
}

#[test]
fn pay_above_the_floor_passes() {
    let mut inputs = candidate();
    inputs.salary_floor = Some(hard_floor(400_000, "USD", PayPeriod::Month));
    let report = run(&offer("Competitive salary."), &inputs);

    assert_eq!(gate(&report, GateKind::Compensation).verdict, Verdict::Pass);
}

#[test]
fn a_soft_floor_below_is_a_flag_not_a_veto() {
    let mut inputs = candidate();
    inputs.salary_floor = Some(SalaryFloor {
        money: money(400_000, "USD", PayPeriod::Month),
        hardness: Hardness::Soft,
    });
    let mut posting = offer("Competitive salary.");
    posting.money = Some(money(350_000, "USD", PayPeriod::Month));

    let result = run(&posting, &inputs);
    let pay = gate(&result, GateKind::Compensation);
    assert_eq!(pay.verdict, Verdict::Flag, "{pay:?}");
    assert_eq!(pay.reason, GateReason::BelowFloor);
}

#[test]
fn unknown_pay_flags_and_only_vetoes_when_the_user_requires_it() {
    let mut inputs = candidate();
    inputs.salary_floor = Some(hard_floor(400_000, "USD", PayPeriod::Month));
    let mut posting = offer("Competitive salary.");
    posting.money = None;

    let unverified = gate(&run(&posting, &inputs), GateKind::Compensation).clone();
    assert_eq!(unverified.verdict, Verdict::Flag, "{unverified:?}");
    assert_eq!(unverified.reason, GateReason::Unverified);

    inputs.requires_known_compensation = true;
    let strict = run(&posting, &inputs);
    let vetoed = gate(&strict, GateKind::Compensation);
    assert_eq!(vetoed.verdict, Verdict::Veto, "{vetoed:?}");
    assert_eq!(vetoed.reason, GateReason::RequirementNotMet);
}

#[test]
fn a_floor_in_another_currency_is_unverified_not_imputed() {
    let mut inputs = candidate();
    inputs.salary_floor = Some(hard_floor(400_000, "EUR", PayPeriod::Month));

    let result = run(&offer("Competitive salary."), &inputs);
    let pay = gate(&result, GateKind::Compensation);

    assert_eq!(pay.verdict, Verdict::Flag, "{pay:?}");
    assert_eq!(
        pay.reason,
        GateReason::Unverified,
        "no FX conversion (AD-8)"
    );
}

#[test]
fn a_floor_in_another_period_is_unverified() {
    let mut inputs = candidate();
    inputs.salary_floor = Some(hard_floor(4_000_000, "USD", PayPeriod::Year));

    let pay = run(&offer("Competitive salary."), &inputs);
    assert_eq!(
        gate(&pay, GateKind::Compensation).reason,
        GateReason::Unverified
    );
}

#[test]
fn no_floor_and_no_posted_pay_are_a_pass_with_no_evidence() {
    let mut posting = offer("Competitive salary.");
    posting.money = None;

    let result = run(&posting, &candidate());
    let pay = gate(&result, GateKind::Compensation);
    assert_eq!(pay.verdict, Verdict::Pass);
    assert!(pay.evidence.is_none(), "nothing was checked: {pay:?}");
}

// --- Location and work mode ------------------------------------------------

#[test]
fn remote_passes_regardless_of_location() {
    let report = run(&offer("Remote from anywhere."), &candidate());
    let location = gate(&report, GateKind::Location);

    assert_eq!(location.verdict, Verdict::Pass, "{location:?}");
    assert_eq!(location.evidence, Some(Evidence::Field("work_mode".into())));
}

#[test]
fn a_work_mode_the_user_excluded_vetoes() {
    let mut posting = offer("Onsite in Madrid.");
    posting.work_mode = WorkMode::Onsite;
    posting.locations = vec!["Madrid".to_string()];
    let mut inputs = candidate();
    inputs.work_modes_allowed = vec![WorkMode::Remote];

    let report = run(&posting, &inputs);
    let location = gate(&report, GateKind::Location);
    assert_eq!(location.verdict, Verdict::Veto, "{location:?}");
    assert_eq!(location.reason, GateReason::RequirementNotMet);
}

#[test]
fn onsite_in_an_accepted_location_passes() {
    let mut posting = offer("Onsite in Madrid.");
    posting.work_mode = WorkMode::Onsite;
    posting.locations = vec!["Madrid, Spain".to_string()];

    let report = run(&posting, &candidate());
    assert_eq!(gate(&report, GateKind::Location).verdict, Verdict::Pass);
}

#[test]
fn onsite_outside_the_accepted_locations_vetoes_unless_the_user_relocates() {
    let mut posting = offer("Onsite in Austin.");
    posting.work_mode = WorkMode::Onsite;
    posting.locations = vec!["Austin, TX".to_string()];

    let mut inputs = candidate();
    inputs.locations = vec!["madrid".to_string()];
    inputs.relocate_ok = false;
    let vetoed = run(&posting, &inputs);
    assert_eq!(gate(&vetoed, GateKind::Location).verdict, Verdict::Veto);

    inputs.relocate_ok = true;
    let allowed = run(&posting, &inputs);
    assert_eq!(gate(&allowed, GateKind::Location).verdict, Verdict::Pass);
}

#[test]
fn an_unknown_work_mode_flags() {
    let mut posting = offer("Join our team.");
    posting.work_mode = WorkMode::Unknown;

    let report = run(&posting, &candidate());
    let location = gate(&report, GateKind::Location);
    assert_eq!(location.verdict, Verdict::Flag, "{location:?}");
    assert_eq!(location.reason, GateReason::Unverified);
}

// --- Deadline --------------------------------------------------------------

#[test]
fn a_far_deadline_passes_and_no_deadline_has_no_evidence() {
    let mut posting = offer("Apply soon.");
    posting.deadline = Some("2026-11-30T00:00:00Z".to_string());
    let report = run(&posting, &candidate());
    assert_eq!(gate(&report, GateKind::Deadline).verdict, Verdict::Pass);

    let open = run(&offer("Always open."), &candidate());
    let deadline = gate(&open, GateKind::Deadline);
    assert_eq!(deadline.verdict, Verdict::Pass);
    assert!(deadline.evidence.is_none(), "{deadline:?}");
}

#[test]
fn a_deadline_inside_the_window_flags_closing_soon() {
    let mut posting = offer("Apply soon.");
    posting.deadline = Some("2026-10-20T00:00:00Z".to_string());

    let report = run(&posting, &candidate());
    let deadline = gate(&report, GateKind::Deadline);
    assert_eq!(deadline.verdict, Verdict::Flag, "{deadline:?}");
    assert_eq!(deadline.reason, GateReason::ClosingSoon);
}

#[test]
fn an_expired_deadline_vetoes() {
    let mut posting = offer("Apply soon.");
    posting.deadline = Some("2026-10-01T00:00:00Z".to_string());

    let report = run(&posting, &candidate());
    let deadline = gate(&report, GateKind::Deadline);
    assert_eq!(deadline.verdict, Verdict::Veto, "{deadline:?}");
    assert_eq!(deadline.reason, GateReason::Expired);
}

#[test]
fn a_deadline_no_clock_can_read_is_unverified() {
    let mut posting = offer("Apply soon.");
    posting.deadline = Some("next Friday".to_string());

    let report = run(&posting, &candidate());
    assert_eq!(
        gate(&report, GateKind::Deadline).reason,
        GateReason::Unverified
    );
}

// --- Evidence spans --------------------------------------------------------

#[test]
fn quoted_spans_point_back_into_the_posting_text() {
    let text = "Requisitos: Portugués fluido requerido. Must be authorized to work in the US.";
    let posting = offer(text);
    let report = run(&posting, &candidate());

    for result in report.results.iter() {
        if let Some(Evidence::Quoted(span)) = &result.evidence {
            assert_eq!(&text[span.start..span.end], span.text, "{result:?}");
            assert!(
                !span.text.is_empty() && !span.text.starts_with(' '),
                "{span:?}"
            );
        }
    }
    assert_eq!(
        quote(gate(&report, GateKind::Eligibility)),
        "Must be authorized to work in the US"
    );
    assert_eq!(
        quote(gate(&report, GateKind::Language)),
        "Requisitos: Portugués fluido requerido"
    );
}

#[test]
fn the_scan_folds_case_and_keeps_offsets_on_char_boundaries() {
    let text = "Requisitos: Español C2. MUST BE AUTHORIZED TO WORK IN THE US. ¡Gracias!";
    let posting = offer(text);
    let report = run(&posting, &candidate());

    let eligibility = gate(&report, GateKind::Eligibility);
    assert_eq!(quote(eligibility), "MUST BE AUTHORIZED TO WORK IN THE US");
    if let Some(Evidence::Quoted(span)) = &eligibility.evidence {
        assert_eq!(&text[span.start..span.end], span.text);
    }
    assert_eq!(
        gate(&report, GateKind::Language).verdict,
        Verdict::Pass,
        "the candidate declares Spanish at C2"
    );
}
