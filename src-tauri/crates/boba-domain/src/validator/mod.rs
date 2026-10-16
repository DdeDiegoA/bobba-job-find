//! The no-fabrication validator and claim traceability (FR-73, FR-74, SM-6, AD-15).
//!
//! Every claim-bearing line of a draft names the Profile facts it was built from
//! ([`Claim::fact_ids`], AD-15). [`validate`] walks the whole draft and answers
//! one question per claim: is it backed by facts the Profile actually has?
//!
//! Two rejections are possible, and both block *Ready* and *Send* (FR-74,
//! SM-6: 0 fabricated claims in sent Documents):
//!
//! * [`Rejection::NoFacts`] — the claim names no fact at all, so nothing in the
//!   Profile stands behind it;
//! * [`Rejection::UnknownFacts`] — the claim names facts the Profile does not
//!   have (the user confirms them into the Profile, or the claim goes).
//!
//! A [`ClaimTrace`] also carries the slot it sits in, so the studio can point at
//! the exact sentence the user must fix (AC2), and the facts behind a linked
//! sentence to show on hover or focus (FR-73).
//!
//! Pure model: no IO, no clock, no LLM. The Profile's fact ids are injected as a
//! predicate, so this crate stays free of the store (AD-1, AD-15: a fact is a
//! set of immutable revisions, so a fact id either exists or never did).

use serde::{Deserialize, Serialize};

use crate::documents::{Claim, DocumentModel};

#[cfg(test)]
mod tests;

/// Where a claim sits in a draft, as the studio addresses it (AC2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimSlot {
    /// The CV summary line.
    Summary,
    /// Item `item` of section `section`, in document order.
    CvSection { section: usize, item: usize },
    /// Paragraph `index` of the cover letter.
    CoverParagraph { index: usize },
}

/// Why a claim was rejected (FR-74).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rejection {
    /// The claim names no facts: nothing in the Profile backs it.
    NoFacts,
    /// The claim names facts the Profile does not have. Only the missing ids are
    /// listed, each once, in the order the claim named them.
    UnknownFacts(Vec<String>),
}

/// One claim of a draft with its verdict (FR-73).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimTrace {
    /// Where the claim sits, so the UI can point at it.
    pub slot: ClaimSlot,
    /// The claim's text, as the studio shows it as the sentence's source check.
    pub text: String,
    /// The facts the claim names (AD-15).
    pub fact_ids: Vec<String>,
    /// `None` when every named fact is in the Profile.
    pub rejection: Option<Rejection>,
}

impl ClaimTrace {
    /// Whether the Profile backs this claim (AC1: the ones that don't are
    /// highlighted in the studio).
    pub fn is_backed(&self) -> bool {
        self.rejection.is_none()
    }
}

/// The verdict of a whole draft (FR-74): every claim, in reading order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationReport {
    pub claims: Vec<ClaimTrace>,
}

impl ValidationReport {
    /// Whether the draft may be marked *Ready* and sent (AC1). A draft with no
    /// claim-bearing text is vacuously ready.
    pub fn is_ready(&self) -> bool {
        self.claims.iter().all(ClaimTrace::is_backed)
    }

    /// The claims the user must fix or confirm into the Profile before *Ready*
    /// and *Send* unblock (FR-74). Read-only: nothing here edits the Profile.
    pub fn rejections(&self) -> Vec<&ClaimTrace> {
        self.claims.iter().filter(|c| !c.is_backed()).collect()
    }
}

/// Validate a draft against the Profile (FR-74).
///
/// `known` answers whether a fact id exists in the Profile; the caller passes
/// the ids of the Profile snapshot the draft was tailored from.
pub fn validate(model: &DocumentModel, known: &dyn Fn(&str) -> bool) -> ValidationReport {
    let mut claims = Vec::new();
    match model {
        DocumentModel::Cv(doc) => {
            if let Some(summary) = &doc.summary {
                claims.push(trace(ClaimSlot::Summary, summary, known));
            }
            for (section, s) in doc.sections.iter().enumerate() {
                for (item, claim) in s.items.iter().enumerate() {
                    claims.push(trace(ClaimSlot::CvSection { section, item }, claim, known));
                }
            }
        }
        DocumentModel::CoverLetter(doc) => {
            for (index, paragraph) in doc.paragraphs.iter().enumerate() {
                claims.push(trace(ClaimSlot::CoverParagraph { index }, paragraph, known));
            }
        }
    }
    ValidationReport { claims }
}

/// One claim's verdict: what it names, and what the Profile is missing.
fn trace(slot: ClaimSlot, claim: &Claim, known: &dyn Fn(&str) -> bool) -> ClaimTrace {
    let missing = claim.fact_ids.iter().filter(|id| !known(id)).fold(
        Vec::new(),
        |mut ids: Vec<String>, id| {
            if !ids.iter().any(|seen| seen == id) {
                ids.push(id.clone());
            }
            ids
        },
    );
    let rejection = if claim.fact_ids.is_empty() {
        Some(Rejection::NoFacts)
    } else if missing.is_empty() {
        None
    } else {
        Some(Rejection::UnknownFacts(missing))
    };
    ClaimTrace {
        slot,
        text: claim.text.clone(),
        fact_ids: claim.fact_ids.clone(),
        rejection,
    }
}
