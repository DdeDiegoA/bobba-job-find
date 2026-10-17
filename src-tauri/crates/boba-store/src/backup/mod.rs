//! Encrypted backup and validated restore (S10.2, FR-111, ADR-009 §7).
//!
//! A backup is **one** SQLCipher file, keyed by a passphrase the user chooses —
//! not by this machine's database key — so it opens on any OS. Creation copies
//! the live store with `VACUUM INTO`, re-keys the copy to that passphrase and
//! records `boba_backup_meta` inside it. Restore opens the file **read-only**
//! first, checks the marker table and `PRAGMA integrity_check`, snapshots the
//! current database and only then replaces its contents, inside one transaction,
//! replaying the forward migrations so an older backup lands on today's schema.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::db::Store;

/// The only backup format this build writes or opens.
pub const FORMAT: &str = "boba-backup-v1";

/// The marker table every backup carries (ADR-009 §7).
pub const META_TABLE: &str = "boba_backup_meta";

/// Failure of a backup, restore or reset.
#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("the passphrase is empty")]
    EmptyPassphrase,
    #[error("the file is not a Boba backup")]
    NotABackup,
    #[error("the passphrase does not open this backup")]
    WrongPassphrase,
    #[error("the backup fails its integrity check")]
    Corrupt,
    #[error("the backup is from schema {db}, newer than this app supports ({app})")]
    TooNew { db: u32, app: u32 },
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("sqlite error: {0}")]
    Sqlite(String),
}

impl From<rusqlite::Error> for BackupError {
    fn from(error: rusqlite::Error) -> Self {
        BackupError::Sqlite(error.to_string())
    }
}

impl From<crate::db::StoreError> for BackupError {
    fn from(error: crate::db::StoreError) -> Self {
        BackupError::Sqlite(error.to_string())
    }
}

/// Where the last backup was written and when (AC2: onboarding and the run gate
/// read this to know a recovery backup exists).
pub const RECOVERY_CREATED_SETTING: &str = "backup.recovery_created_at";
/// The last backup's path; the file itself is what proves it still exists.
pub const RECOVERY_PATH_SETTING: &str = "backup.recovery_path";

/// What the recovery-backup requirement looks like right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupStatus {
    /// When the last backup was written (UTC RFC 3339).
    pub recovery_created_at: Option<String>,
    /// Where it was written.
    pub recovery_path: Option<String>,
    /// Whether that file is still there.
    pub recovery_present: bool,
    /// Whether a recovery backup is required and missing. Every MVP build is
    /// unsigned (ADR-009), so the requirement is always on; a passphrase-
    /// protected backup is what clears it.
    pub recovery_required: bool,
}

/// Read the recovery-backup state from the store.
pub fn status(store: &Store) -> Result<BackupStatus, BackupError> {
    let (created_at, path) = store.read(|conn| {
        let read = |key: &str| {
            crate::repos::settings::get(conn, key)
                .map_err(|error| crate::db::StoreError::Sqlite(error.to_string()))
        };
        Ok((
            read(RECOVERY_CREATED_SETTING)?,
            read(RECOVERY_PATH_SETTING)?,
        ))
    })?;
    let present = path
        .as_deref()
        .is_some_and(|path| Path::new(path).is_file());
    Ok(BackupStatus {
        recovery_present: present,
        recovery_required: !present,
        recovery_created_at: created_at,
        recovery_path: path,
    })
}

/// Remember this backup: path and time live in `setting`, so Reset everything
/// (which wipes the store) clears them with it.
fn record_recovery(store: &Store, dest: &Path, now: &str) -> Result<(), BackupError> {
    let path = dest.to_string_lossy().into_owned();
    let when = now.to_string();
    store.execute(move |conn| {
        let write = |key: &str, value: &str| {
            crate::repos::settings::put(conn, key, value, &when)
                .map_err(|error| crate::db::StoreError::Sqlite(error.to_string()))
        };
        write(RECOVERY_CREATED_SETTING, &when)?;
        write(RECOVERY_PATH_SETTING, &path)?;
        Ok(())
    })?;
    Ok(())
}

/// What a backup file says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupMeta {
    pub format: String,
    pub app_version: String,
    pub schema_version: u32,
    pub created_at: String,
}

