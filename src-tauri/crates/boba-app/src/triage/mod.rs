//! Inbox triage use cases (S6.6, FR-54, FR-66, FR-67): Save, Dismiss (with an
//! optional reason, singly or in bulk), Undo, the Excluded VETO override, and
//! the Preferences change a repeated dismiss reason suggests.
//!
//! What is decided here and what is not:
//!
//! * **Here:** that Save creates an Application in `Saved` and nothing else
//!   (the same Offer saved twice is one Application); that a Dismiss moves the
//!   posting's lifecycle and records the reason the user gave; that Undo puts
//!   both back; that an override is only offered where a Gate actually vetoed;
//!   and when three dismissals for the same reason point at one Preferences
//!   change.
//! * **In the command layer:** *applying* the suggested change. Preferences are
//!   the Settings document (S3.7), which lives above this crate; the use case
//!   names the change and records the user's answer, and the command layer
//!   writes the document.
//! * **In the UI:** the 6-second Undo window (FR-66). The core supports Undo at
//!   any time; how long the offer stays on screen is the toast's business.

use std::sync::Arc;

use boba_domain::application::{
    AppStatus, Application, StatusEvent, StatusSource, new_application_id, new_status_event_id,
};
use boba_domain::posting::WorkMode;
use boba_domain::triage::{
    DismissReason, DismissalRecord, SUGGESTION_THRESHOLD, Suggestion, SuggestionDecision,
    SuggestionDecisionStatus, VetoOverride,
};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::applications::OfferSnapshot;
use crate::ingest::lifecycle;
use crate::ports::clock::ClockPort;
use crate::ports::repo::apply::{ApplyRepo, ApplyRepoError};
use crate::ports::repo::ingestion::{IngestionRepo, IngestionRepoError};
use crate::ports::repo::scoring::{ScoringRepo, ScoringRepoError};
use crate::ports::repo::triage::{TriageRepo, TriageRepoError};

/// A triage failure (AD-3: the UI translates `message_key`, never raw text).
#[derive(Debug, Clone, thiserror::Error)]
pub enum TriageError {
    /// The triage rows could not be read or written.
    #[error(transparent)]
    Triage(#[from] TriageRepoError),
    /// The Offer could not be read.
    #[error(transparent)]
    Postings(#[from] IngestionRepoError),
    /// The ScoreCard could not be read.
    #[error(transparent)]
    Scores(#[from] ScoringRepoError),
    /// The Application could not be written.
    #[error(transparent)]
    Apply(#[from] ApplyRepoError),
    /// The posting the caller named is not in the store.
    #[error("posting {0} is not in the store")]
    UnknownPosting(String),
    /// The Offer no Gate vetoed, so there is nothing to override.
    #[error("posting {0} is not excluded")]
    NotExcluded(String),
    /// The suggestion id is not one this build knows.
    #[error("unknown suggestion: {0}")]
    UnknownSuggestion(String),
}

impl TriageError {
    /// The i18n key the UI shows.
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::Triage(_) | Self::Postings(_) | Self::Scores(_) | Self::Apply(_) => {
                "errors.inbox_actions.store"
            }
            Self::UnknownPosting(_) => "errors.inbox_actions.unknown_offer",
            Self::NotExcluded(_) => "errors.inbox_actions.not_excluded",
            Self::UnknownSuggestion(_) => "errors.inbox_actions.unknown_suggestion",
        }
    }
}

/// What Save did (FR-66). `created = false` means this Offer already had an
/// Application — in any status, not only `Saved`. Save is idempotent: it never
/// creates a second card for one Offer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct SaveOutcome {
    pub application_id: String,
    pub created: bool,
}

/// The Preferences change Boba asks the user to accept (FR-67, AC3). `kind` is
/// `exclude_company` or `exclude_onsite`; the UI builds the sentence from it,
/// `company` and `count`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct SuggestionView {
    pub id: String,
    pub kind: String,
    pub company: Option<String>,
    pub count: u32,
}

/// What a Dismiss (single or bulk) did (FR-66).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct DismissOutcome {
    /// The Offers now dismissed.
    pub dismissed: Vec<String>,
    /// The ids whose posting is not in the store (a stale card).
    pub failed: Vec<String>,
    /// The change Boba suggests now, when this dismissal was the third (AC3).
    pub suggestion: Option<SuggestionView>,
}

/// What an override did (FR-54).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct OverrideOutcome {
    pub posting_id: String,
    /// The Gate the override overruled.
    pub gate: String,
}

