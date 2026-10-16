//! Offer features (FR-55/FR-56 inputs, FR-61, AD-5): everything the scorer may
//! read about an Offer beyond its own structured fields — the skills it asks
//! for, its seniority, the education it requires, the languages it names, its
//! pay and the red flags it shows.
//!
//! Every [`Claim`] carries where it came from ([`FeatureSource`]), how sure the
//! core is of it ([`Confidence`]) and the [`Evidence`] a human can check: the
//! exact sentence of the posting, or the Offer field it was read from. That is
//! the same contract the Gates answer with (FR-53), so the UI explains a score
//! and an extraction the same way.
//!
//! Two halves, one set (AC1):
//!
//! * [`deterministic`] **always** runs: offline, no clock, no LLM. It reads the
//!   posting's own words through a lexicon and boundary matching — a posting
//!   that says "Rust" gets a skill claim, and `shortcut: hand-rolled matching,
//!   because the domain crate has no regex engine and `scoring::gates` scans
//!   text the same way; move to a regex engine if a lexicon needs real grammar`;
//! * an agent run may **propose** more (AD-5). [`from_proposal`] validates what
//!   the model sent and [`resolve`] keeps only a claim whose quote is really in
//!   the posting text, then [`merge`] folds the proposals into the
//!   deterministic set. A proposal never replaces a deterministic claim.
//!
//! Nothing is invented (AD-8): a posting that states no seniority gets no
//! seniority claim, and an empty set is a correct answer for a posting that
//! says nothing useful.

use serde::{Deserialize, Serialize};

use crate::posting::Money;
use crate::scoring::gates::{Confidence, Evidence, QuotedSpan};

mod extract;
mod proposal;

pub use extract::deterministic;
pub use proposal::{Entry, FeatureError, Proposal, ProposedClaim, merge, resolve};

/// What one claim is about. The seven kinds the agent contract and the store
/// both know (`posting_features.kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureKind {
    Skill,
    Seniority,
    Education,
    Language,
    Compensation,
    RedFlag,
    Note,
}

impl FeatureKind {
    /// Every kind, in the order the UI lists them.
    pub const ALL: [FeatureKind; 7] = [
        FeatureKind::Skill,
        FeatureKind::Seniority,
        FeatureKind::Education,
        FeatureKind::Language,
        FeatureKind::Compensation,
        FeatureKind::RedFlag,
        FeatureKind::Note,
    ];

    /// The stored / serialised form; the same string the migration's CHECK
    /// accepts and the agent contract uses as a key.
    pub fn as_str(self) -> &'static str {
        match self {
            FeatureKind::Skill => "skill",
            FeatureKind::Seniority => "seniority",
            FeatureKind::Education => "education",
            FeatureKind::Language => "language",
            FeatureKind::Compensation => "compensation",
            FeatureKind::RedFlag => "red_flag",
            FeatureKind::Note => "note",
        }
    }

    /// The kind an agent key names, `None` for anything else.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }
}

/// Who produced a claim (AD-5): the core's own reader, or an agent run. Stored
/// as `det` / `llm` so the UI can say "refined by AI" (architecture §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureSource {
    /// [`deterministic`]: the lexicon, always runs.
    Det,
    /// An agent run proposed it and the core verified its quote.
    Llm,
}

impl FeatureSource {
    pub const ALL: [FeatureSource; 2] = [FeatureSource::Det, FeatureSource::Llm];

    pub fn as_str(self) -> &'static str {
        match self {
            FeatureSource::Det => "det",
            FeatureSource::Llm => "llm",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.as_str() == value)
    }
}

/// Seniority as the agent contract names it (SKILL.md `extract_postings`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Seniority {
    Intern,
    Junior,
    Mid,
    Senior,
    Lead,
    Principal,
}

