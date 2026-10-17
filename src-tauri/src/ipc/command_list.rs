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
            crate::commands::offers::offer_detail,
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
            crate::commands::agent::agent_budget_snapshot,
            crate::commands::agent::agent_budget_save,
            crate::commands::agent::agent_test_call,
            crate::commands::agent::agent_models,
            crate::commands::embeddings::embeddings_status,
            crate::commands::embeddings::embeddings_opt_in,
            crate::commands::embeddings::embeddings_opt_out,
            crate::commands::apply::apply_email_review,
            crate::commands::apply::apply_email_send,
            crate::commands::apply::apply_answers,
            crate::commands::apply::apply_answer_approve,
            crate::commands::apply::apply_ats_opted_in,
            crate::commands::apply::apply_ats_opt_in,
            crate::commands::apply::apply_ats_start,
            crate::commands::apply::apply_ats_confirm,
            crate::commands::apply::apply_session_start,
            crate::commands::apply::apply_session_fill,
            crate::commands::apply::apply_session_confirm,
            crate::commands::session::session_snapshot,
            crate::commands::session::session_opt_in,
            crate::commands::session::session_opt_out,
            crate::commands::session::session_sign_out,
            crate::commands::session::session_resume,
            crate::commands::profile_import::profile_import_start,
            crate::commands::profile_import::profile_import_proposals,
            crate::commands::profile_import::profile_import_confirm,
            crate::commands::profile_import::profile_import_discard,
            crate::commands::profile_gaps::profile_gaps_questions,
            crate::commands::profile_gaps::profile_gaps_start,
            crate::commands::profile_gaps::profile_gaps_answer,
            crate::commands::profile_gaps::profile_gaps_skip,
            crate::commands::profile_gaps::profile_gaps_translate,
            crate::commands::profile_gaps::profile_gaps_confirm,
            crate::commands::profile_gaps::profile_gaps_discard,
            crate::commands::apply::apply_rules_snapshot,
            crate::commands::apply::apply_rule_save,
            crate::commands::apply::apply_rule_delete,
            crate::commands::apply::apply_rules_set_enabled,
            crate::commands::apply::apply_rules_set_global_cap,
            crate::commands::apply::apply_rules_reset,
            crate::commands::apply::apply_rules_explain,
            crate::commands::sources::sources_catalogue,
            crate::commands::sources::sources_test_rss,
            crate::commands::sources::sources_add_rss,
            crate::commands::sources::sources_remove_rss,

            crate::commands::apply::apply_outbox_view,
            crate::commands::apply::apply_outbox_set_hold,
            crate::commands::apply::apply_outbox_cancel,
            crate::commands::apply::apply_outbox_edit,
            crate::commands::apply::apply_outbox_activate_rule,
            crate::commands::apply::apply_outbox_set_dry_run,
            crate::commands::apply::apply_outbox_release_due,
            crate::commands::daily_search::run_report_latest,
            crate::commands::daily_search::run_report_retry,
            crate::commands::triage::inbox_save,
            crate::commands::triage::inbox_dismiss,
            crate::commands::triage::inbox_undo_dismiss,
            crate::commands::triage::inbox_override_veto,
            crate::commands::triage::inbox_decide_suggestion,
            crate::commands::kill_switch::kill_switch_status,
            crate::commands::kill_switch::kill_switch_set,
            crate::commands::backup::backup_status,
            crate::commands::backup::backup_create,
            crate::commands::backup::backup_restore,
            crate::commands::backup::reset_everything,
            crate::commands::export::export_data,
            crate::commands::onboarding::onboarding_status,
            crate::commands::onboarding::onboarding_complete,
            crate::commands::capabilities::capabilities_snapshot,
            crate::notifications::commands::notifications_snapshot,
            crate::notifications::commands::notifications_save,
            crate::autostart::autostart_status,
            crate::autostart::autostart_set,
            crate::commands::status_proposals::status_proposals_disclosure,
            crate::commands::status_proposals::status_proposals_save_settings,
            crate::commands::status_proposals::status_proposals_classify,
            crate::commands::status_proposals::status_proposals_pending,
            crate::commands::status_proposals::status_proposals_approve,
            crate::commands::status_proposals::status_proposals_approve_many,
            crate::commands::status_proposals::status_proposals_dismiss,
            crate::commands::status_proposals::status_proposals_undo,
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
            crate::commands::daily_search::RunReportProgressed,
            crate::commands::kill_switch::KillSwitchChanged,
            crate::notifications::NotificationOpened,
            // slices append ONE line above this comment; never reorder/remove
        ]
    };
}

pub(crate) use boba_commands;
pub(crate) use boba_events;
