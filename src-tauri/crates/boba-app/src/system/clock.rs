//! Clock implementations for the `ClockPort` (AD-17).

use crate::ports::clock::ClockPort;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// The real wall clock: UTC now as RFC 3339.
pub struct SystemClock;

impl ClockPort for SystemClock {
    fn now_rfc3339(&self) -> String {
        OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
    }
}

/// A clock that always returns one instant, so use cases are deterministic in
/// tests (AD-17).
pub struct FixedClock(String);

impl FixedClock {
    pub fn new(instant: impl Into<String>) -> Self {
        Self(instant.into())
    }
}

impl ClockPort for FixedClock {
    fn now_rfc3339(&self) -> String {
        self.0.clone()
    }
}
