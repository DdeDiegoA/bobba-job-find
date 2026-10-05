//! Settings and hash-chained audit use cases (AD-12, FR-89, FR-106).

pub mod audit;
pub mod clock;
pub mod settings;

#[cfg(test)]
mod testing;

pub use audit::{AuditService, IntentRequest, StartupReport, SystemError, TamperWarning};
pub use clock::{FixedClock, SystemClock};
pub use settings::{SettingKey, Settings, SettingsService, Theme};