impl Seniority {
    /// Ordered lowest to highest, so "the highest word the title uses wins".
    pub const ALL: [Seniority; 6] = [
        Seniority::Intern,
        Seniority::Junior,
        Seniority::Mid,
        Seniority::Senior,
        Seniority::Lead,
        Seniority::Principal,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Seniority::Intern => "intern",
            Seniority::Junior => "junior",
            Seniority::Mid => "mid",
            Seniority::Senior => "senior",
            Seniority::Lead => "lead",
            Seniority::Principal => "principal",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.as_str() == value)
    }
}

/// One thing the posting says, with where it was read and how sure the core is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    pub kind: FeatureKind,
    /// The value as the posting means it: a skill as it is written, a
    /// [`Seniority`]/[`FeatureKind::Language`] in its canonical form.
    pub value: String,
    pub source: FeatureSource,
    pub confidence: Confidence,
    pub evidence: Evidence,
}

impl Claim {
    /// The form claims are compared in: case and surrounding space do not make
    /// two claims different (`Rust` and ` rust ` are one skill).
    pub fn normalized_value(&self) -> String {
        self.value.trim().to_lowercase()
    }

    /// Whether this claim says the same thing as `other` (kind + value), which
    /// is what keeps a proposal from duplicating the deterministic reader.
    pub fn same_as(&self, other: &Claim) -> bool {
        self.kind == other.kind && self.normalized_value() == other.normalized_value()
    }
}

/// Everything known about one Offer, keyed by the (Offer, Profile version) the
/// cache is keyed by (FR-61).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PostingFeatures {
    pub posting_id: String,
    /// The `profile.version` the extraction ran against (AD-15).
    pub profile_version: u32,
    pub claims: Vec<Claim>,
}

impl PostingFeatures {
    /// An empty set for one (Offer, Profile version).
    pub fn new(posting_id: impl Into<String>, profile_version: u32) -> Self {
        Self {
            posting_id: posting_id.into(),
            profile_version,
            claims: Vec::new(),
        }
    }

    /// Add claims, keeping the set's documented order (kind, then value), so a
    /// re-run of the same inputs produces the same rows (FR-61).
    pub fn extend(&mut self, claims: impl IntoIterator<Item = Claim>) {
        self.claims.extend(claims);
        self.sort();
    }

    /// Order the claims by kind then value.
    pub fn sort(&mut self) {
        self.claims.sort_by(|a, b| {
            (a.kind, a.normalized_value(), a.source.as_str()).cmp(&(
                b.kind,
                b.normalized_value(),
                b.source.as_str(),
            ))
        });
    }

    /// Every claim of one kind, in set order.
    pub fn of_kind(&self, kind: FeatureKind) -> impl Iterator<Item = &Claim> {
        self.claims.iter().filter(move |claim| claim.kind == kind)
    }

    /// The first claim of one kind: seniority and compensation have at most one.
    pub fn first(&self, kind: FeatureKind) -> Option<&Claim> {
        self.of_kind(kind).next()
    }

    /// The values of one kind, in set order.
    pub fn values(&self, kind: FeatureKind) -> Vec<&str> {
        self.of_kind(kind)
            .map(|claim| claim.value.as_str())
            .collect()
    }

    /// The skills as the posting wrote them: what the store keeps in the
    /// posting's own `requirements_json` (S4.2's 0008 leaves that column to this
    /// slice's deterministic pass).
    pub fn skill_values(&self) -> Vec<String> {
        self.of_kind(FeatureKind::Skill)
            .map(|claim| claim.value.clone())
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.claims.is_empty()
    }

    /// Whether an agent run contributed (the `agent_done` marker of the cache).
    pub fn has_llm(&self) -> bool {
        self.claims
            .iter()
            .any(|claim| claim.source == FeatureSource::Llm)
    }
}

/// The money of a claim as one stable string: ISO-4217 code, the amount in
/// minor units (how the store holds money, AD-17) and its period. The UI
/// formats prices itself; this is the value the DB row keeps.
pub fn money_value(money: &Money) -> String {
    format!(
        "{} {} {}",
        money.currency,
        money.amount_minor,
        money.period.as_str()
    )
}

