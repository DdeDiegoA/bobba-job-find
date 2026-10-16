//! DB key custody (AD-13, ADR-004, ADR-009).
//!
//! The SQLCipher key is a random 256-bit value stored only in the OS keychain.
//! It is never written to disk, and a new key is minted **only** when no
//! `boba.db` exists — an existing database with an unreadable key is the Locked
//! state, never a silent re-key (ADR-009).

use std::path::Path;
use std::sync::Arc;

use keyring_core::{CredentialStore, Entry};
use thiserror::Error;
use zeroize::Zeroizing;

use boba_app::ports::secret::{SecretError, SecretPort, valid_reference};

/// Keychain service for real builds (ADR-004).
pub const SERVICE: &str = "app.bobajobfind";
/// Separate keychain service for `tauri dev` builds, so local runs never touch a
/// user's real store (ADR-009).
pub const DEV_SERVICE: &str = "app.bobajobfind.dev";
/// Keychain entry name (ADR-004).
pub const DB_KEY_ENTRY: &str = "db-key";

const KEY_LEN: usize = 32;

/// Key-custody failures. `Locked` is the only recoverable one, and it is the
/// entry point of story 1.2b's unlock screen.
#[derive(Debug, Error)]
pub enum KeyError {
    /// A database exists but its key cannot be read; the caller must enter the
    /// Locked state and must not mint a new key or a new database (ADR-009).
    #[error("the database exists but its key cannot be read: locked state required (ADR-009)")]
    Locked,
    /// No OS keychain is usable (headless Linux without Secret Service). Story
    /// 1.2b wires the Argon2id passphrase fallback here.
    #[error("no OS keychain is available on this machine")]
    NoKeychain,
    /// The keychain itself failed (platform error, access denied).
    #[error("keychain error: {0}")]
    Keychain(String),
    /// The stored value is not a 32-byte hex key.
    #[error("stored database key is malformed")]
    InvalidKey,
    /// The OS random source failed while minting a key.
    #[error("could not generate a random key: {0}")]
    Rng(String),
}

/// A 256-bit database key, zeroized on drop and never displayed.
pub struct DbKey(Zeroizing<[u8; KEY_LEN]>);

impl std::fmt::Debug for DbKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DbKey(<redacted 256-bit>)")
    }
}

impl DbKey {
    /// Mint a fresh key from the OS CSPRNG.
    pub fn generate() -> Result<Self, KeyError> {
        let mut bytes = [0u8; KEY_LEN];
        getrandom::fill(&mut bytes).map_err(|e| KeyError::Rng(e.to_string()))?;
        Ok(Self(Zeroizing::new(bytes)))
    }

    /// Decode a 64-char hex key.
    pub fn from_hex(hex: &str) -> Result<Self, KeyError> {
        let hex = hex.trim();
        if hex.len() != KEY_LEN * 2 {
            return Err(KeyError::InvalidKey);
        }
        let mut bytes = [0u8; KEY_LEN];
        for (i, ch) in hex.bytes().enumerate() {
            let nibble = (ch as char).to_digit(16).ok_or(KeyError::InvalidKey)? as u8;
            if i % 2 == 0 {
                bytes[i / 2] = nibble << 4;
            } else {
                bytes[i / 2] |= nibble;
            }
        }
        Ok(Self(Zeroizing::new(bytes)))
    }

    /// Lowercase hex, zeroized on drop. Hex (not raw bytes) because some
    /// Secret Service backends only store UTF-8 secrets.
    pub fn hex(&self) -> Zeroizing<String> {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(KEY_LEN * 2);
        for byte in self.0.iter() {
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0f) as usize] as char);
        }
        Zeroizing::new(out)
    }

    /// Raw key bytes, for SQLCipher and re-keying a backup (S10.2).
    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }
}

/// Entry-name prefix for the secrets a slice stores besides the DB key and the
/// audit anchor (AD-13): one namespace, so a reference can never collide with
/// `db-key` or `audit-head`.
pub const SECRET_ENTRY_PREFIX: &str = "secret";

/// The keychain entry name for one secret reference.
fn secret_entry_name(reference: &str) -> String {
    format!("{SECRET_ENTRY_PREFIX}:{reference}")
}

fn secret_entry(service: &str, reference: &str) -> Result<Entry, KeyError> {
    install_platform_store()?;
    Entry::new(service, &secret_entry_name(reference))
        .map_err(|e| KeyError::Keychain(e.to_string()))
}

