//! Tauri command bodies, one module per area (AD-3).
//!
//! Registration lives in [`crate::ipc::command_list`] (`.gitattributes`
//! `merge=union`); append your command's fn path there, never here. Each area
//! owns `src-tauri/src/commands/<area>.rs`.

pub mod agent;
pub mod apply;
pub mod audit_stats;
pub mod backup;
pub mod capabilities;
pub mod daily_search;
pub mod email;
pub mod embeddings;
pub mod export;
pub mod ingest;
pub mod kill_switch;
pub mod offers;
pub mod onboarding;
pub mod preferences;
pub mod profile;
pub mod profile_gaps;
pub mod profile_import;
pub mod session;
pub mod sources;
pub mod status_proposals;
pub mod studio;
pub mod tracker;
pub mod triage;
pub mod unlock;
