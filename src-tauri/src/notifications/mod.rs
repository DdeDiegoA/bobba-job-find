//! Native notifications (S5.1 stub, replaced by S5.3): the digest a finished
//! run announces (FR-45), the preferences that shape it (FR-107), and the click
//! that opens Today.
//!
//! The decisions live in `boba_app::digest`; this module names them in the UI
//! locale and hands them to the OS. It also owns the one thing the core cannot
//! do without a window: a notification click reaches the app as **focus**, so
//! the digest arms a one-shot "on the next focus, open Today" (FR-45).

pub mod adapter;
pub mod commands;

use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};
use tauri_specta::Event;

pub use adapter::DigestNotifier;

/// Emitted when the user's click reaches the app: the UI opens Today (FR-45).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type, Event)]
#[tauri_specta(event_name = "notification.opened")]
pub struct NotificationOpened {
    /// Where the click wants to land; `today` is the only target today.
    pub target: String,
}

/// A digest was shown and its click has not landed yet.
static OPEN_TODAY_PENDING: AtomicBool = AtomicBool::new(false);
/// The window's focus handler is registered once per process.
static FOCUS_ARMED: AtomicBool = AtomicBool::new(false);

/// Arm "the next time the window comes to the front, open Today" (FR-45).
///
/// Tauri's notification plugin reports no click on desktop, but clicking a
/// notification brings Boba forward: the focus that follows is the click. A
/// host without the main window (a test app) simply never arms.
pub fn arm_open_today<R: Runtime>(app: &AppHandle<R>) {
    OPEN_TODAY_PENDING.store(true, Ordering::SeqCst);
    if FOCUS_ARMED.load(Ordering::SeqCst) {
        return;
    }
    let Some(window) = app.get_webview_window(crate::lifecycle::MAIN_WINDOW) else {
        tracing::debug!("no main window: the digest click opens nothing");
        return;
    };
    if FOCUS_ARMED.swap(true, Ordering::SeqCst) {
        return;
    }
    let handle = app.clone();
    window.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Focused(true)) && claim_pending_open() {
            let _ = NotificationOpened {
                target: "today".to_string(),
            }
            .emit(&handle);
        }
    });
}

/// Take the pending click, if there is one: exactly one focus opens Today.
fn claim_pending_open() -> bool {
    OPEN_TODAY_PENDING.swap(false, Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AC1: the click opens Today once — a second focus is just a user coming
    /// back, not another click.
    #[test]
    fn a_pending_open_is_claimed_exactly_once() {
        OPEN_TODAY_PENDING.store(true, Ordering::SeqCst);
        assert!(claim_pending_open());
        assert!(!claim_pending_open());
    }

    /// Arming without a main window (a mock app) sets the request and arms
    /// nothing, so the next real focus cannot panic on a missing window.
    #[test]
    fn arming_without_a_window_is_a_no_op() {
        let app = tauri::test::mock_app();
        arm_open_today(app.handle());
        OPEN_TODAY_PENDING.store(false, Ordering::SeqCst);
    }
}
