//! System tray (S1.6, FR-42, AD-18).
//!
//! The icon itself is created in `lib.rs` setup (S1.1, ownership convention 8);
//! this slice owns its menu and the state the menu shows. Later slices extend it
//! in place: the scheduler (S5.1) reads [`schedule_paused`], the Kill Switch
//! (S9.8) owns `kill_switch.rs`, and the run controls (S5.1) own `run_controls.rs`.
//!
//! Platform note (NFR-18): the menu is one Tauri menu on every OS. A Linux
//! desktop without an AppIndicator host (GNOME without the extension) shows no
//! tray at all; launching Boba again still brings the window back (AC3).

pub mod kill_switch;
pub mod menu;
pub mod run_controls;

use std::sync::atomic::{AtomicBool, Ordering};

use boba_app::i18n::Locale;
use tauri::{AppHandle, Manager, Runtime};

/// The tray icon id, as `lib.rs` creates it.
pub const TRAY_ID: &str = "boba-tray";

/// What the tray remembers between menu picks: the schedule pause flag (S5.1
/// will persist and honour it) and whether the menu event handler is wired.
#[derive(Debug, Default)]
pub struct TrayState {
    paused: AtomicBool,
    menu_wired: AtomicBool,
}

impl TrayState {
    /// Whether the schedule is paused right now.
    pub fn schedule_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    /// Flip the pause flag, returning its new value.
    pub fn toggle_schedule(&self) -> bool {
        let paused = !self.paused.load(Ordering::SeqCst);
        self.paused.store(paused, Ordering::SeqCst);
        paused
    }

    /// Claim the one menu-event registration; `false` when it is already wired,
    /// so re-localizing the menu never doubles a handler up.
    pub fn wire_menu(&self) -> bool {
        !self.menu_wired.swap(true, Ordering::SeqCst)
    }
}

/// The tray state, once [`crate::lifecycle::install`] has registered it.
fn state<R: Runtime>(app: &AppHandle<R>) -> Option<&TrayState> {
    app.try_state::<TrayState>().map(|state| state.inner())
}

/// Whether the user paused the schedule (S5.1 skips scheduled runs while it is).
pub fn schedule_paused<R: Runtime>(app: &AppHandle<R>) -> bool {
    state(app).is_some_and(TrayState::schedule_paused)
}

/// Attach the menu in `locale` to the tray icon (AC2), and route a pick to
/// [`crate::lifecycle::on_menu_action`].
///
/// Callable again: it replaces the menu in place, which is how a language change
/// re-localizes the tray (the handler is wired only once).
pub fn attach<R: Runtime>(app: &AppHandle<R>, locale: Locale) -> Result<(), String> {
    let Some(icon) = app.tray_by_id(TRAY_ID) else {
        return Err(format!("the {TRAY_ID} icon is missing"));
    };

    // `lifecycle::install` registers the state; a direct call still gets one, so
    // the menu handler below is never silently skipped.
    if app.try_state::<TrayState>().is_none() {
        let _ = app.manage(TrayState::default());
    }

    let paused = schedule_paused(app);
    let menu = menu::build(app, locale, paused).map_err(|e| format!("tray menu: {e}"))?;
    icon.set_menu(Some(menu))
        .map_err(|e| format!("tray menu: {e}"))?;

    if state(app).is_some_and(TrayState::wire_menu) {
        icon.on_menu_event(|app, event| {
            crate::lifecycle::on_menu_action(app, event.id().as_ref());
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use tauri::test::mock_app;

    use super::*;

    /// "Pause schedule" flips one flag, and the flag is what the check mark shows.
    #[test]
    fn the_pause_flag_toggles() {
        let state = TrayState::default();
        assert!(!state.schedule_paused());
        assert!(state.toggle_schedule(), "first pick pauses");
        assert!(state.schedule_paused());
        assert!(!state.toggle_schedule(), "second pick resumes");
        assert!(!state.schedule_paused());
    }

    /// Re-localizing the menu must not register a second handler: two handlers
    /// would run every action twice (a pause toggle would cancel itself out).
    #[test]
    fn the_menu_handler_is_wired_once() {
        let state = TrayState::default();
        assert!(state.wire_menu());
        assert!(!state.wire_menu());
        assert!(!state.wire_menu());
    }

    /// With no tray icon yet (nothing is mounted on the mock runtime), attaching
    /// reports the missing icon instead of panicking in `setup`.
    #[test]
    fn attaching_without_a_tray_icon_reports_it() {
        let app = mock_app();
        let error = attach(app.handle(), Locale::En).expect_err("no tray icon");
        assert!(error.contains(TRAY_ID), "unhelpful error: {error}");
    }
}
