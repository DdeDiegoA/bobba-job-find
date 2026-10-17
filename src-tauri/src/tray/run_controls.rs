//! Tray run controls (S5.4, FR-42, NFR-10): Run now, Pause schedule, and the
//! icon's four states.
//!
//! The menu itself lives in [`super::menu`] (S1.6) and the dispatch in
//! [`crate::lifecycle`]; this module owns what the two buttons *do* and what the
//! icon shows.
//!
//! States are resolved from facts, not from a cached guess:
//!
//! * **Running** — a `daily_search` `job_run` is executing right now (FR-47).
//! * **Paused** — the user paused the schedule from the tray; the flag is
//!   persisted (`schedule.paused`), so a restart keeps it. It is read back on
//!   the first repaint whose store answers: a passphrase unlock mounts the ports
//!   after `install` has already run.
//! * **Needs attention** — the store is locked (the user must unlock) or the
//!   last daily search failed or was skipped while locked.
//! * **Idle** — none of the above.
//!
//! Priority when two hold at once: Running > Paused > Needs attention > Idle. A
//! pick the user just made is visible at once (pausing while an old failure is
//! on record must not look like the click did nothing), and a run in flight is
//! always the most informative state.
//!
//! NFR-10: an idle process does no busy work. [`install`] paints the icon once
//! and one thread wakes every [`PULSE`] to re-read one indexed `job_run` row; a
//! tray pick repaints at once. (The status does not ride the event bus: the
//! core's event names are dotted, which Tauri refuses, so a listener would panic.
//! The pulse is also what catches a run that failed before it could report.)

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use boba_app::error::AppResult;
use boba_app::jobs::{JobKind, JobsRepo, RunStatus};
use tauri::{AppHandle, Manager, Runtime};

use crate::tray::{self, TRAY_ID};
use crate::wiring::{self, StoreStatus};

/// The `setting` row holding whether the schedule is paused (S5.4 AC2): the
/// pause survives a restart, because a user who stopped the schedule is not
/// asking for it to resume overnight.
pub const PAUSED_KEY: &str = "schedule.paused";

/// How often the icon re-reads the run state (NFR-10). One primary-key read, not
/// a busy loop: the tick exists only for a run that died before reporting.
pub const PULSE: Duration = Duration::from_secs(30);

/// Side of the tray icon, in pixels.
pub const ICON_SIZE: u32 = 32;

/// What the tray icon shows (AC2, FR-42).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayStatus {
    /// Nothing to report: no run in flight, nothing failed, nothing paused.
    Idle,
    /// A daily search is executing now.
    Running,
    /// The store is locked or the last daily search failed.
    NeedsAttention,
    /// The user paused the schedule.
    Paused,
}

impl TrayStatus {
    /// The stable text form, used in tests, logs and the painted-state cache.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::NeedsAttention => "needs_attention",
            Self::Paused => "paused",
        }
    }

    /// The icon's colour, RGBA.
    pub const fn color(self) -> [u8; 4] {
        // The boba palette, kept legible on a light and a dark menu bar.
        match self {
            Self::Idle => [0x8A, 0x5A, 0x3B, 0xFF],           // tea brown
            Self::Running => [0x2F, 0x8A, 0x54, 0xFF],        // leaf green
            Self::NeedsAttention => [0xC0, 0x39, 0x2B, 0xFF], // tapioca red
            Self::Paused => [0x8A, 0x84, 0x7C, 0xFF],         // warm grey
        }
    }
}

/// Resolve the one state to show (AC2).
pub fn status_of(paused: bool, running: bool, needs_attention: bool) -> TrayStatus {
    if running {
        TrayStatus::Running
    } else if paused {
        TrayStatus::Paused
    } else if needs_attention {
        TrayStatus::NeedsAttention
    } else {
        TrayStatus::Idle
    }
}

