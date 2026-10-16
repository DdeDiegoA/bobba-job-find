//! Tauri command bodies, one module per area (AD-3).
//!
//! Registration lives in [`crate::ipc::command_list`] (`.gitattributes`
//! `merge=union`); append your command's fn path there, never here. Each area
//! owns `src-tauri/src/commands/<area>.rs`.

pub mod agent;
pub mod email;
pub mod ingest;
pub mod offers;
pub mod preferences;
pub mod profile;
pub mod sources;
pub mod studio;
pub mod tracker;
pub mod unlock;
