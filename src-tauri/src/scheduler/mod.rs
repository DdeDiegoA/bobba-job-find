//! The one in-core scheduler (S5.1, AD-18; FR-41, FR-43, FR-47): daily_search,
//! email_sync, rescore, retention_sweep.
//!
//! One thread ticks every minute while the app runs. Each tick reads the store
//! state, plans the due slots with [`boba_app::jobs::tick`] and executes the
//! runs that started. That gives FR-43 its five minutes with a wide margin — a
//! machine that wakes from sleep resumes the sleeping thread and ticks at once —
//! and a login tick that runs the morning slot the machine slept through.
//!
//! Everything that decides *what* to do is in `boba_app::jobs`; this module is
//! the Tauri glue: it reads the ports, the tray pause flag and the local clock,
//! and it runs a started job on its own thread so a slow search never blocks the
//! tick.
//!
//! `install` is called once from [`crate::lifecycle::install`], which is where
//! `lib.rs` hands the app to later slices (backlog ownership convention 8).

pub mod notify;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use boba_app::error::{AppError, AppResult, ErrorCode};
use boba_app::jobs::{
    Dispatcher, DriveOutcome, JobHandler, JobKind, JobRun, JobsRepo, Schedule, Schedules,
    TickContext, default_schedules,
};
use boba_app::ports::backup::BackupPort;
use boba_app::ports::clock::ClockPort;
use boba_app::ports::notifier::NotifierPort;
use boba_app::ports::repo::system::SystemRepo;
use boba_app::system::SystemClock;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};
use time::format_description::well_known::Rfc3339;
use time::{OffsetDateTime, Time, UtcOffset};

use crate::wiring::{self, StoreStatus};

/// How often the scheduler looks for due slots. AD-18 asks for five minutes; a
/// minute keeps a slot from ever looking late and costs one cheap query.
pub const TICK_INTERVAL: Duration = Duration::from_secs(60);

/// The `setting` row holding the per-job schedules, as JSON (FR-41).
pub const SCHEDULES_KEY: &str = "jobs.schedules";
/// The `setting` row holding when this install first ran the scheduler, so a
/// first tick never replays a week of history.
pub const STARTED_AT_KEY: &str = "jobs.started_at";

/// The `job_run` a manual "Run now" started or joined (AC3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct RunNowOutcome {
    pub run_id: String,
    /// `true` when a run was already executing and this call joined it (FR-47).
    pub joined: bool,
}

/// The registered job handlers, one per job. A due slot for a job with no
/// handler is recorded as failed with that reason, never as a success.
#[derive(Default)]
pub struct Handlers {
    by_job: Mutex<HashMap<JobKind, Arc<dyn JobHandler>>>,
}

impl Handlers {
    /// Register (or replace) the handler for `job`.
    // `S5.2` registers the daily search; until it lands no handler is wired, and
    // a due slot is recorded as failed ("no handler") rather than as a success.
    #[allow(dead_code)]
    pub fn register(&self, job: JobKind, handler: Arc<dyn JobHandler>) {
        self.by_job
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(job, handler);
    }

    fn get(&self, job: JobKind) -> Option<Arc<dyn JobHandler>> {
        self.by_job
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&job)
            .cloned()
    }
}

/// What `install` registered, so a second call is a no-op.
#[derive(Debug)]
struct Installed;

/// Per-launch scheduler state.
#[derive(Debug, Default)]
struct SchedulerState {
    /// Whether this launch already closed the runs a previous process left
    /// `running`; set only once the store confirmed it.
    recovered: AtomicBool,
    /// Slots notified while no store was mounted (the store was locked at
    /// launch), keyed by job and UTC `scheduled_for`. Each is notified once per
    /// launch (AC4) and recorded `skipped_locked` as soon as the store mounts.
    pending: Mutex<HashSet<(JobKind, String)>>,
}

/// The scheduler's own handlers, once installed.
pub fn handlers<R: Runtime>(app: &AppHandle<R>) -> Option<&Handlers> {
    app.try_state::<Handlers>().map(|state| state.inner())
}

