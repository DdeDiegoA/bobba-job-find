//! `SecretPort` — OS-keychain reads/writes for OAuth tokens, app passwords and
//! API keys (ADR-006, ADR-009, AD-13, FR-40). Adapter: `boba-store`, which wraps
//! the one `keyring-core` module (`boba_store::keys`) — there is never a second
//! keychain wrapper in the tree.
//!
//! The DB stores an opaque `secret_ref`; the value lives only in the keychain,
//! is never logged and never exported. A `SecretRef` is app-generated, so a
//! reference outside [`valid_reference`] is a programming error, not user input.

use thiserror::Error;

/// A reference is a short ASCII token: no control characters, no whitespace, and
/// no leading `-` (which some backends read as an option).
pub fn valid_reference(reference: &str) -> bool {
    !reference.is_empty()
        && reference.len() <= 128
        && !reference.starts_with('-')
        && reference
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-.:".contains(&b))
}

/// Keychain failures. `Keychain` strings never contain a secret value.
#[derive(Debug, Clone, Error)]
pub enum SecretError {
    /// The stored reference is not a valid keychain entry name.
    #[error("invalid secret reference")]
    InvalidReference,
    /// No OS keychain is usable (headless Linux without Secret Service).
    #[error("no OS keychain is available on this machine")]
    NoKeychain,
    /// The keychain itself failed (platform error, access denied).
    #[error("keychain error")]
    Keychain,
}

/// The keychain surface the core depends on (AD-13). Values are addressed by the
/// opaque reference stored in the DB.
pub trait SecretPort: Send + Sync {
    /// Store (or replace) the value for `reference`.
    fn put(&self, reference: &str, value: &str) -> Result<(), SecretError>;

    /// The value for `reference`, `None` when nothing is stored.
    fn get(&self, reference: &str) -> Result<Option<String>, SecretError>;

    /// Remove `reference`; returns whether an entry was removed.
    fn delete(&self, reference: &str) -> Result<bool, SecretError>;
}
