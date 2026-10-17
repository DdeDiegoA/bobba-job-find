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

use crate::sdk::SourceAdapter;
use crate::session::sdk::RunControl;

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
