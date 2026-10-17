//! F1 skills match (FR-55, `offer-scoring.md` §3): how much of what the posting
//! asks for the Profile can show.
//!
//! The posting's own skill claims (S6.2) are compared with the Profile's `skill`
//! facts after folding case and applying a synonym map, so `JS` is `JavaScript`
//! and `postgres` is `PostgreSQL` — the ESCO-style normalization the research
//! document asks for. A skill the Profile does not have but that sits in the
//! same family as one it does (`Snowflake` against `SQL`) earns **partial**
//! credit: the Relatedness bonus, taken from a bundled family table, never from
//! an LLM.
//!
//! What comes out is exact on both sides: `matched` and `missing` name the
//! posting's own words, because that is what the user recognises and what the
//! narration may quote (FR-59). The partial credit shows in the score, not by
//! putting a skill the user lacks among the matched.
//!
//! `shortcut: a synonym map and a family table, not a real ESCO/O*NET graph;
//! a related skill the table does not list is simply missing. Extend the two
//! tables when real postings miss what matters.`

use std::collections::BTreeSet;

use super::*;

/// The weight the lexical half and the coverage half carry inside F1
/// (`offer-scoring.md` §3's `α·Jaccard + β·coverage`).
const LEXICAL_SHARE: f32 = 0.5;
/// What one *related* skill is worth next to an exact one (the γ bonus).
const RELATED_CREDIT: f32 = 0.5;

/// Skills that name the same thing. Left side as it is written, right side the
/// canonical form both sides are folded to.
const SYNONYMS: &[(&str, &str)] = &[
    ("js", "javascript"),
    ("ts", "typescript"),
    ("node", "node.js"),
    ("nodejs", "node.js"),
    ("postgres", "postgresql"),
    ("psql", "postgresql"),
    ("k8s", "kubernetes"),
    ("ml", "machine learning"),
    ("ci cd", "ci/cd"),
    ("cicd", "ci/cd"),
    ("rest", "rest api"),
    ("golang", "go"),
    ("c sharp", "c#"),
    ("cplusplus", "c++"),
    ("dotnet", ".net"),
];

/// Skills close enough that having one is partial evidence for the other
/// (`offer-scoring.md` §3: "candidate has React → partial credit for Vue").
/// A family is symmetric: either direction earns the same credit.
const FAMILIES: &[&[&str]] = &[
    &[
        "sql",
        "postgresql",
        "mysql",
        "sqlite",
        "snowflake",
        "bigquery",
        "dbt",
        "redshift",
    ],
    &[
        "machine learning",
        "pytorch",
        "tensorflow",
        "scikit-learn",
        "llm",
        "rag",
    ],
    &["react", "vue", "angular", "svelte", "next.js"],
    &["aws", "gcp", "azure"],
    &["docker", "kubernetes"],
    &["airflow", "spark", "kafka"],
    &["rest api", "graphql", "grpc"],
    &["swift", "kotlin", "dart"],
];

/// One asked-for skill: the canonical form both sides compare on, the posting's
/// own spelling (what the user sees) and the evidence it was read from.
struct Asked {
    canonical: String,
    written: String,
    evidence: Evidence,
}

fn canonical(skill: &str) -> String {
    let folded = skill.trim().to_lowercase();
    SYNONYMS
        .iter()
        .find(|(alias, _)| *alias == folded)
        .map_or(folded.clone(), |(_, canonical)| (*canonical).to_string())
}

/// Whether two canonical skills sit in the same family.
fn related(left: &str, right: &str) -> bool {
    left != right
        && FAMILIES
            .iter()
            .any(|family| family.contains(&left) && family.contains(&right))
}

