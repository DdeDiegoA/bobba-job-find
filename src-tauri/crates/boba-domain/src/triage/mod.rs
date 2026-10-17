//! Inbox triage decisions (S6.6, FR-54, FR-66, FR-67).
//!
//! Three things the Inbox writes down and later reads back:
//!
//! * **A dismiss reason** (FR-66) — why the user said no, when they said why.
//!   Free of charge and optional: an Offer dismissed without a reason is still
//!   dismissed, it just teaches nothing.
//! * **A VETO override** (FR-54) — the user overruled the Gate that hid an
//!   Offer. The override is recorded, never silent: the Excluded section can
//!   always say who let the Offer through.
//! * **A Preferences suggestion** (FR-67) — three dismissals for the same
//!   reason a Preferences change would have captured. This module only decides
//!   *which* change a reason points at and what the suggestion is called; the
//!   use case counts and the UI asks.
//!
//! Everything here is pure: no IO, no clock, no LLM.

use serde::{Deserialize, Serialize};

/// Dismissals with the same reason it takes to suggest a Preferences change
/// (FR-67, AC3: "3 or more").
pub const SUGGESTION_THRESHOLD: usize = 3;

/// Why the user dismissed an Offer (FR-66's list). The stored/serialised form
/// is stable and snake_case; a value this build does not know is refused, never
/// guessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DismissReason {
    /// Not interested in the role.
    NotInterested,
    /// Not eligible (work authorization, degree, …).
    NotEligible,
    /// The pay is too low.
    Salary,
    /// The company is not one the user wants.
    Company,
    /// The place or work mode does not work.
    Location,
    /// Anything else (the user's own words are not stored: this is a signal,
    /// not a diary).
    Other,
}

impl DismissReason {
    /// Every reason, in the order the menu shows them (FR-66).
    pub const ALL: [DismissReason; 6] = [
        DismissReason::NotInterested,
        DismissReason::NotEligible,
        DismissReason::Salary,
        DismissReason::Company,
        DismissReason::Location,
        DismissReason::Other,
    ];

    /// The stored and serialised form.
    pub fn as_str(self) -> &'static str {
        match self {
            DismissReason::NotInterested => "not_interested",
            DismissReason::NotEligible => "not_eligible",
            DismissReason::Salary => "salary",
            DismissReason::Company => "company",
            DismissReason::Location => "location",
            DismissReason::Other => "other",
        }
    }

    /// Parse the stored form, `None` for anything else.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|reason| reason.as_str() == value)
    }

    /// The Preferences change this reason would capture (FR-67 AC3). A reason
    /// that no preference can express returns `None`: Boba then never suggests
    /// a change it could not make anyway.
    pub fn prefs_change(self) -> Option<SuggestionKind> {
        match self {
            DismissReason::Company => Some(SuggestionKind::ExcludeCompany),
            DismissReason::Location => Some(SuggestionKind::ExcludeOnsite),
            DismissReason::NotInterested
            | DismissReason::NotEligible
            | DismissReason::Salary
            | DismissReason::Other => None,
        }
    }
}

/// Which Preferences change a repeat of a reason calls for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestionKind {
    /// Never show Offers from this company again (`excluded_companies`).
    ExcludeCompany,
    /// Stop accepting onsite roles (`work_modes.onsite`).
    ExcludeOnsite,
}

impl SuggestionKind {
    /// Every kind (for the store's exhaustive round trip).
    pub const ALL: [SuggestionKind; 2] = [
        SuggestionKind::ExcludeCompany,
        SuggestionKind::ExcludeOnsite,
    ];

    /// The stable id prefix; `exclude_company` carries the company as its value.
    pub fn as_str(self) -> &'static str {
        match self {
            SuggestionKind::ExcludeCompany => "exclude_company",
            SuggestionKind::ExcludeOnsite => "exclude_onsite",
        }
    }
}

/// One Preferences change Boba asked the user to accept (FR-67).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Suggestion {
    /// Exclude every Offer from `company` (as the user saw it spelled).
    ExcludeCompany(String),
    /// Exclude onsite roles.
    ExcludeOnsite,
}

impl Suggestion {
    /// The stable id the suggestion is recorded and decided under: the kind, and
    /// for a company the name matched case-insensitively.
    pub fn id(&self) -> String {
        match self {
            Suggestion::ExcludeCompany(company) => {
                format!(
                    "{}:{}",
                    SuggestionKind::ExcludeCompany.as_str(),
                    normalize(company)
                )
            }
            Suggestion::ExcludeOnsite => SuggestionKind::ExcludeOnsite.as_str().to_string(),
        }
    }

    /// The kind, as the UI names it.
    pub fn kind(&self) -> SuggestionKind {
        match self {
            Suggestion::ExcludeCompany(_) => SuggestionKind::ExcludeCompany,
            Suggestion::ExcludeOnsite => SuggestionKind::ExcludeOnsite,
        }
    }

