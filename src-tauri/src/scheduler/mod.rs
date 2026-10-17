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

mod notify;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use boba_app::error::{AppError, AppResult, ErrorCode};
use boba_app::jobs::{
    Dispatcher, DriveOutcome, JobHandler, JobKind, JobRun, JobsRepo, Schedule, Schedules,
    TickContext, default_schedules,
};
use boba_app::ports::clock::ClockPort;
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

/// Per-launch scheduler state: the recovery of stale runs happens once.
#[derive(Debug, Default)]
struct SchedulerState {
    recovered: AtomicBool,
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

/// A manual "Run now" (FR-41, AC3): join the run already executing, or start
/// one whose slot is now. While the store is locked the slot is recorded
/// `skipped_locked` and the caller gets the typed Locked error (ADR-009 §3).
pub fn run_now<R: Runtime>(app: &AppHandle<R>) -> AppResult<RunNowOutcome> {
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
    let Some(repo) = jobs_repo(app) else {
        // No store this launch (locked at startup): nothing can be recorded.
        return;
    };

    let clock = SystemClock;
    let now = clock.now_rfc3339();
    let system = system_repo(app);

    // A run that died with the previous process (power loss, crash) must not
    // hold the single-run lock (FR-47).
    if let Some(state) = app.try_state::<SchedulerState>()
        && !state.recovered.swap(true, Ordering::SeqCst)
        && let Err(error) = repo.mark_interrupted(&now, "interrupted by a previous shutdown")
    {
        tracing::warn!(%error, "stale job runs not closed");
    }

    let locked = matches!(wiring::store_state(app), StoreStatus::Locked { .. });
    let schedules = system
        .as_ref()
        .and_then(|system| read_schedules(system.as_ref()))
        .unwrap_or_else(default_schedules);
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

/// Run one started job off the tick thread, then close its row: the outcome is
/// what `job_run` ends with, and a panic in a handler becomes `failed`.
fn execute<R: Runtime>(app: AppHandle<R>, job: JobKind, run: JobRun) {
    let thread_app = app.clone();
    let thread_run = run.clone();
    let spawned = std::thread::Builder::new()
        .name(format!("boba-job-{}", job.as_str()))
        .spawn(move || {
            let handler = handlers(&thread_app).and_then(|handlers| handlers.get(job));
            let outcome = match handler {
                Some(handler) => handler.run(job),
                None => Err(format!("no handler registered for {}", job.as_str())),
            };
            close(&thread_app, &thread_run, outcome);
        });
    if let Err(error) = spawned {
        tracing::error!(%error, job = job.as_str(), "job thread not started");
        close(&app, &run, Err(format!("job thread not started: {error}")));
    }
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
}
