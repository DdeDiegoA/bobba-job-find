//! Store integration tests (S1.2a): encryption, key custody, snapshots,
//! migration discovery, writer actor / read pool and crash consistency.

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, OnceLock};

use boba_store::db::{Store, StoreConfig, StoreError};
use boba_store::keys::{self, DbKey, KeyError};
use boba_store::migrate::{self, MigrateError};
use rusqlite::{Connection, OptionalExtension, params};
use rusqlite_migration::{M, Migrations};
use tempfile::TempDir;

const KEY_HEX: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
const META_SQL: &str = "CREATE TABLE boba_meta(key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL) STRICT; \
                        CREATE TABLE t1(x);";

fn key() -> DbKey {
    DbKey::from_hex(KEY_HEX).expect("test key")
}

/// Synthetic one-migration schema (creates `boba_meta` like the real 0001).
fn schema_v1() -> Migrations<'static> {
    Migrations::new(vec![M::up(META_SQL)])
}

/// Synthetic two-migration schema, so a v1 database has one migration pending.
fn schema_v2() -> Migrations<'static> {
    Migrations::new(vec![M::up(META_SQL), M::up("CREATE TABLE t2(y);")])
}

/// Install a mock credential store once; platform stores need a signed binary.
fn mock_keychain() -> Arc<keyring_core::CredentialStore> {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let store: Arc<keyring_core::CredentialStore> =
            keyring_core::mock::Store::new().expect("mock keychain");
        keyring_core::set_default_store(store);
    });
    keyring_core::get_default_store().expect("default store set")
}

fn keyed_open(path: &Path) -> Connection {
    let conn = Connection::open(path).expect("open");
    conn.execute_batch(&format!("PRAGMA key = \"x'{KEY_HEX}'\";"))
        .expect("set key");
    conn
}

fn user_version(conn: &Connection) -> i64 {
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("user_version")
}

const DAY: u64 = 24 * 60 * 60;
fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

// AC-1: first launch mints a random 256-bit key into keychain `app.bobajobfind/db-key`.
#[test]
fn first_launch_mints_256bit_key_in_keychain() {
    mock_keychain();
    let dir = TempDir::new().unwrap();
    let mut cfg = StoreConfig::new(dir.path().join("boba.db"), dir.path().join("snapshots"));
    cfg.service = "test.mint".to_string();

    let store = Store::open(cfg).expect("store opens on first launch");
    assert!(store.db_path().exists());

    let entry = keyring_core::Entry::new("test.mint", keys::DB_KEY_ENTRY).unwrap();
    let stored = entry.get_password().expect("key in keychain");
    assert_eq!(stored.len(), 64, "256-bit key stored as 64 hex chars");
    assert!(DbKey::from_hex(&stored).is_ok());
}

// AC-1: the DB file is not readable by plain SQLite.
#[test]
fn db_file_is_not_readable_with_plain_sqlite() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("boba.db");
    let _store = Store::open_with_key(StoreConfig::new(&db, dir.path().join("snapshots")), key())
        .expect("store opens");

    let plain = Connection::open(&db).expect("open without key");
    let err = plain
        .query_row("SELECT count(*) FROM sqlite_master", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect_err("plain SQLite must not read an encrypted DB");
    assert!(
        matches!(err, rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::NotADatabase)
    );
}

// AC-5 (ADR-009): an existing DB with no readable key is Locked; never mints.
#[test]
fn existing_db_without_readable_key_is_locked_and_never_mints() {
    mock_keychain();
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("boba.db");
    let snapshots = dir.path().join("snapshots");
    let mut cfg = StoreConfig::new(&db, &snapshots);
    cfg.service = "test.locked".to_string();

    let store = Store::open(cfg.clone()).expect("first run creates key + db");
    drop(store);
    let size_before = std::fs::metadata(&db).unwrap().len();

    // Simulate a wiped keychain / unreadable key.
    keyring_core::Entry::new("test.locked", keys::DB_KEY_ENTRY)
        .unwrap()
        .delete_credential()
        .expect("key removed");

    let err = match Store::open(cfg) {
        Ok(_) => panic!("must fail"),
        Err(e) => e,
    };
    assert!(
        matches!(err, StoreError::Key(KeyError::Locked)),
        "expected Locked, got {err:?}"
    );

    // No new key was minted and the existing DB was left alone.
    let entry = keyring_core::Entry::new("test.locked", keys::DB_KEY_ENTRY).unwrap();
    assert!(entry.get_password().is_err(), "must not mint a new key");
    assert_eq!(std::fs::metadata(&db).unwrap().len(), size_before);
}

