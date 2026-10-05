//! The encrypted store: one writer actor plus a read pool (AD-2, AD-13, AD-20).
//!
//! Only `boba-store` opens `boba.db`; the writer actor serialises every write on
//! one connection, and reads come from a small r2d2 pool. Both apply the
//! SQLCipher key before any statement runs.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;
use thiserror::Error;

use crate::keys::{self, DbKey, KeyError};
use crate::migrate::{self, MigrateError, MigrationReport};

#[derive(Debug, Error)]
pub enum StoreError {
    #[error(transparent)]
    Key(#[from] KeyError),
    #[error(transparent)]
    Migration(#[from] MigrateError),
    /// A database exists but its key is unavailable; enter the Locked state
    /// (ADR-009). Never mint a new key or database.
    #[error("database is locked: its key cannot be read (ADR-009)")]
    Locked,
    #[error("sqlite error: {0}")]
    Sqlite(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("the store writer actor has stopped")]
    WriterGone,
}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Sqlite(e.to_string())
    }
}

/// Where the store lives. Paths are injected so slices and tests own their dirs.
#[derive(Debug, Clone)]
pub struct StoreConfig {
    pub db_path: PathBuf,
    pub snapshots_dir: PathBuf,
    /// Keychain service; defaults to [`keys::SERVICE`], use [`keys::DEV_SERVICE`]
    /// for `tauri dev` (ADR-009).
    pub service: String,
    pub pool_size: u32,
}

impl StoreConfig {
    pub fn new(db_path: impl Into<PathBuf>, snapshots_dir: impl Into<PathBuf>) -> Self {
        Self {
            db_path: db_path.into(),
            snapshots_dir: snapshots_dir.into(),
            service: keys::SERVICE.to_string(),
            pool_size: 4,
        }
    }

    /// Local-dev variant: separate keychain service so dev runs never touch real data.
    pub fn dev(db_path: impl Into<PathBuf>, snapshots_dir: impl Into<PathBuf>) -> Self {
        Self {
            service: keys::DEV_SERVICE.to_string(),
            ..Self::new(db_path, snapshots_dir)
        }
    }
}

/// The open store. Holds the key so later slices (backup, re-key) can reuse it.
pub struct Store {
    db_path: PathBuf,
    snapshots_dir: PathBuf,
    key: DbKey,
    writer: WriterHandle,
    pool: r2d2::Pool<SqliteConnectionManager>,
    migration: MigrationReport,
}

impl Store {
    /// Open the store, resolving the key from the OS keychain (ADR-004/ADR-009).
    pub fn open(config: StoreConfig) -> Result<Self, StoreError> {
        keys::install_platform_store()?;
        let key = keys::load_or_create(&config.db_path, &config.service)?;
        Self::open_with_key(config, key)
    }

    /// Open the store with a key already in hand (unlock screen, tests).
    pub fn open_with_key(config: StoreConfig, key: DbKey) -> Result<Self, StoreError> {
        if let Some(parent) = config.db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::create_dir_all(&config.snapshots_dir)?;

        let mut conn = open_encrypted(&config.db_path, &key)?;
        let migration = migrate::apply(&mut conn, &config.db_path, &config.snapshots_dir)?;
        migrate::record_launch(&conn)?;
        migrate::gc_snapshots(&conn, &config.snapshots_dir)?;

        let writer = WriterHandle::new(conn)?;
        let pool = build_pool(&config, &key)?;

        Ok(Self {
            db_path: config.db_path,
            snapshots_dir: config.snapshots_dir,
            key,
            writer,
            pool,
            migration,
        })
    }

    /// Run a closure on the single writer connection, serialised with every other write.
    pub fn execute<T, F>(&self, f: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, StoreError> + Send + 'static,
    {
        self.writer.execute(f)
    }

    /// Run a closure on a pooled read-only connection.
    pub fn read<T, F>(&self, f: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T, StoreError> + Send + 'static,
    {
        let conn = self
            .pool
            .get()
            .map_err(|e| StoreError::Sqlite(e.to_string()))?;
        f(&conn)
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn snapshots_dir(&self) -> &Path {
        &self.snapshots_dir
    }

    pub fn migration(&self) -> &MigrationReport {
        &self.migration
    }

    /// The DB key, for re-keying backups (S10.2). Never log or serialize it.
    pub fn db_key(&self) -> &DbKey {
        &self.key
    }
}

/// Apply the SQLCipher key. Must be the first statement on a connection.
fn apply_key(conn: &Connection, key: &DbKey) -> Result<(), rusqlite::Error> {
    conn.execute_batch(&format!("PRAGMA key = \"x'{}'\";", key.hex().as_str()))
}

/// Open a connection, set the key and pragmas, and prove the key is right.
fn open_encrypted(path: &Path, key: &DbKey) -> Result<Connection, StoreError> {
    let conn = Connection::open(path)?;
    apply_key(&conn, key)?;

    // Touch sqlite_master *before* any other access: a wrong key fails here with
    // "file is not a database", which is the Locked state (ADR-009).
    match conn.query_row("SELECT count(*) FROM sqlite_master", [], |row| {
        row.get::<_, i64>(0)
    }) {
        Ok(_) => {}
        Err(e) if is_not_a_database(&e) => return Err(StoreError::Locked),
        Err(e) => return Err(e.into()),
    }

    conn.busy_timeout(Duration::from_secs(5))?;
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA secure_delete=ON;")?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    Ok(conn)
}

fn build_pool(
    config: &StoreConfig,
    key: &DbKey,
) -> Result<r2d2::Pool<SqliteConnectionManager>, StoreError> {
    let hex = key.hex();
    let manager = SqliteConnectionManager::file(&config.db_path).with_init(move |conn| {
        conn.execute_batch(&format!(
            "PRAGMA key = \"x'{}'\"; PRAGMA foreign_keys=ON; PRAGMA secure_delete=ON;",
            hex.as_str()
        ))
    });
    r2d2::Pool::builder()
        .max_size(config.pool_size)
        .build(manager)
        .map_err(|e| StoreError::Sqlite(e.to_string()))
}

fn is_not_a_database(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::NotADatabase
    )
}

type Job = Box<dyn FnOnce(&mut Connection) + Send>;

/// A single-threaded actor owning the one write connection (AD-2).
struct WriterHandle {
    tx: Sender<Job>,
    join: Option<JoinHandle<()>>,
}

impl WriterHandle {
    fn new(mut conn: Connection) -> Result<Self, StoreError> {
        let (tx, rx) = channel::<Job>();
        let join = thread::Builder::new()
            .name("boba-db-writer".to_string())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    job(&mut conn);
                }
            })?;
        Ok(Self {
            tx,
            join: Some(join),
        })
    }

    fn execute<T, F>(&self, f: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, StoreError> + Send + 'static,
    {
        let (reply_tx, reply_rx) = channel();
        let job: Job = Box::new(move |conn| {
            let _ = reply_tx.send(f(conn));
        });
        self.tx.send(job).map_err(|_| StoreError::WriterGone)?;
        reply_rx.recv().map_err(|_| StoreError::WriterGone)?
    }
}

impl Drop for WriterHandle {
    fn drop(&mut self) {
        // Replacing the sender drops the real one, which ends the actor loop.
        let (dead_tx, _dead_rx) = channel();
        drop(std::mem::replace(&mut self.tx, dead_tx));
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
