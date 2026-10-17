//! Posting lifecycle use cases: seen state, repost and staleness signals, and
//! expiry (S4.4, FR-50, FR-51, FR-52).
//!
//! The policy is pure and lives in [`boba_domain::posting::lifecycle`]; this
//! module is what moves it against the store:
//!
//! - [`mark_seen`] / [`mark_dismissed`] remember the version of a posting the
//!   user was told about, and [`should_notify`] answers with it — a re-sent,
//!   unchanged posting is not news (AC1).
//! - The store fills a posting's lifecycle row as it is ingested, so the
//!   staleness and repost signals exist from the first observation (AC2); this
//!   module reads them with [`signals`] for Q4 and refreshes them at run end.
//! - [`finish_run`] is the once-per-run sweep: postings the run did not see
//!   accrue a missing run, and the second consecutive miss — or a passed
//!   deadline — marks them *Possibly closed*, with the saved Applications
//!   warned (AC3).

use boba_domain::dedup::description_simhash;
use boba_domain::posting::Offer;
use boba_domain::posting::lifecycle::{
    ClosedWarning, LifecycleSignal, MISSING_RUNS_BEFORE_CLOSED, PostingLifecycle, is_past_deadline,
    material_hash, stale_since,
};

use crate::ports::repo::ingestion::{IngestionRepo, IngestionRepoError};

/// The window one search run covered (S5.1 owns the run record; this slice only
/// needs to know what it touched, FR-52).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunWindow {
    /// When the run started fetching. A posting seen at or after this instant
    /// was seen by this run; the store's `last_seen` carries the same clock.
    pub started_at: String,
    /// When the run ended. The instant a miss or an expiry is stamped with.
    pub ended_at: String,
    /// The Sources the run fetched. Postings of Sources it did not cover are
    /// not evidence of anything and are left alone.
    pub source_ids: Vec<String>,
}

/// What a finished run changed (AC3), so the run report can say it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunOutcome {
    /// Postings newly marked *Possibly closed*.
    pub possibly_closed: Vec<String>,
    /// Postings whose staleness signal fired this run (FR-51).
    pub stale: Vec<String>,
    /// Saved Applications on *Possibly closed* postings, for the warning the
    /// tracker shows (AC3).
    pub warnings: Vec<ClosedWarning>,
}

/// A lifecycle failure (AD-3: the UI translates `message_key`).
#[derive(Debug, Clone, thiserror::Error)]
pub enum LifecycleError {
    /// The posting the caller named is not in the store.
    #[error("posting {0} is not in the store")]
    MissingPosting(String),
    /// The lifecycle row could not be read or written.
    #[error(transparent)]
    Repo(#[from] IngestionRepoError),
}

impl LifecycleError {
    /// The i18n key the UI shows.
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::MissingPosting(_) => "errors.lifecycle.missing_posting",
            Self::Repo(_) => "errors.lifecycle.store",
        }
    }
}

/// Record that the user saw a posting now, remembering the version they saw
/// (FR-50). Seeing it again does not move `seen_at`: "first seen" is what the
/// lifecycle means by it.
pub fn mark_seen(
    repo: &dyn IngestionRepo,
    posting_id: &str,
    now: &str,
) -> Result<(), LifecycleError> {
    let offer = offer(repo, posting_id)?;
    let mut row = row_for(repo, &offer)?;
    row.seen_at.get_or_insert_with(|| now.to_string());
    refresh_fingerprint(&mut row, &offer);
    row.updated_at = now.to_string();
    Ok(repo.save_lifecycle(&row)?)
}

/// Record that the user dismissed a posting now (FR-50). A dismissal is a
/// decision about the version they saw, which is why the fingerprint moves: the
/// next re-send of the same content stays dismissed.
pub fn mark_dismissed(
    repo: &dyn IngestionRepo,
    posting_id: &str,
    now: &str,
) -> Result<(), LifecycleError> {
    let offer = offer(repo, posting_id)?;
    let mut row = row_for(repo, &offer)?;
    row.seen_at.get_or_insert_with(|| now.to_string());
    row.dismissed_at = Some(now.to_string());
    refresh_fingerprint(&mut row, &offer);
    row.updated_at = now.to_string();
    Ok(repo.save_lifecycle(&row)?)
}

/// Whether this posting is worth a notification (FR-50, AC1). A posting Boba
/// has no lifecycle row for is one it has never shown the user: news.
pub fn should_notify(repo: &dyn IngestionRepo, posting_id: &str) -> Result<bool, LifecycleError> {
    let offer = offer(repo, posting_id)?;
    Ok(match repo.lifecycle(posting_id)? {
        Some(row) => row.should_notify(&offer),
        None => true,
    })
}

