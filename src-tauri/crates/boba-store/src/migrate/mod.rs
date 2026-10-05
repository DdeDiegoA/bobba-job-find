//! Forward-only migrations with a pre-migration encrypted snapshot (AD-20, ADR-009).
//!
//! Migration SQL is embedded at build time from `migrations/NNNN_name.sql`,
//! sorted by file name (`build.rs`), so adding a migration needs no code change.
//! Before applying anything to a non-empty database the runner takes an
//! owner-only `VACUUM INTO` snapshot with the same key, but only if free disk is
//! at least 2x the database size.

mod disk;

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};
use rusqlite_migration::{M, Migrations};
use thiserror::Error;

pub use disk::free_disk_bytes;

/// Number of `boba_meta` launches recorded since the last applied migration.
pub const LAUNCH_COUNT_KEY: &str = "launches_since_migration";

/// How long a snapshot is kept at minimum (AD-20: "whichever comes later").
const SNAPSHOT_MIN_AGE: Duration = Duration::from_secs(14 * 24 * 60 * 60);
/// Successful launches on the new schema required before a snapshot is eligible.
const SNAPSHOT_MIN_LAUNCHES: i64 = 3;

include!(concat!(env!("OUT_DIR"), "/migrations_generated.rs"));

#[derive(Debug, Error)]
pub enum MigrateError {
    #[error("database schema {db} is newer than this app supports ({app}); refusing to migrate")]
    DatabaseTooNew { db: u32, app: u32 },
    #[error(
        "not enough free disk space for a pre-migration snapshot: need {needed} bytes, have {available}"
    )]
    InsufficientDiskSpace { available: u64, needed: u64 },
    #[error("migration failed: {0}")]
    Migration(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// What a migration run did.
#[derive(Debug, Clone)]
pub struct MigrationReport {
    pub from: u32,
    pub to: u32,
    pub applied: bool,
    pub snapshot: Option<PathBuf>,
}

/// The migrations embedded from `migrations/*.sql` at build time.
pub fn embedded() -> Migrations<'static> {
    Migrations::new(MIGRATIONS.iter().map(|(_, sql)| M::up(sql)).collect())
}

/// Apply the embedded migrations, snapshotting first when needed.
pub fn apply(
    conn: &mut Connection,
    db_path: &Path,
    snapshots_dir: &Path,
) -> Result<MigrationReport, MigrateError> {
    apply_with(conn, &embedded(), db_path, snapshots_dir, None)
}

/// Apply `migrations`, with an injectable free-space value for tests.
pub fn apply_with(
    conn: &mut Connection,
    migrations: &Migrations<'_>,
    db_path: &Path,
    snapshots_dir: &Path,
    available_bytes: Option<u64>,
) -> Result<MigrationReport, MigrateError> {
    let from = user_version(conn)?;
    let pending = migrations
        .pending_migrations(conn)
        .map_err(|e| MigrateError::Migration(e.to_string()))?;

    if pending < 0 {
        let app = (from as i64 + pending as i64).max(0) as u32;
        return Err(MigrateError::DatabaseTooNew { db: from, app });
    }
    if pending == 0 {
        return Ok(MigrationReport {
            from,
            to: from,
            applied: false,
            snapshot: None,
        });
    }
    let to = (from as i64 + pending as i64) as u32;

    let mut snapshot = None;
    if from > 0 {
        std::fs::create_dir_all(snapshots_dir)?;
        set_owner_only(snapshots_dir, 0o700)?;
        let db_size = std::fs::metadata(db_path).map(|m| m.len()).unwrap_or(0);
        let needed = db_size.saturating_mul(2);
        let available = match available_bytes {
            Some(bytes) => bytes,
            None => free_disk_bytes(snapshots_dir)?,
        };
        if available < needed {
            return Err(MigrateError::InsufficientDiskSpace { available, needed });
        }
        let path = snapshots_dir.join(format!("pre-{from}-{to}-{}.db", now_unix()));
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        conn.execute_batch(&format!("VACUUM INTO {}", quote_sql(&path)))
            .map_err(|e| MigrateError::Migration(e.to_string()))?;
        set_owner_only(&path, 0o600)?;
        snapshot = Some(path);
    }

    migrations
        .to_latest(conn)
        .map_err(|e| MigrateError::Migration(e.to_string()))?;
    reset_launch_count(conn)?;

    Ok(MigrationReport {
        from,
        to,
        applied: true,
        snapshot,
    })
}

