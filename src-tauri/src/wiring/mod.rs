//! Composition root (ADR-012): open the encrypted store and mount its ports.
//!
//! `lib.rs` calls [`mount`] exactly once, in `setup`. After CH4 no slice edits
//! `lib.rs` for wiring: a slice that adds a port appends one line to
//! [`port_list`]. Commands read a port through [`port`] and never `try_state`
//! directly, so a locked store is a typed error instead of a panic.

pub mod agent;
pub mod port_list;
pub mod session;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use boba_app::error::{AppError, AppResult};
use boba_store::db::{Store, StoreConfig};
use boba_store::keys::DbKey;
use boba_store::unlock::{self, KeySession};
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

/// The store state, held mutably: the unlock flow (S1.2b) turns `Locked` into
/// `Ready` once the store opens, and the idle gate turns it back. `manage` can
/// only register a value once, so the value wraps its own lock.
///
/// It also carries whether the ports are mounted. They are mounted once and can
/// never be unmounted (ADR-012 §3), and a second mount panics, so "the store is
/// open" has to be readable *independently* of the status: after the idle timeout
/// the status is `Locked` while the ports are still there.
#[derive(Debug)]
struct StoreState {
    status: Mutex<StoreStatus>,
    mounted: AtomicBool,
}

impl StoreState {
    fn new(status: StoreStatus, mounted: bool) -> Self {
        Self {
            status: Mutex::new(status),
            mounted: AtomicBool::new(mounted),
        }
    }

    /// The current status.
    fn status(&self) -> StoreStatus {
        self.status
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Set the status; whether the ports are mounted never changes back.
    fn set_status(&self, status: StoreStatus) {
        *self.status.lock().unwrap_or_else(|p| p.into_inner()) = status;
    }

    /// Record that the ports are mounted (idempotent).
    fn mount(&self) {
        self.mounted.store(true, Ordering::SeqCst);
    }

    /// Whether the port list is mounted.
    fn is_mounted(&self) -> bool {
        self.mounted.load(Ordering::SeqCst)
    }
}

/// The state holder, when the composition root has registered it.
fn store_state_of(app: &AppHandle<impl Runtime>) -> Option<&StoreState> {
    app.try_state::<StoreState>().map(|state| state.inner())
}

/// The store state this launch reports: what the UI gates on and what the
/// scheduler skips runs on (S5.1, ADR-009 §3).
pub fn store_state(app: &AppHandle<impl Runtime>) -> StoreStatus {
    match store_state_of(app) {
        Some(state) => state.status(),
        None => StoreStatus::Locked {
            reason: "the composition root has not run".to_string(),
        },
    }
}

/// Record the store state, registering the holder on first call.
fn set_store_state(app: &AppHandle<impl Runtime>, status: StoreStatus) {
    match store_state_of(app) {
        Some(state) => state.set_status(status),
        None => {
            let _ = app.manage(StoreState::new(status, false));
        }
    }
}

/// Whether the port list is mounted, which is what decides if a re-unlock has to
/// open and mount again (it never can: see [`mount_with_key`]).
fn ports_mounted(app: &AppHandle<impl Runtime>) -> bool {
    store_state_of(app).is_some_and(StoreState::is_mounted)
}

/// The keychain service for this launch: dev keys never touch real data (ADR-009).
pub fn keychain_service() -> &'static str {
    if cfg!(debug_assertions) {
        boba_store::keys::DEV_SERVICE
    } else {
        boba_store::keys::SERVICE
    }
}

