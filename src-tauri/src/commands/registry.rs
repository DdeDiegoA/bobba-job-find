//! Append-only Tauri command registry (AD-3).
//!
//! **One command path per line.** `.gitattributes` marks this file
//! `merge=union`, so parallel slices append without a merge conflict
//! (backlog ownership convention 3). Append a line; never reorder or remove.

/// Registered command paths, one per line.
pub const COMMANDS: &[&str] = &[
    "commands::agent",
    "commands::email",
    "commands::ingest",
    "commands::offers",
    "commands::profile",
    "commands::sources",
    "commands::studio",
    "commands::tracker",
    "commands::unlock",
];
