//! Job runs and the schedule (S5.1, AD-18; FR-41, FR-43, FR-47).
//!
//! The vocabulary every layer of a job run shares — the `job_run` row, the
//! [`JobsRepo`] port the store implements, and the two pure decisions the
//! scheduler makes:
//!
//! * [`Schedule::due_slots`] — which `scheduled_for` instants the current tick
//!   must run: the one slot that just arrived, plus one catch-up slot per day
//!   that passed while the machine was asleep (AC2).
//! * [`Dispatcher::drive`] — one row per `(job, scheduled_for)` (AC1), a manual
//!   run that joins a running one instead of starting a second (AC3), and a
//!   `skipped_locked` row plus one notification per slot while Boba is locked
//!   (AC4, ADR-009 §3).
//!
//! Nothing here touches the clock, the store or a notifier: the ports come in
//! as arguments, so every decision is deterministic under test (AD-17).

use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime, Time, UtcOffset, Weekday};

use crate::ports::clock::ClockPort;
use crate::ports::notifier::{Notification, NotifierPort};

/// The recurring jobs the one in-core scheduler owns (AD-18).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobKind {
    /// The daily search run (E5; the use case lands in S5.2).
    DailySearch,
    /// Sync mail for application replies (E8).
    EmailSync,
    /// Re-score open Offers after a preferences change (E6).
    Rescore,
    /// Retention sweep (S10.3).
    RetentionSweep,
}

impl JobKind {
    /// Every job, in the order they run when several are due at once.
    pub const ALL: [JobKind; 4] = [
        JobKind::DailySearch,
        JobKind::EmailSync,
        JobKind::Rescore,
        JobKind::RetentionSweep,
    ];

    /// The stable text form, which is also the `job_run.job` CHECK value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DailySearch => "daily_search",
            Self::EmailSync => "email_sync",
            Self::Rescore => "rescore",
            Self::RetentionSweep => "retention_sweep",
        }
    }

    /// Parse the stored text form.
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|job| job.as_str() == text)
    }
}

/// How a `job_run` ended (ADR-18; `skipped_locked` is ADR-009 §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    /// The run is executing; at most one per job at a time (FR-47).
    Running,
    /// The job finished without an error.
    Succeeded,
    /// The job reported an error.
    Failed,
    /// The store was locked when the slot was due: never a success (ADR-009).
    SkippedLocked,
}

impl RunStatus {
    /// The stable text form, which is also the `job_run.status` CHECK value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::SkippedLocked => "skipped_locked",
        }
    }

    /// Parse the stored text form.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "running" => Some(Self::Running),
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            "skipped_locked" => Some(Self::SkippedLocked),
            _ => None,
        }
    }
}

/// One `job_run` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRun {
    /// ULID minted when the row is opened.
    pub id: String,
    pub job: JobKind,
    /// The slot this row belongs to, UTC RFC 3339. Unique per job (AC1).
    pub scheduled_for: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub status: RunStatus,
    /// Diagnostic text for a failure or a skip; never user-facing prose.
    pub detail: Option<String>,
}

/// A repo-level failure, stringified so the port stays free of the adapter's
/// error types (the shape `SystemRepo` uses).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("jobs store error: {0}")]
pub struct JobsRepoError(pub String);

impl JobsRepoError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// What [`JobsRepo::open_run`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenOutcome {
    /// A new row was inserted: the caller executes the job.
    Started(JobRun),
    /// Another run of this job is executing (FR-47): no row was created for
    /// this slot, which stays due for a later tick.
    Busy(JobRun),
    /// This slot already has a row (it ran, failed or was skipped): the caller
    /// never starts it again (AC1).
    Joined(JobRun),
}

/// The `job_run` owner's write and read surface (AD-18; `core/system` owns the
/// table and the store adapter implements this port).
pub trait JobsRepo: Send + Sync {
    /// Open the run for `(job, scheduled_for)`, atomically: the store checks for
    /// a running run, then inserts, in one writer transaction. A second call for
    /// the same slot returns [`OpenOutcome::Joined`] with the existing row, so
    /// exactly one row exists per `(job, scheduled_for)` (AC1).
    fn open_run(
        &self,
        job: JobKind,
        scheduled_for: &str,
        started_at: &str,
    ) -> Result<OpenOutcome, JobsRepoError>;

