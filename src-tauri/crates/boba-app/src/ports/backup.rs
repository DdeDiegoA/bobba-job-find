//! `BackupPort` — encrypted backup, validated restore and Reset everything
//! (S10.2, FR-111, FR-112, ADR-009). Adapter: `boba-store`, which owns the one
//! open database: a restore swaps its pages, and a reset erases it in place.
//!
//! Passphrases cross this port as `&str` and go no further: the adapter zeroizes
//! what it keeps, and no failure variant carries a value (a wrong guess must not
//! come back in an error message).

use std::path::{Path, PathBuf};

use thiserror::Error;

/// A backup, restore or reset failure. Every variant maps to one i18n key.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BackupError {
    /// The user left the passphrase field empty.
    #[error("the passphrase is empty")]
    EmptyPassphrase,
    /// The file is not one of our backups (or not an encrypted database).
    #[error("the file is not a Boba backup")]
    NotABackup,
    /// The passphrase does not open the file.
    #[error("the passphrase does not open this backup")]
    WrongPassphrase,
    /// The file opens but fails its integrity check.
    #[error("the backup fails its integrity check")]
    Corrupt,
    /// The backup is from a schema this build does not support.
    #[error("the backup schema {db} is newer than this app ({app})")]
    TooNew { db: u32, app: u32 },
    /// The reset confirmation text does not match.
    #[error("the confirmation text does not match")]
    WrongConfirmation,
    /// The OS keychain failed.
    #[error("keychain error")]
    Keychain,
    /// Anything else: a filesystem or SQLite failure.
    #[error("internal error")]
    Internal,
}

impl BackupError {
    /// The i18n key the UI shows (AD-3: keys cross IPC, never raw text).
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::EmptyPassphrase => "errors.backup.empty_passphrase",
            Self::NotABackup => "errors.backup.not_a_backup",
            Self::WrongPassphrase => "errors.backup.wrong_passphrase",
            Self::Corrupt => "errors.backup.corrupt",
            Self::TooNew { .. } => "errors.backup.too_new",
            Self::WrongConfirmation => "errors.backup.wrong_confirmation",
            Self::Keychain => "errors.backup.keychain",
            Self::Internal => "errors.backup.internal",
        }
    }
}

/// The recovery-backup requirement (AC1/AC2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupStatus {
    /// When the last backup was written (UTC RFC 3339).
    pub recovery_created_at: Option<String>,
    /// Where it was written.
    pub recovery_path: Option<String>,
    /// Whether that file is still there.
    pub recovery_present: bool,
    /// Whether a recovery backup is required and missing. Every MVP build is
    /// unsigned (ADR-009), so the requirement is on until one exists.
    pub recovery_required: bool,
}

/// What a backup file says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupMeta {
    pub format: String,
    pub app_version: String,
    pub schema_version: u32,
    pub created_at: String,
}

/// What a restore did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreReport {
    /// The backup's own marker.
    pub meta: BackupMeta,
    /// Where the replaced database was snapshotted.
    pub snapshot: PathBuf,
    /// The schema the restored data now sits on (this build's latest).
    pub schema_to: u32,
}

/// What a reset deleted, and where the last files live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetReport {
    /// True when the database file itself is gone.
    pub db_removed: bool,
    /// Every path that was deleted.
    pub removed: Vec<PathBuf>,
    /// Every keychain entry that was deleted.
    pub keychain_removed: Vec<String>,
    /// Paths that still hold data, if any (a locked file, a profile in use).
    pub remaining: Vec<PathBuf>,
    /// The app data directory, where the app would write again.
    pub data_dir: PathBuf,
}

/// Backup, restore and reset (S10.2).
pub trait BackupPort: Send + Sync {
    /// The recovery-backup requirement and where the last backup went.
    fn status(&self) -> Result<BackupStatus, BackupError>;

    /// Write `dest` as a passphrase-encrypted backup (AC1).
    fn create(&self, dest: &Path, passphrase: &str, now: &str) -> Result<BackupMeta, BackupError>;

    /// Validate `src` and replace the current data with it (AC3/AC4).
    fn restore(&self, src: &Path, passphrase: &str) -> Result<RestoreReport, BackupError>;

    /// Delete the database content, snapshots, `profiles` and the keychain
    /// entries for `service` after the user typed `confirmation` (AC5).
    fn reset(
        &self,
        service: &str,
        profiles: &[PathBuf],
        confirmation: &str,
    ) -> Result<ResetReport, BackupError>;
}
