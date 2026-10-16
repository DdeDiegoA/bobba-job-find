//! Tests for the no-fabrication validator and claim traceability (S7.3,
//! FR-73/FR-74, SM-6, AD-15).
//!
//! AC1: an item with no facts, or with facts the Profile does not have, is
//! rejected and the report is not ready. AC2: every claim of both drafts is
//! traced to the slot it sits in, so the studio can flag the sentences that
//! nothing backs.

use super::*;
use crate::documents::{
    Claim, Contact, CoverLetterDocument, CvDocument, Lang, SCHEMA_VERSION, Section,
};

fn contact() -> Contact {
    Contact {
        full_name: "Ada Lovelace".into(),
        headline: None,
        email: Some("ada@example.com".into()),
        phone: None,
        location: None,
        links: vec![],
    }
}

fn claim(text: &str, fact_ids: &[&str]) -> Claim {
    Claim {
        text: text.into(),
        fact_ids: fact_ids.iter().map(|id| (*id).to_string()).collect(),
    }
}

fn cv(summary: Option<Claim>, sections: Vec<Section>) -> DocumentModel {
    DocumentModel::Cv(CvDocument {
        schema_version: SCHEMA_VERSION,
        lang: Lang::En,
        contact: contact(),
        summary,
        sections,
        declared_gaps: vec![],
    })
}

fn cover(paragraphs: Vec<Claim>) -> DocumentModel {
    DocumentModel::CoverLetter(CoverLetterDocument {
        schema_version: SCHEMA_VERSION,
        lang: Lang::En,
        contact: contact(),
        company: None,
        recipient: None,
        date: None,
        salutation: None,
        paragraphs,
        closing: None,
        declared_gaps: vec![],
    })
}

/// The Profile ids every test validates against, as the app layer would pass
/// them: a closure over the facts in the snapshot.
fn profile(ids: &'static [&'static str]) -> impl Fn(&str) -> bool {
    move |id| ids.contains(&id)
}

// AC1: a claim that names no facts is a fabrication, however plausible it reads.
#[test]
fn a_claim_with_no_facts_is_rejected() {
    let model = cv(
        Some(claim("Results-driven engineer.", &[])),
        vec![Section {
            heading: "Experience".into(),
            items: vec![claim("Shipped an API", &["fact_1"])],
        }],
    );

    let report = validate(&model, &profile(&["fact_1"]));

    let summary = &report.claims[0];
    assert_eq!(summary.slot, ClaimSlot::Summary);
    assert_eq!(summary.rejection, Some(Rejection::NoFacts));
    assert!(!summary.is_backed());
    assert!(
        !report.is_ready(),
        "one unbacked claim blocks Ready and Send (AC1)"
    );
    assert_eq!(report.rejections().len(), 1);
}

// AC1: a fact the Profile does not have is as bad as no fact at all, and the
// report names exactly the ids the user must fix (or confirm into the Profile).
#[test]
fn a_claim_naming_an_unknown_fact_is_rejected() {
    let model = cv(
        None,
        vec![Section {
            heading: "Skills".into(),
            items: vec![claim(
                "Kubernetes and Terraform",
                &["fact_1", "fact_ghost", "fact_ghost_2", "fact_ghost"],
            )],
        }],
    );

    let report = validate(&model, &profile(&["fact_1"]));

    assert_eq!(
        report.claims[0].rejection,
        Some(Rejection::UnknownFacts(vec![
            "fact_ghost".into(),
            "fact_ghost_2".into()
        ])),
        "only the missing ids, each once, in the order the claim named them"
    );
    assert!(!report.is_ready());
}

// AC1 + AC2: a draft whose every claim is backed is ready, and every claim of
// both documents is traced to the slot the studio addresses it by.
#[test]
fn every_claim_is_traced_in_reading_order_and_a_backed_draft_is_ready() {
    let cv_report = validate(
        &cv(
            Some(claim("Backend engineer", &["fact_1"])),
            vec![
                Section {
                    heading: "Experience".into(),
                    items: vec![
                        claim("Shipped an API", &["fact_2"]),
                        claim("Led a team of four", &["fact_3"]),
                    ],
                },
                Section {
                    heading: "Education".into(),
                    items: vec![claim("BSc Computer Science", &["fact_4"])],
                },
            ],
        ),
        &profile(&["fact_1", "fact_2", "fact_3", "fact_4"]),
    );

    assert!(cv_report.is_ready());
    assert!(cv_report.rejections().is_empty());
    assert_eq!(
        cv_report.claims.iter().map(|c| c.slot).collect::<Vec<_>>(),
        vec![
            ClaimSlot::Summary,
            ClaimSlot::CvSection {
                section: 0,
                item: 0
            },
            ClaimSlot::CvSection {
                section: 0,
                item: 1
            },
            ClaimSlot::CvSection {
                section: 1,
                item: 0
            },
        ]
    );
    assert_eq!(cv_report.claims[1].text, "Shipped an API");
    assert_eq!(cv_report.claims[1].fact_ids, vec!["fact_2".to_string()]);

    let cover_report = validate(
        &cover(vec![
            claim("I would like to apply.", &["fact_1"]),
            claim("I have worked with Rust since 2019.", &["fact_2"]),
        ]),
        &profile(&["fact_1", "fact_2"]),
    );
    assert!(cover_report.is_ready());
    assert_eq!(
        cover_report
            .claims
            .iter()
            .map(|c| c.slot)
            .collect::<Vec<_>>(),
        vec![
            ClaimSlot::CoverParagraph { index: 0 },
            ClaimSlot::CoverParagraph { index: 1 },
        ]
    );
}

// AC1: `rejections` is what the studio lists as "fix these", and it keeps both
// kinds of rejection side by side.
#[test]
fn rejections_name_only_the_claims_the_user_must_fix() {
    let model = DocumentModel::Cv(CvDocument {
        schema_version: SCHEMA_VERSION,
        lang: Lang::En,
        contact: contact(),
        summary: Some(claim("Nothing backs this.", &[])),
        sections: vec![Section {
            heading: "Experience".into(),
            items: vec![
                claim("Shipped an API", &["fact_1"]),
                claim("Won an award", &["fact_ghost"]),
            ],
        }],
        declared_gaps: vec![],
    });

    let report = validate(&model, &profile(&["fact_1"]));
    let rejected = report.rejections();

    assert_eq!(rejected.len(), 2);
    assert_eq!(rejected[0].rejection, Some(Rejection::NoFacts));
    assert_eq!(
        rejected[1].rejection,
        Some(Rejection::UnknownFacts(vec!["fact_ghost".into()]))
    );
    assert!(!report.is_ready());
}

// AC2: a draft with no claim-bearing text at all is vacuously ready — the panel
// has nothing to flag.
#[test]
fn a_draft_without_claims_is_ready_and_traces_nothing() {
    let report = validate(&cv(None, vec![]), &profile(&[]));

    assert!(report.claims.is_empty());
    assert!(report.is_ready());
}
