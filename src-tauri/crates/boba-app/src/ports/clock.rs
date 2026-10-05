//! `Clock` port — injectable UTC now, so use cases are deterministic under test
//! (AD-17). The real implementation (`SystemClock`) is `boba_app::system`.

/// Injectable UTC clock (AD-17: timestamps are UTC RFC 3339, local time is a UI
/// concern only).
pub trait ClockPort: Send + Sync {
    /// Now, as UTC RFC 3339, e.g. `2026-10-05T01:02:03.456Z`.
    fn now_rfc3339(&self) -> String;
}
