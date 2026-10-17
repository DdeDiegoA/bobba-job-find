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
            crate::commands::preferences::preferences_snapshot,
            crate::commands::preferences::preferences_save,
            crate::commands::email::email_accounts,
            crate::commands::email::email_connect_outlook,
            crate::commands::email::email_connect_gmail,
            crate::commands::email::email_connect_imap_smtp,
            crate::commands::email::email_set_mode,
            crate::commands::email::email_disconnect,
            crate::commands::tracker::tracker_board,
            crate::commands::tracker::tracker_application_detail,
            crate::commands::tracker::tracker_transition_status,
            crate::commands::ingest::ingest_manual_url,
            crate::commands::ingest::ingest_manual_text,
            crate::commands::offers::offers_inbox,
            crate::commands::unlock::unlock_status,
            crate::commands::unlock::unlock_retry_keychain,
            crate::commands::unlock::unlock_with_passphrase,
            crate::commands::studio::studio_view,
            crate::commands::studio::studio_edit,
            crate::commands::studio::studio_render,
            crate::commands::studio::studio_export_docx,
            crate::commands::studio::studio_generate,
            crate::commands::studio::studio_regenerate,
            crate::scheduler::jobs_run_now,
            crate::commands::apply::apply_view,
            crate::commands::apply::apply_mark_applied,
            crate::commands::apply::apply_export_pdfs,
            crate::commands::agent::agent_engine_snapshot,
            crate::commands::agent::agent_engine_choose,
            crate::commands::agent::agent_test_call,
            crate::commands::agent::agent_models,
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
