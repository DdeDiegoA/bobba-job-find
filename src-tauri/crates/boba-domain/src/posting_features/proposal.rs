//! The agent-proposed half (AC1, AD-5): what an `extract_postings` run sends,
//! and what the core does with it.
//!
//! [`Proposal`] is the wire contract, exactly as `agent-pack/skills/extract_postings`
//! documents it (one `{value, quote}` per feature):
//!
//! ```json
//! {"skills":   [{"value": "Rust", "quote": "Rust services"}],
//!  "seniority": {"value": "senior", "quote": "Senior Engineer"},
//!  "languages": [{"value": "en", "quote": "fluent English"}],
//!  "education": [{"value": "bachelor", "quote": "Bachelor's degree"}],
//!  "compensation": {"value": "USD 5000000 year", "quote": "$50,000 a year"},
//!  "red_flags": [{"value": "pay to apply", "quote": "a $50 application fee"}],
//!  "notes": [{"value": "deadline 2026-11-01",
//!             "quote": "Applications close on November 1"}]}
//! ```
//!
//! `id` travels beside it (the tool takes `{"id", "features"}`) and names the
//! posting the batch entry is about; the caller reads it, this module only reads
//! `features`.
//!
//! A feature whose quote is missing, blank or not in the posting is **dropped**
//! ([`resolve`]), never stored on the model's word (AD-5: the model proposes,
//! the core disposes). Everything the model wrote is untrusted text (ADR-010),
//! which is why the caller stores these claims with `untrusted_origin = 1`.

use serde::Deserialize;

use crate::scoring::gates::{Confidence, Evidence, QuotedSpan};

use super::{AgentRunRef, Claim, FeatureKind, FeatureSource, Seniority};
use crate::scoring::gates::scan;

/// Why a proposal was refused: its shape, or a value the contract does not
/// allow. The message is for the run log; the UI never shows it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FeatureError {
    /// The payload did not match the contract (a key outside it, a list where a
    /// single value belongs, an entry without `quote`). `why` is serde's.
    #[error("the proposal does not match the extract_postings contract: {why}")]
    Shape { why: String },
    /// A value the contract does not define, e.g. an invented seniority.
    #[error("`{0}` is not a value this contract allows")]
    UnknownValue(String),
}

/// The `features` object of `propose_posting_features`, as the SKILL documents
/// it. `deny_unknown_fields` is the point: a key outside the contract is
/// refused rather than silently ignored, so a contract drift is visible.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    #[serde(default)]
    pub skills: Vec<Entry>,
    #[serde(default)]
    pub seniority: Option<Entry>,
    #[serde(default)]
    pub education: Vec<Entry>,
    #[serde(default)]
    pub languages: Vec<Entry>,
    #[serde(default)]
    pub compensation: Option<Entry>,
    #[serde(default)]
    pub red_flags: Vec<Entry>,
    #[serde(default)]
    pub notes: Vec<Entry>,
}

/// One feature and the words from the posting that support it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub value: String,
    pub quote: String,
}

/// One feature the model proposed, before the core checks its quote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposedClaim {
    pub kind: FeatureKind,
    /// The model's value, trimmed, as it wrote it.
    pub value: String,
    /// The words from the posting that the model says support it.
    pub quote: String,
}

impl Proposal {
    /// Flatten the contract into one claim per entry, in contract order.
    ///
    /// A value the contract enumerates is checked here (an invented seniority
    /// is refused); quotes are not — [`resolve`] checks them against the
    /// posting text, which this type does not have.
    pub fn claims(self) -> Result<Vec<ProposedClaim>, FeatureError> {
        let mut proposed = Vec::new();
        for entry in self.skills {
            proposed.push(entry.claim(FeatureKind::Skill)?);
        }
        if let Some(entry) = self.seniority {
            proposed.push(entry.seniority()?);
        }
        for entry in self.education {
            proposed.push(entry.claim(FeatureKind::Education)?);
        }
        for entry in self.languages {
            proposed.push(entry.claim(FeatureKind::Language)?);
        }
        if let Some(entry) = self.compensation {
            proposed.push(entry.claim(FeatureKind::Compensation)?);
        }
        for entry in self.red_flags {
            proposed.push(entry.claim(FeatureKind::RedFlag)?);
        }
        for entry in self.notes {
            proposed.push(entry.claim(FeatureKind::Note)?);
        }
        Ok(proposed)
    }
}

impl Entry {
    /// The entry as one claim of `kind`. A blank value or quote is refused
    /// here: an empty feature is the model saying nothing, and it gets no row.
    fn claim(self, kind: FeatureKind) -> Result<ProposedClaim, FeatureError> {
        let value = self.value.trim().to_string();
        let quote = self.quote.trim().to_string();
        if value.is_empty() || quote.is_empty() {
            return Err(FeatureError::Shape {
                why: format!("a `{}` entry needs a value and a quote", kind.as_str()),
            });
        }
        Ok(ProposedClaim { kind, value, quote })
    }

