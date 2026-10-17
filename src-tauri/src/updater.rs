//! Opt-in update checks (S10.5, NFR-1, NFR-19, AD-22, ADR-009).
//!
//! The updater is **off by default** and does nothing unless the user turned it
//! on (`network.update_checks = "on"`, the same row the Network activity view
//! reads). When it is on:
//!
//! * it asks GitHub Releases at startup and every 24 hours — never more, and
//!   never without the setting;
//! * every attempt is written to the network log, host only, so the opt-in is
//!   verifiable from Settings → Privacy;
//! * the `tauri-plugin-updater` verifies the **minisign signature before it
//!   extracts** anything (`Update::download` → `verify_signature`, only then
//!   `install`), and `requireSignedVersion` rejects a downgrade replay;
//! * a native confirmation dialog asks before downloading, and nothing is
//!   installed, relaunched or restarted without that click.
//!
//! Failures are logged and degrade nothing (NFR-14): an offline machine, a
//! suspended release or a bad signature simply leaves the app as it is.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use boba_app::error::{AppError, AppResult};
use boba_app::i18n::Locale;
use boba_app::network_log::{self, Purpose, UPDATE_CHECK_KEY, UPDATE_HOST};
use boba_app::ports::clock::ClockPort;
use boba_app::ports::repo::system::SystemRepo;
use boba_app::system::SystemClock;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Runtime};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_updater::UpdaterExt;

use crate::wiring;

/// The stored value that means "the user opted in" (the same spelling
/// `commands::network` compares against).
pub const ENABLED: &str = "on";
/// The stored value for "off"; kept explicit so switching back is a write, not
/// a delete.
pub const DISABLED: &str = "off";

/// The architecture's cadence: startup and once a day (§11).
pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// Let startup settle (store, scheduler, window) before the first request.
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(15);

/// One check at a time: a manual check racing the daily one must not open two
/// dialogs or download twice.
static CHECKING: AtomicBool = AtomicBool::new(false);
/// The daily loop is installed once per process.
static LOOP_INSTALLED: AtomicBool = AtomicBool::new(false);

/// Settings → Privacy → Network activity reads this (AC2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct UpdateSettings {
    /// Whether update checks are on. The default is off (NFR-1).
    pub enabled: bool,
}

/// Is the stored value the opt-in? Pure, so the gate is tested without a store.
pub fn is_enabled(stored: Option<&str>) -> bool {
    stored.is_some_and(|value| value.trim() == ENABLED)
}

/// The stored choice, when the store is open.
pub fn enabled<R: Runtime>(app: &AppHandle<R>) -> bool {
    let stored = wiring::port::<dyn SystemRepo>(app)
        .ok()
        .and_then(|repo| repo.setting_get(UPDATE_CHECK_KEY).ok().flatten());
    is_enabled(stored.as_deref())
}

/// Persist the choice. Turning it on checks right away (the user asked for it);
/// turning it off stops future checks because the loop re-reads the row.
pub fn set_enabled<R: Runtime>(app: &AppHandle<R>, enabled: bool) -> AppResult<UpdateSettings> {
    let repo = wiring::port::<dyn SystemRepo>(app)?;
    repo.setting_put(
        UPDATE_CHECK_KEY,
        if enabled { ENABLED } else { DISABLED },
        &SystemClock.now_rfc3339(),
    )
    .map_err(|error| {
        AppError::internal("errors.store.setting").with_detail("reason", error.to_string())
    })?;
    if enabled {
        spawn_check(app.clone());
    }
    Ok(UpdateSettings { enabled })
}

/// Run at startup from [`crate::lifecycle::install`]: the daily loop. The first
/// check is a few seconds in, so the window and the scheduler come first.
pub fn install<R: Runtime>(app: &AppHandle<R>) {
    if LOOP_INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK_DELAY).await;
        loop {
            check_once(&handle).await;
            tokio::time::sleep(CHECK_INTERVAL).await;
        }
    });
}

/// One check now, in the background (used when the user turns the setting on).
pub fn spawn_check<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        check_once(&app).await;
    });
}

/// The check itself: gated by the setting, logged, verified by the plugin, and
/// installed only after the native confirmation.
async fn check_once<R: Runtime>(app: &AppHandle<R>) {
    if !enabled(app) {
        return;
    }
    if CHECKING.swap(true, Ordering::SeqCst) {
        return;
    }
    // The log entry exists even if the request fails: the user opted in, and
    // the view should show that Boba tried (host only, never a path).
    network_log::observe(
        Purpose::Update,
        UPDATE_HOST,
        &SystemClock.now_rfc3339(),
        None,
    );

    let result = checked(app).await;
    CHECKING.store(false, Ordering::SeqCst);
    if let Err(error) = result {
        // NFR-14: a failed check changes nothing.
        tracing::warn!(%error, "update check failed");
    }
}

