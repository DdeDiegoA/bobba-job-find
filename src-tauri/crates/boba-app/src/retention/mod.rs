//! Retention sweep (S10.3, FR-113, AD-18): old raw data is purged on schedule.
//!
//! Two windows, each with a compiled default the user can override in the
//! `setting` table ([`POSTINGS_DAYS_KEY`], [`EMAIL_DAYS_KEY`]):
//!
//! * **Offer raw payloads** (default 90 days): the `posting_source` rows that
//!   keep what a board sent, and the Offers the user never acted on. An Offer
//!   with an Application is acted on and stays, so the Tracker keeps working
//!   after the posting's raw copy is gone (architecture §7).
//! * **Stored email** (default 180 days): recruiter mail is third-party
//!   personal data (FR-38), so it expires on its own window (FR-113).
//!
//! Applications, sent Packages (`document`) and the audit log are never
//! touched: they are the user's record of what happened, and AD-12 keeps the
//! log append-only. The store runs on `secure_delete=ON` (ADR-004), so a
//! deleted row's pages are zeroed rather than left readable.
//!
//! Everything here is a decision plus a port: the SQL lives in the `boba-store`
//! adapter, and the clock comes in as an argument, so the cutoffs are
//! deterministic under test (AD-17). A sweep is idempotent, so a retried slot
//! deletes what is left and nothing else (AD-18).

use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime, UtcOffset};

use crate::ports::repo::system::SystemRepo;

/// The `setting` row with the Offer raw-payload window, in days.
pub const POSTINGS_DAYS_KEY: &str = "retention.postings_days";
/// The `setting` row with the stored-email window, in days.
pub const EMAIL_DAYS_KEY: &str = "retention.email_days";
/// The `setting` row with what the last sweep deleted (counts only), so
/// Settings → Privacy can say when it last ran without keeping any content.
pub const LAST_SWEEP_KEY: &str = "retention.last_sweep";
/// Raw Offer payloads are kept this long unless the user says otherwise.
pub const DEFAULT_POSTINGS_DAYS: u32 = 90;
/// Stored email is kept this long unless the user says otherwise.
pub const DEFAULT_EMAIL_DAYS: u32 = 180;
/// The longest window a setting may name (ten years): a typo must not mean
/// "keep everything forever" any more than a zero must mean "delete it all".
const MAX_DAYS: u32 = 3650;

/// How long each class of raw data is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    /// Days a raw Offer payload and an un-acted Offer survive.
    pub postings_days: u32,
    /// Days a stored email survives.
    pub email_days: u32,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            postings_days: DEFAULT_POSTINGS_DAYS,
            email_days: DEFAULT_EMAIL_DAYS,
        }
    }
}

impl RetentionPolicy {
    /// The policy the `setting` rows name, with the compiled default for any
    /// key that is unset, unreadable or not a sane window (a stored typo never
    /// stops the sweep, and never widens it into a wipe).
    pub fn from_settings(system: &dyn SystemRepo) -> Self {
        let default = Self::default();
        Self {
            postings_days: days(system, POSTINGS_DAYS_KEY).unwrap_or(default.postings_days),
            email_days: days(system, EMAIL_DAYS_KEY).unwrap_or(default.email_days),
        }
    }

    /// The oldest instant each class keeps, as UTC RFC 3339 (AD-17), ready for
    /// the adapter's text comparison.
    fn cutoffs(&self, now: OffsetDateTime) -> (String, String) {
        (
            cutoff(now, self.postings_days),
            cutoff(now, self.email_days),
        )
    }
}

/// One setting read into a usable window, or `None` when it must be ignored.
fn days(system: &dyn SystemRepo, key: &str) -> Option<u32> {
    let raw = system.setting_get(key).ok().flatten()?;
    raw.trim()
        .parse::<u32>()
        .ok()
        .filter(|days| (1..=MAX_DAYS).contains(days))
}

fn cutoff(now: OffsetDateTime, days: u32) -> String {
    (now.to_offset(UtcOffset::UTC) - Duration::days(i64::from(days)))
        .format(&Rfc3339)
        .expect("an RFC 3339 timestamp formats")
}

