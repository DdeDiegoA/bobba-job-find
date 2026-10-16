//! FR-75: the Offer's requirements, as covered or a gap, and the rule that a
//! gap is never inserted into a document.

use super::*;
use crate::profile::{FactKind, Provenance};

fn fact(id: &str, text_en: Option<&str>, text_es: Option<&str>) -> ProfileFact {
    ProfileFact {
        id: id.to_string(),
        item_id: format!("item_{id}"),
        kind: FactKind::Skill,
        text_en: text_en.map(str::to_string),
        text_es: text_es.map(str::to_string),
        evidence: None,
        detail_json: "{}".to_string(),
        supersedes: None,
        profile_version: 1,
        provenance: Provenance::User,
        agent_run_id: None,
        created_at: "2026-10-16T10:00:00Z".to_string(),
    }
}

fn requirements(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| item.to_string()).collect()
}

/// The Profile of the UJ-3 fixture: billing, Rust and Postgres, no cloud.
fn profile() -> Vec<ProfileFact> {
    vec![
        fact(
            "fact_1",
            Some("Shipped the billing pipeline in Rust and Postgres, cutting p99 latency by 40%"),
            Some("Entregué el pipeline de facturación en Rust y Postgres"),
        ),
        fact(
            "fact_2",
            Some("Six years of experience building billing systems"),
            Some("Seis años de experiencia construyendo sistemas de facturación"),
        ),
    ]
}

fn only(coverage: Vec<RequirementCoverage>) -> RequirementCoverage {
    assert_eq!(coverage.len(), 1, "the fixture states one requirement");
    coverage.into_iter().next().expect("one requirement")
}

#[test]
fn a_requirements_keywords_drop_the_noise_words_and_the_bare_numbers() {
    assert_eq!(
        requirement_keywords("5+ years of experience with Rust and PostgreSQL"),
        ["years", "experience", "rust", "postgresql"]
    );
    // `+`, `#` and `.` keep C++, C#, Node.js and .NET in one keyword.
    assert_eq!(
        requirement_keywords("C++, C# and Node.js"),
        ["c++", "c#", "node.js"]
    );
    assert_eq!(requirement_keywords(""), Vec::<String>::new());
}

#[test]
fn folding_ignores_case_accents_and_punctuation() {
    assert_eq!(fold("Ingeniería de Datos"), "ingenieria de datos");
    assert_eq!(fold("Bogotá"), "bogota");
    assert_eq!(words("Bogotá, Colombia."), ["bogota", "colombia"]);
    assert_eq!(words("  "), Vec::<String>::new());
}

// FR-75, AC2: a requirement every keyword of which a fact backs is covered.
#[test]
fn a_requirement_the_profile_backs_is_covered() {
    let coverage = only(coverage(
        &requirements(&["Rust and Postgres"]),
        &profile(),
        Lang::En,
    ));

    assert_eq!(
        coverage,
        RequirementCoverage {
            requirement: "Rust and Postgres".to_string(),
            covered: vec!["rust".to_string(), "postgres".to_string()],
            gaps: vec![],
        }
    );
    assert!(coverage.is_covered());
    assert_eq!(coverage.keywords(), 2);
}

// FR-75, AC2: nothing backs it, so it is a gap — and it stays out of the draft.
#[test]
fn a_requirement_no_fact_backs_is_a_gap() {
    let coverage = only(coverage(
        &requirements(&["Kubernetes and Terraform"]),
        &profile(),
        Lang::En,
    ));

    assert!(!coverage.is_covered());
    assert_eq!(coverage.covered, Vec::<String>::new());
    assert_eq!(coverage.gaps, ["kubernetes", "terraform"]);
    assert_eq!(
        coverage.verdicts().collect::<Vec<(&str, Coverage)>>(),
        [("kubernetes", Coverage::Gap), ("terraform", Coverage::Gap)]
    );
}

// FR-75: the panel shows what is missing, not that the whole line failed.
#[test]
fn a_partly_backed_requirement_lists_only_what_is_missing() {
    let coverage = only(coverage(
        &requirements(&["Rust and Terraform"]),
        &profile(),
        Lang::En,
    ));

    assert_eq!(coverage.covered, ["rust"]);
    assert_eq!(coverage.gaps, ["terraform"]);
    assert!(!coverage.is_covered());
    assert_eq!(
        coverage.verdicts().collect::<Vec<(&str, Coverage)>>(),
        [("rust", Coverage::Covered), ("terraform", Coverage::Gap)]
    );
}

