//! Session-board vocabulary shared across the `SessionStore` port (S11.1a,
//! FR-30, AD-9, AD-14).
//!
//! Pure: no IO. Each Session Board Source browses in its own Chromium profile;
//! Chrome protects that profile's cookies with the platform keystore — macOS
//! Keychain, Windows DPAPI, Linux keyring — never with Boba's database key.
//! "Sign out / clear session" deletes exactly one Source's profile.

/// Failures of a Source's browser-profile folder.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionError {
    /// The id would not stay one directory (separator, traversal, case, length).
    #[error("invalid source id '{0}'")]
    InvalidSourceId(String),
    /// Deleting or reading the profile folder failed.
    #[error("session store failed: {0}")]
    Io(String),
    /// A browser still has this Source's profile open. Deleting a live profile
    /// leaves Chrome, and the cookies it rewrites on exit, in an undefined
    /// state, so "Sign out / clear session" waits for the window to close.
    #[error("a browser is still using source '{source_id}' (pid {pid})")]
    InUse { source_id: String, pid: u32 },
    /// No Chromium-family browser is installed.
    #[error("no Chromium-family browser was found")]
    NoBrowser,
}

/// What `clear` did: whether a profile existed and was removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClearReport {
    pub source_id: String,
    pub removed: bool,
}

/// How Chrome protects this platform's cookie store (FR-30 AC2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookieProtection {
    /// Chrome's key lives in the macOS Keychain.
    MacosKeychain,
    /// Chrome encrypts cookies with Windows DPAPI (user-bound).
    WindowsDpapi,
    /// Chrome's key lives in the login keyring (Secret Service).
    LinuxKeyring,
    /// No keyring: Chrome falls back to weaker, machine-local protection.
    LinuxWeak,
}

impl CookieProtection {
    /// Stable wire name the UI maps to its localized sentence.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MacosKeychain => "macos_keychain",
            Self::WindowsDpapi => "windows_dpapi",
            Self::LinuxKeyring => "linux_keyring",
            Self::LinuxWeak => "linux_weak",
        }
    }

    /// Whether the user must give a second, separate acknowledgement (AC2).
    pub fn requires_extra_ack(self) -> bool {
        matches!(self, Self::LinuxWeak)
    }
}
