//! The unlock flow (S1.2b, ADR-009): the Argon2id passphrase fallback and the
//! in-memory key session.
//!
//! Two custody paths exist (ADR-004, ADR-013):
//!
//! * **Keychain** (default): a random 256-bit key, minted only when no `boba.db`
//!   exists ([`crate::keys::load_or_create`]). Nothing here is involved.
//! * **Passphrase** (headless Linux without Secret Service): the user types a
//!   passphrase and the DB key is `Argon2id(passphrase, salt)`.
//!
//! The derived key is **never written to disk**. What lives next to the database
//! is the KDF record — salt and parameters only, never key material — so the same
//! passphrase derives the same key on the next launch. A database whose key did
//! not come from this path has no record, and the passphrase path refuses it: it
//! never mints a key over existing data (ADR-009 §2).
//!
//! A derived key is held for the login session only, in zeroized memory, and the
//! session's copy is dropped on idle timeout, quit or logout ([`KeySession`]).
//! The open store keeps its own copy until quit (see ADR-013 §Consequences): what
//! closing the session really does is close the gate every port lookup goes
//! through, so an idle app can no longer be driven without the passphrase.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use argon2::{Algorithm, Argon2, Params, Version};
use thiserror::Error;
use zeroize::Zeroizing;

use crate::keys::{DbKey, KeyError};

/// Default idle timeout for the passphrase key (ADR-009 §3).
pub const DEFAULT_IDLE: Duration = Duration::from_secs(8 * 60 * 60);

/// KDF record version. A record this build cannot read is refused, never guessed.
const KDF_VERSION: u32 = 1;

/// Argon2id parameters (OWASP-recommended: 19 MiB, 2 passes, 1 lane).
const M_COST: u32 = 19 * 1024;
const T_COST: u32 = 2;
const P_COST: u32 = 1;

/// Salt length in bytes; the Argon2id minimum is 8.
const SALT_LEN: usize = 16;

/// Parameter range accepted from the record. A record is a local file, so a
/// tampered one must not be able to make unlock allocate until the machine dies:
/// 1 GiB of memory, 16 passes and 8 lanes are far above anything this app writes.
/// A record outside the range is refused, never clamped: clamping would derive a
/// different key and lock the user out of data that is otherwise fine.
const MIN_M_COST: u32 = 8;
const MAX_M_COST: u32 = 1024 * 1024;
const MAX_T_COST: u32 = 16;
const MAX_P_COST: u32 = 8;

/// Key length in bytes (AD-13: 256-bit).
const KEY_LEN: usize = 32;

/// A passphrase in memory, zeroized when dropped (NFR-2).
///
/// The command wraps Tauri's decoded `String` in one immediately, so the
/// passphrase never sits in a buffer nobody clears.
pub struct Passphrase(Zeroizing<String>);

impl Passphrase {
    /// Take ownership of a decoded passphrase.
    pub fn new(secret: impl Into<String>) -> Self {
        Self(Zeroizing::new(secret.into()))
    }

    /// The passphrase, for the KDF.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// A passphrase-path failure.
#[derive(Debug, Error)]
pub enum UnlockError {
    /// The key could not be derived or the KDF record could not be read.
    #[error("could not derive the database key: {0}")]
    Kdf(String),
    /// The database exists but its key never came from a passphrase: there is no
    /// KDF record, and minting one would produce a key that cannot open it.
    #[error("this database was not created with a passphrase (ADR-009)")]
    NoPassphrase,
    /// The passphrase does not derive the key this database was written with.
    #[error("the passphrase does not open this database")]
    WrongPassphrase,
    /// The store could not be opened with the derived key (composition root).
    #[error("the store could not be opened: {0}")]
    Store(String),
    /// A file operation on the KDF record failed.
    #[error("kdf record error: {0}")]
    Io(#[from] std::io::Error),
    /// The key is unusable (malformed).
    #[error(transparent)]
    Key(#[from] KeyError),
}

impl UnlockError {
    /// The i18n key the UI shows (AD-3: keys cross IPC, never raw text).
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::NoPassphrase => "errors.unlock.no_passphrase",
            Self::WrongPassphrase => "errors.unlock.wrong_passphrase",
            Self::Store(_) => "errors.unlock.store",
            Self::Kdf(_) | Self::Io(_) | Self::Key(_) => "errors.unlock.kdf",
        }
    }
}

/// The KDF record stored next to the database.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct KdfRecord {
    version: u32,
    kdf: String,
    /// Salt in lowercase hex. Not a secret: it only makes the derivation unique.
    salt: String,
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
    output_len: u32,
}

