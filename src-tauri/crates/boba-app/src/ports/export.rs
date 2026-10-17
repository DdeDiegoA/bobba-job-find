//! `ExportPort` — the unencrypted, upstream-compatible export (S10.1, FR-110,
//! ADR-003). Adapter: `boba-store`, which owns the open database every file is
//! built from.
//!
//! The export is deliberately **not** encrypted: it is the user's way out of
//! the app, so the files are plain JSON, CSV, Markdown, PDF and DOCX. The UI
//! shows the unencrypted warning before it permits the run (AC1).

use std::path::{Path, PathBuf};

/// An export failure. Every variant maps to one i18n key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExportError {
    /// The path the user picked is missing or is not a folder.
    #[error("the destination is not a folder")]
    Destination,
    /// A filesystem or database failure. No message carries a value.
    #[error("internal error")]
    Internal,
}

impl ExportError {
    /// The i18n key the UI shows (AD-3: keys cross IPC, never raw text).
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::Destination => "errors.export.destination",
            Self::Internal => "errors.export.internal",
        }
    }
}

/// The human-readable export (S10.1).
pub trait ExportPort: Send + Sync {
    /// Write the export into `destination` and return every file it created.
    fn export(&self, destination: &Path) -> Result<Vec<PathBuf>, ExportError>;
}