/// What one sweep deleted, per class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SweepReport {
    /// Un-acted Offers older than the Offer window.
    pub postings: u64,
    /// Raw payload rows (`posting_source`) older than the Offer window.
    pub raw_payloads: u64,
    /// Stored emails older than the email window.
    pub emails: u64,
}

/// A store-level failure, stringified so the port stays free of the adapter's
/// error type (the shape the other repo ports use).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("retention store error: {0}")]
pub struct RetentionRepoError(pub String);

impl RetentionRepoError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// Why a sweep could not finish. The run is recorded `failed` and the next
/// slot retries; no partial state needs undoing because every delete is
/// idempotent.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RetentionError {
    #[error(transparent)]
    Repo(#[from] RetentionRepoError),
}

/// The storage surface the sweep needs. The `boba-store` adapter is the only
/// implementation; the port keeps the use case testable without a database.
pub trait RetentionRepo: Send + Sync {
    /// Delete the Offers last seen before `before` that have no Application.
    /// Their `posting_source`/lifecycle/score rows cascade with them. Returns
    /// how many Offers went.
    fn purge_unacted_postings(&self, before: &str) -> Result<u64, RetentionRepoError>;

    /// Delete the raw payload rows fetched before `before`. A posting that is
    /// kept (it has an Application) loses its raw copy; the Application's
    /// frozen Offer snapshot is what the card shows. Returns how many rows went.
    fn purge_raw_payloads(&self, before: &str) -> Result<u64, RetentionRepoError>;

    /// Delete the stored emails received before `before`. Returns how many went.
    fn purge_email(&self, before: &str) -> Result<u64, RetentionRepoError>;
}

/// The record of one sweep, stored at [`LAST_SWEEP_KEY`]: when it ran and how
/// many rows of each class went. It holds counts only — no title, address or
/// body — so the record itself carries no data the sweep was meant to remove.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SweepRecord {
    /// When the sweep ran, UTC RFC 3339 (the clock's instant).
    pub at: String,
    pub postings: u64,
    pub raw_payloads: u64,
    pub emails: u64,
}

/// Store what a finished sweep deleted (counts only). A write that fails must
/// not fail the run: the deletion already happened and is idempotent anyway.
pub fn record(
    system: &dyn SystemRepo,
    report: &SweepReport,
    at: &str,
) -> Result<(), crate::ports::repo::system::SystemRepoError> {
    let record = SweepRecord {
        at: at.to_string(),
        postings: report.postings,
        raw_payloads: report.raw_payloads,
        emails: report.emails,
    };
    let json = serde_json::to_string(&record).expect("a record of counts serializes");
    system.setting_put(LAST_SWEEP_KEY, &json, at)
}

