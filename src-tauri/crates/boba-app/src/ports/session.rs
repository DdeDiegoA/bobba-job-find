//! `SessionStore` — the browser profiles Session Board Sources browse in
//! (S11.1a, FR-30, AD-9/AD-10). Adapter: `boba-browser`.
//!
//! Each Session Board Source gets its own Chromium profile, so "Sign out /
//! clear session" deletes exactly one Source's cookies. The use case never
//! touches the filesystem: it asks this port.

use boba_domain::session::ClearReport;
use boba_domain::session::SessionError;

use crate::ports::browser::{BrowserError, Confirmation, PageField, Screenshot, Upload};

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

/// What a Session Board page asks for that Boba never answers itself (S11.2,
/// AD-10). The core maps it to the handoff the UI shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionChallenge {
    Captcha,
    LoginWall,
    UnusualActivity,
    RateLimited,
}

impl SessionChallenge {
    /// The stable wire name the UI and the pause record share.
    pub fn as_str(self) -> &'static str {
        match self {
            SessionChallenge::Captcha => "captcha",
            SessionChallenge::LoginWall => "login_wall",
            SessionChallenge::UnusualActivity => "unusual_activity",
            SessionChallenge::RateLimited => "rate_limited",
        }
    }
}

/// One read of the page a Session Board session is on (S11.2).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionPage {
    pub url: String,
    pub title: String,
    /// The visible text, for the confirmation evidence.
    pub text: String,
    pub fields: Vec<PageField>,
    /// Set when the page is a wall: the fill stops and the user takes over.
    pub challenge: Option<SessionChallenge>,
    /// The board put its apply form in a cross-origin frame Boba cannot type
    /// into (Indeed's smart-apply). The user fills that one by hand.
    pub embedded_frame: bool,
    /// Set when the page says the submission went through (FR-88).
    pub confirmation: Option<Confirmation>,
}

/// `SessionFormPort` — drive one Session Board apply in the user's own session
/// (S11.2, FR-91, AD-10). Adapter: `boba-browser` (`session_forms`).
///
/// One headed browser per Source, launched on that Source's dedicated profile
/// (S11.1a), over an ephemeral loopback CDP port. The port is synchronous like
/// [`crate::ports::browser::BrowserPort`]: the driver owns its runtime, and the
/// use case runs off the UI thread.
pub trait SessionFormPort: Send + Sync {
    /// Open `url` in `source_id`'s session and press the board's apply button,
    /// then read the step it opened. Never submits and never signs in.
    fn open(&self, source_id: &str, url: &str) -> Result<SessionPage, BrowserError>;

    /// Read the page the session is on now (after the user advanced a step).
    fn snapshot(&self) -> Result<SessionPage, BrowserError>;

    /// Type `value` into the field `handle` the last read reported. The value
    /// travels to the adapter and nowhere else (S9.4 AC1).
    fn fill_text(&self, handle: &str, value: &str) -> Result<(), BrowserError>;

    /// Choose a checkbox or radio `handle` (a click).
    fn choose(&self, handle: &str) -> Result<(), BrowserError>;

    /// Hand `file` to the file input `handle` (the verified CV or letter).
    fn attach(&self, handle: &str, file: &Upload) -> Result<(), BrowserError>;

    /// Capture the page and hash it, for the submission evidence (FR-88).
    fn screenshot(&self) -> Result<Screenshot, BrowserError>;

    /// Close the browser window this session opened.
    fn close(&self) -> Result<(), BrowserError>;
}
