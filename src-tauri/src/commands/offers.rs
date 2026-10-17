//! Offer commands (S6.4, FR-63/FR-64; S6.5, FR-65): what Today, the Inbox and
//! the Offer detail read.
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
use boba_app::offers::{OfferCardView, OfferDetailView, OffersError, OffersInbox};
use boba_app::ports::repo::ingestion::IngestionRepo;
use boba_app::ports::repo::scoring::ScoringRepo;

use crate::wiring;

/// The most cards one read pulls. The port is paged (`offers(limit, offset)`),
/// so the screen — not the store — decides how much it reads: the Inbox asks for
/// the next page when it runs past this one (S5.2), and Today reads the first.
pub const OFFER_PAGE: u32 = 500;

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
///
/// `limit` is clamped to `1..=OFFER_PAGE` and `offset` is where the previous
/// page ended, so the Inbox pages past the first 500 instead of stopping there
/// (S5.2). Absent values read the first page, which is what Today and the
/// first paint want.
#[tauri::command]
#[specta::specta]
pub fn offers_inbox(
    app: tauri::AppHandle,
    limit: Option<u32>,
    offset: Option<u32>,
) -> AppResult<Vec<OfferCardView>> {
    let limit = limit.unwrap_or(OFFER_PAGE).clamp(1, OFFER_PAGE) as usize;
    let offset = offset.unwrap_or(0) as usize;
    service(&app)?.cards(limit, offset).map_err(to_app)
}

/// One Offer in full (S6.5, FR-65): the sanitised posting, its Gates and their
/// quotes, the factor breakdown, its Sources and the copies dedup merged, its
/// Apply Channels and — when one was written — the Explanation in `lang`, the
/// UI language the screen is drawing in.
#[tauri::command]
#[specta::specta]
pub fn offer_detail(
    app: tauri::AppHandle,
    posting_id: String,
    lang: String,
) -> AppResult<OfferDetailView> {
    service(&app)?.detail(&posting_id, &lang).map_err(to_app)
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

        let missing = to_app(OffersError::NotFound);
        assert_eq!(missing.code, ErrorCode::Internal);
        assert_eq!(missing.message_key, "errors.offers.not_found");
    }
}
