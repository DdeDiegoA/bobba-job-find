//! Offer commands (S6.4, FR-63/FR-64): what Today and the Inbox read.
//!
//! One command, [`offers_inbox`], hands back the Inbox's cards — every Offer
//! with its newest ScoreCard (S6.3), its Sources (AD-19), its badges and, when a
//! Gate vetoed it, the Gate and its quote (FR-54). The card itself is built by
//! [`boba_app::offers::OffersInbox`]; this layer validates nothing, decides
//! nothing and maps a failure onto the typed IPC error (AD-3).
//!
//! The sections, the filters and the sort are the screen's (see
//! `src/features/inbox/list`): they are ways of showing this one list.
//!
//! The ports arrive through [`wiring::port`] (ADR-012): a locked or unmounted
//! store is a typed error, never a panic.

use boba_app::error::{AppError, AppResult, ErrorCode};
use boba_app::offers::{OfferCardView, OffersError, OffersInbox};
use boba_app::ports::repo::ingestion::IngestionRepo;
use boba_app::ports::repo::scoring::ScoringRepo;

use crate::wiring;

/// How much of the list one read pulls. The port is paged
/// (`offers(limit, offset)`) so the screen — not the store — decides how much it
/// reads; one page covers a year of searching, and the search-run slice (S5.x)
/// owns paging past it.
const OFFER_PAGE: usize = 500;

/// The read model over the mounted ports (ADR-012: ports come only from the
/// helper).
fn service(app: &tauri::AppHandle) -> AppResult<OffersInbox> {
    Ok(OffersInbox::new(
        wiring::port::<dyn IngestionRepo>(app)?,
        wiring::port::<dyn ScoringRepo>(app)?,
    ))
}

/// Map a use-case failure onto the typed IPC error (AD-3).
fn to_app(error: OffersError) -> AppError {
    AppError::new(ErrorCode::Internal, error.message_key())
}

/// One page of Offer cards, newest first (AC1/AC2).
#[tauri::command]
#[specta::specta]
pub fn offers_inbox(app: tauri::AppHandle) -> AppResult<Vec<OfferCardView>> {
    service(&app)?.cards(OFFER_PAGE, 0).map_err(to_app)
}

#[cfg(test)]
mod tests {
    use super::*;
    use boba_app::ports::repo::ingestion::IngestionRepoError;
    use boba_app::ports::repo::scoring::ScoringRepoError;

    /// AC3/AD-3: a failure crosses IPC as a key, never as raw text.
    #[test]
    fn failures_map_to_keys_the_catalogs_have() {
        let unmounted_inbox = to_app(OffersError::Postings(IngestionRepoError::Store(
            "disk".to_string(),
        )));
        assert_eq!(unmounted_inbox.code, ErrorCode::Internal);
        assert_eq!(unmounted_inbox.message_key, "errors.offers.inbox");

        let unmounted_scores = to_app(OffersError::Scores(ScoringRepoError::new("disk")));
        assert_eq!(unmounted_scores.code, ErrorCode::Internal);
        assert_eq!(unmounted_scores.message_key, "errors.scoring.store");
    }
}