/// The Inbox triage use cases over the owner ports.
pub struct TriageService {
    triage: Arc<dyn TriageRepo>,
    postings: Arc<dyn IngestionRepo>,
    scoring: Arc<dyn ScoringRepo>,
    apply: Arc<dyn ApplyRepo>,
    clock: Arc<dyn ClockPort>,
}

impl TriageService {
    pub fn new(
        triage: Arc<dyn TriageRepo>,
        postings: Arc<dyn IngestionRepo>,
        scoring: Arc<dyn ScoringRepo>,
        apply: Arc<dyn ApplyRepo>,
        clock: Arc<dyn ClockPort>,
    ) -> Self {
        Self {
            triage,
            postings,
            scoring,
            apply,
            clock,
        }
    }

    /// Save an Offer: create an Application in `Saved`, born with its first
    /// status event so the Tracker timeline never starts mid-story (FR-66).
    /// Saving an Offer that already has an Application — in any status — returns
    /// that one and creates nothing.
    pub fn save(&self, posting_id: &str) -> Result<SaveOutcome, TriageError> {
        let offer = self.offer(posting_id)?;
        if let Some(existing) = self
            .apply
            .applications()?
            .into_iter()
            .find(|application| application.posting_id == posting_id)
        {
            return Ok(SaveOutcome {
                application_id: existing.id,
                created: false,
            });
        }

        let now = self.clock.now_rfc3339();
        let application = Application {
            id: new_application_id(),
            posting_id: offer.id.clone(),
            status: AppStatus::Saved,
            offer_snapshot_json: OfferSnapshot::from(&offer).to_json(),
            notes: None,
            next_action: None,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        // The birth event is the same history every later move appends to
        // (AD-16); `from_status: None` is how a row that was never anywhere
        // else is written.
        let event = StatusEvent {
            id: new_status_event_id(),
            application_id: application.id.clone(),
            from_status: None,
            to_status: AppStatus::Saved,
            source: StatusSource::User,
            reason: None,
            created_at: now,
        };
        self.apply.application_create(&application, &event)?;
        Ok(SaveOutcome {
            application_id: application.id,
            created: true,
        })
    }

    /// Dismiss one or more Offers (FR-66): the posting's lifecycle says so
    /// (S4.4), and the optional reason is recorded as the signal FR-67 counts.
    /// A stale card in the batch is reported, not fatal.
    pub fn dismiss(
        &self,
        posting_ids: &[String],
        reason: Option<DismissReason>,
    ) -> Result<DismissOutcome, TriageError> {
        let now = self.clock.now_rfc3339();
        let mut dismissed = Vec::new();
        let mut failed = Vec::new();

        for posting_id in posting_ids {
            match lifecycle::mark_dismissed(self.postings.as_ref(), posting_id, &now) {
                Ok(()) => {
                    if let Some(reason) = reason {
                        self.triage.dismiss(&DismissalRecord {
                            posting_id: posting_id.clone(),
                            reason,
                            created_at: now.clone(),
                        })?;
                    }
                    dismissed.push(posting_id.clone());
                }
                Err(lifecycle::LifecycleError::MissingPosting(_)) => {
                    failed.push(posting_id.clone());
                }
                Err(lifecycle::LifecycleError::Repo(error)) => {
                    return Err(TriageError::Postings(error));
                }
            }
        }

        let suggestion = match reason {
            Some(reason) => self.suggest(reason)?,
            None => None,
        };
        Ok(DismissOutcome {
            dismissed,
            failed,
            suggestion,
        })
    }

    /// Undo a Dismiss: the lifecycle dismissal and its reason are both cleared,
    /// so the Offer is back exactly as it was (FR-66).
    pub fn undo_dismiss(&self, posting_id: &str) -> Result<bool, TriageError> {
        let cleared_reason = self.triage.undo_dismiss(posting_id)?;
        let cleared_lifecycle = match self.postings.lifecycle(posting_id)? {
            Some(mut row) if row.dismissed_at.is_some() => {
                row.dismissed_at = None;
                row.updated_at = self.clock.now_rfc3339();
                self.postings.save_lifecycle(&row)?;
                true
            }
            _ => false,
        };
        Ok(cleared_reason || cleared_lifecycle)
    }

    /// Override the Gate that vetoed an Offer (FR-54): record who let it
    /// through, and stop hiding it. An Offer no Gate vetoed has nothing to
    /// override, which is an error and not a no-op: the screen acted on a stale
    /// card.
    pub fn override_veto(&self, posting_id: &str) -> Result<OverrideOutcome, TriageError> {
        self.offer(posting_id)?;
        let Some(card) = self.scoring.latest_card(posting_id)? else {
            return Err(TriageError::NotExcluded(posting_id.to_string()));
        };
        let Some(veto) = card
            .gates
            .results
            .iter()
            .find(|result| result.verdict == boba_domain::scoring::gates::Verdict::Veto)
        else {
            return Err(TriageError::NotExcluded(posting_id.to_string()));
        };
        let gate = veto.gate.as_str().to_string();
        self.triage.override_veto(&VetoOverride {
            posting_id: posting_id.to_string(),
            gate: gate.clone(),
            created_at: self.clock.now_rfc3339(),
        })?;
        Ok(OverrideOutcome {
            posting_id: posting_id.to_string(),
            gate,
        })
    }

    /// Every Offer the user let past a VETO (FR-54), oldest first.
    pub fn overrides(&self) -> Result<Vec<VetoOverride>, TriageError> {
        Ok(self.triage.overrides()?)
    }

    /// Record the user's answer to a suggestion (FR-67). Applying the change
    /// itself is the command layer's (Preferences live above this crate).
    pub fn decide_suggestion(
        &self,
        id: &str,
        status: SuggestionDecisionStatus,
    ) -> Result<(), TriageError> {
        let suggestion =
            Suggestion::parse(id).ok_or_else(|| TriageError::UnknownSuggestion(id.to_string()))?;
        self.triage
            .save_suggestion_decision(&SuggestionDecision {
                id: suggestion.id(),
                status,
                updated_at: self.clock.now_rfc3339(),
            })
            .map_err(TriageError::from)
    }

    /// The Preferences change `reason` now calls for, when three dismissals with
    /// that reason point at one (FR-67 AC3) and the user has not answered it
    /// yet. `None` for a reason no preference can capture: Boba never suggests a
    /// change it cannot make.
    fn suggest(&self, reason: DismissReason) -> Result<Option<SuggestionView>, TriageError> {
        let Some(kind) = reason.prefs_change() else {
            return Ok(None);
        };
        let dismissals: Vec<DismissalRecord> = self
            .triage
            .dismissals()?
            .into_iter()
            .filter(|row| row.reason == reason)
            .collect();
        if dismissals.len() < SUGGESTION_THRESHOLD {
            return Ok(None);
        }

        // An Offer the store no longer holds teaches nothing.
        let offers: Vec<(DismissalRecord, boba_domain::posting::Offer)> = dismissals
            .into_iter()
            .filter_map(|row| match self.postings.get_offer(&row.posting_id) {
                Ok(Some(offer)) => Some(Ok((row, offer))),
                Ok(None) => None,
                Err(error) => Some(Err(TriageError::Postings(error))),
            })
            .collect::<Result<_, _>>()?;

        let (suggestion, count) = match kind {
            boba_domain::triage::SuggestionKind::ExcludeOnsite => {
                let count = offers
                    .iter()
                    .filter(|(_, offer)| offer.work_mode == WorkMode::Onsite)
                    .count();
                (Suggestion::ExcludeOnsite, count)
            }
            boba_domain::triage::SuggestionKind::ExcludeCompany => {
                // The same company however it is spelled is one suggestion; the
                // spelling the user saw first is the one the change writes.
                let mut counts: Vec<(String, String, usize)> = Vec::new();
                for (_, offer) in &offers {
                    let key = boba_domain::triage::normalize(&offer.company);
                    match counts.iter_mut().find(|(stored, ..)| stored == &key) {
                        Some((_, _, count)) => *count += 1,
                        None => counts.push((key, offer.company.clone(), 1)),
                    }
                }
                let Some((_, company, count)) = counts
                    .into_iter()
                    .max_by(|left, right| left.2.cmp(&right.2).then(right.0.cmp(&left.0)))
                else {
                    return Ok(None);
                };
                (Suggestion::ExcludeCompany(company), count)
            }
        };

        if count < SUGGESTION_THRESHOLD {
            return Ok(None);
        }
        let id = suggestion.id();
        // A change the user already accepted or ignored is not asked again.
        if self.triage.suggestion_status(&id)?.is_some() {
            return Ok(None);
        }
        Ok(Some(SuggestionView {
            id,
            kind: suggestion.kind().as_str().to_string(),
            company: suggestion.company().map(str::to_string),
            count: u32::try_from(count).unwrap_or(u32::MAX),
        }))
    }

    fn offer(&self, posting_id: &str) -> Result<boba_domain::posting::Offer, TriageError> {
        self.postings
            .get_offer(posting_id)?
            .ok_or_else(|| TriageError::UnknownPosting(posting_id.to_string()))
    }
}

#[cfg(test)]
pub(crate) mod testing;
#[cfg(test)]
mod tests;