    /// Record a due slot that could not run because the store is locked
    /// (AC4, ADR-009 §3). `Ok(Some(run))` means the row is new — the caller
    /// sends exactly one notification for it; `Ok(None)` means the slot was
    /// already recorded and nothing must be sent twice.
    fn record_skipped_locked(
        &self,
        job: JobKind,
        scheduled_for: &str,
        at: &str,
    ) -> Result<Option<JobRun>, JobsRepoError>;

    /// Close a running row with its terminal status.
    fn finish_run(
        &self,
        id: &str,
        finished_at: &str,
        status: RunStatus,
        detail: Option<&str>,
    ) -> Result<JobRun, JobsRepoError>;

    fn get_run(&self, job: JobKind, scheduled_for: &str) -> Result<Option<JobRun>, JobsRepoError>;

    /// The run executing right now for `job`, if any (FR-47).
    fn running_run(&self, job: JobKind) -> Result<Option<JobRun>, JobsRepoError>;

    /// The newest slot a job has a row for, whatever its status: the scheduler
    /// plans forward from here, so a slot never runs twice (AC1).
    fn last_run(&self, job: JobKind) -> Result<Option<JobRun>, JobsRepoError>;

    /// Close rows left `running` by a previous process (power loss, crash), so a
    /// stale row never blocks the single-run lock (FR-47). Returns how many.
    fn mark_interrupted(&self, at: &str, detail: &str) -> Result<u64, JobsRepoError>;
}

/// Every weekday, Monday first: the default an active day set starts from.
pub const ALL_DAYS: [Weekday; 7] = [
    Weekday::Monday,
    Weekday::Tuesday,
    Weekday::Wednesday,
    Weekday::Thursday,
    Weekday::Friday,
    Weekday::Saturday,
    Weekday::Sunday,
];

/// One daily schedule: the times a job runs and the days it is active (FR-41).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    /// Local run times, in order of the day.
    pub times: Vec<Time>,
    /// Active weekdays, local. Empty means "never" — the user turned it off.
    pub days: Vec<Weekday>,
}

impl Schedule {
    /// The compiled default: one run every morning at 08:00 local.
    pub fn daily_at(hour: u8, minute: u8) -> Self {
        Self {
            times: vec![Time::from_hms(hour, minute, 0).expect("a valid run time")],
            days: ALL_DAYS.to_vec(),
        }
    }

    /// Whether the schedule runs on `day`.
    pub fn runs_on(&self, day: Weekday) -> bool {
        self.days.contains(&day)
    }

    /// The slots that are due at `now` and must run, oldest first (AC1, AC2):
    ///
    /// * every slot strictly after `after` and not after `now`;
    /// * at most **one per missed day** — the day's last slot — so a machine
    ///   that slept through two run times wakes up with one run that day, and a
    ///   machine that slept for days catches up once per missed day (AC2);
    /// * never before `after`, and never outside the catch-up window the caller
    ///   widens `after` with, so a first launch does not replay history.
    ///
    /// `offset` is the local UTC offset at `now`; the caller reads it from the
    /// OS, which keeps DST honest without a timezone database.
    pub fn due_slots(
        &self,
        offset: UtcOffset,
        after: Option<OffsetDateTime>,
        now: OffsetDateTime,
    ) -> Vec<OffsetDateTime> {
        let now_local = now.to_offset(offset);
        let first_day = match after {
            Some(after) => after.to_offset(offset).date(),
            None => now_local.date(),
        };
        let mut slots = Vec::new();
        let mut day = first_day;
        while day <= now_local.date() {
            if self.runs_on(day.weekday()) {
                // The day's last run time that is not in the future. On a past
                // day that is the last time of the day; on one missed day only
                // this one slot runs (AC2).
                if let Some(time) = self.times.iter().rev().find(|time| {
                    let slot = day.with_time(**time).assume_offset(offset);
                    slot <= now
                }) {
                    let slot = day.with_time(*time).assume_offset(offset);
                    if after.is_none_or(|after| slot > after) {
                        slots.push(slot);
                    }
                }
            }
            let Some(next) = day.next_day() else { break };
            day = next;
        }
        slots
    }
}

/// How far back a missed slot is caught up: one run per missed day, at most a
/// week's worth. AD-18 leaves the window to the scheduler; a week keeps a
/// machine that was off for a month from firing thirty runs at once.
pub const CATCH_UP_WINDOW: Duration = Duration::days(7);

