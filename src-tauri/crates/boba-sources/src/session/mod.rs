//! Session Board Sources (FR-30, AD-9, AD-10): LinkedIn and Indeed browsed in
//! the user's **own** logged-in session, never through an account Boba creates.
//!
//! Each adapter reads pages through the [`SessionPages`] port (the user's own
//! Chrome, one profile per Source) and is gated by the shared
//! [`RunControl`]/`SessionRun` policy: a Source runs only with a live opt-in,
//! no challenge pause and a page budget for the day. A challenge pauses the
//! Source and notifies the user; Boba never solves it.

pub mod indeed;
pub mod linkedin;
pub mod sdk;

use std::sync::Arc;

use boba_app::ports::session::SessionPages;
use boba_domain::source::SourceRecord;

use crate::sdk::SourceAdapter;
use crate::session::sdk::RunControl;

/// The `source` rows for both boards, disabled until the per-Source opt-in.
///
/// The app seeds them lazily (Connections and Settings → Privacy both list
/// them), and `SessionBoardService::enable` turns one on when the user accepts
/// the warning — so the opt-in path always finds its row.
pub fn records(now: &str) -> Vec<SourceRecord> {
    [linkedin::record(now), indeed::record(now)]
        .into_iter()
        .map(|mut record| {
            record.enabled = false;
            record
        })
        .collect()
}

/// The Session Board adapters a run may use (S11.1b), in a stable order.
///
/// They are ordinary [`SourceAdapter`]s: a run registers them next to the feeds
/// and queries them like any Source, while every page load still passes
/// `SessionRun` (opt-in, no pause, paced and audited against the daily cap).
pub fn builtin(
    pages: Arc<dyn SessionPages>,
    control: Arc<dyn RunControl>,
) -> Vec<Box<dyn SourceAdapter>> {
    vec![
        Box::new(linkedin::LinkedIn::new(
            Arc::clone(&pages),
            Arc::clone(&control),
        )),
        Box::new(indeed::Indeed::new(pages, control)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use boba_domain::source::{AccessClass, LegalTier};

    const NOW: &str = "2026-10-17T06:00:00.000Z";

    #[test]
    fn the_seeded_rows_are_prohibited_session_boards_off_until_the_opt_in() {
        let rows = records(NOW);
        let mut ids: Vec<&str> = rows.iter().map(|record| record.id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, vec!["indeed", "linkedin"]);
        for record in &rows {
            assert!(!record.enabled, "{} waits for its own opt-in", record.id);
            assert_eq!(record.access_class, AccessClass::SessionBrowser);
            assert_eq!(record.legal_tier, LegalTier::ProhibitedAutomation);
            assert_eq!(record.added_at, NOW);
        }
    }
}