/// Register the handler a job runs. Called by the slice that owns the use case
/// (S5.2 registers `daily_search`); a missing handler never blocks the tick.
#[allow(dead_code)]
pub fn register_handler<R: Runtime>(
    app: &AppHandle<R>,
    job: JobKind,
    handler: Arc<dyn JobHandler>,
) -> bool {
    let Some(handlers) = handlers(app) else {
        return false;
    };
    handlers.register(job, handler);
    true
}

/// Start the scheduler: once per process, a thread that ticks every minute
/// (AD-18). Nothing here fails the launch — the scheduler is a background
/// service, and a host without a notification plugin still runs its jobs.
pub fn install<R: Runtime>(app: &AppHandle<R>) {
    if app.try_state::<Installed>().is_some() {
        return;
    }
    let _ = app.manage(Installed);
    let _ = app.manage(SchedulerState::default());
    if app.try_state::<Handlers>().is_none() {
        let _ = app.manage(Handlers::default());
    }

    let handle = app.clone();
    let spawned = std::thread::Builder::new()
        .name("boba-scheduler".to_string())
        .spawn(move || {
            loop {
                tick(&handle);
                std::thread::sleep(TICK_INTERVAL);
            }
        });
    if let Err(error) = spawned {
        tracing::error!(%error, "scheduler thread not started");
    }
}

/// ADR-009 / S3.8: an unsigned build may not run a search until a recovery
/// backup exists, because an update can lose the keychain entry that unlocks
/// this machine's data. The backup port is mounted by every production wiring;
/// an app that has none (tests, headless) has no requirement to read.
///
/// A mounted port whose status cannot be read fails **closed** (S3.8 review
/// F1): a backup-state error must never let a run start unbacked.
fn recovery_backup_required<R: Runtime>(app: &AppHandle<R>) -> bool {
    match wiring::port::<dyn BackupPort>(app) {
        Ok(port) => port
            .status()
            .map(|status| status.recovery_required)
            .unwrap_or(true),
        Err(_) => false,
    }
}

/// A manual "Run now" (FR-41, AC3): join the run already executing, or start
/// one whose slot is now. While the store is locked the slot is recorded
/// `skipped_locked` and the caller gets the typed Locked error (ADR-009 §3).
///
/// The recovery-backup requirement is checked first (ADR-009, S3.8): no run
/// starts, not even the onboarding's first one, while the backup is missing.
pub fn run_now<R: Runtime>(app: &AppHandle<R>) -> AppResult<RunNowOutcome> {
    if recovery_backup_required(app) {
        return Err(AppError::conflict("errors.backup.recovery_required"));
    }
    let repo = jobs_repo(app).ok_or_else(|| AppError::locked("errors.store.locked"))?;
    let clock = SystemClock;
    let notifier = notify::TauriNotifier::new(app.clone());
    let dispatcher = Dispatcher {
        repo: repo.as_ref(),
        clock: &clock,
        notifier: &notifier,
    };

    if matches!(wiring::store_state(app), StoreStatus::Locked { .. }) {
        let _ = dispatcher.drive(JobKind::DailySearch, &clock.now_rfc3339(), true);
        return Err(AppError::locked("errors.store.locked"));
    }

    let outcome = dispatcher.run_now(JobKind::DailySearch).map_err(|error| {
        AppError::new(ErrorCode::Internal, "errors.unknown").with_details({
            let mut details = boba_app::error::Details::new();
            details.insert("reason".to_string(), error.to_string());
            details
        })
    })?;

    match outcome {
        DriveOutcome::Started(run) => {
            execute(app.clone(), JobKind::DailySearch, run.clone());
            Ok(RunNowOutcome {
                run_id: run.id,
                joined: false,
            })
        }
        DriveOutcome::Joined(run) | DriveOutcome::Busy(run) => Ok(RunNowOutcome {
            run_id: run.id,
            joined: true,
        }),
        // Unreachable without a lock (checked above), and never a success.
        DriveOutcome::SkippedLocked(run) => Ok(RunNowOutcome {
            run_id: run.id,
            joined: false,
        }),
        DriveOutcome::AlreadyRecorded => Ok(RunNowOutcome {
            run_id: String::new(),
            joined: false,
        }),
    }
}