/// Path of the KDF record for `db_path` (`boba.db` → `boba.db.kdf`).
pub fn kdf_path(db_path: &Path) -> PathBuf {
    let mut name = db_path.as_os_str().to_owned();
    name.push(".kdf");
    PathBuf::from(name)
}

/// Whether `db_path`'s key came from a passphrase (a KDF record exists).
pub fn kdf_exists(db_path: &Path) -> bool {
    kdf_path(db_path).is_file()
}

/// Derive the database key from `passphrase`.
///
/// On a first run (`no boba.db` yet) this writes the KDF record. Otherwise the
/// record must already exist: a database whose key came from the keychain has
/// none, and deriving one from a fresh salt would produce a key that can never
/// open it (ADR-009 §2 — never mint a key over existing data).
pub fn passphrase_key(db_path: &Path, passphrase: &Passphrase) -> Result<DbKey, UnlockError> {
    let record = match std::fs::read_to_string(kdf_path(db_path)) {
        Ok(raw) => parse_record(&raw)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if db_path.exists() {
                return Err(UnlockError::NoPassphrase);
            }
            let record = new_record()?;
            write_record(&kdf_path(db_path), &record)?;
            record
        }
        Err(error) => return Err(UnlockError::Io(error)),
    };
    derive(&record, passphrase)
}

/// Prove that `key` opens `db_path`, before anything is mounted or unlocked.
///
/// A missing store has nothing to prove (the key would create it). The proof is
/// the one [`crate::db`] itself makes on open: a wrong key fails on the first
/// read of `sqlite_master` with "file is not a database" (ADR-009).
pub fn verify_key(db_path: &Path, key: &DbKey) -> Result<(), UnlockError> {
    if !db_path.exists() {
        return Ok(());
    }
    let conn = rusqlite::Connection::open(db_path).map_err(|e| UnlockError::Kdf(e.to_string()))?;
    conn.execute_batch(&format!("PRAGMA key = \"x'{}'\";", key.hex().as_str()))
        .map_err(|e| UnlockError::Kdf(e.to_string()))?;
    match conn.query_row("SELECT count(*) FROM sqlite_master", [], |row| {
        row.get::<_, i64>(0)
    }) {
        Ok(_) => Ok(()),
        Err(rusqlite::Error::SqliteFailure(fault, _))
            if fault.code == rusqlite::ErrorCode::NotADatabase =>
        {
            Err(UnlockError::WrongPassphrase)
        }
        Err(error) => Err(UnlockError::Kdf(error.to_string())),
    }
}

/// Read and check a record. A record this build cannot read is refused, never
/// guessed: guessing would derive a key that cannot open the store.
fn parse_record(raw: &str) -> Result<KdfRecord, UnlockError> {
    let record: KdfRecord = serde_json::from_str(raw)
        .map_err(|e| UnlockError::Kdf(format!("unreadable kdf record: {e}")))?;
    if record.version != KDF_VERSION {
        return Err(UnlockError::Kdf(format!(
            "kdf record version {} is not supported",
            record.version
        )));
    }
    if record.kdf != "argon2id" || record.output_len != KEY_LEN as u32 {
        return Err(UnlockError::Kdf("unexpected kdf or key length".to_string()));
    }
    if !(MIN_M_COST..=MAX_M_COST).contains(&record.m_cost)
        || record.t_cost == 0
        || record.t_cost > MAX_T_COST
        || record.p_cost == 0
        || record.p_cost > MAX_P_COST
    {
        return Err(UnlockError::Kdf(format!(
            "kdf parameters out of range (m={}, t={}, p={})",
            record.m_cost, record.t_cost, record.p_cost
        )));
    }
    salt_bytes(&record.salt)?;
    Ok(record)
}