/// What [`Dispatcher::drive`] did with one due slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriveOutcome {
    /// A new row was opened: the caller runs the job.
    Started(JobRun),
    /// A manual "Run now" that joined the run already executing (AC3).
    Joined(JobRun),
    /// A scheduled slot that waited: the job is still executing its previous
    /// run, so this slot stays due and runs on a later tick (FR-47).
    Busy(JobRun),
    /// The store is locked: recorded as `skipped_locked` and notified (AC4).
    SkippedLocked(JobRun),
    /// The store is locked and this slot was already recorded: nothing to do.
    AlreadyRecorded,
}

/// The two decisions a due slot needs, over the repo, clock and notifier ports.
pub struct Dispatcher<'a> {
    pub repo: &'a dyn JobsRepo,
    pub clock: &'a dyn ClockPort,
    pub notifier: &'a dyn NotifierPort,
}

impl Dispatcher<'_> {
    /// Drive one due slot (AC1, AC3, AC4).
    ///
    /// `locked` is the store state this tick read (ADR-009 §3): while locked the
    /// slot is recorded `skipped_locked`, never as a success, and one
    /// notification is sent for it — a repeated tick for the same slot finds the
    /// row and stays quiet.
    pub fn drive(
        &self,
        job: JobKind,
        scheduled_for: &str,
        locked: bool,
    ) -> Result<DriveOutcome, JobsRepoError> {
        let now = self.clock.now_rfc3339();
        if locked {
            let Some(run) = self.repo.record_skipped_locked(job, scheduled_for, &now)? else {
                return Ok(DriveOutcome::AlreadyRecorded);
            };
            // A notification that cannot be shown never un-records the skip:
            // the row is the truth the UI shows afterwards.
            let _ = self.notifier.notify(Notification::LockedSearch);
            return Ok(DriveOutcome::SkippedLocked(run));
        }
        match self.repo.open_run(job, scheduled_for, &now)? {
            OpenOutcome::Started(run) => Ok(DriveOutcome::Started(run)),
            OpenOutcome::Busy(run) => Ok(DriveOutcome::Busy(run)),
            OpenOutcome::Joined(run) => Ok(DriveOutcome::Joined(run)),
        }
    }

    /// A manual "Run now" (FR-41, AC3): join the run already executing, or
    /// start one whose slot is the moment it was asked for.
    ///
    /// A manual run asked for **after** today's slot stands in for it: the next
    /// tick plans forward from this run's row, so the earlier slot is never due
    /// again (a second search minutes later would be the same search). Asked for
    /// before the slot, that slot still runs at its time — the run is idempotent
    /// (AD-18), so the pair is harmless.
    pub fn run_now(&self, job: JobKind) -> Result<DriveOutcome, JobsRepoError> {
        let now = self.clock.now_rfc3339();
        if let Some(running) = self.repo.running_run(job)? {
            return Ok(DriveOutcome::Joined(running));
        }
        match self.repo.open_run(job, &now, &now)? {
            OpenOutcome::Started(run) => Ok(DriveOutcome::Started(run)),
            OpenOutcome::Busy(run) | OpenOutcome::Joined(run) => Ok(DriveOutcome::Joined(run)),
        }
    }
}

/// What a started run executes. The slice that owns the use case registers one
/// per job (S5.2 registers `daily_search`); the scheduler runs it off the tick
/// thread and records its outcome, so a handler never touches `job_run` itself.
pub trait JobHandler: Send + Sync {
    /// Run the job to completion. `Err` is diagnostic text for `job_run.detail`,
    /// never user-facing prose; the row ends `failed`.
    fn run(&self, job: JobKind) -> Result<(), String>;
}

/// The schedules a tick reads: one per job the user turned on (FR-41).
pub type Schedules = std::collections::HashMap<JobKind, Schedule>;

/// The compiled default: one daily search every morning at 08:00 local, and the
/// retention sweep (S10.3, FR-113) every day at 03:00 local, when the machine
/// is quiet. The other jobs run on the schedule their own slice configures
/// (S5.3).
pub fn default_schedules() -> Schedules {
    Schedules::from([
        (JobKind::DailySearch, Schedule::daily_at(8, 0)),
        (JobKind::RetentionSweep, Schedule::daily_at(3, 0)),
    ])
}