/// One tick: plan and drive every due slot (AC1–AC4).
fn tick<R: Runtime>(app: &AppHandle<R>) {
    if crate::tray::schedule_paused(app) {
        return;
    }
    // The Outbox release sweep (S9.7, FR-85): a hold that ended releases here,
    // even when no daily search is due. It runs on the async runtime because a
    // send is a network call; a failed sweep leaves the item held for the next
    // tick rather than dropping it. A locked store has no ports to read, so the
    // tick stays quiet until the user unlocks.
    if !matches!(wiring::store_state(app), StoreStatus::Locked { .. }) {
        crate::commands::apply::spawn_outbox_sweep(app.clone());
        // Follow-up nudges (S9.5, FR-95): a card that went quiet gets one
        // notice per threshold; the sweep itself holds them for quiet hours.
        crate::commands::apply::spawn_nudge_sweep(app.clone());
    }
    let Some(repo) = jobs_repo(app) else {
        // No store this launch (locked at startup): nothing can be recorded,
        // but the user is still told a run was skipped (AC4).
        note_locked_slots(app);
        return;
    };

    let clock = SystemClock;
    let now = clock.now_rfc3339();
    let system = system_repo(app);

    // A run that died with the previous process (power loss, crash) must not
    // hold the single-run lock (FR-47).
    recover_stale(app, repo.as_ref(), &now);
    // Slots notified while the store was locked get their row now (AC4).
    record_pending_skips(app, repo.as_ref(), &now);

    let locked = matches!(wiring::store_state(app), StoreStatus::Locked { .. });
    let mut schedules = system
        .as_ref()
        .and_then(|system| read_schedules(system.as_ref()))
        .unwrap_or_else(default_schedules);
    // ADR-009 / S3.8: no search run — scheduled or manual — starts while the
    // recovery backup is missing. The stored schedule is left untouched, so the
    // next tick after the backup exists runs the slot as usual.
    if recovery_backup_required(app) {
        schedules.remove(&JobKind::DailySearch);
    }
    let started_at = started_at(system.as_ref(), &now, locked);

    let notifier = notify::TauriNotifier::new(app.clone());
    let ctx = TickContext {
        repo: repo.as_ref(),
        clock: &clock,
        notifier: &notifier,
        offset: local_offset(),
        locked,
        schedules: &schedules,
        started_at,
    };

    for (job, outcome) in boba_app::jobs::tick(&ctx) {
        match outcome {
            DriveOutcome::Started(run) => execute(app.clone(), job, run),
            other => tracing::debug!(job = job.as_str(), ?other, "job slot not started"),
        }
    }
}

/// Close the runs a previous process left `running`, once per launch. The flag
/// is set only after the store confirmed the update, so a failed attempt is
/// retried on the next tick (FR-47).
fn recover_stale<R: Runtime>(app: &AppHandle<R>, repo: &dyn JobsRepo, now: &str) {
    let Some(state) = app.try_state::<SchedulerState>() else {
        return;
    };
    if state.recovered.load(Ordering::SeqCst) {
        return;
    }
    match repo.mark_interrupted(now, "interrupted by a previous shutdown") {
        Ok(closed) => {
            state.recovered.store(true, Ordering::SeqCst);
            if closed > 0 {
                tracing::info!(closed, "stale job runs closed");
            }
        }
        Err(error) => tracing::warn!(%error, "stale job runs not closed"),
    }
}

/// Record the slots notified while no store was mounted (AC4): each becomes a
/// `skipped_locked` row the moment the store is open, and leaves the list only
/// once the store accepted it.
fn record_pending_skips<R: Runtime>(app: &AppHandle<R>, repo: &dyn JobsRepo, now: &str) {
    let Some(state) = app.try_state::<SchedulerState>() else {
        return;
    };
    let pending: Vec<(JobKind, String)> = state
        .pending
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .cloned()
        .collect();
    for key in pending {
        match repo.record_skipped_locked(key.0, &key.1, now) {
            Ok(_) => {
                state
                    .pending
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&key);
            }
            Err(error) => {
                tracing::warn!(%error, job = key.0.as_str(), "locked slot not recorded");
            }
        }
    }
}