/// The Q4 signals a posting carries (FR-51): staleness and repost.
pub fn signals(
    repo: &dyn IngestionRepo,
    posting_id: &str,
) -> Result<Vec<LifecycleSignal>, LifecycleError> {
    Ok(repo
        .lifecycle(posting_id)?
        .map(|row| row.signals())
        .unwrap_or_default())
}

/// The sweep a search run performs when it ends (FR-52, AC3).
///
/// For the Sources the run covered: a posting it did not see accrues a missing
/// run, and the second consecutive miss — or a deadline that has passed, seen
/// or not — marks it *Possibly closed*. A posting that comes back is reopened;
/// it keeps the repost signal the store recorded for it (FR-51), which is
/// history, not state.
pub fn finish_run(repo: &dyn IngestionRepo, run: &RunWindow) -> Result<RunOutcome, LifecycleError> {
    let mut outcome = RunOutcome::default();

    for offer in repo.postings_seen_by_run(&run.source_ids, &run.started_at)? {
        let mut row = row_for(repo, &offer)?;
        // Back in its Source: the missing streak is over, and a posting it had
        // closed is open again. A posting that is closed only because its
        // deadline passed stays closed: the source still listing it does not
        // revise the date. The repost signal the store recorded on the merge
        // stays either way — that is history, not state.
        let was_missing = row.missing_runs > 0;
        row.missing_runs = 0;
        row.disappeared_at = None;
        if was_missing {
            row.possibly_closed_at = None;
        }
        if row.stale_at.is_none()
            && let Some(since) = stale_since(&offer, &run.ended_at)
        {
            row.stale_at = Some(since);
            outcome.stale.push(offer.id.clone());
        }
        close_if_expired(&offer, &mut row, run, &mut outcome);
        row.updated_at = run.ended_at.clone();
        repo.save_lifecycle(&row)?;
    }

    for offer in repo.postings_missing_from_run(&run.source_ids, &run.started_at)? {
        let mut row = row_for(repo, &offer)?;
        row.missing_runs = row.missing_runs.saturating_add(1);
        row.disappeared_at
            .get_or_insert_with(|| run.ended_at.clone());
        if row.missing_runs >= MISSING_RUNS_BEFORE_CLOSED && !row.is_possibly_closed() {
            row.possibly_closed_at = Some(run.ended_at.clone());
            outcome.possibly_closed.push(offer.id.clone());
        }
        close_if_expired(&offer, &mut row, run, &mut outcome);
        row.updated_at = run.ended_at.clone();
        repo.save_lifecycle(&row)?;
    }

    outcome.warnings = repo.possibly_closed_with_applications()?;
    Ok(outcome)
}

/// Every *Possibly closed* posting that has saved Applications on it, so the
/// tracker warns on those cards (AC3).
pub fn closed_warnings(repo: &dyn IngestionRepo) -> Result<Vec<ClosedWarning>, LifecycleError> {
    Ok(repo.possibly_closed_with_applications()?)
}

fn close_if_expired(
    offer: &Offer,
    row: &mut PostingLifecycle,
    run: &RunWindow,
    outcome: &mut RunOutcome,
) {
    if is_past_deadline(offer, &run.ended_at) && !row.is_possibly_closed() {
        row.possibly_closed_at = Some(run.ended_at.clone());
        if !outcome.possibly_closed.contains(&offer.id) {
            outcome.possibly_closed.push(offer.id.clone());
        }
    }
}

fn refresh_fingerprint(row: &mut PostingLifecycle, offer: &Offer) {
    row.content_hash = material_hash(offer);
    row.description_simhash = description_simhash(&offer.description_text);
}

fn offer(repo: &dyn IngestionRepo, posting_id: &str) -> Result<Offer, LifecycleError> {
    repo.get_offer(posting_id)?
        .ok_or_else(|| LifecycleError::MissingPosting(posting_id.to_string()))
}

/// The posting's row, or the one a posting from before the row existed starts
/// with. `posting.first_seen` is the best "now" a missing row can be built on.
fn row_for(repo: &dyn IngestionRepo, offer: &Offer) -> Result<PostingLifecycle, LifecycleError> {
    Ok(match repo.lifecycle(&offer.id)? {
        Some(row) => row,
        None => PostingLifecycle::new(&offer.id, offer, &offer.first_seen),
    })
}

#[cfg(test)]
mod tests;