/// One posting still waiting for features, as an `extract_postings` run reads
/// it: the text is third-party and arrives inside the untrusted envelope
/// (AD-7), never as an instruction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingPosting {
    pub id: String,
    pub title: String,
    pub company: String,
    /// BCP-47 content language of the posting.
    pub lang: String,
    /// The posting as plain text (`posting.description_text`).
    pub text: String,
}

/// The stored text of a [`Confidence`]. [`Confidence`] is the Gates' type and
/// carries no stored form of its own; the two-way map lives here so the adapter
/// and every reader of `posting_features.confidence` agree on it.
pub fn confidence_text(confidence: Confidence) -> &'static str {
    match confidence {
        Confidence::Low => "low",
        Confidence::Medium => "medium",
        Confidence::High => "high",
    }
}

/// A stored `posting_features.confidence` back to its type, `None` when the row
/// says something else (a corrupt row, never a silent default).
pub fn confidence_parse(text: &str) -> Option<Confidence> {
    [Confidence::Low, Confidence::Medium, Confidence::High]
        .into_iter()
        .find(|confidence| confidence_text(*confidence) == text)
}

/// An [`Evidence`] as the four `posting_features.evidence_*` columns: the kind
/// (`quoted` | `field`), the text (the quote, or the field's name) and the
/// offsets a quote carries.
pub fn evidence_columns(evidence: &Evidence) -> (&'static str, &str, Option<usize>, Option<usize>) {
    match evidence {
        Evidence::Quoted(span) => (
            "quoted",
            span.text.as_str(),
            Some(span.start),
            Some(span.end),
        ),
        Evidence::Field(name) => ("field", name.as_str(), None, None),
    }
}