/// The live state, from the tray flag and the `job_run` rows.
pub fn current_status<R: Runtime>(app: &AppHandle<R>) -> TrayStatus {
    let paused = tray::schedule_paused(app);
    let (running, needs_attention) = run_state(app);
    status_of(paused, running, needs_attention)
}

/// `(running, needs_attention)` from the store: a locked store is attention in
/// itself; otherwise the daily search's newest rows decide.
fn run_state<R: Runtime>(app: &AppHandle<R>) -> (bool, bool) {
    // A locked store cannot answer, and the user has to unlock it: that is the
    // one thing the tray must say (ADR-009 §3).
    if matches!(wiring::store_state(app), StoreStatus::Locked { .. }) {
        return (false, true);
    }
    let Some(repo) = jobs(app) else {
        return (false, false);
    };
    let running = repo
        .running_run(JobKind::DailySearch)
        .ok()
        .flatten()
        .is_some();
    let failed = repo
        .last_run(JobKind::DailySearch)
        .ok()
        .flatten()
        .is_some_and(|run| matches!(run.status, RunStatus::Failed | RunStatus::SkippedLocked));
    (running, failed)
}

/// The job-run owner, when the composition root mounted it.
fn jobs<R: Runtime>(app: &AppHandle<R>) -> Option<Arc<dyn JobsRepo>> {
    app.try_state::<Arc<dyn JobsRepo>>()
        .map(|state| Arc::clone(state.inner()))
}

/// The RGBA buffer of the icon for `status` (AC2): the three "healthy" states
/// are a filled disk in their own colour, and paused is a pause glyph, so the
/// states differ by shape as well as by colour.
pub fn icon_rgba(status: TrayStatus) -> Vec<u8> {
    let size = ICON_SIZE as i32;
    let mut rgba = vec![0u8; (ICON_SIZE * ICON_SIZE * 4) as usize];
    let radius = (ICON_SIZE / 3) as i32;
    let center = size / 2;

    let mut put = |x: i32, y: i32| {
        let pixel = ((y * size) + x) as usize * 4;
        rgba[pixel..pixel + 4].copy_from_slice(&status.color());
    };

    match status {
        TrayStatus::Paused => {
            // A pause glyph: two bars, with no ink in the middle column, so the
            // state is unmistakable even where colour is unavailable.
            let bar = (ICON_SIZE / 6) as i32;
            let gap = (ICON_SIZE / 10) as i32;
            let left = center - gap - bar;
            for y in 0..size {
                for x in 0..size {
                    let in_left = x >= left && x < left + bar;
                    let in_right = x >= center + gap && x < center + gap + bar;
                    // The glyph keeps to the disk's vertical extent, so all four
                    // icons share one silhouette.
                    if (in_left || in_right) && (y - center).abs() <= radius {
                        put(x, y);
                    }
                }
            }
        }
        _ => {
            for y in 0..size {
                for x in 0..size {
                    let dx = x - center;
                    let dy = y - center;
                    if dx * dx + dy * dy <= radius * radius {
                        put(x, y);
                    }
                }
            }
        }
    }
    rgba
}

/// The state painted last, so a pulse that changes nothing does not touch the
/// platform icon.
#[derive(Debug, Default)]
struct Painted {
    last: Mutex<Option<&'static str>>,
    /// Whether the stored pause flag has been read back. The store may still be
    /// locked at install (passphrase mode), so the restore retries on every
    /// repaint until the ports are there (F1).
    restored: AtomicBool,
}

/// Wire the run controls once per process: paint the icon (which restores the
/// pause flag as soon as the store answers) and start the pulse (NFR-10).
pub fn install<R: Runtime>(app: &AppHandle<R>) {
    if app.try_state::<Painted>().is_some() {
        return;
    }
    let _ = app.manage(Painted::default());
    paint_current(app);
    spawn_pulse(app.clone());
}

/// Repaint from the store and the tray flag; idempotent.
pub fn refresh<R: Runtime>(app: &AppHandle<R>) {
    paint_current(app);
}