/// A fresh record: random salt, no key material (ADR-004 — the key is derived).
fn new_record() -> Result<KdfRecord, UnlockError> {
    let mut salt = [0u8; SALT_LEN];
    getrandom::fill(&mut salt).map_err(|e| UnlockError::Kdf(format!("no random source: {e}")))?;
    Ok(KdfRecord {
        version: KDF_VERSION,
        kdf: "argon2id".to_string(),
        salt: to_hex(&salt).to_string(),
        m_cost: M_COST,
        t_cost: T_COST,
        p_cost: P_COST,
        output_len: KEY_LEN as u32,
    })
}

/// Write the record owner-only from its first byte, and never over an existing
/// one: the salt is what makes a passphrase reproducible, so it is written once.
/// `create_new` is also the guard against minting a record while another process
/// is doing the same.
fn write_record(path: &Path, record: &KdfRecord) -> Result<(), UnlockError> {
    use std::io::Write as _;

    let json = serde_json::to_string_pretty(record).map_err(|e| UnlockError::Kdf(e.to_string()))?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(json.as_bytes())?;
    // The record is what makes the passphrase path reproducible: make it durable
    // before the store it belongs to is created.
    file.sync_all()?;
    Ok(())
}

/// Derive the 256-bit key, holding the raw bytes zeroized and converting them
/// through the hex form the keychain path already uses ([`DbKey::from_hex`]).
fn derive(record: &KdfRecord, passphrase: &Passphrase) -> Result<DbKey, UnlockError> {
    let params = Params::new(record.m_cost, record.t_cost, record.p_cost, Some(KEY_LEN))
        .map_err(|e| UnlockError::Kdf(e.to_string()))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let salt = salt_bytes(&record.salt)?;

    let mut derived = Zeroizing::new([0u8; KEY_LEN]);
    argon2
        .hash_password_into(passphrase.as_str().as_bytes(), &salt, &mut *derived)
        .map_err(|e| UnlockError::Kdf(e.to_string()))?;
    Ok(DbKey::from_hex(to_hex(&*derived).as_str())?)
}

fn salt_bytes(hex: &str) -> Result<[u8; SALT_LEN], UnlockError> {
    let bad = || UnlockError::Kdf("malformed salt in the kdf record".to_string());
    if hex.len() != SALT_LEN * 2 {
        return Err(bad());
    }
    let mut salt = [0u8; SALT_LEN];
    for (i, pair) in hex.as_bytes().chunks(2).enumerate() {
        let hi = (pair[0] as char).to_digit(16).ok_or_else(bad)? as u8;
        let lo = (pair[1] as char).to_digit(16).ok_or_else(bad)? as u8;
        salt[i] = (hi << 4) | lo;
    }
    Ok(salt)
}

