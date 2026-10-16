//! Unlock commands (S1.2b, FR-109, ADR-009): the Locked state, the keychain
//! retry and the Argon2id passphrase fallback.
//!
//! The store itself is opened by the composition root (ADR-012), so nothing here
//! touches `boba.db`: the passphrase derives a key in [`boba_store::unlock`], the
//! flow proves it against the database, and [`crate::wiring::mount_with_key`]
//! opens and mounts the store. A scheduled run while locked is the scheduler's
//! business (S5.1), which reads the store state this flow keeps current.
//!
//! The passphrase never leaves this module: it arrives as a `String` (Tauri's
//! decode), is wrapped in a [`Passphrase`] immediately, and is zeroized on drop.

use boba_app::error::{AppError, AppResult, ErrorCode};
use boba_store::db::StoreConfig;
use boba_store::keys::DbKey;
use boba_store::unlock::{self, KeySession, Passphrase, UnlockError};
use serde::{Deserialize, Serialize};
use tauri::Manager as _;

use crate::wiring;

/// Whether the store is open or the unlock screen must act (AC2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum UnlockState {
    Ready,
    Locked,
}

/// Why the store is locked. The UI shows a key, never core text (AD-3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockReason {
    /// A key exists for this database but cannot be read (ACL change, wiped
    /// keychain), or the passphrase key went idle: enter the passphrase again.
    KeyUnreadable,
    /// No keychain is usable on this machine (headless Linux without Secret
    /// Service): set a passphrase and the key is derived with Argon2id (AC1).
    NoKeychain,
}

impl LockReason {
    /// The i18n key the UI shows (AD-3).
    pub fn message_key(self) -> &'static str {
        match self {
            Self::KeyUnreadable => "errors.unlock.locked",
            Self::NoKeychain => "errors.unlock.no_keychain",
        }
    }
}

/// What the unlock screen renders (AC1/AC2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct UnlockStatus {
    pub state: UnlockState,
    /// The locked copy to show; absent when the store is open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_key: Option<String>,
}

/// Turn a use-case failure into the typed IPC error (AD-3).
fn to_app(error: UnlockError) -> AppError {
    let code = match error {
        UnlockError::NoPassphrase | UnlockError::WrongPassphrase => ErrorCode::InvalidInput,
        UnlockError::Kdf(_) | UnlockError::Io(_) | UnlockError::Key(_) | UnlockError::Store(_) => {
            ErrorCode::Internal
        }
    };
    AppError::new(code, error.message_key())
}

/// The login-session key holder (mounted by the composition root).
fn session(app: &tauri::AppHandle) -> &KeySession {
    app.state::<KeySession>().inner()
}

/// The renderable state: the store must be open *and* the session usable.
fn status(app: &tauri::AppHandle) -> AppResult<UnlockStatus> {
    let open =
        matches!(wiring::store_state(app), wiring::StoreStatus::Ready) && session(app).peek();
    if open {
        return Ok(UnlockStatus {
            state: UnlockState::Ready,
            message_key: None,
        });
    }
    // No database yet means no keychain was usable, so the passphrase creates the
    // store; an existing one means its key could not be read (ADR-009 §1/§2).
    // An unresolvable path is treated as an existing store: the generic
    // "cannot read the key" copy, never one that invites a passphrase that
    // could mint a key over existing data (ADR-009 §2).
    let db_exists = wiring::store_config(app).is_ok_and(|c| c.db_path.exists());
    let reason = if db_exists {
        LockReason::KeyUnreadable
    } else {
        LockReason::NoKeychain
    };
    Ok(UnlockStatus {
        state: UnlockState::Locked,
        message_key: Some(reason.message_key().to_string()),
    })
}

/// The passphrase flow, with the composition root injected so a test can drive
/// the real store without a Tauri app: derive, prove, then mount and unlock.
fn open_with_passphrase(
    config: &StoreConfig,
    session: &KeySession,
    passphrase: &Passphrase,
    mount: impl FnOnce(DbKey) -> Result<(), String>,
) -> Result<(), UnlockError> {
    let key = unlock::passphrase_key(&config.db_path, passphrase)?;
    // Prove the passphrase before anything is unlocked, so a typo never re-arms a
    // session over a store the key cannot open (ADR-009 §2).
    unlock::verify_key(&config.db_path, &key)?;

    // The session keeps its own copy so it can drop it on idle without the open
    // store losing the key it was opened with.
    let held = DbKey::from_hex(key.hex().as_str())?;
    mount(key).map_err(UnlockError::Store)?;
    session.unlock(held);
    Ok(())
}

/// Whether the store is open or the unlock screen must act (AC2).
#[tauri::command]
#[specta::specta]
pub fn unlock_status(app: tauri::AppHandle) -> AppResult<UnlockStatus> {
    status(&app)
}

/// Retry the keychain in the foreground, where the OS prompt can be answered
/// (AC2). It re-runs the composition root, so a successful read mounts the store.
#[tauri::command]
#[specta::specta]
pub fn unlock_retry_keychain(app: tauri::AppHandle) -> AppResult<UnlockStatus> {
    wiring::mount(&app);
    status(&app)
}

/// Unlock with the passphrase and hold the derived key for this login session
/// (AC1/AC2). The key is never written to disk, and the session's copy is dropped
/// on idle timeout, quit or logout — from then on every port lookup reports the
/// typed Locked error until the passphrase is entered again. The open store keeps
/// its own copy of the key until quit (ADR-013 §Consequences).
#[tauri::command]
#[specta::specta]
pub fn unlock_with_passphrase(
    app: tauri::AppHandle,
    passphrase: String,
) -> AppResult<UnlockStatus> {
    let passphrase = Passphrase::new(passphrase);
    let config = wiring::store_config(&app).map_err(|reason| {
        tracing::warn!(%reason, "unlock: store paths unavailable");
        AppError::internal("errors.unlock.store")
    })?;

    open_with_passphrase(&config, session(&app), &passphrase, |key| {
        wiring::mount_with_key(&app, key)
    })
    .map_err(|error| {
        tracing::warn!(message_key = error.message_key(), "unlock refused");
        to_app(error)
    })?;
    status(&app)
}

