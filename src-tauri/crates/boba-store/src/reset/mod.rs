//! Reset everything (S10.2, FR-112, AC5): the database, every snapshot, the
//! automation browser profiles (Session Board cookies included) and the
//! keychain entries, after the user types the confirmation.
//!
//! The database file is held open by the running app, so the reset is two-tier:
//! the content is erased **in place** (every object dropped, the schema rebuilt
//! empty, `VACUUM` on a `secure_delete` connection), and then the file itself is
//! removed — which succeeds everywhere the platform allows unlinking an open
//! file. When it cannot be removed, the keychain key is kept so the app still
//! starts on the empty store, and the path is reported as remaining.

use std::path::{Path, PathBuf};

use crate::db::Store;

/// What the user must type to mean it. The same literal in both UI languages:
/// a translated word would let a copy-paste from the wrong locale pass.
pub const CONFIRMATION: &str = "DELETE";

/// A reset failure.
#[derive(Debug, thiserror::Error)]
pub enum ResetError {
    #[error("the confirmation text does not match")]
    WrongConfirmation,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("sqlite error: {0}")]
    Sqlite(String),
    #[error("keychain error: {0}")]
    Keychain(String),
}

impl From<rusqlite::Error> for ResetError {
    fn from(error: rusqlite::Error) -> Self {
        ResetError::Sqlite(error.to_string())
    }
}

impl From<crate::db::StoreError> for ResetError {
    fn from(error: crate::db::StoreError) -> Self {
        ResetError::Sqlite(error.to_string())
    }
}

/// What a reset deleted, and where the last files live (the UI's uninstall
/// notes use `data_dir` and `remaining`).
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

/// Delete everything this app stored. `confirmation` must be [`CONFIRMATION`].
pub fn reset(
    store: &Store,
    service: &str,
    profiles: &[PathBuf],
    confirmation: &str,
) -> Result<ResetReport, ResetError> {
    if confirmation != CONFIRMATION {
        return Err(ResetError::WrongConfirmation);
    }
    let data_dir = store
        .db_path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();

    // Read the secret references before the rows that name them are gone: the
    // keychain cannot be enumerated, so the database is the only index.
    let secret_refs = secret_references(store)?;

    // 1. Erase the content in place. The file stays open on every platform; the
    //    schema is rebuilt empty so the app can still start if step 3 fails.
    store.execute(|conn| {
        conn.execute_batch("PRAGMA foreign_keys=OFF; BEGIN IMMEDIATE;")
            .map_err(store_sql)?;
        let erased = crate::backup::drop_all_objects(conn)
            .and_then(|()| {
                crate::backup::replay_migrations(conn, crate::migrate::embedded_files().len())
            })
            .and_then(|()| {
                conn.execute_batch(&format!(
                    "PRAGMA user_version = {};",
                    crate::backup::latest_schema()
                ))
                .map_err(Into::into)
            });
        match erased {
            Ok(()) => conn.execute_batch("COMMIT;").map_err(store_sql)?,
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK;");
                return Err(store_sql(rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
                    Some(error.to_string()),
                )));
            }
        }
        // No page of the old data survives the rewrite (`secure_delete` is on).
        conn.execute_batch("PRAGMA foreign_keys=ON; VACUUM;")
            .map_err(store_sql)
    })?;

    // 2. Files that are not the database: snapshots and the browser profiles,
    //    whose cookies belong to nobody once the keychain entries are gone.
    let mut removed = Vec::new();
    remove_dir(store.snapshots_dir(), &mut removed)?;
    for profile in profiles {
        remove_dir(profile, &mut removed)?;
    }

    // 3. The keychain: the database key, the audit anchor and every secret.
    let mut keychain_removed = Vec::new();
    let db_removed = remove_db_files(store, &mut removed);
    if db_removed {
        // With the file gone, its key must go too: the next launch mints a new
        // one over a new database (ADR-009 §1).
        delete_keychain_entry(service, crate::keys::DB_KEY_ENTRY, &mut keychain_removed)?;
    }
    // The audit chain is gone with the data; its anchor must not outlive it.
    delete_keychain_entry(
        service,
        crate::repos::audit::AUDIT_HEAD_ENTRY,
        &mut keychain_removed,
    )?;
    for reference in secret_refs {
        // `secret_delete` reports whether an entry was there; both outcomes are
        // fine: the secret is gone either way.
        match crate::keys::secret_delete(service, &reference) {
            Ok(true) => {
                keychain_removed.push(format!("{}:{reference}", crate::keys::SECRET_ENTRY_PREFIX))
            }
            Ok(false) => {}
            Err(error) => return Err(ResetError::Keychain(error.to_string())),
        }
    }

    // 4. What the uninstall notes point at: anything that could not be deleted.
    let mut remaining = Vec::new();
    for path in [
        store.db_path().to_path_buf(),
        sidecar(store.db_path(), "wal"),
        sidecar(store.db_path(), "shm"),
        crate::unlock::kdf_path(store.db_path()),
    ] {
        if path.exists() {
            remaining.push(path);
        }
    }
    if store.snapshots_dir().exists() {
        remaining.push(store.snapshots_dir().to_path_buf());
    }
    for profile in profiles {
        if profile.exists() {
            remaining.push(profile.clone());
        }
    }

    Ok(ResetReport {
        db_removed,
        removed,
        keychain_removed,
        remaining,
        data_dir,
    })
}