/// Lowercase hex, zeroized on drop: the only form key material takes in memory
/// here (the same form [`DbKey`] itself speaks).
fn to_hex(bytes: &[u8]) -> Zeroizing<String> {
    use std::fmt::Write as _;
    let mut out = Zeroizing::new(String::with_capacity(bytes.len() * 2));
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// The passphrase-derived key of this login session (ADR-009 §3).
///
/// * [`KeySession::peek`] / [`KeySession::poll`] report whether gated work may
///   proceed. A session that was never unlocked is *idle*: the passphrase path is
///   not in play (a keychain-opened store), so nothing is gated.
/// * Once unlocked, the key is held here and dropped when the idle timeout
///   elapses, or on [`KeySession::lock`] (quit, logout, explicit lock). A dropped
///   key never comes back: the user types the passphrase again, and until then
///   every port lookup reports the typed Locked error. The open store keeps its
///   own copy of the key until quit (ADR-013 §Consequences).
pub struct KeySession {
    idle: Duration,
    state: std::sync::Mutex<Session>,
}

enum Session {
    Idle,
    Unlocked {
        /// Held for the session and zeroized when the session locks. The open
        /// store keeps its own copy, so the value is never read — holding it here
        /// is what makes "dropped on idle timeout, quit or logout" true.
        #[allow(dead_code)]
        key: DbKey,
        last_use: Instant,
    },
    Locked,
}

impl KeySession {
    /// A session with the given idle timeout, not yet unlocked.
    pub fn new(idle: Duration) -> Self {
        Self {
            idle,
            state: std::sync::Mutex::new(Session::Idle),
        }
    }

    /// Hold `key` for this session.
    pub fn unlock(&self, key: DbKey) {
        *self.held() = Session::Unlocked {
            key,
            last_use: Instant::now(),
        };
    }

    /// Drop the key now (quit, logout).
    pub fn lock(&self) {
        *self.held() = Session::Locked;
    }

    /// Report whether gated work may proceed, **without** counting as activity
    /// (status reads must not keep a forgotten session alive).
    pub fn peek(&self) -> bool {
        self.peek_at(Instant::now())
    }

    /// Report whether gated work may proceed, counting this call as activity.
    pub fn poll(&self) -> bool {
        self.poll_at(Instant::now())
    }

    fn peek_at(&self, now: Instant) -> bool {
        self.usable(now, false)
    }

    fn poll_at(&self, now: Instant) -> bool {
        self.usable(now, true)
    }

    /// A poisoned lock means a panic elsewhere while the key was held: the safe
    /// answer is the closed session, so the poison is recovered, not propagated.
    fn held(&self) -> std::sync::MutexGuard<'_, Session> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn usable(&self, now: Instant, touch: bool) -> bool {
        let mut session = self.held();
        let idle_for = match &*session {
            // No passphrase path in play (a keychain store): nothing is gated.
            Session::Idle => return true,
            Session::Locked => return false,
            Session::Unlocked { last_use, .. } => now.saturating_duration_since(*last_use),
        };
        if idle_for >= self.idle {
            // Dropping the enum here zeroizes the held key (zeroize's Drop).
            *session = Session::Locked;
            return false;
        }
        if touch && let Session::Unlocked { last_use, .. } = &mut *session {
            *last_use = now;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::db::{Store, StoreConfig, StoreError};

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp dir")
    }

    fn config(dir: &Path) -> StoreConfig {
        StoreConfig::new(dir.join("boba.db"), dir.join("snapshots"))
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).expect("read")
    }

    /// AC1: the passphrase creates the store, and the next launch reopens it
    /// unattended with the same passphrase (no key on disk).
    #[test]
    fn the_passphrase_creates_the_store_and_reopens_it() {
        let dir = dir();
        let config = config(dir.path());

        let key = passphrase_key(&config.db_path, &Passphrase::new("correct horse battery"))
            .expect("derive");
        let hex = key.hex();
        assert!(kdf_exists(&config.db_path), "the KDF record is written");
        drop(Store::open_with_key(config.clone(), key).expect("store opens"));

        // Next launch, no keychain: the recall is the passphrase, nothing else.
        let again = passphrase_key(&config.db_path, &Passphrase::new("correct horse battery"))
            .expect("derive again");
        assert_eq!(
            again.hex().as_str(),
            hex.as_str(),
            "same passphrase, same key"
        );
        drop(Store::open_with_key(config, again).expect("store reopens"));
    }

    /// ADR-009 §2: a wrong passphrase changes nothing and never opens the store.
    #[test]
    fn another_passphrase_never_opens_the_store() {
        let dir = dir();
        let config = config(dir.path());
        let key = passphrase_key(&config.db_path, &Passphrase::new("correct horse battery"))
            .expect("derive");
        drop(Store::open_with_key(config.clone(), key).expect("store opens"));
        let record = read(&kdf_path(&config.db_path));

        let wrong =
            passphrase_key(&config.db_path, &Passphrase::new("tr0ub4dor&3")).expect("derive");
        assert!(matches!(
            Store::open_with_key(config.clone(), wrong),
            Err(StoreError::Locked)
        ));
        // The salt is untouched, so the key cannot have changed under the user.
        assert_eq!(read(&kdf_path(&config.db_path)), record);
    }

    /// AD-13/ADR-004: the record holds the salt and parameters, never the key.
    #[test]
    fn the_kdf_record_never_holds_the_key() {
        let dir = dir();
        let config = config(dir.path());
        let key = passphrase_key(&config.db_path, &Passphrase::new("correct horse battery"))
            .expect("derive");

        let record = read(&kdf_path(&config.db_path));
        assert!(
            !record.contains(key.hex().as_str()),
            "the key reached the disk"
        );
        assert_eq!(
            record.matches("\"salt\"").count(),
            1,
            "the salt is what is stored"
        );
        let parsed: KdfRecord = serde_json::from_str(&record).expect("record parses");
        assert_eq!(parsed.version, KDF_VERSION);
        assert_eq!(parsed.kdf, "argon2id");
        assert_eq!(parsed.salt.len(), SALT_LEN * 2);
        assert_eq!(parsed.m_cost, M_COST);
        assert_eq!(parsed.output_len, KEY_LEN as u32);
    }

    /// Owner-only on unix, like the pre-migration snapshot (AD-20).
    #[cfg(unix)]
    /// A tampered record is refused before it can be expensive: a local file must
    /// not be able to make unlock allocate gigabytes or spin for hours.
    #[test]
    fn a_tampered_kdf_record_is_refused_before_any_derivation() {
        let dir = dir();
        let config = config(dir.path());
        passphrase_key(&config.db_path, &Passphrase::new("correct horse battery")).expect("derive");
        let path = kdf_path(&config.db_path);
        let record: serde_json::Value = serde_json::from_str(&read(&path)).expect("record parses");

        let tampered: [(&str, serde_json::Value); 6] = [
            ("version", serde_json::json!(99)),
            ("kdf", serde_json::json!("pbkdf2")),
            ("output_len", serde_json::json!(16)),
            ("m_cost", serde_json::json!(MAX_M_COST + 1)),
            ("t_cost", serde_json::json!(MAX_T_COST + 1)),
            ("p_cost", serde_json::json!(0)),
        ];
        for (field, value) in tampered {
            let mut next = record.clone();
            next[field] = value;
            std::fs::write(&path, next.to_string()).expect("write");
            assert!(
                matches!(
                    passphrase_key(&config.db_path, &Passphrase::new("correct horse battery")),
                    Err(UnlockError::Kdf(_))
                ),
                "{field} was accepted"
            );
        }

        // A truncated salt is refused too, and the range accepts what we write.
        let mut next = record.clone();
        next["salt"] = serde_json::json!("00");
        std::fs::write(&path, next.to_string()).expect("write");
        assert!(
            passphrase_key(&config.db_path, &Passphrase::new("correct horse battery")).is_err()
        );

        std::fs::write(&path, record.to_string()).expect("write");
        passphrase_key(&config.db_path, &Passphrase::new("correct horse battery"))
            .expect("the real record still works");
    }

    /// The record is written once and never replaced: a new salt would derive a
    /// key that opens nothing, so it is not even written under an existing store.
    #[test]
    fn an_existing_record_is_never_replaced() {
        let dir = dir();
        let config = config(dir.path());
        passphrase_key(&config.db_path, &Passphrase::new("correct horse battery")).expect("derive");
        let path = kdf_path(&config.db_path);

        // A record this build refuses stays exactly as the user's file was.
        std::fs::write(&path, "{\"version\":99}").expect("tamper");
        assert!(
            passphrase_key(&config.db_path, &Passphrase::new("correct horse battery")).is_err()
        );
        assert_eq!(read(&path), "{\"version\":99}");

        // And nothing is minted over the database that record belonged to.
        drop(Store::open_with_key(config.clone(), DbKey::generate().expect("key")).expect("opens"));
        let before = read(&path);
        assert!(
            passphrase_key(&config.db_path, &Passphrase::new("correct horse battery")).is_err()
        );
        assert_eq!(read(&path), before);
    }

    #[test]
    fn the_kdf_record_is_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = dir();
        let config = config(dir.path());
        passphrase_key(&config.db_path, &Passphrase::new("correct horse battery")).expect("derive");

        let mode = std::fs::metadata(kdf_path(&config.db_path))
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    /// ADR-009 §2: a keychain-created database has no record, so the passphrase
    /// path refuses it instead of deriving a key that cannot open it.
    #[test]
    fn an_existing_database_without_a_record_refuses_the_passphrase() {
        let dir = dir();
        let config = config(dir.path());
        drop(Store::open_with_key(config.clone(), DbKey::generate().expect("key")).expect("opens"));
        let before = std::fs::metadata(&config.db_path).expect("metadata").len();

        assert!(matches!(
            passphrase_key(&config.db_path, &Passphrase::new("correct horse battery")),
            Err(UnlockError::NoPassphrase)
        ));
        assert!(!kdf_exists(&config.db_path), "no record was minted");
        assert_eq!(
            std::fs::metadata(&config.db_path).expect("metadata").len(),
            before
        );
    }

    /// The store is verified before it is unlocked or mounted.
    #[test]
    fn verify_accepts_the_right_key_and_refuses_a_wrong_one() {
        let dir = dir();
        let config = config(dir.path());
        let key = passphrase_key(&config.db_path, &Passphrase::new("correct horse battery"))
            .expect("derive");
        // Nothing to verify yet: the store does not exist.
        verify_key(&config.db_path, &key).expect("a missing store is not a refusal");
        drop(Store::open_with_key(config.clone(), key).expect("store opens"));

        let right = passphrase_key(&config.db_path, &Passphrase::new("correct horse battery"))
            .expect("derive");
        verify_key(&config.db_path, &right).expect("the right key is proven");
        assert!(matches!(
            verify_key(&config.db_path, &DbKey::generate().expect("key")),
            Err(UnlockError::WrongPassphrase)
        ));
    }

    /// AC1: the key is held for the login session only and dropped on idle.
    #[test]
    fn the_session_holds_the_key_until_the_idle_timeout() {
        let idle = Duration::from_secs(60);
        let session = KeySession::new(idle);
        let start = Instant::now();

        // No passphrase path in play: nothing is gated.
        assert!(session.peek_at(start), "a keychain store is never gated");
        assert!(session.peek_at(start + idle * 100));

        session.unlock(DbKey::generate().expect("key"));
        let first_use = start + idle - Duration::from_secs(1);
        assert!(session.poll_at(first_use));
        // The poll above was activity: the window starts again from there.
        assert!(session.peek_at(first_use + idle - Duration::from_secs(1)));
        assert!(!session.peek_at(first_use + idle), "idle drops the key");
        assert!(
            !session.poll_at(first_use + idle * 2),
            "and it stays dropped"
        );

        session.unlock(DbKey::generate().expect("key"));
        assert!(session.poll_at(Instant::now()));
    }

    /// Quit and logout drop the key now, whatever the idle window says.
    #[test]
    fn locking_drops_the_key_immediately() {
        let session = KeySession::new(Duration::from_secs(60));
        session.unlock(DbKey::generate().expect("key"));
        session.lock();
        assert!(!session.peek());
        assert!(!session.poll());
    }
}
