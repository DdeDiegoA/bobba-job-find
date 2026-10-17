//! `NotifierPort` — native notifications for top matches, job outcomes and a
//! locked store (AD-18, AD-21). Adapter: `src-tauri` tray/notification glue.
//!
//! Use cases hand the adapter a [`Notification`] variant, never prose: the text
//! of each notification lives in the `locales/{en,es}/jobs.json` catalog and the
//! adapter resolves it in the UI locale (AD-21), exactly as the tray resolves
//! its menu labels.

/// A native notification the core asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notification {
    /// A scheduled slot was due while the store was locked: "Boba is locked.
    /// Open it to run today's search." (ADR-009 §3, AC4). One per slot.
    LockedSearch,
    /// A Session Board Source hit a challenge (CAPTCHA, sign-in wall, unusual
    /// activity, rate limit) and is paused until the user acts (FR-30, AD-10).
    /// One per pause, never per retry: Boba does not retry around a wall.
    SessionChallengePaused,
    /// A Source failed its third run in a row (S4.8 AC3, FR-31). One per
    /// streak: the fourth and later failures stay quiet until a run succeeds.
    SourceFailing,
}

/// A notification the OS refused to show.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("notification failed: {0}")]
pub struct NotifierError(pub String);

impl NotifierError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// The one way the core shows a native notification.
pub trait NotifierPort: Send + Sync {
    /// Show `notification` now; failing to show one is never fatal for the
    /// caller, which has already recorded what happened.
    fn notify(&self, notification: Notification) -> Result<(), NotifierError>;
}
