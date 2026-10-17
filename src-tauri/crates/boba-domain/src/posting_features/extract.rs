//! The deterministic reader (AC1): the lexicon pass that always runs.
//!
//! It answers from the posting's own words — no clock, no IO, no LLM — so the
//! same Offer always produces byte-identical claims (FR-61). Two rules keep it
//! honest:
//!
//! * **as written, never translated**: a matched skill keeps the casing the
//!   posting used (`Rust`, `TypeScript`), because F1 compares the posting's
//!   wording with the user's, not a taxonomy's;
//! * **a claim carries its quote**: every claim points at the sentence it was
//!   read from ([`QuotedSpan`]), or at the Offer field it came from
//!   ([`Evidence::Field`]) when the posting's own field already holds the value.
//!
//! `shortcut: a lexicon plus whole-word matching, no stemmer and no grammar; a
//! synonym the lexicon does not list is missed rather than guessed. Extend the
//! lists, or move to a real matcher, when real postings miss what matters.`

use std::collections::BTreeSet;

use crate::posting::Offer;
use crate::scoring::gates::{Confidence, Evidence};

use super::{Claim, FeatureKind, PostingFeatures, Seniority, money_value};
use crate::scoring::gates::scan;

/// Tools, technologies and named competencies postings ask for. Lowercase; the
/// match keeps the posting's own casing. `go`, `r` and `c` are deliberately
/// absent: one letter, or an ordinary English word, produces more noise than
/// signal here.
const SKILLS: &[&str] = &[
    "rust",
    "python",
    "typescript",
    "javascript",
    "java",
    "kotlin",
    "swift",
    "scala",
    "ruby",
    "php",
    "elixir",
    "golang",
    "c++",
    "c#",
    ".net",
    "dart",
    "sql",
    "postgresql",
    "postgres",
    "mysql",
    "mongodb",
    "sqlite",
    "redis",
    "elasticsearch",
    "kafka",
    "rabbitmq",
    "graphql",
    "grpc",
    "rest api",
    "html",
    "css",
    "react native",
    "react",
    "next.js",
    "vue",
    "angular",
    "svelte",
    "tailwind",
    "redux",
    "vite",
    "webpack",
    "jest",
    "playwright",
    "cypress",
    "node.js",
    "nodejs",
    "express",
    "django",
    "flask",
    "fastapi",
    "rails",
    "spring boot",
    "laravel",
    "docker",
    "kubernetes",
    "terraform",
    "ansible",
    "aws",
    "gcp",
    "azure",
    "linux",
    "bash",
    "powershell",
    "git",
    "github actions",
    "gitlab ci",
    "jenkins",
    "ci/cd",
    "prometheus",
    "grafana",
    "nginx",
    "serverless",
    "airflow",
    "spark",
    "dbt",
    "snowflake",
    "bigquery",
    "pandas",
    "numpy",
    "pytorch",
    "tensorflow",
    "scikit-learn",
    "machine learning",
    "llm",
    "rag",
    "agile",
    "scrum",
    "kanban",
    "tdd",
    "unit testing",
    "code review",
    "microservices",
    "observability",
    "devops",
];

/// Words that state a seniority, lowest first (`Seniority::ALL` order); the
/// **highest** word a field uses wins, so `Senior Lead` reads as lead.
const SENIORITY: &[(&str, Seniority)] = &[
    ("intern", Seniority::Intern),
    ("internship", Seniority::Intern),
    ("trainee", Seniority::Intern),
    ("practicante", Seniority::Intern),
    ("becario", Seniority::Intern),
    ("jr", Seniority::Junior),
    ("junior", Seniority::Junior),
    ("mid-level", Seniority::Mid),
    ("mid level", Seniority::Mid),
    ("semi-senior", Seniority::Mid),
    ("semisenior", Seniority::Mid),
    ("intermediate", Seniority::Mid),
    ("sr", Seniority::Senior),
    ("senior", Seniority::Senior),
    ("sénior", Seniority::Senior),
    ("tech lead", Seniority::Lead),
    ("team lead", Seniority::Lead),
    ("staff", Seniority::Lead),
    ("jefe", Seniority::Lead),
    ("principal", Seniority::Principal),
    ("head of", Seniority::Principal),
    ("director", Seniority::Principal),
];