/// The posting's skill claims, canonical and deduped, in the set's own order
/// (FR-61: the same features always read the same way).
fn asked(inputs: &ScoreInputs<'_>) -> Vec<Asked> {
    let mut seen = BTreeSet::new();
    inputs
        .features
        .of_kind(crate::posting_features::FeatureKind::Skill)
        .filter_map(|claim| {
            let canonical = canonical(&claim.value);
            seen.insert(canonical.clone()).then(|| Asked {
                canonical,
                written: claim.value.trim().to_string(),
                evidence: claim.evidence.clone(),
            })
        })
        .collect()
}

pub(super) fn score(inputs: &ScoreInputs<'_>, weight: f32) -> FactorScore {
    let asked = asked(inputs);
    if asked.is_empty() {
        return FactorScore::missing_input(
            Factor::Skills,
            weight,
            "no skills stated in the posting",
        );
    }

    let have: Vec<String> = inputs.profile.skills.iter().map(|s| canonical(s)).collect();
    let exact: Vec<&Asked> = asked
        .iter()
        .filter(|skill| have.contains(&skill.canonical))
        .collect();
    let linked: Vec<&Asked> = asked
        .iter()
        .filter(|skill| {
            !have.contains(&skill.canonical)
                && have.iter().any(|mine| related(&skill.canonical, mine))
        })
        .collect();

    let mut union: BTreeSet<&str> = BTreeSet::new();
    union.extend(asked.iter().map(|skill| skill.canonical.as_str()));
    union.extend(have.iter().map(String::as_str));

    let lexical = exact.len() as f32 / union.len() as f32;
    let coverage = (exact.len() as f32 + RELATED_CREDIT * linked.len() as f32) / asked.len() as f32;
    let score = 100.0 * (LEXICAL_SHARE * lexical + (1.0 - LEXICAL_SHARE) * coverage);

    // The posting asks; the Profile says nothing. That is a floor, not a fact.
    let confidence = if inputs.profile.skills.is_empty() {
        Confidence::Low
    } else {
        Confidence::High
    };

    let mut evidence: Vec<Evidence> = Vec::new();
    for skill in &asked {
        if !evidence.contains(&skill.evidence) {
            evidence.push(skill.evidence.clone());
        }
    }

    FactorScore::scored(Factor::Skills, score, weight, confidence, evidence)
        .with_matched(exact.iter().map(|s| s.written.clone()).collect())
        // A related-but-absent skill is still absent: it is named in `missing`
        // (the user does not have it) and shows up in the score (it is close to
        // something they do).
        .with_missing(
            asked
                .iter()
                .filter(|skill| !have.contains(&skill.canonical))
                .map(|skill| skill.written.clone())
                .collect(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::posting_features::{Claim, PostingFeatures};
    use crate::scoring::factors::testing::*;

    fn f1(fixture: &Fixture) -> FactorScore {
        score(&fixture.inputs(), fixture.prefs.weights.of(Factor::Skills))
    }

    // §9: dbt/Airflow matched, Snowflake missing (partly related to the SQL the
    // profile does have).
    #[test]
    fn the_worked_examples_skills_are_reported_one_by_one() {
        let factor = f1(&Fixture::section_9());
        assert_eq!(
            factor.matched,
            vec!["Airflow".to_string(), "dbt".to_string()]
        );
        assert_eq!(factor.missing, vec!["Snowflake".to_string()]);
        let score = factor.score.expect("a score");
        assert!(score > 55.0 && score < 70.0, "{score}");
        assert_eq!(factor.confidence, Confidence::High);
    }

    #[test]
    fn a_synonym_is_the_same_skill() {
        let mut fixture = Fixture::section_9();
        fixture.features = PostingFeatures::new("pst_01SECTION9", 3);
        fixture.features.extend([skill("JavaScript")]);
        fixture.profile.skills = vec!["JS".to_string()];
        let factor = f1(&fixture);
        assert_eq!(factor.matched, vec!["JavaScript".to_string()]);
        assert_eq!(factor.score, Some(100.0));
    }

    #[test]
    fn the_postings_casing_is_kept() {
        let mut fixture = Fixture::section_9();
        fixture.features = PostingFeatures::new("pst_01SECTION9", 3);
        fixture.features.extend([skill("TypeScript")]);
        fixture.profile.skills = vec!["typescript".to_string()];
        let factor = f1(&fixture);
        assert_eq!(factor.matched, vec!["TypeScript".to_string()]);
    }

    #[test]
    fn a_posting_that_asks_for_nothing_leaves_the_factor_missing() {
        let mut fixture = Fixture::section_9();
        fixture.features = PostingFeatures::new("pst_01SECTION9", 3);
        let factor = f1(&fixture);
        assert_eq!(factor.score, None);
        assert_eq!(factor.confidence, Confidence::Low);
        assert_eq!(
            factor.missing,
            vec!["no skills stated in the posting".to_string()]
        );
    }

    #[test]
    fn a_profile_with_no_skills_scores_zero_rather_than_missing() {
        let mut fixture = Fixture::section_9();
        fixture.profile.skills = Vec::new();
        let factor = f1(&fixture);
        assert_eq!(factor.score, Some(0.0));
        assert_eq!(factor.matched, Vec::<String>::new());
        assert_eq!(
            factor.missing,
            vec![
                "Airflow".to_string(),
                "dbt".to_string(),
                "Snowflake".to_string()
            ]
        );
        assert_eq!(factor.confidence, Confidence::Low);
    }

    #[test]
    fn a_related_skill_earns_partial_credit() {
        let mut fixture = Fixture::section_9();
        fixture.features = PostingFeatures::new("pst_01SECTION9", 3);
        fixture.features.extend([skill("Snowflake")]);
        fixture.profile.skills = vec!["SQL".to_string()];
        let factor = f1(&fixture);
        assert_eq!(factor.score, Some(25.0));
        assert_eq!(factor.missing, vec!["Snowflake".to_string()]);
    }

    #[test]
    fn the_postings_own_evidence_travels_with_the_factor() {
        let factor = f1(&Fixture::section_9());
        assert!(!factor.evidence.is_empty());
    }

    // FR-53/FR-59: a skill claim read out of the posting's words keeps the exact
    // span, which is what the UI quotes back to the user.
    #[test]
    fn a_quoted_skill_claim_keeps_its_span() {
        let text = "We build on dbt and Airflow.";
        let mut fixture = Fixture::section_9();
        fixture.posting.description_text = text.to_string();
        fixture.features = PostingFeatures::new("pst_01SECTION9", 3);
        fixture.features.extend([Claim::deterministic(
            crate::posting_features::FeatureKind::Skill,
            "dbt",
            Confidence::High,
            quoted(text, 12, 15),
        )]);
        fixture.profile.skills = vec!["dbt".to_string()];
        let factor = f1(&fixture);
        let Evidence::Quoted(span) = &factor.evidence[0] else {
            panic!("expected a quote");
        };
        assert_eq!(&text[span.start..span.end], "dbt");
    }

    #[test]
    fn the_same_skill_asked_twice_is_one_skill() {
        let mut fixture = Fixture::section_9();
        fixture.features = PostingFeatures::new("pst_01SECTION9", 3);
        fixture.features.extend([skill("dbt"), skill("dbt")]);
        fixture.profile.skills = vec!["dbt".to_string()];
        let factor = f1(&fixture);
        assert_eq!(factor.score, Some(100.0));
        assert_eq!(factor.matched, vec!["dbt".to_string()]);
    }

    #[test]
    fn a_skill_from_the_other_family_is_simply_missing() {
        let mut fixture = Fixture::section_9();
        fixture.features = PostingFeatures::new("pst_01SECTION9", 3);
        fixture.features.extend([skill("Rust")]);
        fixture.profile.skills = vec!["SQL".to_string()];
        let factor = f1(&fixture);
        assert_eq!(factor.score, Some(0.0));
        assert_eq!(factor.missing, vec!["Rust".to_string()]);
    }
}