fn store_sql(error: rusqlite::Error) -> crate::db::StoreError {
    crate::db::StoreError::Sqlite(error.to_string())
}

/// The `secret_ref` values the database knows (the keychain has no index).
fn secret_references(store: &Store) -> Result<Vec<String>, ResetError> {
    let refs = store.read(|conn| {
        let mut stmt = conn
            .prepare("SELECT secret_ref FROM email_account ORDER BY secret_ref")
            .map_err(|error| crate::db::StoreError::Sqlite(error.to_string()))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| crate::db::StoreError::Sqlite(error.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| crate::db::StoreError::Sqlite(error.to_string()))?;
        Ok(rows)
    })?;
    Ok(refs)
}

/// Remove one directory tree; a missing one is already gone.
fn remove_dir(path: &Path, removed: &mut Vec<PathBuf>) -> Result<(), ResetError> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => removed.push(path.to_path_buf()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(ResetError::Io(error)),
    }
    Ok(())
}

/// Try to unlink the database and its sidecars; not every platform allows
/// unlinking a file a running process holds open (Windows does not). The KDF
/// record only goes with the database it belongs to: keeping it is what lets a
/// passphrase-built store still open the empty schema when the unlink failed.
fn remove_db_files(store: &Store, removed: &mut Vec<PathBuf>) -> bool {
    for path in [
        store.db_path().to_path_buf(),
        sidecar(store.db_path(), "wal"),
        sidecar(store.db_path(), "shm"),
    ] {
        if std::fs::remove_file(&path).is_ok() {
            removed.push(path);
        }
    }
    let removed_db = !store.db_path().exists();
    if removed_db {
        let kdf = crate::unlock::kdf_path(store.db_path());
        if std::fs::remove_file(&kdf).is_ok() {
            removed.push(kdf);
        }
    }
    removed_db
}

/// `boba.db` + `-suffix`.
fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(format!("-{suffix}"));
    PathBuf::from(name)
}

