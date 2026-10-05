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

/// Register the platform credential store, once per process. If a default store
/// is already installed (a test mock, or an earlier call) this is a no-op.
pub fn install_platform_store() -> Result<(), KeyError> {
    if keyring_core::get_default_store().is_some() {
        return Ok(());
    }
    keyring_core::set_default_store(platform_store()?);
    Ok(())
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