/// Start a Search Run from the tray ("Run now", AC2, FR-41).
pub fn run_now<R: Runtime>(app: &AppHandle<R>) -> AppResult<crate::scheduler::RunNowOutcome> {
    let outcome = crate::scheduler::run_now(app);
    refresh(app);
    outcome
}

/// Pause or resume the schedule from the tray (AC2): the flag, its check mark
/// and the icon, and the choice survives a restart.
pub fn toggle_schedule<R: Runtime>(app: &AppHandle<R>) {
    if let Some(state) = app.try_state::<tray::TrayState>() {
        state.toggle_schedule();
    }
    let paused = tray::schedule_paused(app);
    persist_paused(app, paused);
    // AC2: the check mark on "Pause schedule" shows the new state.
    if let Err(error) = tray::attach(app, crate::lifecycle::ui_locale(app)) {
        tracing::warn!(%error, "tray menu not refreshed");
    }
    refresh(app);
}

/// Store the pause flag (best effort: a locked store must not undo the pick).
fn persist_paused<R: Runtime>(app: &AppHandle<R>, paused: bool) {
    use boba_app::ports::clock::ClockPort;
    use boba_app::ports::repo::system::SystemRepo;

    let Ok(repo) = wiring::port::<dyn SystemRepo>(app) else {
        tracing::debug!("store locked; the pause applies to this launch only");
        return;
    };
    let now = boba_app::system::SystemClock.now_rfc3339();
    let value = if paused { "true" } else { "false" };
    if let Err(error) = repo.setting_put(PAUSED_KEY, value, &now) {
        tracing::warn!(%error, "schedule pause not stored");
    }
}

/// Bring the pause the last launch left back into the tray flag (AC2).
///
/// `false` means "could not read it yet" (the store is still locked at launch in
/// passphrase mode), so the caller retries on the next repaint; `true` means the
/// flag now matches the store, whether it had to change or not.
fn restore_paused<R: Runtime>(app: &AppHandle<R>) -> bool {
    use boba_app::ports::repo::system::SystemRepo;

    let Ok(repo) = wiring::port::<dyn SystemRepo>(app) else {
        return false;
    };
    let Ok(stored) = repo.setting_get(PAUSED_KEY) else {
        return false;
    };
    let stored = stored.is_some_and(|value| value == "true");
    let Some(state) = app.try_state::<tray::TrayState>() else {
        return false;
    };
    if stored != state.schedule_paused() {
        state.toggle_schedule();
    }
    true
}

/// Restore once per process, as soon as the store can answer (F1: after a
/// passphrase unlock the ports mount long after `install` ran).
fn try_restore<R: Runtime>(app: &AppHandle<R>) {
    let Some(painted) = app.try_state::<Painted>() else {
        return;
    };
    if painted.restored.load(Ordering::SeqCst) {
        return;
    }
    if restore_paused(app) {
        painted.restored.store(true, Ordering::SeqCst);
    }
}

/// One cheap pulse (NFR-10): a run that died before reporting still clears.
fn spawn_pulse<R: Runtime>(app: AppHandle<R>) {
    let spawned = std::thread::Builder::new()
        .name("boba-tray-status".to_string())
        .spawn(move || {
            loop {
                std::thread::sleep(PULSE);
                paint_current(&app);
            }
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "tray status thread not started");
    }
}