/// Study the posting asks for (FR-55 F5), as postings word it (EN and ES).
const EDUCATION: &[&str] = &[
    "bachelor's degree",
    "bachelor's",
    "bachelors",
    "bachelor",
    "master's degree",
    "master's",
    "masters",
    "master",
    "mba",
    "phd",
    "doctorate",
    "doctoral degree",
    "university degree",
    "degree",
    "licenciatura",
    "ingeniería",
    "ingenieria",
    "maestría",
    "maestria",
    "doctorado",
    "técnico",
    "tecnólogo",
];

/// Language names as postings write them (EN and ES) → BCP-47 tag (FR-55 F6).
const LANGUAGES: &[(&str, &str)] = &[
    ("english", "en"),
    ("inglés", "en"),
    ("ingles", "en"),
    ("spanish", "es"),
    ("español", "es"),
    ("castellano", "es"),
    ("german", "de"),
    ("alemán", "de"),
    ("deutsch", "de"),
    ("french", "fr"),
    ("francés", "fr"),
    ("portuguese", "pt"),
    ("portugués", "pt"),
    ("italian", "it"),
    ("italiano", "it"),
    ("dutch", "nl"),
    ("neerlandés", "nl"),
    ("japanese", "ja"),
    ("japonés", "ja"),
    ("mandarin", "zh"),
    ("chinese", "zh"),
    ("korean", "ko"),
    ("russian", "ru"),
    ("polish", "pl"),
    ("swedish", "sv"),
    ("norwegian", "no"),
    ("danish", "da"),
    ("hindi", "hi"),
    ("arabic", "ar"),
    ("hebrew", "he"),
];

/// Phrase patterns the Quality Score's anti-ghost and fraud signals read
/// (FR-60, Q4): a fee to apply, contact off the platform, a courier-job
/// pattern. The lexicon states the pattern; the factor decides what it means.
const RED_FLAGS: &[&str] = &[
    "pay to apply",
    "application fee",
    "registration fee",
    "processing fee",
    "training fee",
    "equipment fee",
    "security deposit",
    "refundable deposit",
    "send your id",
    "copy of your passport",
    "bank details",
    "wire transfer",
    "western union",
    "gift card",
    "bitcoin",
    "crypto payment",
    "money mule",
    "reship",
    "no experience necessary",
    "immediate start no interview",
];

/// Read everything the deterministic pass can read from one Offer (AC1). Claims
/// come out grouped by kind and ordered by value inside a kind — the exact
/// order a re-run produces (FR-61).
pub fn deterministic(offer: &Offer) -> Vec<Claim> {
    let title = offer.title.as_str();
    let text = offer.description_text.as_str();
    let mut claims = Vec::new();

    claims.extend(lexicon_claims(
        text,
        SKILLS,
        FeatureKind::Skill,
        Confidence::High,
    ));
    claims.extend(seniority_claims(title, text));

    // Money is a structured Offer field (AD-17): read it there, never by
    // re-parsing the prose a second time (AD-8 — one reader per field).
    if let Some(money) = &offer.money {
        claims.push(Claim::deterministic(
            FeatureKind::Compensation,
            money_value(money),
            Confidence::High,
            Evidence::Field("money".to_string()),
        ));
    }

    claims.extend(education_claims(text));
    claims.extend(language_claims(text));
    claims.extend(lexicon_claims(
        text,
        RED_FLAGS,
        FeatureKind::RedFlag,
        Confidence::High,
    ));

    let mut set = PostingFeatures::new(offer.id.clone(), 0);
    set.extend(claims);
    set.claims
}

