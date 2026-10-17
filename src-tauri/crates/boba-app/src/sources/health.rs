//! Source health (S4.8 AC1/AC3, FR-31).
//!
//! Health is a fold over `source_run` history, never a stored status: the run
//! rows are the one truth, and recomputing means a restart can never leave a
//! tile stale. The notification rule is a pure function of the same history:
//! exactly the run that turns a streak into three failures notifies, so a
//! fourth, fifth or sixth failure stays quiet (AC3).

use boba_domain::source::{RunStatus, SourceRunRecord};

use crate::ports::notifier::{Notification, NotifierPort};

/// The status a tile shows (AC1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum SourceStatus {
    /// The newest run succeeded, or the source has never run.
    Ok,
    /// The newest run failed, but not three times in a row yet.
    Degraded,
    /// Three or more consecutive failures (FR-31).
    Failing,
}

impl SourceStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Degraded => "degraded",
            Self::Failing => "failing",
        }
    }
}

/// Everything a Source tile shows about the source's last runs (AC1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceHealth {
    pub status: SourceStatus,
    /// `finished_at` of the newest completed run.
    pub last_run: Option<String>,
    /// `postings_found` of the newest completed run.
    pub offers_found: i64,
    /// The newest run's error, when it failed.
    pub last_error: Option<String>,
    /// How many failures the newest runs form (0 when the newest succeeded).
    pub consecutive_failures: u32,
}

impl SourceHealth {
    /// The health of a source that has never run.
    pub fn never_run() -> Self {
        Self {
            status: SourceStatus::Ok,
            last_run: None,
            offers_found: 0,
            last_error: None,
            consecutive_failures: 0,
        }
    }
}

/// Fold run history into the tile's health view (AC1).
///
/// `runs` is newest first. A `running` row is ignored: it has not finished and
/// says nothing about health yet.
pub fn health_from_runs(runs: &[SourceRunRecord]) -> SourceHealth {
    let mut health = SourceHealth::never_run();
    let Some(newest) = runs.iter().find(|run| run.status != RunStatus::Running) else {
        return health;
    };

    health.last_run = newest.finished_at.clone();
    health.offers_found = newest.postings_found;
    health.consecutive_failures = runs
        .iter()
        .filter(|run| run.status != RunStatus::Running)
        .take_while(|run| run.status == RunStatus::Failed)
        .count() as u32;
    health.status = match newest.status {
        RunStatus::Failed if health.consecutive_failures >= FAILING_THRESHOLD => {
            SourceStatus::Failing
        }
        RunStatus::Failed => SourceStatus::Degraded,
        _ => SourceStatus::Ok,
    };
    if newest.status == RunStatus::Failed {
        health.last_error = newest.error.clone();
    }
    health
}

/// How many consecutive failures turn a Source into `Failing` (FR-31).
pub const FAILING_THRESHOLD: u32 = 3;

/// Whether the run that just ended deserves the one notification of its streak.
///
/// `history` is the source's finished runs **before** this one, newest first.
/// True for exactly the third consecutive failure: the run itself failed, the
/// two before it failed, and the one before those did not (or does not exist).
pub fn should_notify(history: &[SourceRunRecord], just_finished: RunStatus) -> bool {
    if just_finished != RunStatus::Failed {
        return false;
    }
    let mut finished = history
        .iter()
        .filter(|run| run.status != RunStatus::Running);
    let first = finished.next();
    let second = finished.next();
    let third = finished.next();
    match (first, second, third) {
        (Some(first), Some(second), third) => {
            first.status == RunStatus::Failed
                && second.status == RunStatus::Failed
                && third.is_none_or(|run| run.status != RunStatus::Failed)
        }
        _ => false,
    }
}