/// Paint the icon from the live state (AC2).
fn paint_current<R: Runtime>(app: &AppHandle<R>) {
    // The store may have opened since the last paint (the unlock flow mounts the
    // ports); restoring here is what gets a paused schedule back after a restart.
    try_restore(app);
    let status = current_status(app);
    let Some(painted) = app.try_state::<Painted>() else {
        return;
    };
    let Some(icon) = app.tray_by_id(TRAY_ID) else {
        // No tray (a test app, or a Linux desktop without AppIndicator): the
        // cache stays unset, so an icon that appears later still gets painted.
        return;
    };
    let mut last = painted
        .last
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if *last == Some(status.as_str()) {
        return;
    }

    let image = tauri::image::Image::new_owned(icon_rgba(status), ICON_SIZE, ICON_SIZE);
    match icon.set_icon(Some(image)) {
        Ok(()) => *last = Some(status.as_str()),
        Err(error) => {
            tracing::warn!(%error, status = status.as_str(), "tray icon not painted");
        }
    }
    // A coloured icon must not be treated as a monochrome template on macOS.
    #[cfg(target_os = "macos")]
    {
        let _ = icon.set_icon_as_template(false);
    }
}

#[cfg(test)]
mod tests {
    use tauri::test::mock_app;

    use super::*;

    /// AC2: the four states are exclusive, and the priority is what the user
    /// just did (Running), then their own pick (Paused), then a failure.
    #[test]
    fn the_states_have_one_priority_order() {
        assert_eq!(status_of(false, false, false), TrayStatus::Idle);
        assert_eq!(status_of(true, false, false), TrayStatus::Paused);
        assert_eq!(status_of(false, false, true), TrayStatus::NeedsAttention);
        assert_eq!(status_of(true, false, true), TrayStatus::Paused);
        assert_eq!(status_of(true, true, true), TrayStatus::Running);
        assert_eq!(status_of(false, true, false), TrayStatus::Running);
    }

    /// Every state has its own stable name (used by the painted-state cache).
    #[test]
    fn every_state_has_a_distinct_name() {
        let names = [
            TrayStatus::Idle.as_str(),
            TrayStatus::Running.as_str(),
            TrayStatus::NeedsAttention.as_str(),
            TrayStatus::Paused.as_str(),
        ];
        let unique: std::collections::HashSet<&str> = names.iter().copied().collect();
        assert_eq!(unique.len(), names.len());
    }

    /// AC2: each state paints a different icon, at the tray's size.
    #[test]
    fn every_state_paints_its_own_icon() {
        let states = [
            TrayStatus::Idle,
            TrayStatus::Running,
            TrayStatus::NeedsAttention,
            TrayStatus::Paused,
        ];
        let images: Vec<Vec<u8>> = states.iter().map(|state| icon_rgba(*state)).collect();
        for image in &images {
            assert_eq!(
                image.len(),
                (ICON_SIZE * ICON_SIZE * 4) as usize,
                "RGBA buffer is the icon's size"
            );
            assert!(image.chunks(4).any(|pixel| pixel[3] > 0), "all blank");
        }
        for (i, a) in images.iter().enumerate() {
            for (j, b) in images.iter().enumerate().skip(i + 1) {
                assert_ne!(
                    a,
                    b,
                    "states {} and {j} paint the same icon",
                    states[i].as_str()
                );
            }
        }
    }

    /// AC2: paused is a pause glyph (no ink in the middle column); the others
    /// are disks with a filled centre.
    #[test]
    fn paused_is_a_glyph_and_the_others_are_disks() {
        let centre = (ICON_SIZE / 2) as usize;
        let row = (ICON_SIZE / 2) as usize;
        let alpha_at =
            |image: &[u8], x: usize, y: usize| image[(y * ICON_SIZE as usize + x) * 4 + 3];

        for status in [
            TrayStatus::Idle,
            TrayStatus::Running,
            TrayStatus::NeedsAttention,
        ] {
            let image = icon_rgba(status);
            assert!(
                alpha_at(&image, centre, row) > 0,
                "{} is a filled disk",
                status.as_str()
            );
        }
        let paused = icon_rgba(TrayStatus::Paused);
        assert_eq!(alpha_at(&paused, centre, row), 0, "paused has a gap");
        let bar = (ICON_SIZE / 6) as usize;
        let gap = (ICON_SIZE / 10) as usize;
        assert!(
            alpha_at(&paused, centre - gap - bar, row) > 0,
            "paused has a left bar"
        );
        assert!(
            alpha_at(&paused, centre + gap, row) > 0,
            "paused has a right bar"
        );
    }

