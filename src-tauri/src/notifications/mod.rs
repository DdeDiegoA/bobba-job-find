//! Native notifications: the digest a finished run announces (FR-45), the
//! preferences that shape it (FR-107), and the click that opens Today.
//!
//! The decisions live in `boba_app::digest`; this module names them in the UI
//! locale and hands them to the OS. It also owns the one thing the core cannot
//! do without a window: a notification click reaches the app as **focus**, so
//! the digest arms a one-shot "on the next focus, open Today" (FR-45).

pub mod adapter;
pub mod commands;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};
use tauri_specta::Event;

pub use adapter::DigestNotifier;

/// Emitted when the user's click reaches the app: the UI opens Today (FR-45).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type, Event)]
#[tauri_specta(event_name = "notification:opened")]
pub struct NotificationOpened {
    /// Where the click wants to land; `today` is the only target today.
    pub target: String,
}

/// How long a digest click has to land: a banner the user dismissed (or never
/// clicked) must not redirect a visit long after the fact (review F1).
const OPEN_TODAY_TTL_SECS: u64 = 120;

/// When the pending open stops counting, epoch seconds (0 = nothing pending).
static OPEN_TODAY_UNTIL: AtomicU64 = AtomicU64::new(0);
/// The window's focus handler is registered once per process.
static FOCUS_ARMED: AtomicBool = AtomicBool::new(false);

/// Arm "the next time the window comes to the front, open Today" (FR-45).
///
/// Tauri's notification plugin reports no click on desktop, but clicking a
/// notification brings Boba forward: the focus that follows is the click. A
/// host without the main window (a test app) simply never arms.
pub fn arm_open_today<R: Runtime>(app: &AppHandle<R>) {
    arm_until(unix_now().saturating_add(OPEN_TODAY_TTL_SECS));
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
            let opened = NotificationOpened {
                target: "today".to_string(),
            };
            if let Err(error) = opened.emit(&handle) {
                tracing::warn!(%error, "notification:opened not delivered");
            }
        }
    });
}

/// Arm a click that lands at `deadline` (epoch seconds) at the latest.
fn arm_until(deadline: u64) {
    OPEN_TODAY_UNTIL.store(deadline, Ordering::SeqCst);
}

/// Take the pending click at `now`, if it is still inside its window: exactly
/// one focus opens Today.
fn claim_at(now: u64) -> bool {
    let deadline = OPEN_TODAY_UNTIL.load(Ordering::SeqCst);
    if deadline == 0 || now > deadline {
        return false;
    }
    OPEN_TODAY_UNTIL
        .compare_exchange(deadline, 0, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
}

fn claim_pending_open() -> bool {
    claim_at(unix_now())
}

/// Seconds since the epoch; a clock before it reads as 0 (nothing pending).
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The window's static is shared by the whole test binary: one test at a
    /// time so the two below cannot see each other's arm.
    static SERIAL: Mutex<()> = Mutex::new(());

    /// F1: a digest dismissed (or never clicked) must stop meaning "open
    /// Today" after its window, or a later visit redirects mid-task.
    #[test]
    fn a_pending_open_expires_after_its_window() {
        let _serial = SERIAL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        arm_until(1_000);
        assert!(
            !claim_at(1_001),
            "a stale click never redirects a later visit"
        );
        assert!(claim_at(1_000), "inside the window the click lands");
        assert!(!claim_at(1_000), "one click, one open");
        arm_until(0);
    }

    /// F1: arming sets a bounded deadline, not a permanent flag.
    #[test]
    fn arming_bounds_the_window() {
        let _serial = SERIAL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = unix_now();
        arm_until(now + OPEN_TODAY_TTL_SECS);
        assert!(!claim_at(now + OPEN_TODAY_TTL_SECS + 1));
        assert!(claim_at(now + OPEN_TODAY_TTL_SECS - 1));
        arm_until(0);
    }

    /// Arming without a main window (a mock app) sets the request and arms
    /// nothing, so the next real focus cannot panic on a missing window.
    #[test]
    fn arming_without_a_window_is_a_no_op() {
        let _serial = SERIAL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let app = tauri::test::mock_app();
        arm_open_today(app.handle());
        assert!(claim_pending_open());
        assert!(!claim_pending_open());
        arm_until(0);
    }
}