/// Every lexicon phrase in `text`, with the sentence that states it (AC1).
///
/// The longest entry is matched first and a match claims its bytes, so
/// `React Native` is one skill and not also `React`; matches come back in
/// document order, and a phrase said twice is one claim.
fn lexicon_claims(
    text: &str,
    lexicon: &[&str],
    kind: FeatureKind,
    confidence: Confidence,
) -> Vec<Claim> {
    let folded = scan::fold(text);
    let mut longest_first: Vec<&str> = lexicon.to_vec();
    longest_first.sort_by_key(|phrase| std::cmp::Reverse(phrase.len()));

    let mut taken: Vec<(usize, usize)> = Vec::new();
    let mut found: Vec<(usize, usize)> = Vec::new();
    for phrase in longest_first {
        for start in scan::matches(&folded, phrase) {
            let end = start + phrase.len();
            if taken
                .iter()
                .any(|(taken_start, taken_end)| start < *taken_end && *taken_start < end)
            {
                continue;
            }
            taken.push((start, end));
            found.push((start, end));
        }
    }
    found.sort_unstable();

    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut claims = Vec::new();
    for (start, end) in found {
        let value = text[start..end].trim();
        if value.is_empty() || !seen.insert(value.to_lowercase()) {
            continue;
        }
        claims.push(Claim::deterministic(
            kind,
            // FR-55 F1 compares the posting's wording with the user's: the
            // phrase is kept exactly as written.
            value,
            confidence,
            Evidence::Quoted(scan::sentence(text, start, end)),
        ));
    }
    claims
}

/// The seniority the posting states, if any (AC1). The title is the strong
/// signal — it is the field that names the role — and the highest word it uses
/// wins (`Senior Tech Lead` is a lead role); the body is read only when the
/// title states nothing, and a body word is a weaker reading.
fn seniority_claims(title: &str, text: &str) -> Vec<Claim> {
    if let Some(seniority) = highest_seniority(title) {
        return vec![Claim::deterministic(
            FeatureKind::Seniority,
            seniority.as_str(),
            Confidence::High,
            Evidence::Field("title".to_string()),
        )];
    }
    let folded = scan::fold(text);
    let Some(seniority) = highest_seniority(text) else {
        return Vec::new();
    };
    // The first mention of the winning word is the one the UI quotes.
    let (start, end) = SENIORITY
        .iter()
        .filter(|(_, word)| *word == seniority)
        .filter_map(|(phrase, _)| {
            scan::matches(&folded, phrase)
                .first()
                .map(|start| (*start, *start + phrase.len()))
        })
        .min()
        .unwrap_or((0, 0));
    vec![Claim::deterministic(
        FeatureKind::Seniority,
        seniority.as_str(),
        Confidence::Medium,
        Evidence::Quoted(scan::sentence(text, start, end)),
    )]
}

/// The highest seniority word `field` uses, `None` when it states none (AD-8:
/// silence is not "unknown", it is nothing).
fn highest_seniority(field: &str) -> Option<Seniority> {
    let folded = scan::fold(field);
    SENIORITY
        .iter()
        .filter(|(phrase, _)| !scan::matches(&folded, phrase).is_empty())
        .map(|(_, seniority)| *seniority)
        .max()
}