#[cfg(test)]
mod tests {
    use boba_store::db::Store;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn config(tag: &str) -> StoreConfig {
        // One store per call: parallel tests must not share it.
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("boba-unlock-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        StoreConfig::new(dir.join("boba.db"), dir.join("snapshots"))
    }

    fn opened(tag: &str, passphrase: &str) -> StoreConfig {
        let config = config(tag);
        let key =
            unlock::passphrase_key(&config.db_path, &Passphrase::new(passphrase)).expect("derive");
        drop(Store::open_with_key(config.clone(), key).expect("store opens"));
        config
    }

    #[allow(clippy::type_complexity)]
    fn counting() -> (Arc<AtomicUsize>, impl FnOnce(DbKey) -> Result<(), String>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let handle = calls.clone();
        (calls, move |_key| {
            handle.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }

    /// AC1: the passphrase creates the store, and the key is held for the session.
    #[test]
    fn the_passphrase_opens_the_store_and_holds_the_key_for_the_session() {
        let config = config("first");
        let session = KeySession::new(unlock::DEFAULT_IDLE);
        let opened = Arc::new(AtomicUsize::new(0));
        let counting = opened.clone();
        let opening = config.clone();

        open_with_passphrase(
            &config,
            &session,
            &Passphrase::new("correct horse battery"),
            move |key| {
                // What the composition root does: open the store with the derived key.
                let store = Store::open_with_key(opening, key).map_err(|e| e.to_string())?;
                counting.fetch_add(1, Ordering::SeqCst);
                drop(store);
                Ok(())
            },
        )
        .expect("unlock");

        assert_eq!(
            opened.load(Ordering::SeqCst),
            1,
            "the store was opened once"
        );
        assert!(config.db_path.exists(), "the store exists");
        assert!(session.peek(), "the session holds the key");
    }

    /// AC2: a wrong passphrase opens nothing and never re-arms a locked session.
    #[test]
    fn a_wrong_passphrase_leaves_the_session_locked_and_the_store_closed() {
        let config = opened("wrong", "correct horse battery");
        let session = KeySession::new(unlock::DEFAULT_IDLE);
        // The key went idle (or the user logged out): the session is closed.
        session.lock();
        let (mounted, mount) = counting();

        let error = open_with_passphrase(&config, &session, &Passphrase::new("tr0ub4dor&3"), mount)
            .expect_err("the passphrase is refused");

        assert!(matches!(error, UnlockError::WrongPassphrase), "{error}");
        assert_eq!(error.message_key(), "errors.unlock.wrong_passphrase");
        assert_eq!(
            mounted.load(Ordering::SeqCst),
            0,
            "the store was never opened"
        );
        assert!(
            !session.peek(),
            "a wrong passphrase never re-arms the session"
        );
    }

    /// AC1: unlocking again after the idle timeout only re-arms the session; the
    /// composition root decides whether the store has to be (re)opened.
    #[test]
    fn the_right_passphrase_re_arms_a_locked_session() {
        let config = opened("idle", "correct horse battery");
        let session = KeySession::new(unlock::DEFAULT_IDLE);
        // The key went idle (the idle timeout and a logout close it the same way).
        session.lock();
        assert!(!session.peek(), "the session is closed");
        let (mounted, mount) = counting();

        open_with_passphrase(
            &config,
            &session,
            &Passphrase::new("correct horse battery"),
            mount,
        )
        .expect("unlock");

        assert_eq!(
            mounted.load(Ordering::SeqCst),
            1,
            "the root is asked to mount"
        );
        assert!(session.peek(), "the session is armed again");
    }

    /// The errors the UI shows are keys, never Rust text (AD-3).
    #[test]
    fn unlock_errors_map_to_codes_and_i18n_keys() {
        let refusal = to_app(UnlockError::WrongPassphrase);
        assert_eq!(refusal.code, ErrorCode::InvalidInput);
        assert_eq!(refusal.message_key, "errors.unlock.wrong_passphrase");

        let refusal = to_app(UnlockError::NoPassphrase);
        assert_eq!(refusal.code, ErrorCode::InvalidInput);
        assert_eq!(refusal.message_key, "errors.unlock.no_passphrase");

        let broken = to_app(UnlockError::Store("mount failed".to_string()));
        assert_eq!(broken.code, ErrorCode::Internal);
        assert_eq!(broken.message_key, "errors.unlock.store");
        assert!(
            !format!("{broken:?}").contains("mount failed"),
            "no core text crosses IPC"
        );

        assert_eq!(
            LockReason::NoKeychain.message_key(),
            "errors.unlock.no_keychain"
        );
        assert_eq!(
            LockReason::KeyUnreadable.message_key(),
            "errors.unlock.locked"
        );
    }

    /// A passphrase is never echoed back, in a payload or in a log (NFR-2).
    #[test]
    fn a_passphrase_never_reaches_a_payload() {
        let status = UnlockStatus {
            state: UnlockState::Locked,
            message_key: Some(LockReason::NoKeychain.message_key().to_string()),
        };
        let json = serde_json::to_string(&status).expect("json");
        assert_eq!(
            json,
            "{\"state\":\"locked\",\"message_key\":\"errors.unlock.no_keychain\"}"
        );

        let config: PathBuf = config("json").db_path;
        assert!(config.to_string_lossy().ends_with("boba.db"));
    }
}