    /// A seniority is the one value the contract enumerates, so it is parsed
    /// rather than stored as free text the UI would have to interpret.
    fn seniority(self) -> Result<ProposedClaim, FeatureError> {
        let claim = self.claim(FeatureKind::Seniority)?;
        if Seniority::parse(&claim.value.to_lowercase()).is_none() {
            return Err(FeatureError::UnknownValue(claim.value));
        }
        Ok(ProposedClaim {
            value: claim.value.to_lowercase(),
            ..claim
        })
    }
}

/// Keep the proposals whose quote is really in the posting text (AC1: the core
/// disposes), with the span of that quote and the run that made them (AD-5:
/// every agent output carries `{agent_run_id, cli, model}`).
///
/// A missing, blank or unverifiable quote drops **that claim**, never the batch
/// and never the claim without evidence.
pub fn resolve(proposed: &[ProposedClaim], text: &str, run: &AgentRunRef) -> Vec<Claim> {
    let folded = scan::fold(text);
    let mut claims = Vec::new();
    for proposal in proposed {
        let Some((quote, start)) = quoted(&folded, &proposal.quote) else {
            continue;
        };
        claims.push(Claim {
            kind: proposal.kind,
            value: proposal.value.clone(),
            source: FeatureSource::Llm,
            // The quote is verified, the reading is the model's: Medium.
            confidence: Confidence::Medium,
            evidence: Evidence::Quoted(span_of(text, start, start + quote.len())),
            run: Some(run.clone()),
        });
    }
    claims
}

/// How many of `proposed` really quote `text`, and how many do not — the same
/// check [`resolve`] makes, without building the claims: what the tool boundary
/// answers with (AD-5), where nothing is stored yet.
pub fn verified_counts(proposed: &[ProposedClaim], text: &str) -> (usize, usize) {
    let folded = scan::fold(text);
    let verified = proposed
        .iter()
        .filter(|proposal| quoted(&folded, &proposal.quote).is_some())
        .count();
    (verified, proposed.len() - verified)
}

/// Where a proposal's quote sits in the folded posting text, with the trimmed
/// quote itself. `None` when the quote is blank or the posting does not contain
/// it.
///
/// Case-insensitive: a model that quotes the posting's words in another case is
/// accepted, one that reworded them is dropped.
fn quoted(folded_text: &str, quote: &str) -> Option<(String, usize)> {
    let quote = quote.trim();
    if quote.is_empty() {
        return None;
    }
    let start = folded_text.find(&scan::fold(quote))?;
    Some((quote.to_string(), start))
}

/// The trimmed span of `[start, end)` in `text`, as a quote the UI can show.
fn span_of(text: &str, start: usize, end: usize) -> QuotedSpan {
    let raw = &text[start..end];
    let lead = raw.len() - raw.trim_start().len();
    let trimmed = raw.trim();
    QuotedSpan {
        text: trimmed.to_string(),
        start: start + lead,
        end: start + lead + trimmed.len(),
    }
}

