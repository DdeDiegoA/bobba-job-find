//! App lifecycle (S1.6, FR-42, AD-18): close-to-tray, the tray menu actions and
//! the single-instance handoff.
//!
//! `lib.rs` calls [`install`] once in `setup` (after the tray icon exists) and
//! hands a second launch to [`on_second_instance`], the single-instance plugin
//! callback. Both entries stay here so no later slice edits `lib.rs`.

pub mod close_notice;

use boba_app::i18n::Locale;
use boba_app::ports::repo::system::SystemRepo;
use boba_app::system::SettingKey;
use tauri::{AppHandle, Manager, Runtime, WindowEvent};
use tauri_plugin_dialog::{Dialog, MessageDialogKind};

use crate::tray::{self, TrayState, menu};
use close_notice::CloseNotice;

/// The main window label, as `tauri.conf.json` declares it.
pub const MAIN_WINDOW: &str = "main";

/// Marks the launch as installed, so the window handler is registered once even
/// though the menu may be re-localized later.
#[derive(Debug)]
struct Installed;

/// Wire the tray menu (AC2) and close-to-tray (AC1).
///
/// Never fails the launch: a missing tray icon or window is logged, because an
/// app that starts without its tray is still usable.
pub fn install<R: Runtime>(app: &AppHandle<R>) {
    if app.try_state::<TrayState>().is_none() {
        let _ = app.manage(TrayState::default());
    }
    if app.try_state::<Installed>().is_none() {
        let _ = app.manage(Installed);
        watch_close(app);
    }
    if let Err(error) = tray::attach(app, ui_locale(app)) {
        tracing::warn!(%error, "tray menu not installed");
    }
    // The one panic shortcut (S9.8, FR-86): Cmd/Ctrl+Shift+K stops automation.
    crate::shortcuts::install(app);
    // The one in-core scheduler (S5.1, AD-18): a login or a wake ticks at once,
    // which is how a slot missed while asleep still runs (FR-43).
    crate::scheduler::install(app);
    // Launch at login (S5.4, FR-44): reconcile the OS with the stored choice,
    // and hide the window when the OS launched us at login.
    crate::autostart::install(app);
    // The tray run controls and the state icon (S5.4, FR-42).
    crate::tray::run_controls::install(app);
}

/// Bring Boba to the front (AC3): "Open" from the tray, and the second-instance
/// handoff, both come back to the one window.
pub fn focus_main<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        tracing::warn!("no {MAIN_WINDOW} window to focus");
        return;
    };
    if window.is_minimized().unwrap_or(false) {
        let _ = window.unminimize();
    }
    let _ = window.show();
    let _ = window.set_focus();
}

/// The single-instance callback (AC3, AD-18): the second launch never reaches
/// `Builder::run`, so the running Boba is what the user gets.
pub fn on_second_instance<R: Runtime>(app: &AppHandle<R>, _argv: Vec<String>, _cwd: String) {
    focus_main(app);
}

/// Run the action of a picked tray item (AC2).
pub fn on_menu_action<R: Runtime>(app: &AppHandle<R>, picked: &str) {
    match menu::action(picked) {
        menu::Action::Open => focus_main(app),
        // AC2: the flag, its check mark and the icon all come from S5.4.
        menu::Action::ToggleSchedule => crate::tray::run_controls::toggle_schedule(app),
        menu::Action::Quit => app.exit(0),
        // The Kill Switch (S9.8, FR-86): the tray only ever stops automation.
        // Turning it back off is the header chip's job, where the user can see
        // what they are re-enabling.
        menu::Action::KillSwitch => {
            if let Err(error) = tray::kill_switch::engage(app) {
                tracing::warn!(key = error.message_key, "tray: kill switch failed");
            }
        }
        // Run now (S5.4, FR-41): start a Search Run; a locked store or a missing
        // recovery backup is a typed error the tray reports, never a panic.
        menu::Action::RunNow => {
            if let Err(error) = crate::tray::run_controls::run_now(app) {
                tracing::warn!(key = error.message_key, "tray: Run now failed");
            }
        }
        menu::Action::Ignore => {}
    }
}

/// The UI language for core-side strings: the stored choice when the store is
/// open, otherwise the OS locale — the rule the UI itself uses (S1.4
/// `pickLocale`), so the tray and the window agree.
pub fn ui_locale<R: Runtime>(app: &AppHandle<R>) -> Locale {
    locale_from(
        stored_ui_locale(app).as_deref(),
        sys_locale::get_locale().as_deref(),
    )
}

/// Pick a shipped locale: an explicit stored choice wins, otherwise the OS tag.
pub fn locale_from(stored: Option<&str>, os_tag: Option<&str>) -> Locale {
    let stored_choice =
        stored.filter(|tag| tag.eq_ignore_ascii_case("en") || tag.eq_ignore_ascii_case("es"));
    Locale::from_tag(stored_choice.or(os_tag).unwrap_or("en"))
}

/// The stored `ui_locale` row, when the store is open.
fn stored_ui_locale<R: Runtime>(app: &AppHandle<R>) -> Option<String> {
    let repo = crate::wiring::port::<dyn SystemRepo>(app).ok()?;
    repo.setting_get(SettingKey::UiLocale.as_str())
        .ok()
        .flatten()
}