/// The fallible half of [`check_once`].
async fn checked<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let updater = app.updater().map_err(|error| error.to_string())?;
    let Some(update) = updater.check().await.map_err(|error| error.to_string())? else {
        return Ok(());
    };
    if !confirm(app, &update.version).await {
        return Ok(());
    }
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|error| error.to_string())?;
    // macOS and Linux replace the bundle in place and need the relaunch; on
    // Windows the NSIS installer already restarts the app (`/R`).
    #[cfg(not(windows))]
    {
        app.restart()
    }
    #[cfg(windows)]
    {
        Ok(())
    }
}

/// The native confirmation (AC2): nothing is downloaded before this click.
async fn confirm<R: Runtime>(app: &AppHandle<R>, version: &str) -> bool {
    let locale = crate::lifecycle::ui_locale(app);
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .message(fill(message(locale, &DIALOG_BODY), version))
        .title(message(locale, &DIALOG_TITLE))
        .kind(MessageDialogKind::Info)
        .buttons(MessageDialogButtons::OkCancelCustom(
            message(locale, &DIALOG_UPDATE).to_string(),
            message(locale, &DIALOG_LATER).to_string(),
        ))
        .show(move |proceed| {
            let _ = tx.send(proceed);
        });
    rx.await.unwrap_or(false)
}

/// A dialog string: `updater:<key>` in the shipped catalogs, embedded at compile
/// time like the notification and tray labels (AD-21).
pub struct Text {
    pub message_key: &'static str,
}

pub const DIALOG_TITLE: Text = Text {
    message_key: "updater:dialog.title",
};
pub const DIALOG_BODY: Text = Text {
    message_key: "updater:dialog.body",
};
pub const DIALOG_UPDATE: Text = Text {
    message_key: "updater:dialog.update",
};
pub const DIALOG_LATER: Text = Text {
    message_key: "updater:dialog.later",
};

const UPDATER_EN: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../locales/en/updater.json"
));
const UPDATER_ES: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../locales/es/updater.json"
));

fn catalog(locale: Locale) -> &'static HashMap<String, String> {
    static EN: OnceLock<HashMap<String, String>> = OnceLock::new();
    static ES: OnceLock<HashMap<String, String>> = OnceLock::new();
    match locale {
        Locale::En => EN.get_or_init(|| parse(UPDATER_EN, "en")),
        Locale::Es => ES.get_or_init(|| parse(UPDATER_ES, "es")),
    }
}

fn parse(raw: &str, lang: &str) -> HashMap<String, String> {
    serde_json::from_str::<Value>(raw)
        .unwrap_or_else(|e| panic!("locales/{lang}/updater.json is not valid JSON: {e}"))
        .as_object()
        .map(|map| {
            map.iter()
                .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// Resolve `text` for `locale`, falling back to English (a missing ES string
/// must never show a raw key in a dialog).
fn message(locale: Locale, text: &Text) -> &'static str {
    let key = text
        .message_key
        .strip_prefix("updater:")
        .unwrap_or(text.message_key);
    let resolved = catalog(locale)
        .get(key)
        .or_else(|| catalog(Locale::En).get(key));
    match resolved {
        Some(value) => value.as_str(),
        // Only reachable if a catalog loses a key the gate would have caught.
        None => text.message_key,
    }
}

/// `{version}` substitution for the dialog body (ICU's simple placeholder).
pub fn fill(template: &str, version: &str) -> String {
    template.replace("{version}", version)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// NFR-1: anything but the explicit opt-in means "do not check".
    #[test]
    fn only_the_explicit_opt_in_enables_checks() {
        assert!(is_enabled(Some("on")));
        assert!(is_enabled(Some(" on ")));
        assert!(!is_enabled(None));
        assert!(!is_enabled(Some("")));
        assert!(!is_enabled(Some("off")));
        assert!(!is_enabled(Some("true")));
        assert!(!is_enabled(Some("ON")));
    }

    /// AC2: the dialog names the version it is about.
    #[test]
    fn the_dialog_body_names_the_version() {
        let body = fill(message(Locale::En, &DIALOG_BODY), "1.4.0");
        assert!(body.contains("1.4.0"), "got {body}");
        assert!(!body.contains("{version}"), "placeholder left in {body}");
    }

    /// AD-21: both dialogs read in both languages.
    #[test]
    fn both_dialogs_resolve_in_both_languages() {
        for locale in [Locale::En, Locale::Es] {
            for text in [&DIALOG_TITLE, &DIALOG_BODY, &DIALOG_UPDATE, &DIALOG_LATER] {
                let value = message(locale, text);
                assert_ne!(value, text.message_key, "unresolved {locale:?} key");
                assert!(!value.is_empty(), "empty {locale:?} value");
            }
        }
        assert_ne!(
            message(Locale::En, &DIALOG_TITLE),
            message(Locale::Es, &DIALOG_TITLE)
        );
    }

    /// The command surface reports the stored value and never a guess.
    #[test]
    fn settings_reflect_the_stored_value() {
        assert_eq!(
            UpdateSettings {
                enabled: is_enabled(Some("on"))
            },
            UpdateSettings { enabled: true }
        );
        assert_eq!(
            UpdateSettings {
                enabled: is_enabled(Some("off"))
            },
            UpdateSettings { enabled: false }
        );
    }
}