/// Store location inside the OS app-data dir. Public so the unlock flow can prove
/// a passphrase against the database it is about to open (S1.2b, ADR-009).
pub fn store_config(app: &AppHandle<impl Runtime>) -> Result<StoreConfig, String> {
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
/// where every [`port`] lookup reports the typed Locked error. Callable again:
/// that is how the unlock screen retries the keychain in the foreground (S1.2b).
pub fn mount(app: &AppHandle<impl Runtime>) {
    // Retrying while the store is open has nothing left to do, and opening it
    // again would mount every port a second time (ADR-012 §3 panics).
    if ports_mounted(app) {
        return;
    }

    // ADR-010 §4: a run that died without cleaning up (panic-abort, SIGKILL,
    // power loss) leaves its workspace and endpoint behind. Nothing is running
    // during setup, so everything under `runs/` and `ipc/` is a leftover and is
    // swept before any port can start one. Disk hygiene only: a sweep that fails
    // must not stop the launch, and this does not read the store.
    match boba_agent::sweep::sweep_orphans(&agent::sandbox_config(app, None)) {
        Ok(report) => tracing::info!(
            workspaces = report.workspaces,
            endpoints = report.endpoints,
            "startup sweep"
        ),
        Err(error) => {
            tracing::warn!(%error, "startup sweep failed; leftover run workspaces stay on disk")
        }
    }

    // One key session per process (ADR-009 §3): the passphrase-derived key is
    // held here and dropped on idle timeout, quit or logout.
    if app.try_state::<KeySession>().is_none() {
        let _ = app.manage(KeySession::new(unlock::DEFAULT_IDLE));
    }

    let opened =
        store_config(app).and_then(|c| Store::open(c).map(Arc::new).map_err(|e| e.to_string()));
    match opened {
        Ok(store) => adopt(app, &store),
        Err(reason) => {
            tracing::warn!(reason = %reason, "store open failed; ports stay unmounted");
            set_store_state(app, StoreStatus::Locked { reason });
        }
    }
}

/// Open the store with a key already in hand — the passphrase path (S1.2b) — and
/// mount its ports.
///
/// A store that is already open is left alone: after the idle timeout the mounts
/// are still there and unlocking again only re-arms the key session (mounting a
/// port twice panics, which is the point of [`mount_port`]).
pub fn mount_with_key(app: &AppHandle<impl Runtime>, key: DbKey) -> Result<(), String> {
    // The store is already open when its ports are mounted, whatever the status
    // says: after an idle expiry the gate reports `Locked` while the ports stay.
    // Re-opening here would mount a second time and panic (ADR-012 §3).
    if ports_mounted(app) {
        // Re-unlock after an idle timeout: the store never closed, so the state
        // the gate flipped to `Locked` goes back to `Ready` with it.
        set_store_state(app, StoreStatus::Ready);
        return Ok(());
    }
    let store = Arc::new(Store::open_with_key(store_config(app)?, key).map_err(|e| e.to_string())?);
    adopt(app, &store);
    Ok(())
}

/// Mount every port of an open store and record it as ready and mounted
/// (ADR-012 §2): the one place that runs the mount, so it is the one place that
/// can say the ports are there.
fn adopt(app: &AppHandle<impl Runtime>, store: &Arc<Store>) {
    mount_ports(app, store);
    match store_state_of(app) {
        Some(state) => {
            state.mount();
            state.set_status(StoreStatus::Ready);
        }
        None => {
            let _ = app.manage(StoreState::new(StoreStatus::Ready, true));
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
    gate(app)?;
    app.try_state::<Arc<P>>()
        .map(|state| Arc::clone(state.inner()))
        .ok_or_else(|| AppError::locked("errors.store.locked"))
}

/// Gate the store behind the login-session key (ADR-009 §3, S1.2b).
///
/// While the passphrase path is in play, an idle-expired key closes every port
/// lookup: gated work gets the typed Locked error and the user types the
/// passphrase again. `poll` counts this lookup as activity, so the window is
/// "eight hours without a command", and the store state turns Locked with it —
/// which is what the scheduler skips runs on (S5.1).
fn gate(app: &AppHandle<impl Runtime>) -> AppResult<()> {
    let Some(session) = app.try_state::<KeySession>() else {
        return Ok(());
    };
    if session.poll() {
        return Ok(());
    }
    set_store_state(
        app,
        StoreStatus::Locked {
            reason: "the unlock key went idle".to_string(),
        },
    );
    Err(AppError::locked("errors.store.locked"))
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
    use boba_store::unlock::{self, KeySession};
    use tauri::Manager as _;
    use tauri::test::mock_app;

    use super::{
        StoreStatus, adopt, mount_ports, mount_with_key, port, ports_mounted, set_store_state,
        store_state,
    };

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
        set_store_state(
            app.handle(),
            StoreStatus::Locked {
                reason: "key unreadable".to_string(),
            },
        );

        let err = port::<dyn SourcesRepo>(app.handle())
            .err()
            .expect("must be locked");
        assert_eq!(err.code, ErrorCode::Locked);
        assert_eq!(err.message_key, "errors.store.locked");
        assert_eq!(err, AppError::locked("errors.store.locked"));
    }

    /// AC1 (S1.2b): an idle-expired passphrase key closes every gated command and
    /// turns the state the scheduler reads back to Locked (ADR-009 §3).
    #[test]
    fn an_idle_expired_session_gates_every_port() {
        let app = mock_app();
        let store = open_store();
        mount_ports(app.handle(), &store);
        set_store_state(app.handle(), StoreStatus::Ready);
        // Zero idle window: the key is past its timeout the moment it is held.
        app.manage(KeySession::new(std::time::Duration::ZERO));
        app.state::<KeySession>()
            .unlock(DbKey::from_hex(KEY_HEX).expect("test key"));

        let err = port::<dyn SourcesRepo>(app.handle())
            .err()
            .expect("an idle key gates every port");
        assert_eq!(err, AppError::locked("errors.store.locked"));
        assert!(matches!(
            store_state(app.handle()),
            StoreStatus::Locked { .. }
        ));
    }

    /// The keychain retry is a button, so it can be pressed late: with the store
    /// already open it must do nothing rather than open and mount it twice.
    #[test]
    fn retrying_the_keychain_on_an_open_store_is_a_no_op() {
        let app = mock_app();
        let store = open_store();
        adopt(app.handle(), &store);

        super::mount(app.handle());

        assert!(
            port::<dyn SourcesRepo>(app.handle()).is_ok(),
            "ports survive"
        );
        assert!(ports_mounted(app.handle()));
    }

    /// B1 (reviewer-4): after an idle expiry the ports are still mounted, so
    /// re-unlocking must re-arm the session, never mount them a second time (that
    /// panics), and the store must work again afterwards.
    #[test]
    fn an_idle_gated_store_unlocks_again_without_mounting_ports_twice() {
        let app = mock_app();
        let store = open_store();
        adopt(app.handle(), &store);
        app.manage(KeySession::new(unlock::DEFAULT_IDLE));

        // The passphrase key goes idle: the gate closes, the mounts stay.
        let session = app.state::<KeySession>();
        session.unlock(DbKey::from_hex(KEY_HEX).expect("test key"));
        session.lock();
        assert!(
            port::<dyn SourcesRepo>(app.handle()).is_err(),
            "an idle key gates every port"
        );
        assert!(ports_mounted(app.handle()), "the ports are still there");

        // The user types the passphrase again: the store is open, so the mount is
        // a no-op...
        mount_with_key(app.handle(), DbKey::from_hex(KEY_HEX).expect("test key"))
            .expect("no second mount");
        // ...and the session is re-armed, so the store works again.
        app.state::<KeySession>()
            .unlock(DbKey::from_hex(KEY_HEX).expect("test key"));
        assert!(port::<dyn SourcesRepo>(app.handle()).is_ok());
        assert_eq!(store_state(app.handle()), StoreStatus::Ready);
    }
}
