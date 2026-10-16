//! Applications and their status machine (FR-93/94, AD-16, AD-17).
//!
//! One [`Application`] is the user's pursuit of one Offer. Its status is the
//! single source of truth for the Tracker: the stored vocabulary is the upstream
//! one (`drafted, applied, interview, offer, hired, rejected, no_response,
//! offer_declined, withdrawn`) plus `saved`, and the board columns are derived
//! from it here, never kept in UI state (AC1).
//!
//! Every change is one [`StatusEvent`] (from → to, who caused it, why). The
//! writer is the apply area's `TransitionStatus` use case (AD-16); this module
//! only decides what the data means.

use serde::{Deserialize, Serialize};

/// A fresh ULID for an application, prefixed by type (AD-17: `app_…`).
pub fn new_application_id() -> String {
    format!("app_{}", ulid::Ulid::generate())
}

/// A fresh ULID for a status event, prefixed by type (AD-17: `evt_…`).
pub fn new_status_event_id() -> String {
    format!("evt_{}", ulid::Ulid::generate())
}

/// Where an Application stands (AC1). The stored vocabulary is upstream's plus
/// `saved` — the upstream tracker CSV leaves `saved` rows out because they are
/// not applications yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppStatus {
    Saved,
    Drafted,
    Applied,
    Interview,
    Offer,
    Hired,
    Rejected,
    NoResponse,
    OfferDeclined,
    Withdrawn,
}

impl AppStatus {
    /// Every status, in pipeline order (`offer_declined` last, with the exits).
    pub const ALL: [AppStatus; 10] = [
        AppStatus::Saved,
        AppStatus::Drafted,
        AppStatus::Applied,
        AppStatus::Interview,
        AppStatus::Offer,
        AppStatus::Hired,
        AppStatus::Rejected,
        AppStatus::NoResponse,
        AppStatus::OfferDeclined,
        AppStatus::Withdrawn,
    ];

    /// The stored form.
    pub fn as_str(self) -> &'static str {
        match self {
            AppStatus::Saved => "saved",
            AppStatus::Drafted => "drafted",
            AppStatus::Applied => "applied",
            AppStatus::Interview => "interview",
            AppStatus::Offer => "offer",
            AppStatus::Hired => "hired",
            AppStatus::Rejected => "rejected",
            AppStatus::NoResponse => "no_response",
            AppStatus::OfferDeclined => "offer_declined",
            AppStatus::Withdrawn => "withdrawn",
        }
    }

    /// Parse the stored form, `None` for anything else.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.as_str() == value)
    }

    /// The board column this status appears in (AC1: one machine, no UI state).
    pub fn stage(self) -> Stage {
        match self {
            AppStatus::Saved => Stage::Saved,
            AppStatus::Drafted => Stage::Preparing,
            AppStatus::Applied => Stage::Applied,
            AppStatus::Interview => Stage::Interviewing,
            AppStatus::Offer => Stage::Offer,
            AppStatus::Hired => Stage::Hired,
            AppStatus::Rejected => Stage::Rejected,
            AppStatus::NoResponse => Stage::NoResponse,
            AppStatus::OfferDeclined | AppStatus::Withdrawn => Stage::Withdrawn,
        }
    }

    /// Whether this status is past the point of no return (FR-93).
    pub fn is_terminal(self) -> bool {
        self.stage().is_terminal()
    }
}

/// A Tracker column (FR-93). Stage names are binding and not renameable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Saved,
    Preparing,
    Applied,
    Interviewing,
    Offer,
    Hired,
    Rejected,
    NoResponse,
    Withdrawn,
}

impl Stage {
    /// The columns, left to right.
    pub const ALL: [Stage; 9] = [
        Stage::Saved,
        Stage::Preparing,
        Stage::Applied,
        Stage::Interviewing,
        Stage::Offer,
        Stage::Hired,
        Stage::Rejected,
        Stage::NoResponse,
        Stage::Withdrawn,
    ];