/// Write `dest` as a passphrase-encrypted backup of `store`.
pub fn create(
    store: &Store,
    dest: &Path,
    passphrase: &str,
    now: &str,
) -> Result<BackupMeta, BackupError> {
    if passphrase.is_empty() {
        return Err(BackupError::EmptyPassphrase);
    }
    let temp = temp_path(dest);

    // One consistent copy, taken on the writer connection so no write can land
    // in the middle of it. VACUUM INTO keeps this machine's key; the next step
    // is what makes the file portable.
    let temp_for_copy = temp.clone();
    store.execute(move |conn| {
        let _ = std::fs::remove_file(&temp_for_copy);
        conn.execute_batch(&format!("VACUUM INTO {}", quote_sql(&temp_for_copy)))
            .map_err(|error| crate::db::StoreError::Sqlite(error.to_string()))
    })?;

    let result = fill_and_rekey(store, &temp, passphrase, now);
    match result {
        Ok(meta) => {
            if dest.exists() {
                std::fs::remove_file(dest)?;
            }
            std::fs::rename(&temp, dest)?;
            set_owner_only(dest, 0o600)?;
            record_recovery(store, dest, now)?;
            Ok(meta)
        }
        Err(error) => {
            let _ = std::fs::remove_file(&temp);
            Err(error)
        }
    }
}

/// The temp file a backup is built in, next to its destination so the final
/// `rename` is atomic within one filesystem.
fn temp_path(dest: &Path) -> PathBuf {
    let name = dest
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "backup".to_string());
    dest.with_file_name(format!(".{name}.tmp"))
}

/// Add the marker table, re-key the copy to `passphrase`, and prove the result
/// opens with it.
fn fill_and_rekey(
    store: &Store,
    temp: &Path,
    passphrase: &str,
    now: &str,
) -> Result<BackupMeta, BackupError> {
    let conn = Connection::open(temp)?;
    conn.execute_batch(&format!(
        "PRAGMA key = \"x'{}'\";",
        store.db_key().hex().as_str()
    ))
    .map_err(sql)?;
    let schema_version: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .map_err(sql)?
        .max(0) as u32;
    let meta = BackupMeta {
        format: FORMAT.to_string(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        schema_version,
        created_at: now.to_string(),
    };
    conn.execute_batch(&format!(
        "CREATE TABLE {META_TABLE} (
             format TEXT NOT NULL,
             app_version TEXT NOT NULL,
             schema_version INTEGER NOT NULL,
             created_at TEXT NOT NULL
         ) STRICT;"
    ))
    .map_err(sql)?;
    conn.execute(
        &format!(
            "INSERT INTO {META_TABLE} (format, app_version, schema_version, created_at)
             VALUES (?1, ?2, ?3, ?4)"
        ),
        rusqlite::params![
            meta.format,
            meta.app_version,
            meta.schema_version,
            meta.created_at
        ],
    )
    .map_err(sql)?;
    // Re-key to the user's passphrase: SQLCipher writes a fresh random salt in
    // the file header, so the same passphrase opens it on every OS.
    conn.execute_batch(&format!("PRAGMA rekey = {};", quote_sql_value(passphrase)))
        .map_err(sql)?;
    drop(conn);

    // Prove what was written: read-only open with the passphrase, marker and
    // integrity check, exactly what a restore will do.
    let check = open_read_only(temp, passphrase)?;
    check_meta(&check)?;
    drop(check);
    Ok(meta)
}

fn sql(error: rusqlite::Error) -> BackupError {
    BackupError::Sqlite(error.to_string())
}

/// Open `path` read-only and unlock it with `passphrase`.
fn open_read_only(path: &Path, passphrase: &str) -> Result<Connection, BackupError> {
    let conn = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.execute_batch(&format!("PRAGMA key = {};", quote_sql_value(passphrase)))
        .map_err(sql)?;
    match conn.query_row("SELECT count(*) FROM sqlite_master", [], |row| {
        row.get::<_, i64>(0)
    }) {
        Ok(_) => Ok(conn),
        Err(rusqlite::Error::SqliteFailure(fault, _))
            if fault.code == rusqlite::ErrorCode::NotADatabase =>
        {
            Err(BackupError::WrongPassphrase)
        }
        Err(rusqlite::Error::SqliteFailure(fault, _))
            if fault.code == rusqlite::ErrorCode::DatabaseCorrupt =>
        {
            Err(BackupError::Corrupt)
        }
        Err(error) => Err(sql(error)),
    }
}