/// Show the one notification a failure streak earns (AC3).
///
/// The run owner calls this right after `SourcesRepo::finish_run` with the
/// source's finished runs **before** this one, newest first. Returns whether
/// the notification was shown; a notifier failure is reported, never fatal —
/// the run itself has already been recorded (AD-18).
pub fn notify_if_streak_ended(
    notifier: &dyn NotifierPort,
    history: &[SourceRunRecord],
    just_finished: RunStatus,
) -> bool {
    if !should_notify(history, just_finished) {
        return false;
    }
    notifier.notify(Notification::SourceFailing).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::notifier::NotifierError;

    fn run(id: &str, status: RunStatus, found: i64, error: Option<&str>) -> SourceRunRecord {
        SourceRunRecord {
            id: id.into(),
            source_id: "remotive".into(),
            status,
            started_at: format!("2026-10-12T08:0{id}:00Z"),
            finished_at: Some(format!("2026-10-12T08:0{id}:30Z")),
            postings_found: found,
            error: error.map(str::to_string),
        }
    }

    #[test]
    fn a_source_that_never_ran_shows_ok_and_nothing_else() {
        let health = health_from_runs(&[]);
        assert_eq!(health, SourceHealth::never_run());
        assert_eq!(health.status, SourceStatus::Ok);
        assert_eq!(health.last_run, None);
    }

    #[test]
    fn a_running_row_is_not_health_until_it_finishes() {
        let mut running = run("3", RunStatus::Running, 0, None);
        running.finished_at = None;
        let history = vec![running, run("2", RunStatus::Ok, 7, None)];
        let health = health_from_runs(&history);
        assert_eq!(health.status, SourceStatus::Ok);
        assert_eq!(health.offers_found, 7);
        assert_eq!(health.last_run.as_deref(), Some("2026-10-12T08:02:30Z"));
    }

    #[test]
    fn the_newest_finished_run_decides_the_tile() {
        let history = vec![
            run("1", RunStatus::Ok, 12, None),
            run("2", RunStatus::Failed, 0, Some("status 503")),
        ];
        let health = health_from_runs(&history);
        assert_eq!(health.status, SourceStatus::Ok);
        assert_eq!(health.offers_found, 12);
        assert_eq!(health.last_error, None);
    }

    #[test]
    fn one_failure_is_degraded_and_three_are_failing() {
        let one = health_from_runs(&[run("1", RunStatus::Failed, 0, Some("timeout"))]);
        assert_eq!(one.status, SourceStatus::Degraded);
        assert_eq!(one.consecutive_failures, 1);
        assert_eq!(one.last_error.as_deref(), Some("timeout"));

        let two = health_from_runs(&[
            run("1", RunStatus::Failed, 0, Some("newest")),
            run("2", RunStatus::Failed, 0, Some("older")),
        ]);
        assert_eq!(two.status, SourceStatus::Degraded);
        assert_eq!(two.consecutive_failures, 2);
        assert_eq!(two.last_error.as_deref(), Some("newest"));

        let three = health_from_runs(&[
            run("1", RunStatus::Failed, 0, Some("newest")),
            run("2", RunStatus::Failed, 0, Some("older")),
            run("3", RunStatus::Failed, 0, None),
        ]);
        assert_eq!(three.status, SourceStatus::Failing);
        assert_eq!(three.consecutive_failures, 3);
    }

    #[test]
    fn a_success_resets_the_streak() {
        let history = vec![
            run("1", RunStatus::Ok, 3, None),
            run("2", RunStatus::Failed, 0, None),
            run("3", RunStatus::Failed, 0, None),
            run("4", RunStatus::Failed, 0, None),
        ];
        let health = health_from_runs(&history);
        assert_eq!(health.status, SourceStatus::Ok);
        assert_eq!(health.consecutive_failures, 0);
    }

    #[test]
    fn the_third_consecutive_failure_notifies_once() {
        // First failure: two before it did not fail (none exist).
        assert!(!should_notify(&[], RunStatus::Failed));
        // Second failure.
        assert!(!should_notify(
            &[run("1", RunStatus::Failed, 0, None)],
            RunStatus::Failed
        ));
        // Third failure: notify.
        assert!(should_notify(
            &[
                run("1", RunStatus::Failed, 0, None),
                run("2", RunStatus::Failed, 0, None),
            ],
            RunStatus::Failed
        ));
        // Fourth failure: the streak already notified.
        assert!(!should_notify(
            &[
                run("1", RunStatus::Failed, 0, None),
                run("2", RunStatus::Failed, 0, None),
                run("3", RunStatus::Failed, 0, None),
            ],
            RunStatus::Failed
        ));
    }

    #[test]
    fn a_success_never_notifies_and_breaks_the_streak() {
        assert!(!should_notify(
            &[
                run("1", RunStatus::Failed, 0, None),
                run("2", RunStatus::Failed, 0, None),
            ],
            RunStatus::Ok
        ));
        // The run after a success starts a new streak.
        assert!(!should_notify(
            &[
                run("1", RunStatus::Ok, 4, None),
                run("2", RunStatus::Failed, 0, None),
                run("3", RunStatus::Failed, 0, None),
            ],
            RunStatus::Failed
        ));
    }

    #[test]
    fn an_unfinished_row_does_not_count_as_a_failure() {
        let mut running = run("1", RunStatus::Running, 0, None);
        running.finished_at = None;
        // The running row is skipped, so only one failure precedes this one.
        assert!(!should_notify(
            &[running.clone(), run("2", RunStatus::Failed, 0, None)],
            RunStatus::Failed
        ));
        // Two finished failures before it are the third of the streak.
        assert!(should_notify(
            &[
                running,
                run("2", RunStatus::Failed, 0, None),
                run("3", RunStatus::Failed, 0, None),
            ],
            RunStatus::Failed
        ));
    }

    #[test]
    fn the_status_strings_are_the_ipc_values() {
        assert_eq!(SourceStatus::Ok.as_str(), "ok");
        assert_eq!(SourceStatus::Degraded.as_str(), "degraded");
        assert_eq!(SourceStatus::Failing.as_str(), "failing");
    }

    #[derive(Default)]
    struct CountingNotifier {
        shown: std::sync::Mutex<Vec<Notification>>,
        failing: bool,
    }

    impl NotifierPort for CountingNotifier {
        fn notify(&self, notification: Notification) -> Result<(), NotifierError> {
            self.shown.lock().unwrap().push(notification);
            if self.failing {
                Err(NotifierError::new("no notification service"))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn the_third_failure_shows_exactly_one_notification() {
        let notifier = CountingNotifier::default();
        let none: [SourceRunRecord; 0] = [];
        assert!(!notify_if_streak_ended(&notifier, &none, RunStatus::Failed));
        let one = [run("1", RunStatus::Failed, 0, None)];
        assert!(!notify_if_streak_ended(&notifier, &one, RunStatus::Failed));
        assert!(notifier.shown.lock().unwrap().is_empty(), "nothing yet");

        let two = [
            run("1", RunStatus::Failed, 0, None),
            run("2", RunStatus::Failed, 0, None),
        ];
        assert!(notify_if_streak_ended(&notifier, &two, RunStatus::Failed));
        assert_eq!(
            *notifier.shown.lock().unwrap(),
            vec![Notification::SourceFailing]
        );

        // The fourth failure does not notify again.
        let three = [
            run("0", RunStatus::Failed, 0, None),
            run("1", RunStatus::Failed, 0, None),
            run("2", RunStatus::Failed, 0, None),
        ];
        assert!(!notify_if_streak_ended(
            &notifier,
            &three,
            RunStatus::Failed
        ));
        assert_eq!(notifier.shown.lock().unwrap().len(), 1, "one per streak");
    }

    #[test]
    fn a_success_never_notifies() {
        let notifier = CountingNotifier::default();
        let history = [
            run("1", RunStatus::Failed, 0, None),
            run("2", RunStatus::Failed, 0, None),
        ];
        assert!(!notify_if_streak_ended(&notifier, &history, RunStatus::Ok));
        assert!(notifier.shown.lock().unwrap().is_empty());
    }

    #[test]
    fn a_notifier_that_fails_is_not_reported_as_shown() {
        let notifier = CountingNotifier {
            failing: true,
            ..CountingNotifier::default()
        };
        let history = [
            run("1", RunStatus::Failed, 0, None),
            run("2", RunStatus::Failed, 0, None),
        ];
        assert!(!notify_if_streak_ended(
            &notifier,
            &history,
            RunStatus::Failed
        ));
    }
}