/// Delete one keychain entry by name; absent is success.
fn delete_keychain_entry(
    service: &str,
    entry: &str,
    removed: &mut Vec<String>,
) -> Result<(), ResetError> {
    crate::keys::install_platform_store()
        .map_err(|error| ResetError::Keychain(error.to_string()))?;
    let handle = keyring_core::Entry::new(service, entry)
        .map_err(|error| ResetError::Keychain(error.to_string()))?;
    match handle.delete_credential() {
        Ok(()) => {
            removed.push(entry.to_string());
            Ok(())
        }
        Err(keyring_core::Error::NoEntry) => Ok(()),
        Err(error) => Err(ResetError::Keychain(error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::db::StoreConfig;
    use crate::keys::{self, DbKey};

    const SERVICE: &str = "app.bobajobfind.test.reset";

    fn temp() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp dir")
    }

    fn store(dir: &Path) -> Store {
        let config = StoreConfig::new(dir.join("boba.db"), dir.join("snapshots"));
        Store::open_with_key(config, DbKey::generate().expect("key")).expect("store")
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
        std::fs::write(path, b"data").expect("write");
    }

    fn keychain_has(service: &str, entry: &str) -> bool {
        keys::install_platform_store().expect("store");
        keyring_core::Entry::new(service, entry)
            .expect("entry")
            .get_password()
            .is_ok()
    }

    /// AC5: the DB content, snapshots, browser profiles and keychain entries are
    /// gone after the confirmation is typed.
    #[test]
    fn reset_deletes_the_database_snapshots_profiles_and_keychain() {
        let dir = temp();
        let store = store(dir.path());
        let data = dir.path();
        let snapshots = data.join("snapshots");
        std::fs::create_dir_all(&snapshots).expect("snapshots");
        touch(&snapshots.join("pre-1-2-1700000000.db"));
        let ats = data.join("browser-profile");
        touch(&ats.join("Default").join("Cookies"));
        let board = data.join("browser-sessions").join("linkedin");
        touch(&board.join("Cookies"));

        store
            .execute(|conn| {
                conn.execute_batch(
                    "INSERT INTO setting (key, value, updated_at) VALUES ('kept', '1', '2026-10-17T00:00:00Z');
                     INSERT INTO email_account
                         (id, provider, address, mode, client_id, client_id_kind, scopes, secret_ref, connected_at, updated_at)
                         VALUES ('01ACC', 'gmail', 'me@example.com', 'send_only', 'cid', 'byo', '', 'mail:01ACC', '2026-10-17T00:00:00Z', '2026-10-17T00:00:00Z');",
                )
                .map_err(|e| crate::db::StoreError::Sqlite(e.to_string()))
            })
            .expect("seed");

        // The keychain entries a reset must delete.
        keys::secret_put(SERVICE, "mail:01ACC", "{\"token\":\"secret\"}").expect("secret");
        crate::repos::audit::anchor_set(SERVICE, "abc123").expect("anchor");
        keyring_core::Entry::new(SERVICE, crate::keys::DB_KEY_ENTRY)
            .expect("entry")
            .set_password("00".repeat(32).as_str())
            .expect("db key");

        let report = reset(
            &store,
            SERVICE,
            &[ats.clone(), data.join("browser-sessions")],
            CONFIRMATION,
        )
        .expect("reset");

        // The database content is gone; the store still answers on the fresh schema.
        let rows: i64 = store
            .read(|conn| {
                conn.query_row("SELECT count(*) FROM setting", [], |row| row.get(0))
                    .map_err(|e| crate::db::StoreError::Sqlite(e.to_string()))
            })
            .expect("read");
        assert_eq!(rows, 0, "no setting survived");
        let accounts: i64 = store
            .read(|conn| {
                conn.query_row("SELECT count(*) FROM email_account", [], |row| row.get(0))
                    .map_err(|e| crate::db::StoreError::Sqlite(e.to_string()))
            })
            .expect("read");
        assert_eq!(accounts, 0);

        assert!(!snapshots.exists(), "the snapshots are gone");
        assert!(!ats.exists(), "the automation profile is gone");
        assert!(
            !data.join("browser-sessions").exists(),
            "the board cookies are gone"
        );

        assert_eq!(keys::secret_get(SERVICE, "mail:01ACC").expect("get"), None);
        assert_eq!(
            crate::repos::audit::anchor_get(SERVICE).expect("anchor"),
            None
        );
        assert!(!keychain_has(SERVICE, crate::keys::DB_KEY_ENTRY));

        assert!(report.keychain_removed.contains(&"db-key".to_string()));
        assert!(report.keychain_removed.contains(&"audit-head".to_string()));
        assert!(
            report
                .keychain_removed
                .contains(&"secret:mail:01ACC".to_string())
        );
        assert!(report.removed.contains(&ats));
        assert_eq!(report.data_dir, data);

        #[cfg(unix)]
        assert!(report.db_removed, "on unix an open file can be unlinked");
        #[cfg(unix)]
        assert!(!data.join("boba.db").exists());
    }

    /// AC5: typing the wrong confirmation changes nothing.
    #[test]
    fn a_wrong_confirmation_changes_nothing() {
        let dir = temp();
        let store = store(dir.path());
        let snapshots = dir.path().join("snapshots");
        std::fs::create_dir_all(&snapshots).expect("snapshots");
        touch(&snapshots.join("pre-1-2-1700000000.db"));
        keys::secret_put(SERVICE, "mail:01KEEP", "token").expect("secret");

        for wrong in ["", "delete", "DELETE everything", "yes"] {
            assert!(matches!(
                reset(&store, SERVICE, &[], wrong),
                Err(ResetError::WrongConfirmation)
            ));
        }

        assert!(snapshots.exists());
        assert_eq!(
            keys::secret_get(SERVICE, "mail:01KEEP")
                .expect("get")
                .as_deref(),
            Some("token")
        );
    }
}