/// Store one secret (OAuth tokens, API key) for `reference` (FR-40). The value
/// is written to the keychain and nowhere else.
pub fn secret_put(service: &str, reference: &str, value: &str) -> Result<(), KeyError> {
    secret_entry(service, reference)?
        .set_password(value)
        .map_err(|e| KeyError::Keychain(e.to_string()))
}

/// Read one secret, `None` when nothing is stored under `reference`.
pub fn secret_get(service: &str, reference: &str) -> Result<Option<String>, KeyError> {
    match secret_entry(service, reference)?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring_core::Error::NoEntry) => Ok(None),
        Err(e) => Err(KeyError::Keychain(e.to_string())),
    }
}

/// Remove one secret; returns whether an entry was removed (FR-39).
pub fn secret_delete(service: &str, reference: &str) -> Result<bool, KeyError> {
    match secret_entry(service, reference)?.delete_credential() {
        Ok(()) => Ok(true),
        Err(keyring_core::Error::NoEntry) => Ok(false),
        Err(e) => Err(KeyError::Keychain(e.to_string())),
    }
}

/// The [`SecretPort`] adapter over this module (AD-13, grant 2026-10-16).
///
/// It is a thin scope over [`secret_put`]/[`secret_get`]/[`secret_delete`]: the
/// keychain is reached through this module only, so there is exactly one
/// `keyring-core` site in the tree. Errors carry no payload — a keychain error
/// message can quote stored bytes, so none of it ever crosses a port.
pub struct SecretStore {
    service: String,
}

impl SecretStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }
}

fn secret_error(error: KeyError) -> SecretError {
    match error {
        KeyError::NoKeychain => SecretError::NoKeychain,
        _ => SecretError::Keychain,
    }
}

impl SecretPort for SecretStore {
    fn put(&self, reference: &str, value: &str) -> Result<(), SecretError> {
        if !valid_reference(reference) {
            return Err(SecretError::InvalidReference);
        }
        secret_put(&self.service, reference, value).map_err(secret_error)
    }

    fn get(&self, reference: &str) -> Result<Option<String>, SecretError> {
        if !valid_reference(reference) {
            return Err(SecretError::InvalidReference);
        }
        secret_get(&self.service, reference).map_err(secret_error)
    }

    fn delete(&self, reference: &str) -> Result<bool, SecretError> {
        if !valid_reference(reference) {
            return Err(SecretError::InvalidReference);
        }
        secret_delete(&self.service, reference).map_err(secret_error)
    }
}

/// Register the platform credential store, once per process. If a default store
/// is already installed (a test mock, or an earlier call) this is a no-op.
pub fn install_platform_store() -> Result<(), KeyError> {
    if keyring_core::get_default_store().is_some() {
        return Ok(());
    }
    keyring_core::set_default_store(platform_store()?);
    Ok(())
}

/// The one mock credential store a **test binary** installs.
///
/// Every test helper routes through it (`keys` and `repos::audit`), so the
/// process-wide default is set exactly once and never replaced: a second
/// installer used to swap the store under a running test, and one reached the
/// real macOS keychain. Tests install this before any other keychain call, so
/// [`install_platform_store`] never opens a platform store from a test (S8.1).
#[cfg(test)]
pub(crate) fn use_mock_keychain() -> Arc<CredentialStore> {
    static STORE: std::sync::OnceLock<Arc<CredentialStore>> = std::sync::OnceLock::new();
    STORE
        .get_or_init(|| {
            let store: Arc<CredentialStore> =
                keyring_core::mock::Store::new().expect("mock keychain");
            keyring_core::set_default_store(Arc::clone(&store));
            store
        })
        .clone()
}

#[cfg(target_os = "macos")]
fn platform_store() -> Result<Arc<CredentialStore>, KeyError> {
    let store: Arc<CredentialStore> = apple_native_keyring_store::keychain::Store::new()
        .map_err(|e| KeyError::Keychain(e.to_string()))?;
    Ok(store)
}

#[cfg(target_os = "windows")]
fn platform_store() -> Result<Arc<CredentialStore>, KeyError> {
    let store: Arc<CredentialStore> = windows_native_keyring_store::Store::new()
        .map_err(|e| KeyError::Keychain(e.to_string()))?;
    Ok(store)
}