/// The four `posting_features.evidence_*` columns back to an [`Evidence`],
/// `None` when they do not describe one (an unknown kind, or a quote without
/// its offsets).
pub fn evidence_from_columns(
    kind: &str,
    text: String,
    start: Option<usize>,
    end: Option<usize>,
) -> Option<Evidence> {
    match (kind, start, end) {
        ("quoted", Some(start), Some(end)) => {
            Some(Evidence::Quoted(QuotedSpan { text, start, end }))
        }
        ("field", None, None) => Some(Evidence::Field(text)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scoring::gates::QuotedSpan;

    fn claim(kind: FeatureKind, value: &str) -> Claim {
        Claim {
            kind,
            value: value.to_string(),
            source: FeatureSource::Det,
            confidence: Confidence::High,
            evidence: Evidence::Field("title".to_string()),
        }
    }

    fn span(text: &str, start: usize, end: usize) -> Evidence {
        Evidence::Quoted(QuotedSpan {
            text: text[start..end].to_string(),
            start,
            end,
        })
    }

    #[test]
    fn kinds_and_sources_round_trip_through_their_stored_form() {
        for kind in FeatureKind::ALL {
            assert_eq!(FeatureKind::parse(kind.as_str()), Some(kind));
        }
        for source in FeatureSource::ALL {
            assert_eq!(FeatureSource::parse(source.as_str()), Some(source));
        }
        assert_eq!(FeatureKind::parse("salary"), None);
        assert_eq!(FeatureSource::parse("agent"), None);
        assert_eq!(FeatureSource::Det.as_str(), "det");
        assert_eq!(FeatureSource::Llm.as_str(), "llm");
    }

    #[test]
    fn seniority_parses_only_what_the_contract_lists() {
        assert_eq!(Seniority::parse("senior"), Some(Seniority::Senior));
        assert_eq!(Seniority::parse("unknown"), None, "unknown is silence");
        assert_eq!(Seniority::parse("Sr"), None);
        assert!(Seniority::Intern < Seniority::Principal);
    }

    #[test]
    fn a_set_orders_its_claims_by_kind_then_value() {
        let mut set = PostingFeatures::new("pst_1", 3);
        set.extend([
            claim(FeatureKind::Skill, "TypeScript"),
            claim(FeatureKind::Skill, "rust"),
            claim(FeatureKind::Seniority, "senior"),
        ]);
        assert_eq!(set.values(FeatureKind::Skill), vec!["rust", "TypeScript"]);
        assert_eq!(
            set.first(FeatureKind::Seniority).map(|c| c.value.as_str()),
            Some("senior")
        );
        assert_eq!(set.profile_version, 3);
        assert!(!set.is_empty());
        assert_eq!(set.skill_values(), vec!["rust", "TypeScript"]);
    }

    #[test]
    fn an_empty_set_is_a_valid_answer() {
        let set = PostingFeatures::new("pst_1", 0);
        assert!(set.is_empty());
        assert_eq!(set.values(FeatureKind::Skill), Vec::<&str>::new());
        assert!(!set.has_llm());
    }

    #[test]
    fn only_an_agent_claim_marks_the_set_as_refined() {
        let mut set = PostingFeatures::new("pst_1", 1);
        set.extend([claim(FeatureKind::Skill, "Rust")]);
        assert!(!set.has_llm());
        let mut proposed = claim(FeatureKind::Skill, "Kubernetes");
        proposed.source = FeatureSource::Llm;
        set.extend([proposed]);
        assert!(set.has_llm());
    }

    #[test]
    fn claims_are_compared_without_case_or_padding() {
        let stored = Claim {
            value: " React ".to_string(),
            ..claim(FeatureKind::Skill, "React")
        };
        assert!(stored.same_as(&claim(FeatureKind::Skill, "react")));
        assert!(!stored.same_as(&claim(FeatureKind::Education, "react")));
        assert_eq!(stored.normalized_value(), "react");
    }

    #[test]
    fn confidence_round_trips_through_its_stored_text() {
        for confidence in [Confidence::Low, Confidence::Medium, Confidence::High] {
            assert_eq!(
                confidence_parse(confidence_text(confidence)),
                Some(confidence)
            );
        }
        assert_eq!(confidence_parse("certain"), None);
    }

    #[test]
    fn evidence_round_trips_through_the_evidence_columns() {
        let quoted = Evidence::Quoted(QuotedSpan {
            text: "Rust services".to_string(),
            start: 12,
            end: 25,
        });
        let (kind, text, start, end) = evidence_columns(&quoted);
        assert_eq!(kind, "quoted");
        assert_eq!(
            evidence_from_columns(kind, text.to_string(), start, end),
            Some(quoted)
        );

        let field = Evidence::Field("money".to_string());
        let (kind, text, start, end) = evidence_columns(&field);
        assert_eq!((kind, start, end), ("field", None, None));
        assert_eq!(
            evidence_from_columns(kind, text.to_string(), start, end),
            Some(field)
        );

        assert_eq!(
            evidence_from_columns("quoted", "x".to_string(), None, None),
            None
        );
        assert_eq!(
            evidence_from_columns("magic", "x".to_string(), None, None),
            None
        );
    }

    #[test]
    fn a_pending_posting_carries_the_text_a_run_reads() {
        let pending = PendingPosting {
            id: "pst_1".to_string(),
            title: "Backend Engineer".to_string(),
            company: "Boba Labs".to_string(),
            lang: "es".to_string(),
            text: "Buscamos Rust.".to_string(),
        };
        assert_eq!(pending.lang, "es");
        assert_eq!(pending.text, "Buscamos Rust.");
    }

    #[test]
    fn money_becomes_one_stable_value() {
        let money = Money {
            amount_minor: 5_000_000,
            currency: "USD".to_string(),
            period: crate::posting::PayPeriod::Year,
        };
        assert_eq!(money_value(&money), "USD 5000000 year");
    }

    #[test]
    fn evidence_of_a_deterministic_claim_points_into_the_posting_text() {
        let text = "Senior Rust engineer.";
        let evidence = span(text, 0, 6);
        let Evidence::Quoted(quoted) = &evidence else {
            panic!("expected a quote");
        };
        assert_eq!(quoted.text, "Senior");
        assert_eq!(&text[quoted.start..quoted.end], "Senior");
    }
}