// AC-5 (ADR-009): opening an existing DB with a wrong key is Locked, not corruption.
#[test]
fn opening_existing_db_with_wrong_key_is_locked() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("boba.db");
    let snapshots = dir.path().join("snapshots");
    let cfg = StoreConfig::new(&db, &snapshots);
    drop(Store::open_with_key(cfg.clone(), key()).expect("first open"));

    let wrong = DbKey::from_hex(&"ab".repeat(32)).unwrap();
    let err = match Store::open_with_key(cfg, wrong) {
        Ok(_) => panic!("wrong key must not open the store"),
        Err(e) => e,
    };
    assert!(
        matches!(err, StoreError::Locked),
        "expected Locked, got {err:?}"
    );
}

// AC-4 (AD-20): migrations are discovered from the embedded, sorted file list.
#[test]
fn migrations_are_discovered_by_sorted_file_name() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("boba.db");
    let snapshots = dir.path().join("snapshots");
    let mut conn = keyed_open(&db);
    let migrations = migrate::embedded();
    assert!(
        migrations.pending_migrations(&conn).unwrap() >= 1,
        "0001_init.sql must be embedded"
    );
    migrate::apply_with(&mut conn, &migrations, &db, &snapshots, None).expect("migrate");
    let has_meta: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='boba_meta'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(has_meta, 1);
}

// AC-4 (AD-20): no placeholder/comment-only file may enter the migration chain.
#[test]
fn no_placeholder_migrations_are_embedded() {
    assert!(
        !migrate::embedded_files().is_empty(),
        "at least 0001_init.sql must be embedded"
    );
    for (name, sql) in migrate::embedded_files() {
        let first = sql.lines().map(str::trim).find(|line| !line.is_empty());
        assert!(
            !first.unwrap_or("").starts_with("-- Placeholder"),
            "{name} is a placeholder migration; it would advance user_version without a schema change"
        );
    }
}

// AC-4: user_version advances exactly once per embedded migration on a fresh DB.
#[test]
fn fresh_database_records_one_version_per_migration() {
    let mut conn = Connection::open_in_memory().unwrap();
    let migrations = migrate::embedded();
    let pending = migrations.pending_migrations(&conn).unwrap();
    assert!(pending >= 1);
    migrations.to_latest(&mut conn).expect("migrate");
    assert_eq!(user_version(&conn), pending as i64);
}

// AC-2 (AD-20): a pending migration snapshots first (encrypted, owner-only).
#[test]
fn pre_migration_snapshot_is_encrypted_owner_only_and_same_key() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("boba.db");
    let snapshots = dir.path().join("snapshots");
    let mut conn = keyed_open(&db);

    migrate::apply_with(&mut conn, &schema_v1(), &db, &snapshots, None).expect("v1");

    let v2 = schema_v2();
    let report = migrate::apply_with(&mut conn, &v2, &db, &snapshots, None).expect("v2");

    let snap = report.snapshot.expect("snapshot taken");
    assert!(snap.exists());
    assert_eq!(
        snap.file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .split('-')
            .next(),
        Some("pre")
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&snap).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "snapshot must be owner-only");
    }

    // Same key opens the snapshot; no key cannot.
    let with_key = keyed_open(&snap);
    with_key
        .query_row("SELECT count(*) FROM t1", [], |row| row.get::<_, i64>(0))
        .expect("snapshot opens with the same key");
    let plain = Connection::open(&snap).unwrap();
    assert!(
        plain
            .query_row("SELECT count(*) FROM t1", [], |row| row.get::<_, i64>(0))
            .is_err(),
        "snapshot must be encrypted"
    );
}

// AC-2 (AD-20): a migration is refused when free disk is below 2x the DB size.
#[test]
fn migration_refused_when_free_disk_below_twice_db_size() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("boba.db");
    let snapshots = dir.path().join("snapshots");
    let mut conn = keyed_open(&db);

    migrate::apply_with(&mut conn, &schema_v1(), &db, &snapshots, None).expect("v1");

    let v2 = schema_v2();
    let err =
        migrate::apply_with(&mut conn, &v2, &db, &snapshots, Some(0)).expect_err("must refuse");
    assert!(matches!(err, MigrateError::InsufficientDiskSpace { .. }));
    assert_eq!(user_version(&conn), 1, "schema must be unchanged");
}