// AC2 ("gaps are never inserted"), enforced by construction: the list of
// keywords a document must show can never contain one.
#[test]
fn only_backed_keywords_are_ever_required_of_a_document() {
    let offers = requirements(&["Rust and Postgres", "Terraform", "Kubernetes", "Rust"]);

    assert_eq!(
        backed_keywords(&offers, &profile(), Lang::En),
        ["postgres", "rust"]
    );
    assert_eq!(
        gaps(&offers, &profile(), Lang::En),
        ["kubernetes", "terraform"]
    );

    let required = backed_keywords(&offers, &profile(), Lang::En);
    for gap in gaps(&offers, &profile(), Lang::En) {
        assert!(
            !required.contains(&gap),
            "`{gap}` is a gap and must never be required of a document"
        );
    }
}

// AC2: the panel's totals — "Coverage 12/14" is backed keywords out of the ones
// the Offer names, so the two counts have to add up.
#[test]
fn the_panel_counts_every_keyword_of_every_requirement() {
    let offers = requirements(&["Rust and Postgres", "Kubernetes and Terraform"]);
    let coverage = coverage(&offers, &profile(), Lang::En);

    let covered: usize = coverage.iter().map(|c| c.covered.len()).sum();
    let gaps: usize = coverage.iter().map(|c| c.gaps.len()).sum();
    let total: usize = coverage.iter().map(RequirementCoverage::keywords).sum();

    assert_eq!((covered, gaps, total), (2, 2, 4));
    assert_eq!(
        backed_keywords(&offers, &profile(), Lang::En).len(),
        covered
    );
    assert_eq!(coverage.iter().filter(|c| c.is_covered()).count(), 1);
}

// AD-15: a fact backs a keyword through the variant the draft is written in.
#[test]
fn a_keyword_is_backed_by_the_facts_text_in_the_drafts_language() {
    let spanish_only = vec![fact("fact_1", None, Some("Experiencia con Kubernetes"))];

    assert_eq!(
        only(coverage(
            &requirements(&["Kubernetes"]),
            &spanish_only,
            Lang::En
        ))
        .gaps,
        ["kubernetes"],
        "a fact with no English variant backs nothing in an English draft"
    );
    assert!(
        only(coverage(
            &requirements(&["Kubernetes"]),
            &spanish_only,
            Lang::Es
        ))
        .is_covered()
    );
}

// AC1: a fact that carries no text at all is a fact that backs nothing — and it
// must not look like a match.
#[test]
fn a_fact_with_no_text_backs_nothing() {
    let empty = vec![fact("fact_1", Some("   "), None)];

    assert_eq!(
        only(coverage(&requirements(&["Rust"]), &empty, Lang::En)).gaps,
        ["rust"]
    );
    assert!(backed_keywords(&requirements(&["Rust"]), &empty, Lang::En).is_empty());
}

// AC1: the Offer's own order is the panel's order, and a repeated keyword counts
// once.
#[test]
fn coverage_follows_the_offers_order_and_never_doubles_a_keyword() {
    let coverage = coverage(
        &requirements(&["Kubernetes", "Rust and Rust"]),
        &profile(),
        Lang::En,
    );

    assert_eq!(
        coverage
            .iter()
            .map(|c| c.requirement.as_str())
            .collect::<Vec<&str>>(),
        ["Kubernetes", "Rust and Rust"]
    );
    assert_eq!(
        coverage[1].covered,
        ["rust"],
        "a repeated keyword counts once"
    );
    assert_eq!(coverage[1].keywords(), 1);
}

// An empty Profile is the cold-start case: every requirement is a gap, and the
// panel says so instead of inventing a match.
#[test]
fn an_empty_profile_covers_nothing() {
    let coverage = coverage(&requirements(&["Rust"]), &[], Lang::En);
    assert_eq!(coverage, vec![only_coverage("Rust", vec![], vec!["rust"])]);
}

fn only_coverage(requirement: &str, covered: Vec<&str>, gaps: Vec<&str>) -> RequirementCoverage {
    RequirementCoverage {
        requirement: requirement.to_string(),
        covered: covered.into_iter().map(str::to_string).collect(),
        gaps: gaps.into_iter().map(str::to_string).collect(),
    }
}