/// Run one sweep: both windows, every class, in a fixed order. `now` is the
/// clock's UTC instant, so the same instant always cuts in the same place.
pub fn sweep(
    repo: &dyn RetentionRepo,
    policy: &RetentionPolicy,
    now: OffsetDateTime,
) -> Result<SweepReport, RetentionError> {
    let (postings_before, email_before) = policy.cutoffs(now);
    Ok(SweepReport {
        postings: repo.purge_unacted_postings(&postings_before)?,
        raw_payloads: repo.purge_raw_payloads(&postings_before)?,
        emails: repo.purge_email(&email_before)?,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    use time::OffsetDateTime;

    use super::*;
    use crate::system::testing::FakeSystemRepo;

    const NOW: &str = "2026-10-17T12:00:00Z";

    #[derive(Default)]
    struct RecordingRepo {
        calls: Mutex<Vec<(&'static str, String)>>,
        postings: u64,
        raw: u64,
        emails: u64,
        fail: AtomicBool,
    }

    impl RecordingRepo {
        fn calls(&self) -> Vec<(&'static str, String)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl RetentionRepo for RecordingRepo {
        fn purge_unacted_postings(&self, before: &str) -> Result<u64, RetentionRepoError> {
            self.calls
                .lock()
                .unwrap()
                .push(("postings", before.to_string()));
            if self.fail.load(Ordering::SeqCst) {
                return Err(RetentionRepoError::new("store down"));
            }
            Ok(self.postings)
        }

        fn purge_raw_payloads(&self, before: &str) -> Result<u64, RetentionRepoError> {
            self.calls.lock().unwrap().push(("raw", before.to_string()));
            Ok(self.raw)
        }

        fn purge_email(&self, before: &str) -> Result<u64, RetentionRepoError> {
            self.calls
                .lock()
                .unwrap()
                .push(("email", before.to_string()));
            Ok(self.emails)
        }
    }

    fn now() -> OffsetDateTime {
        OffsetDateTime::parse(NOW, &Rfc3339).unwrap()
    }

    #[test]
    fn the_compiled_defaults_are_ninety_and_one_hundred_eighty_days() {
        let system = FakeSystemRepo::default();

        let policy = RetentionPolicy::from_settings(&system);

        assert_eq!(policy.postings_days, 90);
        assert_eq!(policy.email_days, 180);
    }

    #[test]
    fn stored_settings_override_each_window_separately() {
        let mut system = FakeSystemRepo::default();
        system.seed_setting(POSTINGS_DAYS_KEY, "30");
        system.seed_setting(EMAIL_DAYS_KEY, "365");

        let policy = RetentionPolicy::from_settings(&system);

        assert_eq!(policy.postings_days, 30);
        assert_eq!(policy.email_days, 365);
    }

    #[test]
    fn an_unusable_setting_falls_back_to_its_default() {
        // Not a number, zero days and a value older than the app: each is a
        // typo that must not turn into a wipe or a freeze.
        for value in ["soon", "0", "-3", "99999"] {
            let mut system = FakeSystemRepo::default();
            system.seed_setting(POSTINGS_DAYS_KEY, value);
            let policy = RetentionPolicy::from_settings(&system);
            assert_eq!(policy.postings_days, 90, "'{value}' is not a window");
        }
    }

    #[test]
    fn each_class_is_purged_before_its_own_cutoff() {
        let policy = RetentionPolicy {
            postings_days: 90,
            email_days: 180,
        };
        let repo = RecordingRepo::default();

        sweep(&repo, &policy, now()).unwrap();

        let calls = repo.calls();
        assert_eq!(
            calls,
            vec![
                ("postings", "2026-07-19T12:00:00Z".to_string()),
                ("raw", "2026-07-19T12:00:00Z".to_string()),
                ("email", "2026-04-20T12:00:00Z".to_string()),
            ],
            "postings and their payloads share a window; email has its own"
        );
    }

    #[test]
    fn the_report_counts_what_each_purge_deleted() {
        let policy = RetentionPolicy::default();
        let repo = RecordingRepo {
            postings: 3,
            raw: 7,
            emails: 2,
            ..RecordingRepo::default()
        };

        let report = sweep(&repo, &policy, now()).unwrap();

        assert_eq!(
            report,
            SweepReport {
                postings: 3,
                raw_payloads: 7,
                emails: 2,
            }
        );
    }

    #[test]
    fn the_last_sweep_is_recorded_as_counts_only() {
        let system = FakeSystemRepo::default();
        let report = SweepReport {
            postings: 4,
            raw_payloads: 9,
            emails: 1,
        };

        record(&system, &report, NOW).unwrap();

        let stored = system.setting_get(LAST_SWEEP_KEY).unwrap().unwrap();
        let record: SweepRecord = serde_json::from_str(&stored).unwrap();
        assert_eq!(
            record,
            SweepRecord {
                at: NOW.to_string(),
                postings: 4,
                raw_payloads: 9,
                emails: 1,
            },
            "a count per class and when: nothing about what the rows said"
        );
    }

    #[test]
    fn a_store_failure_fails_the_sweep() {
        let policy = RetentionPolicy::default();
        let repo = RecordingRepo::default();
        repo.fail.store(true, Ordering::SeqCst);

        let error = sweep(&repo, &policy, now()).unwrap_err();

        assert!(matches!(error, RetentionError::Repo(_)));
    }
}