/// AC1: closing the window hides it, and the first time it happens Boba says why.
fn watch_close<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        tracing::warn!("no {MAIN_WINDOW} window: close-to-tray is not installed");
        return;
    };

    let notice = match app.path().app_config_dir() {
        Ok(dir) => CloseNotice::in_dir(&dir),
        Err(error) => {
            tracing::warn!(%error, "app config dir unavailable; explaining once per launch");
            CloseNotice::ephemeral()
        }
    };

    let handle = window.clone();
    window.on_window_event(move |event| {
        let WindowEvent::CloseRequested { api, .. } = event else {
            return;
        };
        // The window is hidden, never destroyed: the process (and the tray) live on.
        api.prevent_close();
        if let Err(error) = handle.hide() {
            tracing::warn!(%error, "could not hide the window");
        }
        if notice.claim() {
            explain_close_to_tray(handle.app_handle());
        }
    });
}

/// AC1: the one-time explanation, as an OS message dialog. A dialog rather than
/// a notification on purpose: it is shown the moment the window goes, and it
/// cannot be suppressed by Do Not Disturb or a denied notification permission.
fn explain_close_to_tray<R: Runtime>(app: &AppHandle<R>) {
    let locale = ui_locale(app);
    let title = menu::message(locale, close_notice::NOTICE.message_key).unwrap_or_default();
    let body = menu::message(locale, close_notice::NOTICE.body_message_key).unwrap_or_default();

    // No dialog plugin (a test app) is not worth a panic in a close handler.
    let Some(dialog) = app.try_state::<Dialog<R>>() else {
        tracing::warn!("dialog plugin is not registered; close-to-tray stays unexplained");
        return;
    };
    dialog
        .message(body)
        .title(title)
        .kind(MessageDialogKind::Info)
        .show(|_shown| {});
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stored choice wins over the OS, exactly as the UI resolves it (S1.4).
    #[test]
    fn a_shipped_stored_choice_beats_the_os_locale() {
        assert_eq!(locale_from(Some("es"), Some("en-US")), Locale::Es);
        assert_eq!(locale_from(Some("EN"), Some("es-MX")), Locale::En);
    }

    /// With nothing stored (a fresh install), the OS decides.
    #[test]
    fn the_os_locale_is_the_fallback() {
        assert_eq!(locale_from(None, Some("es-419")), Locale::Es);
        assert_eq!(locale_from(None, Some("fr-CA")), Locale::En);
        assert_eq!(locale_from(None, None), Locale::En);
    }

    /// A stored tag the UI would not honour (`es-MX`, `fr`) is not a choice: the
    /// two must not disagree about the language.
    #[test]
    fn a_stored_tag_outside_en_es_is_not_a_choice() {
        assert_eq!(locale_from(Some("es-MX"), Some("en")), Locale::En);
        assert_eq!(locale_from(Some("fr"), Some("en")), Locale::En);
        assert_eq!(locale_from(Some(""), Some("es")), Locale::Es);
    }

    /// Every wired action survives a locked store (the launch state before
    /// unlock), and an id from a newer build does nothing at all.
    #[test]
    fn menu_actions_survive_a_locked_store() {
        let app = tauri::test::mock_app();
        on_menu_action(app.handle(), menu::id::RUN_NOW);
        on_menu_action(app.handle(), menu::id::KILL_SWITCH);
        on_menu_action(app.handle(), "tray.unknown");
    }

    /// AC2: toggling the schedule flips the flag the menu's check mark shows.
    #[test]
    fn toggling_the_schedule_flips_the_flag() {
        let app = tauri::test::mock_app();
        let _ = app.manage(TrayState::default());
        assert!(!tray::schedule_paused(app.handle()));
        on_menu_action(app.handle(), menu::id::PAUSE_SCHEDULE);
        assert!(tray::schedule_paused(app.handle()));
        on_menu_action(app.handle(), menu::id::PAUSE_SCHEDULE);
        assert!(!tray::schedule_paused(app.handle()));
    }

    /// AC3: focusing with no window is a no-op, never a panic — the single
    /// instance callback runs whatever state the app is in.
    #[test]
    fn focusing_without_a_window_is_a_no_op() {
        let app = tauri::test::mock_app();
        focus_main(app.handle());
        on_second_instance(app.handle(), vec!["boba".to_string()], "/tmp".to_string());
    }

    /// AC1/AC2: `install` registers the state and wires close-to-tray; a second
    /// call (a language change re-localizing the menu) must not re-register the
    /// window handler. Without a tray icon the menu is skipped, not fatal.
    ///
    /// The mock app carries the global-shortcut and notification plugins,
    /// exactly as `lib.rs` registers them: `install` reads the first (S9.8,
    /// FR-86) and the scheduler's first tick reads the second (S5.1).
    #[test]
    fn install_wires_what_is_there_and_only_the_window_handler_once() {
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_global_shortcut::Builder::new().build())
            .plugin(tauri_plugin_notification::init())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app");
        let _window =
            tauri::WebviewWindowBuilder::new(&app, MAIN_WINDOW, tauri::WebviewUrl::default())
                .build()
                .expect("window");

        install(app.handle());
        install(app.handle());

        assert!(app.try_state::<TrayState>().is_some());
        assert!(app.try_state::<Installed>().is_some());
    }

    /// AC3: the running window is the one a second launch lands on.
    #[test]
    fn focusing_finds_the_main_window() {
        let app = tauri::test::mock_app();
        let _window =
            tauri::WebviewWindowBuilder::new(&app, MAIN_WINDOW, tauri::WebviewUrl::default())
                .build()
                .expect("window");

        on_second_instance(app.handle(), vec!["boba".to_string()], "/tmp".to_string());

        assert!(app.get_webview_window(MAIN_WINDOW).is_some());
    }

    /// AC2: the UI language of the tray is the one the store holds when the
    /// settings page has set it (the store is not mounted here, so this only
    /// proves the lookup does not panic without a store).
    #[test]
    fn the_ui_language_resolves_without_a_store() {
        let app = tauri::test::mock_app();
        let _ = ui_locale(app.handle());
    }
}
