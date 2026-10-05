//! Tauri command surface. One module per area (backlog ownership convention 3).
//!
//! The append-only list of registered command paths lives in [`registry`];
//! `.gitattributes` marks it `merge=union` (AD-3).

pub mod agent;
pub mod email;
pub mod ingest;
pub mod offers;
pub mod profile;
pub mod registry;
pub mod sources;
pub mod studio;
pub mod tracker;
pub mod unlock;