/// Record one successful launch on the current schema.
pub fn record_launch(conn: &Connection) -> Result<(), MigrateError> {
    conn.execute(
        "INSERT INTO boba_meta (key, value) VALUES (?1, '1')
         ON CONFLICT(key) DO UPDATE SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)",
        params![LAUNCH_COUNT_KEY],
    )
    .map_err(|e| MigrateError::Migration(e.to_string()))?;
    Ok(())
}

/// Delete snapshots that are both old enough and past their launch count
/// (AD-20: 3 launches or 14 days, whichever comes later). Returns the deleted paths.
pub fn gc_snapshots(conn: &Connection, snapshots_dir: &Path) -> Result<Vec<PathBuf>, MigrateError> {
    if !snapshots_dir.is_dir() {
        return Ok(Vec::new());
    }
    let current = user_version(conn)?;
    let launches = launch_count(conn)?;
    let now = now_unix();

    let mut deleted = Vec::new();
    for entry in std::fs::read_dir(snapshots_dir)? {
        let path = entry?.path();
        let Some((_, target, ts)) = parse_snapshot_name(&path) else {
            continue;
        };
        if target > current || launches < SNAPSHOT_MIN_LAUNCHES {
            continue;
        }
        if now.saturating_sub(ts) < SNAPSHOT_MIN_AGE.as_secs() {
            continue;
        }
        std::fs::remove_file(&path)?;
        deleted.push(path);
    }
    Ok(deleted)
}

/// List the pre-migration snapshots present, newest first (Settings -> Privacy).
pub fn list_snapshots(snapshots_dir: &Path) -> Result<Vec<PathBuf>, MigrateError> {
    let mut found = Vec::new();
    if !snapshots_dir.is_dir() {
        return Ok(found);
    }
    for entry in std::fs::read_dir(snapshots_dir)? {
        let path = entry?.path();
        if parse_snapshot_name(&path).is_some() {
            found.push(path);
        }
    }
    found.sort();
    found.reverse();
    Ok(found)
}

fn user_version(conn: &Connection) -> Result<u32, MigrateError> {
    conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .map(|v| v.max(0) as u32)
        .map_err(|e| MigrateError::Migration(e.to_string()))
}

fn launch_count(conn: &Connection) -> Result<i64, MigrateError> {
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM boba_meta WHERE key = ?1",
            params![LAUNCH_COUNT_KEY],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| MigrateError::Migration(e.to_string()))?;
    Ok(value.and_then(|v| v.parse().ok()).unwrap_or(0))
}

fn reset_launch_count(conn: &Connection) -> Result<(), MigrateError> {
    conn.execute(
        "DELETE FROM boba_meta WHERE key = ?1",
        params![LAUNCH_COUNT_KEY],
    )
    .map_err(|e| MigrateError::Migration(e.to_string()))?;
    Ok(())
}

/// `pre-<from>-<to>-<unix_ts>.db`.
fn parse_snapshot_name(path: &Path) -> Option<(u32, u32, u64)> {
    let name = path.file_name()?.to_str()?;
    let stem = name.strip_suffix(".db")?;
    let mut parts = stem.split('-');
    if parts.next()? != "pre" {
        return None;
    }
    let from = parts.next()?.parse().ok()?;
    let to = parts.next()?.parse().ok()?;
    let ts = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((from, to, ts))
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn quote_sql(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "''"))
}

#[cfg(unix)]
fn set_owner_only(path: &Path, mode: u32) -> Result<(), MigrateError> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    Ok(())
}

/// Windows: the app data directory already has a user-only DACL, so this is a no-op.
#[cfg(not(unix))]
fn set_owner_only(_path: &Path, _mode: u32) -> Result<(), MigrateError> {
    Ok(())
}