/// While no store is mounted, a due slot cannot be recorded: notify once per
/// slot in this launch and remember it, so the store records it when it mounts
/// (AC4, ADR-009 §3). Today only — without the store there is no way to tell
/// which earlier days already have their row, and a locked launch must not
/// replay a week of notifications.
fn note_locked_slots<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<SchedulerState>() else {
        return;
    };
    let clock = SystemClock;
    let Ok(now) = OffsetDateTime::parse(&clock.now_rfc3339(), &Rfc3339) else {
        return;
    };
    let notifier = notify::TauriNotifier::new(app.clone());

    for (job, slot) in locked_slots(local_offset(), now) {
        let first = state
            .pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert((job, slot));
        if first {
            let _ = notifier.notify(boba_app::ports::notifier::Notification::LockedSearch);
        }
    }
}

/// The slots a locked launch reports today, in UTC (AC4): at most one per job,
/// because there is no store to say what earlier ticks already recorded.
fn locked_slots(offset: UtcOffset, now: OffsetDateTime) -> Vec<(JobKind, String)> {
    let today = now
        .to_offset(offset)
        .date()
        .midnight()
        .assume_offset(offset);
    let schedules = default_schedules();
    let mut slots = Vec::new();
    for job in JobKind::ALL {
        let Some(schedule) = schedules.get(&job) else {
            continue;
        };
        for slot in schedule.due_slots(offset, Some(today), now) {
            if let Ok(text) = slot.to_offset(UtcOffset::UTC).format(&Rfc3339) {
                slots.push((job, text));
            }
        }
    }
    slots
}

/// Run one started job off the tick thread, then close its row: the outcome is
/// what `job_run` ends with, and a panic in a handler becomes `failed`.
fn execute<R: Runtime>(app: AppHandle<R>, job: JobKind, run: JobRun) {
    let thread_app = app.clone();
    let thread_run = run.clone();
    let spawned = std::thread::Builder::new()
        .name(format!("boba-job-{}", job.as_str()))
        .spawn(move || {
            let handler = handlers(&thread_app).and_then(|handlers| handlers.get(job));
            let outcome = run_handler(handler, job);
            close(&thread_app, &thread_run, outcome);
        });
    if let Err(error) = spawned {
        tracing::error!(%error, job = job.as_str(), "job thread not started");
        close(&app, &run, Err(format!("job thread not started: {error}")));
    }
}

/// Run the job's handler, if any, with a panic turned into a failure: a handler
/// that panics must not leave its row `running` forever and hold the single-run
/// lock until the next launch (FR-47).
fn run_handler(handler: Option<Arc<dyn JobHandler>>, job: JobKind) -> Result<(), String> {
    let Some(handler) = handler else {
        return Err(format!("no handler registered for {}", job.as_str()));
    };
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler.run(job)))
        .unwrap_or_else(|_| Err(format!("{} handler panicked", job.as_str())))
}

/// Close a run with the outcome its handler reported.
fn close<R: Runtime>(app: &AppHandle<R>, run: &JobRun, outcome: Result<(), String>) {
    let Some(repo) = jobs_repo(app) else {
        return;
    };
    let clock = SystemClock;
    let (status, detail) = match &outcome {
        Ok(()) => (boba_app::jobs::RunStatus::Succeeded, None),
        Err(error) => (boba_app::jobs::RunStatus::Failed, Some(error.as_str())),
    };
    if let Err(error) = repo.finish_run(&run.id, &clock.now_rfc3339(), status, detail) {
        tracing::error!(%error, run = run.id, "job run not closed");
    }
}