    /// The id used in IPC and to build the i18n key `stage.<…>`.
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Saved => "saved",
            Stage::Preparing => "preparing",
            Stage::Applied => "applied",
            Stage::Interviewing => "interviewing",
            Stage::Offer => "offer",
            Stage::Hired => "hired",
            Stage::Rejected => "rejected",
            Stage::NoResponse => "no_response",
            Stage::Withdrawn => "withdrawn",
        }
    }

    /// Parse the column id, `None` for anything else.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.as_str() == value)
    }

    /// The status a card carries when the user drops it in this column. The
    /// board never maps a column to a status itself (AC1): `withdrawn` is the
    /// canonical exit, `offer_declined` comes only from upstream rows.
    pub fn status(self) -> AppStatus {
        match self {
            Stage::Saved => AppStatus::Saved,
            Stage::Preparing => AppStatus::Drafted,
            Stage::Applied => AppStatus::Applied,
            Stage::Interviewing => AppStatus::Interview,
            Stage::Offer => AppStatus::Offer,
            Stage::Hired => AppStatus::Hired,
            Stage::Rejected => AppStatus::Rejected,
            Stage::NoResponse => AppStatus::NoResponse,
            Stage::Withdrawn => AppStatus::Withdrawn,
        }
    }

    /// Whether the column is collapsed by default (FR-93, DESIGN §9.6).
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Stage::Hired | Stage::Rejected | Stage::NoResponse | Stage::Withdrawn
        )
    }
}

/// Who caused a status change (AC4: a user move, an approved Status Proposal or
/// an apply outcome all travel the same path).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusSource {
    /// The user dragged or keyboard-moved the card (AC2).
    User,
    /// The user approved a Status Proposal (FR-102).
    StatusProposal,
    /// The apply engine recorded a terminal outcome (FR-82).
    ApplyOutcome,
    /// The upstream-compatible import restored an application (FR-110).
    Import,
}

impl StatusSource {
    pub const ALL: [StatusSource; 4] = [
        StatusSource::User,
        StatusSource::StatusProposal,
        StatusSource::ApplyOutcome,
        StatusSource::Import,
    ];

    /// The stored form.
    pub fn as_str(self) -> &'static str {
        match self {
            StatusSource::User => "user",
            StatusSource::StatusProposal => "status_proposal",
            StatusSource::ApplyOutcome => "apply_outcome",
            StatusSource::Import => "import",
        }
    }

    /// Parse the stored form, `None` for anything else.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.as_str() == value)
    }
}

/// The user's pursuit of one Offer (FR-93/94). `offer_snapshot_json` freezes the
/// Offer as it was when the application was created, so the card keeps showing
/// what was applied to even after the posting is refreshed or expires.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Application {
    pub id: String,
    pub posting_id: String,
    pub status: AppStatus,
    /// The frozen Offer snapshot, as JSON (shape owned by `boba-app`).
    pub offer_snapshot_json: String,
    pub notes: Option<String>,
    pub next_action: Option<String>,
    /// UTC RFC 3339 (AD-17).
    pub created_at: String,
    pub updated_at: String,
}

/// One move in an application's history (AC2/AC4). `from_status` is `None` for
/// the row that records how the application was born.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusEvent {
    pub id: String,
    pub application_id: String,
    pub from_status: Option<AppStatus>,
    pub to_status: AppStatus,
    pub source: StatusSource,
    pub reason: Option<String>,
    pub created_at: String,
}

/// One document of the Package actually sent (AC3/FR-94). Read-only view of the
/// `document` rows the application owns (AD-16: read through the owner).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageItem {
    pub document_id: String,
    /// `cv` or `cover_letter`.
    pub kind: String,
    /// `en` or `es`.
    pub lang: String,
    pub version: i64,
    /// Set only when the document passed verification (AD-14).
    pub verified_at: Option<String>,
}

/// A move the status machine refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RuleError {
    #[error("the application is already in that status")]
    SameStatus,
}

