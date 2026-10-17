//! The notification preferences over IPC (S5.3, FR-107): read them, and save
//! what the Settings page changed.
//!
//! Stored as one JSON row in the encrypted `setting` table (S1.7, AD-16) read
//! through the mounted [`SystemRepo`] port (ADR-012), exactly like the other
//! settings the run reads. Reads are tolerant: a row this build cannot read
//! falls back to the defaults, so the page always has values to show.

use std::sync::Arc;

use boba_app::digest::NotificationSettings;
use boba_app::digest::settings::SETTINGS_KEY;
use boba_app::error::{AppError, AppResult, ErrorCode};
use boba_app::ports::clock::ClockPort;
use boba_app::ports::repo::system::{SystemRepo, SystemRepoError};
use boba_app::system::SystemClock;

use crate::wiring;

/// The store, through the composition root (ADR-012): locked is a typed error.
fn repo(app: &tauri::AppHandle) -> AppResult<Arc<dyn SystemRepo>> {
    wiring::port::<dyn SystemRepo>(app)
}

/// The stored settings, or the defaults when nothing is stored yet (FR-107).
pub(crate) fn snapshot(repo: &dyn SystemRepo) -> Result<NotificationSettings, SystemRepoError> {
    Ok(NotificationSettings::from_json(
        repo.setting_get(SETTINGS_KEY)?.as_deref(),
    ))
}

/// Persist the user's choices, clamped to what the app can honour.
pub(crate) fn save(
    repo: &dyn SystemRepo,
    clock: &dyn ClockPort,
    settings: NotificationSettings,
) -> Result<NotificationSettings, SaveError> {
    let settings = settings.sanitized();
    let json =
        serde_json::to_string(&settings).map_err(|error| SaveError::Encode(error.to_string()))?;
    repo.setting_put(SETTINGS_KEY, &json, &clock.now_rfc3339())?;
    Ok(settings)
}

/// A save that failed, as the IPC layer tells it (AD-3: keys, never prose).
#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error(transparent)]
    Repo(#[from] SystemRepoError),
    #[error("the settings could not be encoded: {0}")]
    Encode(String),
}

impl SaveError {
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::Repo(_) | Self::Encode(_) => "errors.store.setting",
        }
    }
}

/// Map a save failure onto the typed IPC error (AD-3).
fn to_app(error: SaveError) -> AppError {
    AppError::new(ErrorCode::Internal, error.message_key())
}

/// The current notification settings (FR-107).
#[tauri::command]
#[specta::specta]
pub fn notifications_snapshot(app: tauri::AppHandle) -> AppResult<NotificationSettings> {
    snapshot(repo(&app)?.as_ref())
        .map_err(|_| AppError::new(ErrorCode::Internal, "errors.store.locked"))
}

/// Save the notification settings (FR-107).
#[tauri::command]
#[specta::specta]
pub fn notifications_save(
    app: tauri::AppHandle,
    settings: NotificationSettings,
) -> AppResult<NotificationSettings> {
    save(repo(&app)?.as_ref(), &SystemClock, settings).map_err(to_app)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use boba_app::system::FixedClock;
    use boba_store::db::{Store, StoreConfig};
    use boba_store::keys::DbKey;
    use boba_store::repos::audit::SqliteSystemRepo;

    use super::*;

    const KEY_HEX: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

    fn tmp_dir(tag: &str) -> PathBuf {
        // One store file per call: parallel tests must not share it.
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("boba-notif-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// The real encrypted store over the real `setting` table: no keychain, so
    /// the round trip these tests exercise is the one the app uses.
    fn repo(tag: &str) -> SqliteSystemRepo {
        let dir = tmp_dir(tag);
        let key = DbKey::from_hex(KEY_HEX).expect("test key");
        let config = StoreConfig::new(dir.join("boba.db"), dir.join("snapshots"));
        let store = Arc::new(Store::open_with_key(config, key).expect("store opens"));
        SqliteSystemRepo::new(store, "test")
    }

    fn clock() -> FixedClock {
        FixedClock::new("2026-10-17T12:00:00Z")
    }

    /// FR-107: a fresh install notifies about everything.
    #[test]
    fn a_fresh_store_reads_as_the_defaults() {
        let store = repo("defaults");
        assert_eq!(
            snapshot(&store).expect("snapshot"),
            NotificationSettings::default()
        );
    }

    /// FR-107: what the page saves is what the next read returns.
    #[test]
    fn a_saved_choice_round_trips() {
        let store = repo("roundtrip");
        let mut settings = NotificationSettings::default();
        settings.events.errors = false;
        settings.quiet_hours.enabled = true;
        settings.quiet_hours.from = 21;
        settings.quiet_hours.to = 8;
        settings.top_matches_shown = 5;

        let saved = save(&store, &clock(), settings).expect("save");
        assert_eq!(saved, settings);
        assert_eq!(snapshot(&store).expect("snapshot"), settings);
    }

    /// A value the UI could never produce (a count past the cap) is stored
    /// clamped, so the notification never names eleven matches.
    #[test]
    fn an_out_of_range_count_is_clamped_on_save() {
        let store = repo("clamped");
        let settings = NotificationSettings {
            top_matches_shown: 200,
            ..NotificationSettings::default()
        };
        let saved = save(&store, &clock(), settings).expect("save");
        assert_eq!(
            saved.top_matches_shown,
            boba_app::digest::settings::MAX_TOP_MATCHES
        );
    }
}
