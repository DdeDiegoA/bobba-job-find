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
