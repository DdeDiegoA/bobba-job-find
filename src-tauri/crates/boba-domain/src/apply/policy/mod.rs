//! The apply policy engine (S9.6, FR-83/FR-87, AD-11).
//!
//! Pure: [`decide`] is a function of the stored rules, the global Auto
//! settings, the facts of one candidate and the counts of what already went out
//! today. It reads no clock, no store and no LLM; the app layer computes the
//! facts and the caps and records the result.
//!
//! `decide` runs at every place a send can begin — attempt creation, Outbox
//! release and just before submit — so a value that changed in between (a
//! cancelled rule, a new duplicate, a cap reached) is re-checked there, never
//! trusted from an earlier screen (AC2).

pub mod decide;
pub mod pace;
pub mod rule;
pub mod settings;

pub use decide::{
    AutoAttemptEntry, Decision, PolicyCandidate, PolicyCounts, Reason, SessionAutoFacts,
    company_key, decide, new_auto_attempt_id,
};
pub use pace::{MAX_GAP_MINUTES, MIN_GAP_MINUTES, gap_minutes};
pub use rule::{ActiveHours, AutoRule, RuleChannel, RuleViolation};
pub use settings::{AutoSettings, EnableRefused, SettingsViolation, reset_to_defaults};
