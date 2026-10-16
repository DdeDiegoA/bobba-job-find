//! Port registration list (ADR-012 §3, `.gitattributes` `merge=union`).
//!
//! Append **one** `mount_port::<dyn Port>(app, store.clone());` line per port,
//! above the marker comment. Never reorder or remove a line: a duplicate panics
//! at startup (and in the app-start smoke test), which is the point.
//!
//! The store implements its repo traits directly, so a port line is only the
//! type name plus `store.clone()`. A port whose adapter needs a constructor
//! passes the built adapter instead, e.g. `Arc::new(SqliteSystemRepo::new(…))`.

use std::sync::Arc;

use boba_app::ports::agent::AgentPort;
use boba_app::ports::repo::agent::AgentRepo;
use boba_app::ports::repo::profile::ProfileRepo;
use boba_app::ports::repo::sources::SourcesRepo;
use boba_app::ports::repo::system::SystemRepo;
use boba_store::db::Store;
use boba_store::repos::audit::SqliteSystemRepo;
use tauri::{AppHandle, Runtime};

use super::{agent, keychain_service, mount_port};

/// Mount every port that has a store adapter (ADR-012 §3).
pub fn mount_ports<R: Runtime>(app: &AppHandle<R>, store: &Arc<Store>) {
    mount_port::<dyn SourcesRepo>(app, store.clone());
    mount_port::<dyn AgentRepo>(app, store.clone());
    // The system repo also needs the keychain service for the audit anchor.
    mount_port::<dyn SystemRepo>(
        app,
        Arc::new(SqliteSystemRepo::new(store.clone(), keychain_service())),
    );
    mount_port::<dyn ProfileRepo>(app, store.clone());
    // The agent runtime is built by this slice (CH5): it needs the app handle
    // for the event sink and the app-data paths, so it cannot be a plain
    // `store.clone()` line.
    let agent: Arc<dyn AgentPort> = Arc::new(agent::agent_runtime(app, store));
    mount_port::<dyn AgentPort>(app, agent);
    // slices append ONE line above this comment; never reorder/remove
}