/// The languages the posting names, as BCP-47 tags (FR-55 F6). A named language
/// is a weaker signal than the Gates' requirement wording, so it is Medium: the
/// Gates decide whether it is a requirement.
fn language_claims(text: &str) -> Vec<Claim> {
    let folded = scan::fold(text);
    let mut found: Vec<(usize, usize, &'static str)> = Vec::new();
    for (name, tag) in LANGUAGES {
        for start in scan::matches(&folded, name) {
            if found.iter().any(|(_, _, found_tag)| found_tag == tag) {
                continue;
            }
            found.push((start, start + name.len(), tag));
        }
    }
    found.sort_unstable();
    found
        .into_iter()
        .map(|(start, end, tag)| {
            Claim::deterministic(
                FeatureKind::Language,
                tag,
                Confidence::Medium,
                Evidence::Quoted(scan::sentence(text, start, end)),
            )
        })
        .collect()
}

/// The study the posting asks for, with the phrase it was read from.
fn education_claims(text: &str) -> Vec<Claim> {
    lexicon_claims(text, EDUCATION, FeatureKind::Education, Confidence::High)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::posting::{ApplyHints, Money, PayPeriod, WorkMode};
    use crate::posting_features::{FeatureSource, PostingFeatures};
    use crate::scoring::gates::QuotedSpan;

    /// An Offer with nothing but a title and a body; every other field is the
    /// empty/default shape ingestion produces.
    fn offer(title: &str, body: &str) -> Offer {
        Offer {
            id: "pst_test".to_string(),
            source_id: "manual".to_string(),
            external_id: "test".to_string(),
            title: title.to_string(),
            company: "Boba Labs".to_string(),
            locations: Vec::new(),
            work_mode: WorkMode::Unknown,
            money: None,
            posted_at: None,
            deadline: None,
            first_seen: "2026-10-16T00:00:00Z".to_string(),
            last_seen: "2026-10-16T00:00:00Z".to_string(),
            lang: "en".to_string(),
            description_html: String::new(),
            description_text: body.to_string(),
            requirements: Vec::new(),
            apply: ApplyHints {
                url: "https://example.com/job".to_string(),
                email: None,
            },
        }
    }

    fn values(claims: &[Claim], kind: FeatureKind) -> Vec<String> {
        claims
            .iter()
            .filter(|claim| claim.kind == kind)
            .map(|claim| claim.value.clone())
            .collect()
    }

    fn quote_of(claim: &Claim) -> &QuotedSpan {
        match &claim.evidence {
            Evidence::Quoted(span) => span,
            Evidence::Field(name) => panic!("expected a quote, got the `{name}` field"),
        }
    }

    #[test]
    fn skills_are_read_as_written_and_quote_the_sentence_they_came_from() {
        let text = "We build APIs in Rust and TypeScript. You ship to Kubernetes.";
        let claims = deterministic(&offer("Backend Engineer", text));

        assert_eq!(
            values(&claims, FeatureKind::Skill),
            vec!["Kubernetes", "Rust", "TypeScript"]
        );
        let rust = claims
            .iter()
            .find(|claim| claim.value == "Rust")
            .expect("a Rust claim");
        let span = quote_of(rust);
        assert_eq!(
            span.text, "We build APIs in Rust and TypeScript",
            "the quote stops at the sentence, without its full stop (S6.1 does the same)"
        );
        assert_eq!(&text[span.start..span.end], span.text);
        assert_eq!(rust.source, FeatureSource::Det);
        assert_eq!(rust.confidence, Confidence::High);
    }

    #[test]
    fn a_skill_is_never_read_out_of_a_longer_word() {
        let claims = deterministic(&offer(
            "Sales",
            "We sell rustic furniture to German buyers.",
        ));
        assert!(values(&claims, FeatureKind::Skill).is_empty(), "{claims:?}");
    }

    #[test]
    fn the_longest_lexicon_entry_wins_over_its_own_prefix() {
        let claims = deterministic(&offer("Mobile Developer", "React Native and Postgres."));
        assert_eq!(
            values(&claims, FeatureKind::Skill),
            vec!["Postgres", "React Native"]
        );
    }

    #[test]
    fn a_skill_stated_twice_is_one_claim() {
        let claims = deterministic(&offer(
            "Backend",
            "Rust here. We also write rust and Rust daily.",
        ));
        assert_eq!(values(&claims, FeatureKind::Skill), vec!["Rust"]);
    }

    #[test]
    fn seniority_comes_from_the_title_and_points_at_that_field() {
        let claims = deterministic(&offer("Senior Backend Engineer", "Join our team."));
        let seniority = claims
            .iter()
            .find(|claim| claim.kind == FeatureKind::Seniority)
            .expect("a seniority claim");
        assert_eq!(seniority.value, "senior");
        assert_eq!(seniority.evidence, Evidence::Field("title".to_string()));
    }

    #[test]
    fn the_highest_seniority_word_of_a_field_wins() {
        let claims = deterministic(&offer("Senior Tech Lead", ""));
        assert_eq!(values(&claims, FeatureKind::Seniority), vec!["lead"]);

        let claims = deterministic(&offer("Engineer", "We are hiring a principal engineer."));
        assert_eq!(values(&claims, FeatureKind::Seniority), vec!["principal"]);
        let seniority = claims
            .iter()
            .find(|claim| claim.kind == FeatureKind::Seniority)
            .expect("a seniority claim");
        assert_eq!(
            seniority.confidence,
            Confidence::Medium,
            "a body word is a weaker reading than the title"
        );
        assert_eq!(
            quote_of(seniority).text,
            "We are hiring a principal engineer"
        );
    }

    #[test]
    fn a_posting_that_states_no_seniority_gets_no_claim() {
        let claims = deterministic(&offer("Backend Engineer", "Build things with Rust."));
        assert!(values(&claims, FeatureKind::Seniority).is_empty());
    }

    #[test]
    fn education_is_read_with_its_quote() {
        let text = "Bachelor's degree in Computer Science required.";
        let claims = deterministic(&offer("Engineer", text));
        assert_eq!(
            values(&claims, FeatureKind::Education),
            vec!["Bachelor's degree"]
        );
        let education = claims
            .iter()
            .find(|claim| claim.kind == FeatureKind::Education)
            .expect("an education claim");
        assert_eq!(
            quote_of(education).text,
            "Bachelor's degree in Computer Science required"
        );
    }

    #[test]
    fn languages_are_read_as_bcp47_tags() {
        let claims = deterministic(&offer(
            "Support Engineer",
            "Fluent English required; Spanish is a plus.",
        ));
        assert_eq!(values(&claims, FeatureKind::Language), vec!["en", "es"]);
    }

    #[test]
    fn compensation_comes_from_the_offer_field_and_nowhere_else() {
        let mut posting = offer("Engineer", "Competitive salary, share options.");
        assert!(values(&deterministic(&posting), FeatureKind::Compensation).is_empty());

        posting.money = Some(Money {
            amount_minor: 5_000_000,
            currency: "USD".to_string(),
            period: PayPeriod::Year,
        });
        let claims = deterministic(&posting);
        let compensation = claims
            .iter()
            .find(|claim| claim.kind == FeatureKind::Compensation)
            .expect("a compensation claim");
        assert_eq!(compensation.value, "USD 5000000 year");
        assert_eq!(compensation.evidence, Evidence::Field("money".to_string()));
    }

    #[test]
    fn fraud_phrases_are_read_as_red_flags() {
        let text = "A refundable deposit is required and we accept gift card payments.";
        let claims = deterministic(&offer("Warehouse", text));
        assert_eq!(
            values(&claims, FeatureKind::RedFlag),
            vec!["gift card", "refundable deposit"]
        );
    }

    #[test]
    fn the_same_offer_always_gives_the_same_claims() {
        let posting = offer(
            "Senior Rust Engineer",
            "Rust, TypeScript, Kubernetes. Fluent English. Bachelor's degree.",
        );
        let first = deterministic(&posting);
        let second = deterministic(&posting);
        assert_eq!(first, second);
        assert!(!first.is_empty());
    }

    #[test]
    fn an_offer_with_nothing_to_read_gets_an_empty_answer() {
        assert!(deterministic(&offer("", "")).is_empty());
        let set = PostingFeatures::new("pst_test", 0);
        assert!(set.is_empty());
    }
}