// AC-2 (AD-20): snapshots are deleted after 3 launches and 14 days (whichever is later).
#[test]
fn snapshots_deleted_after_three_launches_and_fourteen_days() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("boba.db");
    let snapshots = dir.path().join("snapshots");
    std::fs::create_dir_all(&snapshots).unwrap();
    let mut conn = keyed_open(&db);
    migrate::apply_with(&mut conn, &schema_v1(), &db, &snapshots, None).expect("v1");

    let old = snapshots.join(format!("pre-0-1-{}.db", now_unix() - 15 * DAY));
    let fresh = snapshots.join(format!("pre-0-1-{}.db", now_unix()));
    std::fs::write(&old, b"x").unwrap();
    std::fs::write(&fresh, b"x").unwrap();

    assert!(migrate::gc_snapshots(&conn, &snapshots).unwrap().is_empty()); // < 3 launches

    for _ in 0..3 {
        migrate::record_launch(&conn).unwrap();
    }
    let deleted = migrate::gc_snapshots(&conn, &snapshots).unwrap();
    assert_eq!(deleted, vec![old.clone()]);
    assert!(!old.exists());
    assert!(fresh.exists(), "age < 14 days must be kept");
}

// AC-3 (AD-2): one writer actor + a read pool; concurrent writers are serialised.
#[test]
fn writer_serialises_writes_and_read_pool_reads() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(
        Store::open_with_key(
            StoreConfig::new(dir.path().join("boba.db"), dir.path().join("snapshots")),
            key(),
        )
        .expect("store opens"),
    );

    let mut handles = Vec::new();
    for i in 0..8 {
        let store = Arc::clone(&store);
        handles.push(std::thread::spawn(move || {
            store
                .execute(move |conn| {
                    conn.execute(
                        "INSERT INTO boba_meta (key, value) VALUES (?1, 'v')",
                        params![format!("k{i}")],
                    )?;
                    Ok(())
                })
                .unwrap();
        }));
    }
    for handle in handles {
        handle.join().unwrap();
    }

    let count: i64 = store
        .read(|conn| {
            Ok(conn.query_row(
                "SELECT count(*) FROM boba_meta WHERE key LIKE 'k%'",
                [],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(count, 8);
}

// AC-3 (NFR-12): a crash mid-write leaves the DB consistent.
#[test]
fn crash_mid_write_leaves_db_consistent() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("boba.db");
    let snapshots = dir.path().join("snapshots");
    let store =
        Store::open_with_key(StoreConfig::new(&db, &snapshots), key()).expect("store opens");
    store
        .execute(|conn| {
            conn.execute(
                "INSERT INTO boba_meta (key, value) VALUES ('committed', 'yes')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    drop(store);

    let exe = std::env::current_exe().unwrap();
    let status = Command::new(exe)
        .args(["--exact", "crash_child_helper", "--nocapture"])
        .env("BOBA_CRASH_DB", &db)
        .env("BOBA_CRASH_KEY", KEY_HEX)
        .status()
        .expect("spawn crash child");
    assert!(!status.success(), "child must abort mid-write");

    let conn = keyed_open(&db);
    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");

    let committed: Option<String> = conn
        .query_row(
            "SELECT value FROM boba_meta WHERE key='committed'",
            [],
            |row| row.get(0),
        )
        .optional()
        .unwrap();
    assert_eq!(committed.as_deref(), Some("yes"));

    let uncommitted: Option<String> = conn
        .query_row(
            "SELECT value FROM boba_meta WHERE key='uncommitted'",
            [],
            |row| row.get(0),
        )
        .optional()
        .unwrap();
    assert_eq!(uncommitted, None, "uncommitted write must not survive");
}

// Child process run by `crash_mid_write_leaves_db_consistent`; aborts before COMMIT.
#[test]
fn crash_child_helper() {
    let Ok(db) = std::env::var("BOBA_CRASH_DB") else {
        return;
    };
    let hex = std::env::var("BOBA_CRASH_KEY").unwrap();
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch(&format!(
        "PRAGMA key = \"x'{hex}'\"; PRAGMA foreign_keys=ON;"
    ))
    .unwrap();
    conn.execute_batch("BEGIN IMMEDIATE;").unwrap();
    conn.execute(
        "INSERT INTO boba_meta (key, value) VALUES ('uncommitted', 'no')",
        [],
    )
    .unwrap();
    // Simulate a crash (power cut / kill) before COMMIT.
    std::process::abort();
}