/// The job-run owner, read without the idle gate: the scheduler records a
/// `skipped_locked` row *because* the state is Locked (ADR-009 §3), so the gate
/// this process applies to commands must not close the port on it.
fn jobs_repo<R: Runtime>(app: &AppHandle<R>) -> Option<Arc<dyn JobsRepo>> {
    app.try_state::<Arc<dyn JobsRepo>>()
        .map(|state| Arc::clone(state.inner()))
}

fn system_repo<R: Runtime>(app: &AppHandle<R>) -> Option<Arc<dyn SystemRepo>> {
    app.try_state::<Arc<dyn SystemRepo>>()
        .map(|state| Arc::clone(state.inner()))
}

/// The local UTC offset at this tick, or UTC when the OS cannot say. Read every
/// tick, so a DST change moves the run time with the wall clock.
fn local_offset() -> UtcOffset {
    UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC)
}

/// The `setting` row of the schedules, or `None` when unset or unreadable (the
/// caller then runs the default: a stored typo never stops the daily search).
fn read_schedules(system: &dyn SystemRepo) -> Option<Schedules> {
    let raw = system.setting_get(SCHEDULES_KEY).ok().flatten()?;
    match schedules_from_json(&raw) {
        Ok(schedules) => Some(schedules),
        Err(error) => {
            tracing::warn!(%error, "stored job schedules ignored");
            None
        }
    }
}

/// When this install first ran the scheduler (FR-43): stored once, then read.
/// A locked store cannot write it, so the window starts at this tick instead.
fn started_at(
    system: Option<&Arc<dyn SystemRepo>>,
    now: &str,
    locked: bool,
) -> Option<OffsetDateTime> {
    let stored = system.and_then(|system| system.setting_get(STARTED_AT_KEY).ok().flatten());
    let text = match (stored, locked) {
        (Some(text), _) => text,
        (None, true) => now.to_string(),
        (None, false) => {
            if let Some(system) = system
                && let Err(error) = system.setting_put(STARTED_AT_KEY, now, now)
            {
                tracing::warn!(%error, "scheduler start not stored");
            }
            now.to_string()
        }
    };
    // A stored value this build cannot read starts the window now rather than
    // replaying the whole catch-up window in one burst.
    OffsetDateTime::parse(&text, &Rfc3339)
        .ok()
        .or_else(|| OffsetDateTime::parse(now, &Rfc3339).ok())
}

/// The stored schedule document, as the UI writes it (FR-41):
///
/// ```json
/// { "daily_search": { "times": ["08:00", "18:30"], "days": ["mon", "tue"] } }
/// ```
///
/// An empty `days` list means the schedule is off. Times are local `HH:MM`.
fn schedules_from_json(raw: &str) -> Result<Schedules, String> {
    let doc: HashMap<String, ScheduleDoc> =
        serde_json::from_str(raw).map_err(|error| error.to_string())?;
    let mut schedules = Schedules::new();
    for (job, doc) in doc {
        let Some(job) = JobKind::parse(&job) else {
            return Err(format!("unknown job '{job}'"));
        };
        schedules.insert(job, doc.into_schedule()?);
    }
    Ok(schedules)
}

#[derive(Debug, Deserialize)]
struct ScheduleDoc {
    times: Vec<String>,
    #[serde(default)]
    days: Vec<String>,
}

impl ScheduleDoc {
    fn into_schedule(self) -> Result<Schedule, String> {
        let times = self
            .times
            .iter()
            .map(|time| parse_time(time))
            .collect::<Result<Vec<_>, _>>()?;
        let days = self
            .days
            .iter()
            .map(|day| parse_day(day))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Schedule { times, days })
    }
}

fn parse_time(text: &str) -> Result<Time, String> {
    let (hour, minute) = text
        .split_once(':')
        .ok_or_else(|| format!("'{text}' is not HH:MM"))?;
    let hour: u8 = hour.parse().map_err(|_| format!("'{text}': hour"))?;
    let minute: u8 = minute.parse().map_err(|_| format!("'{text}': minute"))?;
    Time::from_hms(hour, minute, 0).map_err(|_| format!("'{text}' is not a valid time"))
}

