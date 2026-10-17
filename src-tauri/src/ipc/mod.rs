//! Typed IPC layer: the single tauri-specta [`Builder`] (AD-3).
//!
//! Every command returns `Result<T, AppError>` and every event is a typed
//! struct from [`crate::events`]. The Builder exports `src/lib/ipc/bindings.ts`
//! (via `scripts/gen-bindings.mjs`); the file is never hand-edited.
//!
//! Registration lives in [`command_list`] (`merge=union`): append a command or
//! event there, never here. This module is the only `Builder` site.

pub mod command_list;

use serde::{Deserialize, Serialize};
use specta::Type;
use specta_typescript::Typescript;
use tauri::Manager;
use tauri_specta::{Builder, ErrorHandlingMode};

use crate::events::TaskRegistry;
use crate::ipc::command_list::{boba_commands, boba_events};
use boba_app::error::{AppError, AppResult};

/// Build the one and only specta [`Builder`] for the app.
pub fn builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(boba_commands!())
        .events(boba_events!())
        // Commands return `Result<T, AppError>`; the UI gets a typed envelope.
        .error_handling(ErrorHandlingMode::Result)
}

/// Export the generated TypeScript bindings to `path`.
pub fn export_bindings(path: impl AsRef<std::path::Path>) -> Result<(), specta_typescript::Error> {
    builder().export(Typescript::default(), path)
}

/// Static information about the running app.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct AppInfo {
    /// Product name.
    pub name: String,
    /// Core version.
    pub version: String,
}

/// Return the app name and version (infrastructure command, AD-3).
#[tauri::command]
#[specta::specta]
pub fn app_info() -> AppResult<AppInfo> {
    Ok(AppInfo {
        name: "Boba Job Find".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
    })
}

/// Cancel an in-flight long task by id (AD-3). Returns whether it was found.
#[tauri::command]
#[specta::specta]
pub fn task_cancel(app: tauri::AppHandle, task_id: String) -> AppResult<bool> {
    if task_id.is_empty() {
        return Err(AppError::invalid_input("errors.task.missing_id"));
    }
    Ok(app.state::<TaskRegistry>().cancel(&task_id))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tauri::Emitter;

    /// Write the raw generated bindings to a deterministic temp path.
    /// `scripts/gen-bindings.mjs` runs this test, then formats the output and
    /// writes/checks `src/lib/ipc/bindings.ts` (AD-3).
    #[test]
    fn export_bindings() {
        let out = std::env::temp_dir().join("boba-bindings.generated.ts");
        crate::ipc::export_bindings(&out).expect("export bindings");
        assert!(PathBuf::from(&out).exists(), "bindings not written");
    }

    /// CH10: Tauri 2.12 rejects `.` in an event name (`IllegalEventName`), and
    /// the failure was swallowed — the app-wide event bus was silently dead.
    ///
    /// This walks **every registered event** (via the export, so a `collect_
    /// events!` entry added without regenerating bindings still shows up) and
    /// puts its name through Tauri's own validation. A dotted name fails CI.
    #[test]
    fn every_registered_event_name_is_legal_for_tauri() {
        let out = std::env::temp_dir().join("boba-events.generated.ts");
        crate::ipc::export_bindings(&out).expect("export bindings");
        let bindings = std::fs::read_to_string(&out).expect("read bindings");
        let names = event_names(&bindings);

        // A tiny parser that matched nothing must never pass for free.
        for expected in [
            "task:progressed",
            "task:finished",
            "run_report:progressed",
            "kill_switch:changed",
            "notification:opened",
        ] {
            assert!(
                names.contains(&expected),
                "`{expected}` is not in the exported bindings: {names:?}"
            );
        }

        let app = tauri::test::mock_app();
        for name in &names {
            assert!(
                !name.contains('.'),
                "`{name}`: event names use ':' between segments (`<area>:<verb-ed>`), never '.'"
            );
            assert!(
                app.emit(name, ()).is_ok(),
                "`{name}` is not a legal Tauri event name"
            );
        }
    }

    /// The `makeEvent<Type>("name")` names of the exported bindings, one per
    /// registered event. The `makeEvent` helper's own signature is spread over
    /// lines, so it does not match.
    fn event_names(bindings: &str) -> Vec<&str> {
        bindings
            .lines()
            .filter_map(|line| {
                let (_, rest) = line.split_once("makeEvent<")?;
                let (_, rest) = rest.split_once(">(\"")?;
                let (name, _) = rest.split_once("\")")?;
                Some(name)
            })
            .collect()
    }
}
