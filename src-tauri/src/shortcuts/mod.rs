//! Global shortcuts (S9.8, FR-86): Cmd/Ctrl+Shift+K engages the Kill Switch.
//!
//! One shortcut exists, and it only ever **stops** automation: pressing a
//! panic key must never turn automatic sending back on. The shortcut is
//! registered once, from [`crate::lifecycle::install`], so no later slice edits
//! `lib.rs`.
//!
//! A host where the combination is already taken keeps working: registration
//! fails, the failure is logged, and the tray and the header still flip the
//! switch.

use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_global_shortcut::{
    Code, GlobalShortcut, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState,
};

/// The modifier the platform calls "Cmd" (macOS) or "Ctrl" (everywhere else).
pub fn primary_modifier(is_macos: bool) -> Modifiers {
    if is_macos {
        Modifiers::SUPER
    } else {
        Modifiers::CONTROL
    }
}

/// Cmd+Shift+K on macOS, Ctrl+Shift+K elsewhere.
pub fn kill_switch_shortcut(is_macos: bool) -> Shortcut {
    Shortcut::new(
        Some(primary_modifier(is_macos) | Modifiers::SHIFT),
        Code::KeyK,
    )
}

/// Register the shortcut once the global-shortcut plugin is up.
///
/// A host without the plugin (the mock runtime a test builds, or a build that
/// dropped it) has no shortcut table at all: reading it would panic, so the
/// install is a no-op there and the tray and the header still flip the switch.
pub fn install<R: Runtime>(app: &AppHandle<R>) {
    if app.try_state::<GlobalShortcut<R>>().is_none() {
        tracing::debug!("global-shortcut plugin missing; the Kill Switch shortcut is off");
        return;
    }
    let shortcut = kill_switch_shortcut(cfg!(target_os = "macos"));
    let result = app
        .global_shortcut()
        .on_shortcut(shortcut, |app, _shortcut, event| {
            if event.state() != ShortcutState::Pressed {
                return;
            }
            if let Err(error) = crate::tray::kill_switch::engage(app) {
                tracing::warn!(key = error.message_key, "kill switch shortcut failed");
            }
        });
    if let Err(error) = result {
        tracing::warn!(%error, "the Kill Switch shortcut is not registered");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AC1: the panic key is Cmd+Shift+K on macOS and Ctrl+Shift+K elsewhere.
    #[test]
    fn the_shortcut_matches_the_platform() {
        assert!(primary_modifier(true).contains(Modifiers::SUPER));
        assert!(!primary_modifier(true).contains(Modifiers::CONTROL));
        assert!(primary_modifier(false).contains(Modifiers::CONTROL));
        assert!(!primary_modifier(false).contains(Modifiers::SUPER));
        for is_macos in [true, false] {
            let shortcut = kill_switch_shortcut(is_macos);
            let modifiers = shortcut.mods;
            assert!(modifiers.contains(Modifiers::SHIFT), "Shift is part of it");
            assert_eq!(shortcut.key, Code::KeyK);
        }
    }

    /// A mock runtime has no global-shortcut plugin, and reading its state would
    /// panic: installing on such a host is a no-op (the tray still works).
    #[test]
    fn installing_without_the_plugin_is_a_no_op() {
        let app = tauri::test::mock_app();
        install(app.handle());
    }
}