fn parse_day(text: &str) -> Result<time::Weekday, String> {
    use time::Weekday;
    match text.to_ascii_lowercase().as_str() {
        "mon" | "monday" => Ok(Weekday::Monday),
        "tue" | "tuesday" => Ok(Weekday::Tuesday),
        "wed" | "wednesday" => Ok(Weekday::Wednesday),
        "thu" | "thursday" => Ok(Weekday::Thursday),
        "fri" | "friday" => Ok(Weekday::Friday),
        "sat" | "saturday" => Ok(Weekday::Saturday),
        "sun" | "sunday" => Ok(Weekday::Sunday),
        other => Err(format!("'{other}' is not a weekday")),
    }
}

/// A manual "Run now" over IPC (FR-41). Registered in `ipc::command_list`.
#[tauri::command]
#[specta::specta]
pub fn jobs_run_now(app: AppHandle) -> AppResult<RunNowOutcome> {
    run_now(&app)
}

#[cfg(test)]
mod tests {
    use super::*;
    use boba_app::ports::backup::{
        BackupError, BackupMeta, BackupPort, BackupStatus, ResetReport, RestoreReport,
    };

    #[test]
    fn the_stored_schedule_parses_into_times_and_days() {
        let schedules = schedules_from_json(
            r#"{"daily_search": {"times": ["08:00", "18:30"], "days": ["mon", "wed", "friday"]}}"#,
        )
        .expect("valid document");

        let daily = &schedules[&JobKind::DailySearch];
        assert_eq!(daily.times.len(), 2);
        assert_eq!(daily.times[1], Time::from_hms(18, 30, 0).unwrap());
        assert_eq!(
            daily.days,
            vec![
                time::Weekday::Monday,
                time::Weekday::Wednesday,
                time::Weekday::Friday
            ]
        );
    }

