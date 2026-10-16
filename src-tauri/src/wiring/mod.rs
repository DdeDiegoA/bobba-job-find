//! Composition root (ADR-012): open the encrypted store and mount its ports.
//!
//! `lib.rs` calls [`mount`] exactly once, in `setup`. After CH4 no slice edits
//! `lib.rs` for wiring: a slice that adds a port appends one line to
//! [`port_list`]. Commands read a port through [`port`] and never `try_state`
//! directly, so a locked store is a typed error instead of a panic.

pub mod agent;
pub mod port_list;

use std::path::PathBuf;
use std::sync::Arc;

use boba_app::error::{AppError, AppResult};
use boba_store::db::{Store, StoreConfig};
use tauri::{AppHandle, Manager, Runtime};

pub use port_list::mount_ports;

/// State of the encrypted store on this launch (ADR-012 §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreStatus {
    /// The store is open and [`mount_ports`] ran.
    Ready,
    /// The key is unavailable, so only the unlock flow (S1.2b) can recover.
    Locked { reason: String },
}

/// The keychain service for this launch: dev keys never touch real data (ADR-009).
pub fn keychain_service() -> &'static str {
    if cfg!(debug_assertions) {
        boba_store::keys::DEV_SERVICE
    } else {
        boba_store::keys::SERVICE
    }
}

/// Store location inside the OS app-data dir.
fn config(app: &AppHandle<impl Runtime>) -> Result<StoreConfig, String> {
    let dir: PathBuf = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir unavailable: {e}"))?;
    let (db, snapshots) = (dir.join("boba.db"), dir.join("snapshots"));
    Ok(if cfg!(debug_assertions) {
        StoreConfig::dev(db, snapshots)
    } else {
        StoreConfig::new(db, snapshots)
    })
}

/// Mount the composition root: open the store and wire its ports (ADR-012 §2/§4).
///
/// Never panics: a failed open leaves the app running in [`StoreStatus::Locked`],
/// where every [`port`] lookup reports the typed Locked error.
pub fn mount(app: &AppHandle<impl Runtime>) {
    let opened = config(app).and_then(|c| Store::open(c).map(Arc::new).map_err(|e| e.to_string()));
    match opened {
        Ok(store) => {
            mount_ports(app, &store);
            let _ = app.manage(StoreStatus::Ready);
        }
        Err(reason) => {
            tracing::warn!(reason = %reason, "store open failed; ports stay unmounted");
            let _ = app.manage(StoreStatus::Locked { reason });
        }
    }
}

/// Mount one port adapter; panics on a duplicate line in [`port_list`]
/// (ADR-012 §Consequences). `Tauri`'s `manage` only returns `false` on a second
/// registration of the same type, which would silently drop an adapter.
pub fn mount_port<P>(app: &AppHandle<impl Runtime>, adapter: Arc<P>)
where
    P: ?Sized + Send + Sync + 'static,
{
    assert!(
        app.manage::<Arc<P>>(adapter),
        "duplicate port line in port_list for {}",
        std::any::type_name::<P>()
    );
}

/// The port mounted for `P`, or the typed Locked error (ADR-012 §4): commands
/// translate `errors.store.locked` instead of panicking on unmounted state.
pub fn port<P>(app: &AppHandle<impl Runtime>) -> AppResult<Arc<P>>
where
    P: ?Sized + Send + Sync + 'static,
{
    app.try_state::<Arc<P>>()
        .map(|state| Arc::clone(state.inner()))
        .ok_or_else(|| AppError::locked("errors.store.locked"))
}

// Composition root tests (ADR-012): the app-start smoke test that mounts the
// real `port_list` over an encrypted store, and the locked-port helper.
//
// `tauri` is a dev-dependency here with the `test` feature so `tauri::test`
// (mock runtime) is available; the store is opened with a fixed key in a temp
// dir, so nothing touches the OS keychain or real data.

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use boba_app::error::{AppError, ErrorCode};
    use boba_app::ports::repo::agent::AgentRepo;
    use boba_app::ports::repo::sources::SourcesRepo;
    use boba_app::ports::repo::system::SystemRepo;
    use boba_store::db::{Store, StoreConfig};
    use boba_store::keys::DbKey;
    use tauri::Manager as _;
    use tauri::test::mock_app;

    use super::{StoreStatus, mount_ports, port};

    /// Fixed key: the store never resolves one from the keychain here.
    const KEY_HEX: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

    fn tmp_dir(tag: &str) -> PathBuf {
        // One store file per call: parallel tests in this process must not share it.
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("boba-wiring-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// An open store with every migration applied (S1.2a path, no keychain).
    fn open_store() -> Arc<Store> {
        let dir = tmp_dir("store");
        let key = DbKey::from_hex(KEY_HEX).expect("test key");
        let config = StoreConfig::new(dir.join("boba.db"), dir.join("snapshots"));
        Arc::new(Store::open_with_key(config, key).expect("store opens"))
    }

    /// App-start smoke test (ADR-012 §Consequences): `mount_ports` over the real
    /// port list must mount every port without a panic, and each one must work.
    #[test]
    fn app_start_smoke_mounts_every_port() {
        let app = mock_app();
        let store = open_store();

        mount_ports(app.handle(), &store);

        let sources = port::<dyn SourcesRepo>(app.handle()).expect("sources port");
        assert!(sources.list_sources().expect("list").is_empty());

        let system = port::<dyn SystemRepo>(app.handle()).expect("system port");
        assert_eq!(system.setting_get("theme").expect("get"), None);

        let agent = port::<dyn AgentRepo>(app.handle()).expect("agent port");
        assert!(agent.unfinished_runs().expect("unfinished").is_empty());
    }

    /// A bad union merge appends the same port twice; mounting it again must fail
    /// loudly instead of silently dropping the second adapter (ADR-012 §3).
    #[test]
    #[should_panic(expected = "duplicate port line in port_list")]
    fn duplicate_port_line_panics() {
        let app = mock_app();
        let store = open_store();

        mount_ports(app.handle(), &store);
        mount_ports(app.handle(), &store);
    }

    /// A port that was never mounted reports the typed Locked state (ADR-012 §4):
    /// commands translate `errors.store.locked` instead of panicking.
    #[test]
    fn unmounted_port_is_a_typed_locked_error() {
        let app = mock_app();
        app.manage(StoreStatus::Locked {
            reason: "key unreadable".to_string(),
        });

        let err = port::<dyn SourcesRepo>(app.handle())
            .err()
            .expect("must be locked");
        assert_eq!(err.code, ErrorCode::Locked);
        assert_eq!(err.message_key, "errors.store.locked");
        assert_eq!(err, AppError::locked("errors.store.locked"));
    }
}