    /// A launch with no composition root (a test app) is a locked store: that
    /// is "needs attention", never a panic.
    #[test]
    fn a_mock_app_without_a_store_reports_attention() {
        let app = mock_app();
        assert_eq!(current_status(app.handle()), TrayStatus::NeedsAttention);
        refresh(app.handle());
    }

    /// AC2: "Run now" without a store reports the typed Locked error and leaves
    /// the icon on the attention state, instead of silently doing nothing.
    #[test]
    fn run_now_without_a_store_reports_locked() {
        let app = mock_app();
        let error = run_now(app.handle()).expect_err("no store to run against");
        assert_eq!(error.message_key, "errors.store.locked");
        assert_eq!(current_status(app.handle()), TrayStatus::NeedsAttention);
    }

    /// AC2: pausing works before any store exists (the pick is in-memory then,
    /// and painting must not panic without a tray).
    #[test]
    fn toggling_the_schedule_works_without_a_store_or_tray() {
        let app = mock_app();
        let _ = app.manage(tray::TrayState::default());
        assert!(!tray::schedule_paused(app.handle()));
        toggle_schedule(app.handle());
        assert!(tray::schedule_paused(app.handle()));
        assert_eq!(current_status(app.handle()), TrayStatus::Paused);
        toggle_schedule(app.handle());
        assert!(!tray::schedule_paused(app.handle()));
    }

    /// F1: in passphrase mode the store is still locked when `install` runs, so
    /// the pause must come back on the first repaint after the unlock mounts the
    /// ports. Nothing here touches the keychain or real app data: the store is a
    /// temp-dir file under a fixed key.
    #[test]
    fn the_pause_comes_back_after_a_late_unlock() {
        use boba_app::ports::repo::system::SystemRepo;
        use boba_store::db::{Store, StoreConfig};
        use boba_store::keys::DbKey;
        use boba_store::repos::audit::SqliteSystemRepo;

        let app = mock_app();
        let _ = app.manage(tray::TrayState::default());

        // Locked at install: there is nothing to read yet, so the flag is off.
        install(app.handle());
        assert!(!tray::schedule_paused(app.handle()));

        // The unlock mounts the ports on an open store that holds a pause.
        let dir = std::env::temp_dir().join(format!("boba-s54-pause-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let key =
            DbKey::from_hex("00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff")
                .expect("test key");
        let config = StoreConfig::new(dir.join("boba.db"), dir.join("snapshots"));
        let store = Arc::new(Store::open_with_key(config, key).expect("store opens"));
        let system: Arc<dyn SystemRepo> = Arc::new(SqliteSystemRepo::new(
            store,
            crate::wiring::keychain_service(),
        ));
        crate::wiring::mount_port(app.handle(), system);
        crate::wiring::port::<dyn SystemRepo>(app.handle())
            .expect("system port")
            .setting_put(PAUSED_KEY, "true", "seed")
            .expect("seed the pause");

        // The next repaint (the pulse, or any tray action) restores it.
        refresh(app.handle());

        assert!(
            tray::schedule_paused(app.handle()),
            "a paused schedule must come back after the unlock"
        );
    }

    /// NFR-10: the idle tick is a slow pulse, not a busy loop.
    #[test]
    fn the_pulse_is_slow() {
        assert!(
            PULSE >= Duration::from_secs(10),
            "an idle tray must not wake often"
        );
    }

    /// `install` is idempotent: a second call (a test, a re-install) must not
    /// start a second pulse thread.
    #[test]
    fn installing_twice_is_a_no_op() {
        let app = mock_app();
        install(app.handle());
        install(app.handle());
        assert!(app.try_state::<Painted>().is_some());
    }
}