#[cfg(target_os = "linux")]
fn platform_store() -> Result<Arc<CredentialStore>, KeyError> {
    let store: Arc<CredentialStore> = dbus_secret_service_keyring_store::Store::new()
        .map_err(|e| KeyError::Keychain(e.to_string()))?;
    Ok(store)
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn platform_store() -> Result<Arc<CredentialStore>, KeyError> {
    Err(KeyError::NoKeychain)
}

/// Load the DB key, minting one on first run.
///
/// * `db_path` exists and the key reads -> that key.
/// * `db_path` exists and the key is missing or unreadable -> [`KeyError::Locked`].
///   It never mints a new key or a new database over existing data (ADR-009).
/// * `db_path` does not exist and no key is stored -> mint and store one.
pub fn load_or_create(db_path: &Path, service: &str) -> Result<DbKey, KeyError> {
    let entry = Entry::new(service, DB_KEY_ENTRY).map_err(|e| KeyError::Keychain(e.to_string()))?;
    let db_exists = db_path.exists();

    if db_exists {
        return match entry.get_password() {
            Ok(hex) => DbKey::from_hex(&hex),
            Err(_) => Err(KeyError::Locked),
        };
    }

    match entry.get_password() {
        Ok(hex) => DbKey::from_hex(&hex),
        Err(keyring_core::Error::NoEntry) => {
            let key = DbKey::generate()?;
            entry
                .set_password(key.hex().as_str())
                .map_err(|e| KeyError::Keychain(e.to_string()))?;
            Ok(key)
        }
        Err(keyring_core::Error::NoStorageAccess(_)) => Err(KeyError::NoKeychain),
        Err(e) => Err(KeyError::Keychain(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole test binary shares one mock keychain (see `use_mock_keychain`);
    /// this module's tests never touch a real platform store (S8.1).
    fn store() -> SecretStore {
        use_mock_keychain();
        SecretStore::new("app.bobajobfind.test")
    }

    #[test]
    fn secret_round_trips_and_deletes() {
        let secrets = store();
        assert_eq!(secrets.get("case:1").expect("get"), None);

        secrets
            .put("case:1", "{\"access_token\":\"a\"}")
            .expect("put");
        assert_eq!(
            secrets.get("case:1").expect("get").as_deref(),
            Some("{\"access_token\":\"a\"}")
        );

        // A second reference never sees the first value.
        assert_eq!(secrets.get("case:2").expect("get"), None);

        assert!(secrets.delete("case:1").expect("delete"));
        assert!(!secrets.delete("case:1").expect("delete again"));
        assert_eq!(secrets.get("case:1").expect("get"), None);
    }

    #[test]
    fn secrets_share_one_entry_namespace_of_their_own() {
        // The DB key and the audit anchor keep their own names: a reference can
        // never address them (AD-13, one wrapper).
        assert_eq!(secret_entry_name("db-key"), "secret:db-key");
        assert_ne!(secret_entry_name(DB_KEY_ENTRY), DB_KEY_ENTRY);
    }

    #[test]
    fn malformed_references_are_refused_before_any_store_call() {
        let secrets = store();
        for bad in [
            "",
            " ",
            "-flag",
            "with space",
            "line\nbreak",
            &"x".repeat(129),
        ] {
            assert!(matches!(
                secrets.put(bad, "v"),
                Err(SecretError::InvalidReference)
            ));
            assert!(matches!(
                secrets.get(bad),
                Err(SecretError::InvalidReference)
            ));
            assert!(matches!(
                secrets.delete(bad),
                Err(SecretError::InvalidReference)
            ));
        }
        assert!(valid_reference("mail:01J0ACC"));
    }

    #[test]
    fn a_secret_error_never_carries_the_value() {
        let error = secret_error(KeyError::Keychain(
            "platform failure while writing hunter2-token".to_string(),
        ));
        let rendered = format!("{error} {error:?}");
        // The adapter maps to payload-free variants, so no keychain detail —
        // and therefore no stored bytes — can reach a log or the UI.
        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert_eq!(rendered, "keychain error Keychain");
    }

    #[test]
    fn no_keychain_maps_to_the_recoverable_variant() {
        assert!(matches!(
            secret_error(KeyError::NoKeychain),
            SecretError::NoKeychain
        ));
        assert!(matches!(
            secret_error(KeyError::Locked),
            SecretError::Keychain
        ));
    }
}