/// Check the marker table and `PRAGMA integrity_check` on an open backup.
fn check_meta(conn: &Connection) -> Result<BackupMeta, BackupError> {
    let meta: BackupMeta = conn
        .query_row(
            &format!("SELECT format, app_version, schema_version, created_at FROM {META_TABLE}"),
            [],
            |row| {
                Ok(BackupMeta {
                    format: row.get(0)?,
                    app_version: row.get(1)?,
                    schema_version: row.get(2)?,
                    created_at: row.get(3)?,
                })
            },
        )
        .map_err(|error| match error {
            rusqlite::Error::SqliteFailure(fault, _)
                if fault.code == rusqlite::ErrorCode::DatabaseCorrupt
                    || fault.code == rusqlite::ErrorCode::NotADatabase =>
            {
                BackupError::Corrupt
            }
            other => {
                if matches!(other, rusqlite::Error::QueryReturnedNoRows) {
                    BackupError::NotABackup
                } else {
                    sql(other)
                }
            }
        })?;
    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|error| match error {
            rusqlite::Error::SqliteFailure(fault, _)
                if fault.code == rusqlite::ErrorCode::DatabaseCorrupt
                    || fault.code == rusqlite::ErrorCode::NotADatabase =>
            {
                BackupError::Corrupt
            }
            other => sql(other),
        })?;
    if integrity != "ok" {
        return Err(BackupError::Corrupt);
    }
    Ok(meta)
}

/// Open `path` **read-only**, check that it is one of our backups and that its
/// schema is not newer than this build (AC3/AC4). Touches nothing else.
pub fn validate(path: &Path, passphrase: &str) -> Result<BackupMeta, BackupError> {
    if passphrase.is_empty() {
        return Err(BackupError::EmptyPassphrase);
    }
    let header = read_header(path)?;
    if header == *b"SQLite format 3\0" {
        // A plain SQLite file: no passphrase can open it, and it carries none
        // of Boba's encryption.
        return Err(BackupError::NotABackup);
    }
    let conn = open_read_only(path, passphrase)?;
    let meta = check_meta(&conn)?;
    drop(conn);
    if meta.format != FORMAT {
        return Err(BackupError::NotABackup);
    }
    let app = latest_schema();
    if meta.schema_version > app {
        return Err(BackupError::TooNew {
            db: meta.schema_version,
            app,
        });
    }
    Ok(meta)
}

/// The first 16 bytes of a file, zero-padded when the file is shorter.
fn read_header(path: &Path) -> Result<[u8; 16], BackupError> {
    use std::io::Read as _;

    let mut header = [0u8; 16];
    match std::fs::File::open(path) {
        Ok(mut file) => {
            let _ = file.read(&mut header)?;
            Ok(header)
        }
        Err(error) => Err(BackupError::Io(error)),
    }
}

/// What a restore did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreReport {
    /// The backup's own marker.
    pub meta: BackupMeta,
    /// Where the replaced database was snapshotted (pre-restore, same key).
    pub snapshot: PathBuf,
    /// The schema the restored data now sits on (this build's latest).
    pub snapshot_to: u32,
}

/// Replace the live store's data with `src`, after validating it and
/// snapshotting what is about to be replaced (AC3/AC4).
pub fn restore(store: &Store, src: &Path, passphrase: &str) -> Result<RestoreReport, BackupError> {
    let meta = validate(src, passphrase)?;
    std::fs::create_dir_all(store.snapshots_dir())?;
    set_owner_only(store.snapshots_dir(), 0o700)?;
    let snapshot =
        store
            .snapshots_dir()
            .join(format!("restore-{}-{}.db", meta.schema_version, unix_now()));
    let source = src.to_path_buf();
    let snapshot_for_write = snapshot.clone();
    let passphrase = passphrase.to_string();

    store.execute(move |conn| {
        // The point of no return: the current database can always be brought
        // back from this file (same key, same directory as migration snapshots).
        conn.execute_batch(&format!("VACUUM INTO {}", quote_sql(&snapshot_for_write)))
            .map_err(|error| crate::db::StoreError::Sqlite(error.to_string()))?;
        replace_contents(conn, &source, &passphrase)
            .map_err(|error| crate::db::StoreError::Sqlite(format!("restore: {error}")))
    })?;
    let schema_to = latest_schema();
    Ok(RestoreReport {
        meta,
        snapshot,
        snapshot_to: schema_to,
    })
}