/// Whether a move is allowed (AC4). Only a no-op is invalid: the four terminal
/// columns collapse for display (FR-93) but never trap the user, because Undo
/// (AC2) and a mis-drop both need a way back.
pub fn can_transition(from: AppStatus, to: AppStatus) -> Result<(), RuleError> {
    if from == to {
        return Err(RuleError::SameStatus);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_status_round_trips_and_parses_nothing_else() {
        assert_eq!(AppStatus::ALL.len(), 10);
        for status in AppStatus::ALL {
            assert_eq!(AppStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(AppStatus::parse("needs_check"), None);
    }

    #[test]
    fn the_stored_vocabulary_is_the_upstream_one_plus_saved() {
        let stored: Vec<&str> = AppStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            stored,
            vec![
                "saved",
                "drafted",
                "applied",
                "interview",
                "offer",
                "hired",
                "rejected",
                "no_response",
                "offer_declined",
                "withdrawn",
            ]
        );
    }

    #[test]
    fn every_status_has_exactly_one_board_column() {
        assert_eq!(Stage::ALL.len(), 9);
        assert_eq!(AppStatus::Saved.stage(), Stage::Saved);
        assert_eq!(AppStatus::Drafted.stage(), Stage::Preparing);
        assert_eq!(AppStatus::Applied.stage(), Stage::Applied);
        assert_eq!(AppStatus::Interview.stage(), Stage::Interviewing);
        assert_eq!(AppStatus::Offer.stage(), Stage::Offer);
        assert_eq!(AppStatus::Hired.stage(), Stage::Hired);
        assert_eq!(AppStatus::Rejected.stage(), Stage::Rejected);
        assert_eq!(AppStatus::NoResponse.stage(), Stage::NoResponse);
        // Upstream's `offer_declined` is the user walking away, like Withdrawn.
        assert_eq!(AppStatus::OfferDeclined.stage(), Stage::Withdrawn);
        assert_eq!(AppStatus::Withdrawn.stage(), Stage::Withdrawn);
    }

    // AC1: a drop targets a column, and the column names the status it writes.
    #[test]
    fn every_column_round_trips_through_its_canonical_status() {
        for stage in Stage::ALL {
            assert_eq!(stage.status().stage(), stage);
        }
        assert_eq!(Stage::Withdrawn.status(), AppStatus::Withdrawn);
    }

    #[test]
    fn the_four_terminal_columns_are_the_ones_that_collapse() {
        let terminal: Vec<&str> = Stage::ALL
            .iter()
            .filter(|stage| stage.is_terminal())
            .map(|stage| stage.as_str())
            .collect();
        assert_eq!(
            terminal,
            vec!["hired", "rejected", "no_response", "withdrawn"]
        );
    }

    #[test]
    fn stages_and_sources_round_trip() {
        for stage in Stage::ALL {
            assert_eq!(Stage::parse(stage.as_str()), Some(stage));
        }
        assert_eq!(Stage::parse("needs_check"), None);

        assert_eq!(StatusSource::ALL.len(), 4);
        for source in StatusSource::ALL {
            assert_eq!(StatusSource::parse(source.as_str()), Some(source));
        }
        assert_eq!(StatusSource::parse("llm"), None);
    }

    // AC2: Undo must be possible after any move, so a move is only invalid when
    // it changes nothing. Terminal columns are collapsed for display (FR-93),
    // they do not lock the user out of correcting a wrong drag.
    #[test]
    fn a_move_is_valid_unless_it_changes_nothing() {
        for from in AppStatus::ALL {
            for to in AppStatus::ALL {
                let allowed = can_transition(from, to).is_ok();
                assert_eq!(allowed, from != to, "{from:?} → {to:?}");
            }
        }
        assert_eq!(
            can_transition(AppStatus::Applied, AppStatus::Applied),
            Err(RuleError::SameStatus)
        );
        // A wrong terminal drag is undoable (AC2).
        assert!(can_transition(AppStatus::Hired, AppStatus::Interview).is_ok());
    }

    #[test]
    fn application_ids_are_prefixed_ulids() {
        let first = new_application_id();
        let second = new_application_id();
        assert!(first.starts_with("app_"));
        assert_ne!(first, second);
        assert_eq!(first.len(), "app_".len() + 26);
        assert!(new_status_event_id().starts_with("evt_"));
    }
}