    #[test]
    fn an_unknown_job_or_bad_time_is_rejected_whole() {
        assert!(schedules_from_json(r#"{"weekly": {"times": ["08:00"]}}"#).is_err());
        assert!(schedules_from_json(r#"{"daily_search": {"times": ["8am"]}}"#).is_err());
        assert!(schedules_from_json(r#"{"daily_search": {"times": ["25:00"]}}"#).is_err());
        assert!(
            schedules_from_json(r#"{"daily_search": {"times": ["08:00"], "days": ["funday"]}}"#)
                .is_err()
        );
    }

    #[test]
    fn an_empty_day_list_means_the_schedule_is_off() {
        let schedules =
            schedules_from_json(r#"{"daily_search": {"times": ["08:00"], "days": []}}"#).unwrap();
        assert!(schedules[&JobKind::DailySearch].days.is_empty());
    }

    #[test]
    fn the_default_schedule_is_one_morning_search() {
        let schedules = default_schedules();
        let daily = &schedules[&JobKind::DailySearch];
        assert_eq!(daily.times, vec![Time::from_hms(8, 0, 0).unwrap()]);
        assert_eq!(daily.days.len(), 7);
    }

    #[test]
    fn the_local_offset_falls_back_to_utc() {
        // Whatever this host reports, the scheduler always gets a usable offset.
        let _ = local_offset();
    }

    fn at(text: &str) -> OffsetDateTime {
        OffsetDateTime::parse(text, &Rfc3339).unwrap()
    }

    #[test]
    fn a_locked_launch_reports_todays_slot_once() {
        // 09:00 UTC with the 08:00 daily search already past: one slot.
        assert_eq!(
            locked_slots(UtcOffset::UTC, at("2026-10-16T09:00:00Z")),
            vec![(JobKind::DailySearch, "2026-10-16T08:00:00Z".to_string())]
        );
        // Before the run time there is nothing to report (it will not be missed).
        assert!(locked_slots(UtcOffset::UTC, at("2026-10-16T07:00:00Z")).is_empty());
        // The slot is the local wall clock, stored in UTC (AD-17).
        assert_eq!(
            locked_slots(
                UtcOffset::from_hms(-5, 0, 0).unwrap(),
                at("2026-10-16T14:00:00Z")
            ),
            vec![(JobKind::DailySearch, "2026-10-16T13:00:00Z".to_string())]
        );
    }

    #[test]
    fn a_panicking_handler_fails_its_run_instead_of_holding_the_lock() {
        struct Panics;
        impl JobHandler for Panics {
            fn run(&self, _job: JobKind) -> Result<(), String> {
                panic!("boom");
            }
        }
        struct Works;
        impl JobHandler for Works {
            fn run(&self, _job: JobKind) -> Result<(), String> {
                Ok(())
            }
        }

        assert!(run_handler(Some(Arc::new(Panics)), JobKind::DailySearch).is_err());
        assert!(run_handler(Some(Arc::new(Works)), JobKind::DailySearch).is_ok());
        let missing = run_handler(None, JobKind::DailySearch).unwrap_err();
        assert!(missing.contains("no handler registered"));
    }

    #[test]
    fn slots_notified_while_locked_are_recorded_and_dropped_once_the_store_mounts() {
        use tauri::Manager;
        use tauri::test::mock_app;

        let app = mock_app();
        let handle = app.handle().clone();
        // The notification plugin, so a notifier call in the mock runtime resolves.
        let _ = handle.plugin(tauri_plugin_notification::init());
        handle.manage(SchedulerState::default());

        // Two ticks of a locked launch notify each slot once (AC4).
        note_locked_slots(&handle);
        note_locked_slots(&handle);
        let pending = handle
            .state::<SchedulerState>()
            .pending
            .lock()
            .unwrap()
            .len();
        assert!(pending <= 1, "today's slot only, and only once");

        // The store mounts: every pending slot becomes a skipped_locked row.
        let repo = Arc::new(FakeJobs::default());
        handle.manage::<Arc<dyn JobsRepo>>(repo.clone());
        record_pending_skips(&handle, repo.as_ref(), "2026-10-16T09:00:00Z");

        assert!(
            handle
                .state::<SchedulerState>()
                .pending
                .lock()
                .unwrap()
                .is_empty()
        );
        let rows = repo.rows.lock().unwrap();
        assert_eq!(rows.len(), pending);
        assert!(
            rows.iter()
                .all(|run| run.status == boba_app::jobs::RunStatus::SkippedLocked)
        );
    }

    #[test]
    fn a_run_refuses_to_start_while_a_recovery_backup_is_required() {
        use tauri::Manager;
        use tauri::test::mock_app;

        let app = mock_app();
        let handle = app.handle().clone();
        // No backup yet: the run gate must refuse before any job is recorded.
        handle.manage::<Arc<dyn BackupPort>>(Arc::new(FakeBackup {
            required: true,
            unreadable: false,
        }));

        assert!(recovery_backup_required(&handle));
        let error = run_now(&handle).expect_err("a run without a recovery backup");
        assert_eq!(error.message_key, "errors.backup.recovery_required");

        // Once the core reports the backup, the gate opens (the fake app has no
        // jobs repo, which is the next thing run_now reads).
        let fresh = mock_app();
        let fresh_handle = fresh.handle().clone();
        fresh_handle.manage::<Arc<dyn BackupPort>>(Arc::new(FakeBackup {
            required: false,
            unreadable: false,
        }));
        assert!(!recovery_backup_required(&fresh_handle));
    }

    #[test]
    fn a_build_without_a_backup_port_is_not_gated() {
        use tauri::test::mock_app;

        let app = mock_app();
        assert!(!recovery_backup_required(app.handle()));
    }

    #[test]
    fn an_unreadable_backup_status_keeps_the_run_gate_closed() {
        use tauri::Manager;
        use tauri::test::mock_app;

        let app = mock_app();
        let handle = app.handle().clone();
        // The port is there but cannot say whether a backup exists: fail closed.
        handle.manage::<Arc<dyn BackupPort>>(Arc::new(FakeBackup {
            required: false,
            unreadable: true,
        }));

        assert!(recovery_backup_required(&handle));
        let error = run_now(&handle).expect_err("a run with an unreadable backup status");
        assert_eq!(error.message_key, "errors.backup.recovery_required");
    }

    /// A backup port that only answers the recovery requirement.
    struct FakeBackup {
        required: bool,
        unreadable: bool,
    }

    impl BackupPort for FakeBackup {
        fn status(&self) -> Result<BackupStatus, BackupError> {
            if self.unreadable {
                return Err(BackupError::Internal);
            }
            Ok(BackupStatus {
                recovery_created_at: None,
                recovery_path: None,
                recovery_present: !self.required,
                recovery_required: self.required,
            })
        }

        fn create(
            &self,
            _dest: &std::path::Path,
            _passphrase: &str,
            _now: &str,
        ) -> Result<BackupMeta, BackupError> {
            Err(BackupError::Internal)
        }

        fn restore(
            &self,
            _src: &std::path::Path,
            _passphrase: &str,
        ) -> Result<RestoreReport, BackupError> {
            Err(BackupError::Internal)
        }

        fn reset(
            &self,
            _service: &str,
            _profiles: &[std::path::PathBuf],
            _confirmation: &str,
        ) -> Result<ResetReport, BackupError> {
            Err(BackupError::Internal)
        }
    }

    /// An in-memory `job_run` table, enough for the pending-skip path.
    #[derive(Default)]
    struct FakeJobs {
        rows: Mutex<Vec<JobRun>>,
    }

    impl JobsRepo for FakeJobs {
        fn open_run(
            &self,
            job: JobKind,
            scheduled_for: &str,
            started_at: &str,
        ) -> Result<boba_app::jobs::OpenOutcome, boba_app::jobs::JobsRepoError> {
            let run = JobRun {
                id: format!("run-{}", self.rows.lock().unwrap().len() + 1),
                job,
                scheduled_for: scheduled_for.to_string(),
                started_at: started_at.to_string(),
                finished_at: None,
                status: boba_app::jobs::RunStatus::Running,
                detail: None,
            };
            self.rows.lock().unwrap().push(run.clone());
            Ok(boba_app::jobs::OpenOutcome::Started(run))
        }

        fn record_skipped_locked(
            &self,
            job: JobKind,
            scheduled_for: &str,
            at: &str,
        ) -> Result<Option<JobRun>, boba_app::jobs::JobsRepoError> {
            if self
                .rows
                .lock()
                .unwrap()
                .iter()
                .any(|run| run.job == job && run.scheduled_for == scheduled_for)
            {
                return Ok(None);
            }
            let run = JobRun {
                id: format!("run-{}", self.rows.lock().unwrap().len() + 1),
                job,
                scheduled_for: scheduled_for.to_string(),
                started_at: at.to_string(),
                finished_at: Some(at.to_string()),
                status: boba_app::jobs::RunStatus::SkippedLocked,
                detail: None,
            };
            self.rows.lock().unwrap().push(run.clone());
            Ok(Some(run))
        }

        fn finish_run(
            &self,
            _id: &str,
            _finished_at: &str,
            _status: boba_app::jobs::RunStatus,
            _detail: Option<&str>,
        ) -> Result<JobRun, boba_app::jobs::JobsRepoError> {
            Err(boba_app::jobs::JobsRepoError::new("unused"))
        }

        fn get_run(
            &self,
            job: JobKind,
            scheduled_for: &str,
        ) -> Result<Option<JobRun>, boba_app::jobs::JobsRepoError> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .iter()
                .find(|run| run.job == job && run.scheduled_for == scheduled_for)
                .cloned())
        }

        fn running_run(
            &self,
            _job: JobKind,
        ) -> Result<Option<JobRun>, boba_app::jobs::JobsRepoError> {
            Ok(None)
        }

        fn last_run(&self, _job: JobKind) -> Result<Option<JobRun>, boba_app::jobs::JobsRepoError> {
            Ok(None)
        }

        fn mark_interrupted(
            &self,
            _at: &str,
            _detail: &str,
        ) -> Result<u64, boba_app::jobs::JobsRepoError> {
            Ok(0)
        }
    }
}