/// Swap the contents of the open database for the backup's, in one transaction.
fn replace_contents(
    conn: &mut Connection,
    src: &Path,
    passphrase: &str,
) -> Result<(), BackupError> {
    // ATTACH cannot run inside a transaction, and the FK pragma is a no-op
    // inside one, so both happen first.
    conn.prepare("ATTACH DATABASE ?1 AS bak KEY ?2")
        .map_err(sql)?
        .execute(rusqlite::params![src.to_string_lossy(), passphrase])
        .map_err(sql)?;
    conn.execute_batch("PRAGMA foreign_keys=OFF;")
        .map_err(sql)?;

    let result = (|| -> Result<(), BackupError> {
        conn.execute_batch("BEGIN IMMEDIATE;").map_err(sql)?;
        drop_all_objects(conn)?;
        replay_migrations(conn)?;
        copy_tables(conn)?;
        conn.execute_batch(&format!("PRAGMA user_version = {};", latest_schema()))
            .map_err(sql)?;
        conn.execute_batch("COMMIT;").map_err(sql)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = conn.execute_batch("ROLLBACK;");
    }
    let _ = conn.execute_batch("PRAGMA foreign_keys=ON; DETACH DATABASE bak;");
    result
}

/// Drop every object the live database owns, so the migrations can rebuild the
/// schema from zero. `sqlite_%` internals stay. Shared with `reset`.
pub(crate) fn drop_all_objects(conn: &Connection) -> Result<(), BackupError> {
    let mut stmt = conn
        .prepare(
            "SELECT type, name FROM main.sqlite_master
             WHERE type IN ('trigger', 'view', 'index', 'table') AND name NOT LIKE 'sqlite_%'
             ORDER BY CASE type WHEN 'trigger' THEN 0 WHEN 'view' THEN 1 WHEN 'index' THEN 2 ELSE 3 END",
        )
        .map_err(sql)?;
    let objects: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(sql)?
        .collect::<Result<_, _>>()
        .map_err(sql)?;
    drop(stmt);
    for (kind, name) in objects {
        conn.execute_batch(&format!("DROP {kind} IF EXISTS {}", quote_ident(&name)))
            .map_err(sql)?;
    }
    Ok(())
}

/// Rebuild the schema exactly as the shipped migrations do. Shared with `reset`.
pub(crate) fn replay_migrations(conn: &Connection) -> Result<(), BackupError> {
    for (_, migration) in crate::migrate::embedded_files() {
        conn.execute_batch(migration).map_err(sql)?;
    }
    Ok(())
}

/// Copy every table the backup and this schema share, column by column: an
/// older backup has fewer columns, and the migrations above already added the
/// new ones.
fn copy_tables(conn: &Connection) -> Result<(), BackupError> {
    let tables: Vec<String> = {
        let mut stmt = conn
            .prepare(
                "SELECT name FROM bak.sqlite_master
                 WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name <> ?1
                 ORDER BY name",
            )
            .map_err(sql)?;
        stmt.query_map(rusqlite::params![META_TABLE], |row| row.get(0))
            .map_err(sql)?
            .collect::<Result<_, _>>()
            .map_err(sql)?
    };
    for table in tables {
        if !table_exists(conn, "main", &table)? || !table_exists(conn, "bak", &table)? {
            continue;
        }
        let backup_columns = columns_of(conn, "bak", &table)?;
        let live_columns = columns_of(conn, "main", &table)?;
        let columns: Vec<String> = backup_columns
            .into_iter()
            .filter(|column| live_columns.contains(column))
            .collect();
        if columns.is_empty() {
            continue;
        }
        let list = columns
            .iter()
            .map(|column| quote_ident(column))
            .collect::<Vec<_>>()
            .join(", ");
        // `OR REPLACE`: a migration may seed a row (0007 seeds the singleton
        // profile); the backup's own row is the one being restored.
        conn.execute_batch(&format!(
            "INSERT OR REPLACE INTO main.{} ({list}) SELECT {list} FROM bak.{}",
            quote_ident(&table),
            quote_ident(&table)
        ))
        .map_err(sql)?;
    }
    Ok(())
}

fn table_exists(conn: &Connection, schema: &str, table: &str) -> Result<bool, BackupError> {
    let count: i64 = conn
        .query_row(
            &format!(
                "SELECT count(*) FROM {}.sqlite_master WHERE type = 'table' AND name = ?1",
                quote_ident(schema)
            ),
            rusqlite::params![table],
            |row| row.get(0),
        )
        .map_err(sql)?;
    Ok(count > 0)
}

fn columns_of(conn: &Connection, schema: &str, table: &str) -> Result<Vec<String>, BackupError> {
    let mut stmt = conn
        .prepare("SELECT name FROM pragma_table_info(?1, ?2) ORDER BY cid")
        .map_err(sql)?;
    let columns = stmt
        .query_map(rusqlite::params![table, schema], |row| row.get(0))
        .map_err(sql)?
        .collect::<Result<_, _>>()
        .map_err(sql)?;
    Ok(columns)
}

/// A quoted identifier (table, column), `"` escaped.
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// The latest schema this build knows (`user_version` after every migration).
pub fn latest_schema() -> u32 {
    crate::migrate::embedded_files().len() as u32
}

/// A SQL string literal.
fn quote_sql_value(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// A SQL string literal for a file path.
fn quote_sql(path: &Path) -> String {
    quote_sql_value(&path.to_string_lossy())
}

#[cfg(unix)]
fn set_owner_only(path: &Path, mode: u32) -> Result<(), BackupError> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    Ok(())
}

/// Windows: the app data directory already has a user-only DACL.
#[cfg(not(unix))]
fn set_owner_only(_path: &Path, _mode: u32) -> Result<(), BackupError> {
    Ok(())
}

/// The [`BackupPort`](boba_app::ports::backup::BackupPort) adapter (ADR-012):
/// commands reach the open store only through this port.
impl boba_app::ports::backup::BackupPort for Store {
    fn status(&self) -> Result<boba_app::ports::backup::BackupStatus, AppBackupError> {
        let state = status(self).map_err(port_error)?;
        Ok(boba_app::ports::backup::BackupStatus {
            recovery_created_at: state.recovery_created_at,
            recovery_path: state.recovery_path,
            recovery_present: state.recovery_present,
            recovery_required: state.recovery_required,
        })
    }

    fn create(
        &self,
        dest: &Path,
        passphrase: &str,
        now: &str,
    ) -> Result<AppBackupMeta, AppBackupError> {
        let meta = create(self, dest, passphrase, now).map_err(port_error)?;
        Ok(AppBackupMeta {
            format: meta.format,
            app_version: meta.app_version,
            schema_version: meta.schema_version,
            created_at: meta.created_at,
        })
    }

    fn restore(&self, src: &Path, passphrase: &str) -> Result<AppRestoreReport, AppBackupError> {
        let report = restore(self, src, passphrase).map_err(port_error)?;
        Ok(AppRestoreReport {
            meta: AppBackupMeta {
                format: report.meta.format,
                app_version: report.meta.app_version,
                schema_version: report.meta.schema_version,
                created_at: report.meta.created_at,
            },
            snapshot: report.snapshot,
            schema_to: report.snapshot_to,
        })
    }

    fn reset(
        &self,
        service: &str,
        profiles: &[PathBuf],
        confirmation: &str,
    ) -> Result<AppResetReport, AppBackupError> {
        let report =
            crate::reset::reset(self, service, profiles, confirmation).map_err(reset_port_error)?;
        Ok(AppResetReport {
            db_removed: report.db_removed,
            removed: report.removed,
            keychain_removed: report.keychain_removed,
            remaining: report.remaining,
            data_dir: report.data_dir,
        })
    }
}

use boba_app::ports::backup::{
    BackupError as AppBackupError, BackupMeta as AppBackupMeta, ResetReport as AppResetReport,
    RestoreReport as AppRestoreReport,
};

/// This module's failures as the port's (no variant carries a value).
fn port_error(error: BackupError) -> AppBackupError {
    match error {
        BackupError::EmptyPassphrase => AppBackupError::EmptyPassphrase,
        BackupError::NotABackup => AppBackupError::NotABackup,
        BackupError::WrongPassphrase => AppBackupError::WrongPassphrase,
        BackupError::Corrupt => AppBackupError::Corrupt,
        BackupError::TooNew { db, app } => AppBackupError::TooNew { db, app },
        BackupError::Io(_) | BackupError::Sqlite(_) => AppBackupError::Internal,
    }
}

fn reset_port_error(error: crate::reset::ResetError) -> AppBackupError {
    match error {
        crate::reset::ResetError::WrongConfirmation => AppBackupError::WrongConfirmation,
        crate::reset::ResetError::Keychain(_) => AppBackupError::Keychain,
        crate::reset::ResetError::Io(_) | crate::reset::ResetError::Sqlite(_) => {
            AppBackupError::Internal
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::db::StoreConfig;
    use crate::keys::DbKey;

    fn temp() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp dir")
    }

    fn store(dir: &Path) -> Store {
        let config = StoreConfig::new(dir.join("boba.db"), dir.join("snapshots"));
        let store = Store::open_with_key(config, DbKey::generate().expect("key")).expect("store");
        store
            .execute(|conn| {
                conn.execute(
                    "INSERT INTO setting (key, value, updated_at) VALUES ('test.row', '42', '2026-10-17T00:00:00Z')",
                    [],
                )
                .map_err(Into::into)
            })
            .expect("seed");
        store
    }

    fn opens_with_key(path: &Path, key: &DbKey) -> bool {
        let Ok(conn) = Connection::open(path) else {
            return false;
        };
        conn.execute_batch(&format!("PRAGMA key = \"x'{}'\";", key.hex().as_str()))
            .expect("key");
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |row| {
            row.get::<_, i64>(0)
        })
        .is_ok()
    }

    fn opens_with_passphrase(path: &Path, passphrase: &str) -> Option<Connection> {
        let conn = Connection::open(path).expect("open");
        conn.execute_batch(&format!(
            "PRAGMA key = '{}';",
            passphrase.replace('\'', "''")
        ))
        .expect("key");
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |row| {
            row.get::<_, i64>(0)
        })
        .ok()?;
        Some(conn)
    }

    /// AC1: one re-keyed SQLCipher file that only its passphrase opens, carrying
    /// the marker table.
    #[test]
    fn a_backup_is_one_file_encrypted_with_its_own_passphrase() {
        let dir = temp();
        let store = store(dir.path());
        let key = DbKey::from_hex(store.db_key().hex().as_str()).expect("key");
        let dest = dir.path().join("recovery.boba");

        let meta = create(
            &store,
            &dest,
            "correct horse battery",
            "2026-10-17T09:00:00Z",
        )
        .expect("backup");

        assert!(dest.is_file(), "the backup is one file");
        assert_eq!(meta.format, FORMAT);
        assert_eq!(meta.created_at, "2026-10-17T09:00:00Z");
        assert_eq!(
            meta.schema_version,
            crate::migrate::embedded_files().len() as u32
        );

        // The machine's own key never opens it: the backup is its passphrase's.
        assert!(!opens_with_key(&dest, &key));

        let conn = opens_with_passphrase(&dest, "correct horse battery").expect("passphrase opens");
        let stored: String = conn
            .query_row(
                "SELECT value FROM setting WHERE key = 'test.row'",
                [],
                |row| row.get(0),
            )
            .expect("the backup carries the data");
        assert_eq!(stored, "42");
        let row: (String, u32, String) = conn
            .query_row(
                "SELECT format, schema_version, created_at FROM boba_backup_meta",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("the marker table");
        assert_eq!(
            row,
            (FORMAT.to_string(), meta.schema_version, meta.created_at)
        );
        assert_eq!(
            conn.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
                .expect("integrity"),
            "ok"
        );

        // No stray temp file is left next to it.
        let names: Vec<PathBuf> = std::fs::read_dir(dir.path())
            .expect("dir")
            .map(|entry| entry.expect("entry").path())
            .filter(|path| {
                path.file_name().is_some_and(|name| {
                    name != "boba.db"
                        && name != "boba.db-wal"
                        && name != "boba.db-shm"
                        && name != "snapshots"
                        && name != "recovery.boba"
                })
            })
            .collect();
        assert!(names.is_empty(), "left behind: {names:?}");
    }

    /// AC3/AC4: a backup is opened read-only first; every refusal is typed.
    #[test]
    fn validate_refuses_what_it_cannot_open() {
        let dir = temp();
        let store = store(dir.path());
        let dest = dir.path().join("recovery.boba");
        let meta = create(
            &store,
            &dest,
            "correct horse battery",
            "2026-10-17T09:00:00Z",
        )
        .expect("backup");

        let checked = validate(&dest, "correct horse battery").expect("its passphrase opens it");
        assert_eq!(checked, meta);

        assert!(matches!(
            validate(&dest, ""),
            Err(BackupError::EmptyPassphrase)
        ));
        assert!(matches!(
            validate(&dest, "tr0ub4dor&3"),
            Err(BackupError::WrongPassphrase)
        ));

        // Not a backup at all: a plain (unencrypted) SQLite file.
        let plain = dir.path().join("plain.db");
        Connection::open(&plain)
            .expect("open")
            .execute_batch("CREATE TABLE t (x INTEGER);")
            .expect("create");
        assert!(matches!(
            validate(&plain, "correct horse battery"),
            Err(BackupError::NotABackup)
        ));

        // A damaged file is refused, never half-opened.
        let damaged = dir.path().join("damaged.boba");
        let bytes = std::fs::read(&dest).expect("read");
        std::fs::write(&damaged, &bytes[..bytes.len() / 2]).expect("write");
        let got = validate(&damaged, "correct horse battery");
        assert!(
            matches!(
                got,
                Err(BackupError::WrongPassphrase) | Err(BackupError::Corrupt)
            ),
            "got {got:?}"
        );
    }

    /// A passphrase is user text: a quote must round-trip through the
    /// SQLCipher key setup, and the file is owner-only like the database.
    #[test]
    fn a_passphrase_with_a_quote_round_trips() {
        let dir = temp();
        let store = store(dir.path());
        let dest = dir.path().join("recovery.boba");
        let passphrase = "it's a \"secret\" with 'quotes'";

        create(&store, &dest, passphrase, "2026-10-17T09:00:00Z").expect("backup");
        validate(&dest, passphrase).expect("its passphrase opens it");
        assert!(matches!(
            validate(&dest, "it is a secret"),
            Err(BackupError::WrongPassphrase)
        ));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&dest)
                .expect("metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "the backup is owner-only");
        }
    }

    /// AC1/AC2: the recovery backup is tracked by path, so a deleted file puts
    /// the requirement back. Every MVP build is unsigned: it is always required.
    #[test]
    fn status_tracks_the_recovery_backup_file() {
        let dir = temp();
        let store = store(dir.path());
        let dest = dir.path().join("recovery.boba");

        let before = status(&store).expect("status");
        assert!(!before.recovery_present);
        assert!(
            before.recovery_required,
            "an unsigned build always requires one"
        );

        create(
            &store,
            &dest,
            "correct horse battery",
            "2026-10-17T09:00:00Z",
        )
        .expect("backup");
        let after = status(&store).expect("status");
        assert!(after.recovery_present);
        assert!(!after.recovery_required);
        assert_eq!(
            after.recovery_created_at.as_deref(),
            Some("2026-10-17T09:00:00Z")
        );
        assert_eq!(
            after.recovery_path.as_deref(),
            Some(dest.to_string_lossy().as_ref())
        );

        std::fs::remove_file(&dest).expect("the user deletes the file");
        let lost = status(&store).expect("status");
        assert!(!lost.recovery_present, "a missing file is no backup");
        assert!(lost.recovery_required);
    }

    #[test]
    fn restore_snapshots_the_current_data_then_replaces_it() {
        let dir = temp();
        let store = store(dir.path());
        let dest = dir.path().join("recovery.boba");
        create(
            &store,
            &dest,
            "correct horse battery",
            "2026-10-17T09:00:00Z",
        )
        .expect("backup");

        let setting = |store: &Store, key: &str| -> Option<String> {
            let key = key.to_string();
            store
                .read(move |conn| {
                    crate::repos::settings::get(conn, &key)
                        .map_err(|e| crate::db::StoreError::Sqlite(e.to_string()))
                })
                .expect("read")
        };

        // Diverge from the backup.
        store
            .execute(|conn| {
                conn.execute_batch(
                    "DELETE FROM setting WHERE key = 'test.row';
                     INSERT INTO setting (key, value, updated_at) VALUES ('after.backup', '7', '2026-10-17T10:00:00Z');",
                )
                .map_err(|e| crate::db::StoreError::Sqlite(e.to_string()))
            })
            .expect("diverge");
        assert_eq!(setting(&store, "test.row"), None);

        let report = restore(&store, &dest, "correct horse battery").expect("restore");
        assert_eq!(report.meta.schema_version, latest_schema());
        assert_eq!(report.snapshot_to, latest_schema());
        assert!(report.snapshot.is_file(), "the current DB was snapshotted");
        assert_eq!(setting(&store, "test.row").as_deref(), Some("42"));
        assert_eq!(setting(&store, "after.backup"), None);
        assert!(
            report.snapshot.starts_with(store.snapshots_dir()),
            "the snapshot lives with the migration snapshots"
        );
    }

    /// AC3: an older backup is migrated forward through the shipped migrations,
    /// keeping its rows and picking up the columns added since.
    #[test]
    fn an_older_backup_is_migrated_forward() {
        let dir = temp();
        let store = store(dir.path());
        let old = dir.path().join("old.boba");

        let key = DbKey::generate().expect("key");
        let conn = Connection::open(dir.path().join("old-source.db")).expect("open");
        conn.execute_batch(&format!("PRAGMA key = \"x'{}'\";", key.hex().as_str()))
            .expect("key");
        for (_, migration) in crate::migrate::embedded_files().iter().take(6) {
            conn.execute_batch(migration).expect("partial migration");
        }
        conn.execute_batch(
            "PRAGMA user_version = 6;
             INSERT INTO setting (key, value, updated_at)
                 VALUES ('old.row', 'from-6', '2026-01-01T00:00:00Z');
             INSERT INTO agent_run (id, task_kind, input_trust, cli, model, status, started_at)
                 VALUES ('01OLD', 'extract_postings', 'untrusted', 'claude', NULL, 'done', '2026-01-01T00:00:00Z');",
        )
        .expect("old rows");
        conn.execute_batch(&format!(
            "CREATE TABLE {META_TABLE} (
                 format TEXT NOT NULL, app_version TEXT NOT NULL,
                 schema_version INTEGER NOT NULL, created_at TEXT NOT NULL
             ) STRICT;
             INSERT INTO {META_TABLE} VALUES ('{FORMAT}', '0.0.1', 6, '2026-01-01T00:00:00Z');
             PRAGMA rekey = 'old pass';"
        ))
        .expect("rekey");
        drop(conn);
        std::fs::rename(dir.path().join("old-source.db"), &old).expect("into place");

        let report = restore(&store, &old, "old pass").expect("restore");
        assert_eq!(report.meta.schema_version, 6);
        assert_eq!(report.snapshot_to, latest_schema());

        let version: i64 = store
            .read(|conn| {
                conn.query_row("PRAGMA user_version", [], |row| row.get(0))
                    .map_err(|e| crate::db::StoreError::Sqlite(e.to_string()))
            })
            .expect("user_version");
        assert_eq!(version as u32, latest_schema());

        let old_row: Option<String> = store
            .read(|conn| {
                crate::repos::settings::get(conn, "old.row")
                    .map_err(|e| crate::db::StoreError::Sqlite(e.to_string()))
            })
            .expect("read");
        assert_eq!(old_row.as_deref(), Some("from-6"), "the row crossed over");

        // A column added after schema 6 defaults in, and a later table exists.
        let pack: String = store
            .read(|conn| {
                conn.query_row(
                    "SELECT pack_version FROM agent_run WHERE id = '01OLD'",
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| crate::db::StoreError::Sqlite(e.to_string()))
            })
            .expect("pack_version");
        assert_eq!(pack, "");
        let triage: i64 = store
            .read(|conn| {
                conn.query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'triage_dismissal'",
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| crate::db::StoreError::Sqlite(e.to_string()))
            })
            .expect("later table");
        assert_eq!(triage, 1, "the latest schema is in place");
    }
    #[test]
    fn a_refused_restore_leaves_the_current_data_untouched() {
        let dir = temp();
        let store = store(dir.path());
        let dest = dir.path().join("recovery.boba");
        create(
            &store,
            &dest,
            "correct horse battery",
            "2026-10-17T09:00:00Z",
        )
        .expect("backup");
        store
            .execute(|conn| {
                conn.execute_batch(
                    "DELETE FROM setting WHERE key = 'test.row';
                     INSERT INTO setting (key, value, updated_at) VALUES ('kept', '1', '2026-10-17T10:00:00Z');",
                )
                .map_err(|e| crate::db::StoreError::Sqlite(e.to_string()))
            })
            .expect("diverge");
        let snapshots_before = std::fs::read_dir(store.snapshots_dir())
            .expect("dir")
            .count();

        assert!(matches!(
            restore(&store, &dest, "tr0ub4dor&3"),
            Err(BackupError::WrongPassphrase)
        ));

        let kept: Option<String> = store
            .read(|conn| {
                crate::repos::settings::get(conn, "kept")
                    .map_err(|e| crate::db::StoreError::Sqlite(e.to_string()))
            })
            .expect("read");
        assert_eq!(kept.as_deref(), Some("1"), "the rows are untouched");
        assert_eq!(
            std::fs::read_dir(store.snapshots_dir())
                .expect("dir")
                .count(),
            snapshots_before,
            "a refused restore takes no snapshot either"
        );
    }
    #[test]
    fn validate_refuses_a_newer_schema() {
        let dir = temp();
        let store = store(dir.path());
        let dest = dir.path().join("recovery.boba");
        create(
            &store,
            &dest,
            "correct horse battery",
            "2026-10-17T09:00:00Z",
        )
        .expect("backup");

        // Pretend the file was written by a future build.
        let conn = opens_with_passphrase(&dest, "correct horse battery").expect("opens");
        conn.execute_batch(&format!(
            "UPDATE {META_TABLE} SET schema_version = {}",
            latest_schema() + 1
        ))
        .expect("age the marker");
        drop(conn);

        match validate(&dest, "correct horse battery") {
            Err(BackupError::TooNew { db, app }) => {
                assert_eq!(db, latest_schema() + 1);
                assert_eq!(app, latest_schema());
            }
            other => panic!("expected TooNew, got {other:?}"),
        }
    }
}