/// Fold the agent's claims into the deterministic set (AC1). A deterministic
/// claim is never replaced by a proposal (AD-5), and a proposal that repeats
/// one is dropped; the rest join the set as [`FeatureSource::Llm`] claims.
pub fn merge(deterministic: Vec<Claim>, proposed: Vec<Claim>) -> Vec<Claim> {
    let mut claims = deterministic;
    for candidate in proposed {
        if claims.iter().any(|claim| claim.same_as(&candidate)) {
            continue;
        }
        claims.push(candidate);
    }
    claims
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::posting::{ApplyHints, Offer, WorkMode};
    use crate::posting_features::{PostingFeatures, deterministic};

    fn entry(value: &str, quote: &str) -> Entry {
        Entry {
            value: value.to_string(),
            quote: quote.to_string(),
        }
    }

    fn proposed(kind: FeatureKind, value: &str, quote: &str) -> ProposedClaim {
        ProposedClaim {
            kind,
            value: value.to_string(),
            quote: quote.to_string(),
        }
    }

    /// The run a proposal came from, as the use case threads it in.
    fn run() -> AgentRunRef {
        AgentRunRef {
            run_id: "01RUN".to_string(),
            cli: "claude_code".to_string(),
            model: Some("haiku".to_string()),
        }
    }

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

    #[test]
    fn a_proposal_is_read_key_by_key_in_contract_order() {
        let proposal = Proposal {
            skills: vec![entry("Rust", "Rust services")],
            seniority: Some(entry("Senior", "Senior Engineer")),
            compensation: Some(entry("USD 5000000 year", "$50,000 a year")),
            notes: vec![entry("closes 2026-11-01", "Applications close on Nov 1")],
            ..Proposal::default()
        };
        assert_eq!(
            proposal.claims().expect("a valid proposal"),
            vec![
                proposed(FeatureKind::Skill, "Rust", "Rust services"),
                proposed(FeatureKind::Seniority, "senior", "Senior Engineer"),
                proposed(
                    FeatureKind::Compensation,
                    "USD 5000000 year",
                    "$50,000 a year"
                ),
                proposed(
                    FeatureKind::Note,
                    "closes 2026-11-01",
                    "Applications close on Nov 1"
                ),
            ]
        );
    }

    #[test]
    fn an_empty_proposal_is_an_empty_list() {
        assert_eq!(Proposal::default().claims().unwrap(), Vec::new());
    }

    #[test]
    fn an_entry_without_a_value_or_a_quote_is_refused() {
        let no_quote = Proposal {
            skills: vec![entry("Rust", "   ")],
            ..Proposal::default()
        };
        assert!(matches!(no_quote.claims(), Err(FeatureError::Shape { .. })));

        let no_value = Proposal {
            notes: vec![entry("", "Applications close on Nov 1")],
            ..Proposal::default()
        };
        assert!(matches!(no_value.claims(), Err(FeatureError::Shape { .. })));
    }

    #[test]
    fn an_invented_seniority_is_refused() {
        let proposal = Proposal {
            seniority: Some(entry("rockstar", "We want a rockstar")),
            ..Proposal::default()
        };
        assert_eq!(
            proposal.claims().unwrap_err(),
            FeatureError::UnknownValue("rockstar".to_string())
        );
    }

    #[test]
    fn a_quote_found_in_the_posting_becomes_the_claim_evidence() {
        let text = "We need a Senior Engineer who ships Rust services.";
        let claims = resolve(
            &[proposed(FeatureKind::Skill, "Rust", "Rust services")],
            text,
            &run(),
        );
        assert_eq!(claims.len(), 1);
        let Evidence::Quoted(QuotedSpan {
            text: quoted,
            start,
            end,
        }) = &claims[0].evidence
        else {
            panic!("expected a quote");
        };
        assert_eq!(quoted, "Rust services");
        assert_eq!(&text[*start..*end], "Rust services");
        assert_eq!(claims[0].source, FeatureSource::Llm);
        assert_eq!(claims[0].confidence, Confidence::Medium);
        assert_eq!(
            claims[0].run(),
            Some(&run()),
            "an agent claim names the run, CLI and model that wrote it (AD-5)"
        );
    }

    #[test]
    fn a_quote_the_posting_does_not_contain_drops_the_claim() {
        let text = "We need a Senior Engineer.";
        let claims = resolve(
            &[
                proposed(FeatureKind::Skill, "Rust", "Rust services"),
                proposed(FeatureKind::Skill, "Go", "   "),
            ],
            text,
            &run(),
        );
        assert!(claims.is_empty(), "{claims:?}");
    }

    #[test]
    fn a_proposal_in_another_case_still_matches_the_posting() {
        let claims = resolve(
            &[proposed(FeatureKind::Skill, "Rust", "RUST")],
            "we build apis in rust.",
            &run(),
        );
        assert_eq!(claims.len(), 1);
        let Evidence::Quoted(span) = &claims[0].evidence else {
            panic!("expected a quote");
        };
        assert_eq!(span.text, "rust");
    }

    #[test]
    fn a_proposal_never_replaces_a_deterministic_claim() {
        let posting = offer("Senior Rust Engineer", "Rust and TypeScript.");
        let proposed = resolve(
            &[
                proposed(FeatureKind::Skill, "rust", "Rust and TypeScript."),
                proposed(FeatureKind::Skill, "Kubernetes", "Rust and TypeScript."),
            ],
            &posting.description_text,
            &run(),
        );
        let merged = merge(deterministic(&posting), proposed);
        let rust: Vec<&Claim> = merged
            .iter()
            .filter(|claim| claim.normalized_value() == "rust")
            .collect();
        assert_eq!(rust.len(), 1, "one claim per value: {merged:?}");
        assert_eq!(rust[0].source, FeatureSource::Det);
        assert!(
            merged
                .iter()
                .any(|claim| claim.value == "Kubernetes" && claim.source == FeatureSource::Llm)
        );
    }

    #[test]
    fn a_merged_set_keeps_its_key_and_reports_itself_refined() {
        let posting = offer(
            "Backend Engineer",
            "Docker and Postgres. Remote within the EU.",
        );
        let proposed = resolve(
            &[proposed(
                FeatureKind::Note,
                "remote within the EU",
                "remote within the EU",
            )],
            &posting.description_text,
            &run(),
        );
        let mut set = PostingFeatures::new(posting.id.clone(), 7);
        set.extend(merge(deterministic(&posting), proposed));
        assert!(set.has_llm());
        assert_eq!(set.values(FeatureKind::Note), vec!["remote within the EU"]);
        assert_eq!(set.profile_version, 7);
    }
}