    /// The company a `exclude_company` suggestion names; `None` otherwise.
    pub fn company(&self) -> Option<&str> {
        match self {
            Suggestion::ExcludeCompany(company) => Some(company),
            Suggestion::ExcludeOnsite => None,
        }
    }

    /// Read an id back, so a decision the UI sends is applied to the change it
    /// names and never to another. `None` for an id this build cannot read.
    pub fn parse(id: &str) -> Option<Self> {
        if id == SuggestionKind::ExcludeOnsite.as_str() {
            return Some(Suggestion::ExcludeOnsite);
        }
        let company = id.strip_prefix("exclude_company:")?;
        if company.is_empty() {
            return None;
        }
        Some(Suggestion::ExcludeCompany(company.to_string()))
    }
}

/// The case/spacing-insensitive key a company is matched by: `"Boba  Labs"` and
/// `"boba labs"` are one company, and the suggestion is decided once for it.
pub fn normalize(company: &str) -> String {
    company
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// One dismissal as the store keeps it (FR-66/FR-67): the Offer, the reason and
/// when. There is one reason per posting; dismissing again replaces it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DismissalRecord {
    pub posting_id: String,
    pub reason: DismissReason,
    pub created_at: String,
}

/// One Gate override the user granted (FR-54): the Offer, the Gate it overruled
/// and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VetoOverride {
    pub posting_id: String,
    /// The Gate that vetoed: `eligibility` … `deadline`.
    pub gate: String,
    pub created_at: String,
}

/// What the user did with a suggestion (FR-67). A suggestion nobody decided is
/// absent, which is how it stays on offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionDecisionStatus {
    /// The user accepted; the Preferences change was applied.
    Accepted,
    /// The user ignored it; Boba does not ask again for the same change.
    Ignored,
}

impl SuggestionDecisionStatus {
    /// Every status.
    pub const ALL: [SuggestionDecisionStatus; 2] = [
        SuggestionDecisionStatus::Accepted,
        SuggestionDecisionStatus::Ignored,
    ];

    /// The stored form.
    pub fn as_str(self) -> &'static str {
        match self {
            SuggestionDecisionStatus::Accepted => "accepted",
            SuggestionDecisionStatus::Ignored => "ignored",
        }
    }

    /// Parse the stored form, `None` for anything else.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|status| status.as_str() == value)
    }
}

/// One suggestion the user answered (FR-67), so the same change is never
/// suggested twice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuggestionDecision {
    /// The suggestion id; see [`Suggestion::id`].
    pub id: String,
    pub status: SuggestionDecisionStatus,
    pub updated_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_reason_round_trips_through_its_stored_form() {
        for reason in DismissReason::ALL {
            assert_eq!(DismissReason::parse(reason.as_str()), Some(reason));
        }
        assert_eq!(DismissReason::parse("bored"), None);
    }

    /// FR-67 AC3: only reasons a Preferences change can capture point at one.
    #[test]
    fn only_expressible_reasons_point_at_a_preferences_change() {
        assert_eq!(
            DismissReason::Company.prefs_change(),
            Some(SuggestionKind::ExcludeCompany)
        );
        assert_eq!(
            DismissReason::Location.prefs_change(),
            Some(SuggestionKind::ExcludeOnsite)
        );
        for reason in [
            DismissReason::NotInterested,
            DismissReason::NotEligible,
            DismissReason::Salary,
            DismissReason::Other,
        ] {
            assert_eq!(reason.prefs_change(), None);
        }
    }

    /// The same company spelled differently is one suggestion, and its id reads
    /// back to the same change.
    #[test]
    fn a_company_suggestion_is_one_per_company_however_it_is_spelled() {
        let a = Suggestion::ExcludeCompany("Boba  Labs".to_string());
        let b = Suggestion::ExcludeCompany("boba labs".to_string());
        assert_eq!(a.id(), b.id());
        assert_eq!(a.id(), "exclude_company:boba labs");
        assert_eq!(Suggestion::parse(&a.id()), Some(b));
        assert_eq!(a.kind(), SuggestionKind::ExcludeCompany);
        assert_eq!(a.company(), Some("Boba  Labs"));
    }

    #[test]
    fn the_onsite_suggestion_round_trips_and_refuses_an_empty_company() {
        let onsite = Suggestion::ExcludeOnsite;
        assert_eq!(onsite.id(), "exclude_onsite");
        assert_eq!(Suggestion::parse("exclude_onsite"), Some(onsite.clone()));
        assert_eq!(onsite.company(), None);
        assert_eq!(SuggestionKind::ExcludeOnsite.as_str(), "exclude_onsite");
        assert_eq!(Suggestion::parse("exclude_company:"), None);
        assert_eq!(Suggestion::parse("something_else"), None);
    }

    #[test]
    fn a_decision_round_trips_through_its_stored_form() {
        for status in SuggestionDecisionStatus::ALL {
            assert_eq!(
                SuggestionDecisionStatus::parse(status.as_str()),
                Some(status)
            );
        }
        assert_eq!(SuggestionDecisionStatus::parse("maybe"), None);
    }
}