/// Everything one scheduler tick needs, as references: the ports, the state the
/// caller read (store locked or not), and the schedule.
pub struct TickContext<'a> {
    pub repo: &'a dyn JobsRepo,
    pub clock: &'a dyn ClockPort,
    pub notifier: &'a dyn NotifierPort,
    /// The local UTC offset at this tick (the caller reads it from the OS).
    pub offset: UtcOffset,
    /// The store state this tick read (ADR-009 §3): while locked every due slot
    /// is recorded `skipped_locked` and notified, never run (AC4).
    pub locked: bool,
    pub schedules: &'a Schedules,
    /// When this install first ran the scheduler. The catch-up window starts
    /// here, so a fresh install never replays a week of history.
    pub started_at: Option<OffsetDateTime>,
}

/// One scheduler tick: plan each job from its newest row, then drive every due
/// slot (AC1, AC2, AC3, AC4). Returns what happened, for logging and tests.
///
/// A store failure stops that job for this tick (the next tick retries) but
/// never panics and never keeps another job from running.
pub fn tick(ctx: &TickContext<'_>) -> Vec<(JobKind, DriveOutcome)> {
    let Ok(now) = OffsetDateTime::parse(&ctx.clock.now_rfc3339(), &Rfc3339) else {
        return Vec::new();
    };
    let floor = now - CATCH_UP_WINDOW;
    let dispatcher = Dispatcher {
        repo: ctx.repo,
        clock: ctx.clock,
        notifier: ctx.notifier,
    };

    let mut outcomes = Vec::new();
    for job in JobKind::ALL {
        let Some(schedule) = ctx.schedules.get(&job) else {
            continue;
        };
        let last = ctx
            .repo
            .last_run(job)
            .ok()
            .flatten()
            .and_then(|run| OffsetDateTime::parse(&run.scheduled_for, &Rfc3339).ok());
        let after = last
            .or(ctx.started_at)
            .map(|start| start.max(floor))
            .or(Some(floor));
        for slot in schedule.due_slots(ctx.offset, after, now) {
            // `scheduled_for` is UTC (AD-17), whatever offset the schedule runs
            // in: the column is also the uniqueness key and the sort order, so
            // one instant must have exactly one spelling.
            let Ok(scheduled_for) = slot.to_offset(UtcOffset::UTC).format(&Rfc3339) else {
                continue;
            };
            match dispatcher.drive(job, &scheduled_for, ctx.locked) {
                Ok(outcome) => outcomes.push((job, outcome)),
                Err(_) => break,
            }
        }
    }
    outcomes
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use time::{Date, Month, UtcOffset};

    use super::*;

    /// A UTC instant, spelled the way the tests read it.
    fn utc(y: i32, m: u8, d: u8, h: u8, min: u8) -> OffsetDateTime {
        let date = Date::from_calendar_date(y, Month::try_from(m).unwrap(), d).unwrap();
        date.with_hms(h, min, 0).unwrap().assume_utc()
    }

    fn utc_offset(hours: i8) -> UtcOffset {
        UtcOffset::from_hms(hours, 0, 0).unwrap()
    }
    use crate::ports::notifier::NotifierError;

    struct FixedClock(&'static str);

    impl ClockPort for FixedClock {
        fn now_rfc3339(&self) -> String {
            self.0.to_string()
        }
    }

    #[derive(Default)]
    struct CountingNotifier {
        sent: Mutex<Vec<Notification>>,
    }

    impl NotifierPort for CountingNotifier {
        fn notify(&self, notification: Notification) -> Result<(), NotifierError> {
            self.sent.lock().unwrap().push(notification);
            Ok(())
        }
    }

    /// An in-memory `job_run` table with the adapter's contract: one row per
    /// `(job, scheduled_for)`, and a running run joins instead of a second one.
    #[derive(Default)]
    struct FakeRepo {
        rows: Mutex<Vec<JobRun>>,
    }

    impl FakeRepo {
        fn find(&self, job: JobKind, scheduled_for: &str) -> Option<JobRun> {
            self.rows
                .lock()
                .unwrap()
                .iter()
                .find(|r| r.job == job && r.scheduled_for == scheduled_for)
                .cloned()
        }
    }

    impl JobsRepo for FakeRepo {
        fn open_run(
            &self,
            job: JobKind,
            scheduled_for: &str,
            started_at: &str,
        ) -> Result<OpenOutcome, JobsRepoError> {
            if let Some(run) = self
                .rows
                .lock()
                .unwrap()
                .iter()
                .find(|r| r.job == job && r.status == RunStatus::Running)
            {
                return Ok(OpenOutcome::Busy(run.clone()));
            }
            if let Some(run) = self.find(job, scheduled_for) {
                return Ok(OpenOutcome::Joined(run));
            }
            let run = JobRun {
                id: format!("run-{}", self.rows.lock().unwrap().len() + 1),
                job,
                scheduled_for: scheduled_for.to_string(),
                started_at: started_at.to_string(),
                finished_at: None,
                status: RunStatus::Running,
                detail: None,
            };
            self.rows.lock().unwrap().push(run.clone());
            Ok(OpenOutcome::Started(run))
        }

        fn record_skipped_locked(
            &self,
            job: JobKind,
            scheduled_for: &str,
            at: &str,
        ) -> Result<Option<JobRun>, JobsRepoError> {
            if self.find(job, scheduled_for).is_some() {
                return Ok(None);
            }
            let run = JobRun {
                id: format!("run-{}", self.rows.lock().unwrap().len() + 1),
                job,
                scheduled_for: scheduled_for.to_string(),
                started_at: at.to_string(),
                finished_at: Some(at.to_string()),
                status: RunStatus::SkippedLocked,
                detail: None,
            };
            self.rows.lock().unwrap().push(run.clone());
            Ok(Some(run))
        }

        fn finish_run(
            &self,
            id: &str,
            finished_at: &str,
            status: RunStatus,
            detail: Option<&str>,
        ) -> Result<JobRun, JobsRepoError> {
            let mut rows = self.rows.lock().unwrap();
            let run = rows
                .iter_mut()
                .find(|r| r.id == id)
                .ok_or_else(|| JobsRepoError::new("no such run"))?;
            run.status = status;
            run.finished_at = Some(finished_at.to_string());
            run.detail = detail.map(str::to_string);
            Ok(run.clone())
        }

        fn get_run(
            &self,
            job: JobKind,
            scheduled_for: &str,
        ) -> Result<Option<JobRun>, JobsRepoError> {
            Ok(self.find(job, scheduled_for))
        }

        fn running_run(&self, job: JobKind) -> Result<Option<JobRun>, JobsRepoError> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .iter()
                .find(|r| r.job == job && r.status == RunStatus::Running)
                .cloned())
        }

        fn last_run(&self, job: JobKind) -> Result<Option<JobRun>, JobsRepoError> {
            let rows = self.rows.lock().unwrap();
            let mut rows: Vec<&JobRun> = rows.iter().filter(|r| r.job == job).collect();
            rows.sort_by(|a, b| a.scheduled_for.cmp(&b.scheduled_for));
            Ok(rows.last().map(|r| (*r).clone()))
        }

        fn mark_interrupted(&self, at: &str, detail: &str) -> Result<u64, JobsRepoError> {
            let mut count = 0;
            for run in self.rows.lock().unwrap().iter_mut() {
                if run.status == RunStatus::Running {
                    run.status = RunStatus::Failed;
                    run.finished_at = Some(at.to_string());
                    run.detail = Some(detail.to_string());
                    count += 1;
                }
            }
            Ok(count)
        }
    }

    #[test]
    fn every_configured_time_is_a_slot() {
        let schedule = Schedule {
            times: vec![
                Time::from_hms(8, 0, 0).unwrap(),
                Time::from_hms(18, 30, 0).unwrap(),
            ],
            days: ALL_DAYS.to_vec(),
        };

        // 08:00 arrives: only that slot is due.
        assert_eq!(
            schedule.due_slots(
                UtcOffset::UTC,
                Some(utc(2026, 10, 16, 0, 0)),
                utc(2026, 10, 16, 8, 0)
            ),
            vec![utc(2026, 10, 16, 8, 0)]
        );
        // 18:30 arrives: the second slot of the same day.
        assert_eq!(
            schedule.due_slots(
                UtcOffset::UTC,
                Some(utc(2026, 10, 16, 8, 0)),
                utc(2026, 10, 16, 18, 31)
            ),
            vec![utc(2026, 10, 16, 18, 30)]
        );
    }

    #[test]
    fn a_wake_after_a_sleep_runs_each_missed_day_once() {
        let schedule = Schedule {
            times: vec![
                Time::from_hms(8, 0, 0).unwrap(),
                Time::from_hms(18, 30, 0).unwrap(),
            ],
            days: ALL_DAYS.to_vec(),
        };

        // Asleep from yesterday 07:00 until today 10:00: one run yesterday (its
        // last slot) and one today (the slot already past) — never two per day.
        let slots = schedule.due_slots(
            UtcOffset::UTC,
            Some(utc(2026, 10, 15, 7, 0)),
            utc(2026, 10, 16, 10, 0),
        );

        assert_eq!(
            slots,
            vec![utc(2026, 10, 15, 18, 30), utc(2026, 10, 16, 8, 0)]
        );
    }

    #[test]
    fn a_slot_some_minutes_late_is_still_due_within_five_minutes() {
        // The scheduler ticks every minute, so the 08:00 slot is seen at 08:04
        // (AC2's five minutes) and runs.
        let schedule = Schedule::daily_at(8, 0);
        let slots = schedule.due_slots(
            UtcOffset::UTC,
            Some(utc(2026, 10, 16, 0, 0)),
            utc(2026, 10, 16, 8, 4),
        );

        assert_eq!(slots, vec![utc(2026, 10, 16, 8, 0)]);
    }

    #[test]
    fn an_inactive_day_has_no_slots() {
        // 2026-10-17 is a Saturday.
        let schedule = Schedule {
            times: vec![Time::from_hms(8, 0, 0).unwrap()],
            days: vec![Weekday::Monday, Weekday::Tuesday, Weekday::Wednesday],
        };

        assert!(
            schedule
                .due_slots(UtcOffset::UTC, None, utc(2026, 10, 17, 9, 0))
                .is_empty()
        );
        assert_eq!(
            schedule.due_slots(UtcOffset::UTC, None, utc(2026, 10, 19, 9, 0)),
            vec![utc(2026, 10, 19, 8, 0)]
        );
    }

    #[test]
    fn slots_are_local_wall_clock_even_at_an_offset() {
        // 08:00 in UTC-5 is 13:00 UTC.
        let schedule = Schedule::daily_at(8, 0);
        let slots = schedule.due_slots(
            utc_offset(-5),
            Some(utc(2026, 10, 16, 0, 0)),
            utc(2026, 10, 16, 14, 0),
        );

        assert_eq!(slots, vec![utc(2026, 10, 16, 13, 0)]);
    }

    #[test]
    fn a_slot_at_or_before_the_last_run_never_runs_again() {
        let schedule = Schedule::daily_at(8, 0);
        assert!(
            schedule
                .due_slots(
                    UtcOffset::UTC,
                    Some(utc(2026, 10, 16, 8, 0)),
                    utc(2026, 10, 16, 8, 5)
                )
                .is_empty(),
            "the 08:00 slot already has a row"
        );
    }

    #[test]
    fn a_due_slot_opens_exactly_one_row() {
        let repo = FakeRepo::default();
        let notifier = CountingNotifier::default();
        let clock = FixedClock("2026-10-16T08:00:00Z");
        let dispatcher = Dispatcher {
            repo: &repo,
            clock: &clock,
            notifier: &notifier,
        };

        let first = dispatcher
            .drive(JobKind::DailySearch, "2026-10-16T08:00:00Z", false)
            .unwrap();
        let second = dispatcher
            .drive(JobKind::DailySearch, "2026-10-16T08:00:00Z", false)
            .unwrap();

        assert!(matches!(first, DriveOutcome::Started(_)));
        assert!(matches!(second, DriveOutcome::Busy(_)));
        assert_eq!(repo.rows.lock().unwrap().len(), 1);
        assert!(notifier.sent.lock().unwrap().is_empty());
    }

    #[test]
    fn a_manual_run_joins_the_running_one() {
        let repo = FakeRepo::default();
        let notifier = CountingNotifier::default();
        let clock = FixedClock("2026-10-16T08:00:00Z");
        let dispatcher = Dispatcher {
            repo: &repo,
            clock: &clock,
            notifier: &notifier,
        };

        dispatcher
            .drive(JobKind::DailySearch, "2026-10-16T08:00:00Z", false)
            .unwrap();
        // "Run now", while the scheduled run is still executing (AC3).
        let manual = dispatcher.run_now(JobKind::DailySearch).unwrap();

        assert!(
            matches!(manual, DriveOutcome::Joined(_)),
            "a manual run during a scheduled run joins it"
        );
        assert_eq!(repo.rows.lock().unwrap().len(), 1, "no second row");
    }

    #[test]
    fn a_locked_store_records_the_skip_and_notifies_once() {
        let repo = FakeRepo::default();
        let notifier = CountingNotifier::default();
        let clock = FixedClock("2026-10-16T08:00:00Z");
        let dispatcher = Dispatcher {
            repo: &repo,
            clock: &clock,
            notifier: &notifier,
        };

        let first = dispatcher
            .drive(JobKind::DailySearch, "2026-10-16T08:00:00Z", true)
            .unwrap();
        let second = dispatcher
            .drive(JobKind::DailySearch, "2026-10-16T08:00:00Z", true)
            .unwrap();

        assert!(matches!(first, DriveOutcome::SkippedLocked(_)));
        assert!(matches!(second, DriveOutcome::AlreadyRecorded));
        let rows = repo.rows.lock().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, RunStatus::SkippedLocked);
        assert_ne!(rows[0].status, RunStatus::Succeeded, "never a success");
        assert_eq!(
            *notifier.sent.lock().unwrap(),
            vec![Notification::LockedSearch]
        );
    }

    #[test]
    fn a_stale_running_row_is_closed_on_startup() {
        let repo = FakeRepo::default();
        repo.rows.lock().unwrap().push(JobRun {
            id: "run-1".into(),
            job: JobKind::DailySearch,
            scheduled_for: "2026-10-15T08:00:00Z".into(),
            started_at: "2026-10-15T08:00:00Z".into(),
            finished_at: None,
            status: RunStatus::Running,
            detail: None,
        });

        let closed = repo
            .mark_interrupted("2026-10-16T08:00:00Z", "interrupted")
            .unwrap();

        assert_eq!(closed, 1);
        assert_eq!(repo.running_run(JobKind::DailySearch).unwrap(), None);
    }

    #[test]
    fn job_kinds_round_trip_their_stored_form() {
        for job in JobKind::ALL {
            assert_eq!(JobKind::parse(job.as_str()), Some(job));
        }
        for status in [
            RunStatus::Running,
            RunStatus::Succeeded,
            RunStatus::Failed,
            RunStatus::SkippedLocked,
        ] {
            assert_eq!(RunStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(JobKind::parse("nope"), None);
    }

    #[test]
    fn the_schedule_table_is_indexed_by_job() {
        let mut schedule: HashMap<JobKind, Schedule> = HashMap::new();
        schedule.insert(JobKind::DailySearch, Schedule::daily_at(8, 0));
        assert_eq!(schedule[&JobKind::DailySearch].times.len(), 1);
    }

    /// The daily search's schedule alone, for the ticks below: they exercise
    /// that job, and the compiled defaults also carry the retention sweep
    /// (03:00, S10.3) whose slots would otherwise join every assertion.
    fn daily_only() -> Schedules {
        Schedules::from([(JobKind::DailySearch, Schedule::daily_at(8, 0))])
    }

    #[test]
    fn a_tick_plans_each_job_from_its_newest_row() {
        let repo = FakeRepo::default();
        let notifier = CountingNotifier::default();
        let clock = FixedClock("2026-10-16T08:00:00Z");
        let schedules = daily_only();

        // The 08:00 slot of the day before was recorded: this tick runs 08:00.
        repo.rows.lock().unwrap().push(JobRun {
            id: "old".into(),
            job: JobKind::DailySearch,
            scheduled_for: "2026-10-15T08:00:00Z".into(),
            started_at: "2026-10-15T08:00:00Z".into(),
            finished_at: Some("2026-10-15T08:02:00Z".into()),
            status: RunStatus::Succeeded,
            detail: None,
        });

        let ticked = tick(&TickContext {
            repo: &repo,
            clock: &clock,
            notifier: &notifier,
            offset: UtcOffset::UTC,
            locked: false,
            schedules: &schedules,
            started_at: Some(utc(2026, 10, 1, 0, 0)),
        });

        assert_eq!(ticked.len(), 1);
        assert_eq!(ticked[0].0, JobKind::DailySearch);
        assert!(matches!(ticked[0].1, DriveOutcome::Started(_)));
        assert_eq!(
            repo.rows.lock().unwrap().len(),
            2,
            "one old row plus today's"
        );
    }

    #[test]
    fn a_tick_stores_the_slot_in_utc_whatever_the_local_offset() {
        // 08:00 at UTC-5 is 13:00 UTC; the row says UTC (AD-17).
        let repo = FakeRepo::default();
        let notifier = CountingNotifier::default();
        let clock = FixedClock("2026-10-16T14:00:00Z");
        let schedules = daily_only();

        let ticked = tick(&TickContext {
            repo: &repo,
            clock: &clock,
            notifier: &notifier,
            offset: utc_offset(-5),
            locked: false,
            schedules: &schedules,
            started_at: Some(utc(2026, 10, 16, 0, 0)),
        });

        assert_eq!(ticked.len(), 1, "one run was due");
        assert_eq!(
            repo.rows.lock().unwrap()[0].scheduled_for,
            "2026-10-16T13:00:00Z",
            "one instant, one UTC spelling"
        );
    }

    #[test]
    fn a_tick_after_a_long_sleep_catches_up_once_per_missed_day() {
        let repo = FakeRepo::default();
        let notifier = CountingNotifier::default();
        let schedules = daily_only();

        // Asleep from 14 Oct 07:00 to 16 Oct 09:00, with the 14 Oct run done.
        repo.rows.lock().unwrap().push(JobRun {
            id: "old".into(),
            job: JobKind::DailySearch,
            scheduled_for: "2026-10-14T08:00:00Z".into(),
            started_at: "2026-10-14T08:00:00Z".into(),
            finished_at: Some("2026-10-14T08:10:00Z".into()),
            status: RunStatus::Succeeded,
            detail: None,
        });
        let wake_clock = FixedClock("2026-10-16T09:00:00Z");

        // The wake: the 15 Oct slot starts; the 16 Oct slot waits (FR-47).
        let wake = tick(&TickContext {
            repo: &repo,
            clock: &wake_clock,
            notifier: &notifier,
            offset: UtcOffset::UTC,
            locked: false,
            schedules: &schedules,
            started_at: Some(utc(2026, 10, 1, 0, 0)),
        });
        assert_eq!(wake.len(), 2);
        assert!(matches!(wake[0].1, DriveOutcome::Started(_)));
        assert!(matches!(wake[1].1, DriveOutcome::Busy(_)));

        // The first catch-up run finishes; the next tick picks the rest up.
        let first = repo.rows.lock().unwrap()[1].clone();
        repo.finish_run(
            &first.id,
            "2026-10-16T09:02:00Z",
            RunStatus::Succeeded,
            None,
        )
        .unwrap();
        let next_clock = FixedClock("2026-10-16T09:03:00Z");
        let next = tick(&TickContext {
            repo: &repo,
            clock: &next_clock,
            notifier: &notifier,
            offset: UtcOffset::UTC,
            locked: false,
            schedules: &schedules,
            started_at: Some(utc(2026, 10, 1, 0, 0)),
        });

        let slots: Vec<String> = repo
            .rows
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.scheduled_for.clone())
            .collect();
        assert_eq!(
            slots,
            vec![
                "2026-10-14T08:00:00Z".to_string(),
                "2026-10-15T08:00:00Z".to_string(),
                "2026-10-16T08:00:00Z".to_string(),
            ],
            "one run per missed day, today's included"
        );
        assert_eq!(next.len(), 1);
        assert!(matches!(next[0].1, DriveOutcome::Started(_)));
    }

    #[test]
    fn a_locked_tick_records_the_skip_and_notifies_that_morning() {
        let repo = FakeRepo::default();
        let notifier = CountingNotifier::default();
        let clock = FixedClock("2026-10-16T09:00:00Z");
        let schedules = daily_only();

        let ticked = tick(&TickContext {
            repo: &repo,
            clock: &clock,
            notifier: &notifier,
            offset: UtcOffset::UTC,
            locked: true,
            schedules: &schedules,
            started_at: Some(utc(2026, 10, 16, 7, 0)),
        });
        let ticked_again = tick(&TickContext {
            repo: &repo,
            clock: &clock,
            notifier: &notifier,
            offset: UtcOffset::UTC,
            locked: true,
            schedules: &schedules,
            started_at: Some(utc(2026, 10, 16, 7, 0)),
        });

        assert_eq!(ticked.len(), 1);
        assert!(matches!(ticked[0].1, DriveOutcome::SkippedLocked(_)));
        assert!(
            ticked_again.is_empty(),
            "the slot already has its row, so the next tick is silent"
        );
        assert_eq!(
            *notifier.sent.lock().unwrap(),
            vec![Notification::LockedSearch]
        );
    }
}
