//! `SessionStore` — the browser profiles Session Board Sources browse in
//! (S11.1a, FR-30, AD-9/AD-10). Adapter: `boba-browser`.
//!
//! Each Session Board Source gets its own Chromium profile, so "Sign out /
//! clear session" deletes exactly one Source's cookies. The use case never
//! touches the filesystem: it asks this port.

use boba_domain::session::ClearReport;
use boba_domain::session::SessionError;

/// Where one Source's browser session lives, and its deletion.
pub trait SessionStore: Send + Sync {
    /// Delete `source_id`'s profile and its cookies; idempotent.
    fn clear(&self, source_id: &str) -> Result<ClearReport, SessionError>;
}

/// Reading a page inside one Source's session (S11.1b, FR-30, AD-10).
///
/// Adapter: `boba-browser`, over the user's own Chrome. The read script is
/// read-only by construction (the Session Board adapters' tests assert it):
/// the port opens the page and evaluates that script, nothing else. Cookies
/// never leave Chrome, so no value crosses this port besides the page JSON.
pub trait SessionPages: Send + Sync {
    /// Open `url` in `source_id`'s session profile, evaluate `script` and
    /// return its JSON. Failure is a plain message: the caller maps it.
    fn read(&self, source_id: &str, url: &str, script: &str) -> Result<String, String>;

    /// Close every browser this port opened for the run.
    fn close(&self) -> Result<(), String>;
}
