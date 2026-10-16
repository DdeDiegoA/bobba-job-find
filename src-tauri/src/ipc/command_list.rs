//! The single command/event registration list (AD-3, build convention 3).
//!
//! Append one fully qualified fn or type path per line, ending in a comma,
//! inside [`boba_commands!`] or [`boba_events!`]. `.gitattributes` marks this
//! file `merge=union`, so parallel slices append without conflicts. A duplicate
//! line or a missing comma fails compilation. Do **not** edit `ipc/mod.rs` to
//! register a command or event.

/// All Tauri commands, passed to `tauri_specta::Builder::commands`.
#[rustfmt::skip]
macro_rules! boba_commands {
    () => {
        tauri_specta::collect_commands![
            crate::ipc::app_info,
            crate::ipc::task_cancel,
            crate::commands::profile::profile_snapshot,
            crate::commands::profile::profile_save_fact,
            crate::commands::profile::profile_history,
            crate::commands::profile::profile_restore,
            // slices append ONE line above this comment; never reorder/remove
        ]
    };
}

/// All typed events, passed to `tauri_specta::Builder::events`.
#[rustfmt::skip]
macro_rules! boba_events {
    () => {
        tauri_specta::collect_events![
            crate::events::TaskProgressed,
            crate::events::TaskFinished,
            // slices append ONE line above this comment; never reorder/remove
        ]
    };
}

pub(crate) use boba_commands;
pub(crate) use boba_events;
