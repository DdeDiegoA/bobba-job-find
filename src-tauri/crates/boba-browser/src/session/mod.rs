//! Session-board browser profiles (S11.1a, FR-30, AD-10).
//!
//! Each Session Board Source browses in **its own** dedicated Chrome profile
//! under `<app data>/browser-sessions/<source>`. Chrome protects that profile's
//! cookies with the platform keystore — macOS Keychain, Windows DPAPI, Linux
//! keyring — and never with Boba's database key. "Sign out / clear session"
//! deletes that one profile, cookies included; every other Source keeps its
//! session.
//!
//! The browser is driven exactly like the ATS form driver (S9.4): headed, over
//! an **ephemeral loopback** CDP port (`0`), never a fixed one.

use std::path::{Path, PathBuf};

use boba_app::ports::session::SessionStore;
use boba_domain::session::{ClearReport, CookieProtection, SessionError};

use crate::driver::{self, LaunchPlan};

/// The folder under the app data dir that holds every Source's profile.
pub const SESSIONS_DIR: &str = "browser-sessions";

/// Longest source id accepted as a directory name.
pub const MAX_SOURCE_ID_LEN: usize = 64;

/// The OS family the protection is resolved for (pure input for tests).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Macos,
    Windows,
    Linux,
    Other,
}

/// Resolve the protection for `os`, given whether a keyring is available.
pub fn protection_for(os: Os, keyring_available: bool) -> CookieProtection {
    match os {
        Os::Macos => CookieProtection::MacosKeychain,
        Os::Windows => CookieProtection::WindowsDpapi,
        Os::Linux if keyring_available => CookieProtection::LinuxKeyring,
        // No keyring (or an unknown OS): Chrome's fallback key is weak, so the
        // user must acknowledge it separately (AC2).
        Os::Linux | Os::Other => CookieProtection::LinuxWeak,
    }
}

/// The protection of the machine this build runs on.
pub fn detect_cookie_protection() -> CookieProtection {
    if cfg!(target_os = "macos") {
        protection_for(Os::Macos, true)
    } else if cfg!(target_os = "windows") {
        protection_for(Os::Windows, true)
    } else if cfg!(target_os = "linux") {
        protection_for(Os::Linux, keyring_available())
    } else {
        protection_for(Os::Other, false)
    }
}

/// Whether a keyring is available to hold Chrome's cookie key.
///
/// The app installs its platform credential store at unlock (S1.2a); on Linux
/// that store only exists when a Secret Service answered, which is exactly the
/// condition Chrome needs. A passphrase-only unlock leaves no store, and the
/// platform then counts as keyring-less.
fn keyring_available() -> bool {
    keyring_core::get_default_store().is_some()
}

/// Whether `id` is safe as one directory name: lowercase ASCII, digits, `-`
/// and `_`, at most [`MAX_SOURCE_ID_LEN`] bytes. Real source ids are slugs.
pub fn valid_source_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_SOURCE_ID_LEN
        && id != "."
        && id != ".."
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_'))
}

/// `app_data/browser-sessions/<id>`; refuses an id that could leave the root.
pub fn profile_dir(app_data: &Path, source_id: &str) -> Result<PathBuf, SessionError> {
    if !valid_source_id(source_id) {
        return Err(SessionError::InvalidSourceId(source_id.to_string()));
    }
    Ok(app_data.join(SESSIONS_DIR).join(source_id))
}

/// The launch plan for one Source's session profile: headed, ephemeral port.
pub fn session_plan(profile: PathBuf, executable: PathBuf) -> LaunchPlan {
    LaunchPlan {
        executable,
        profile,
        // AD-10/§6.4: Chrome picks an ephemeral loopback port; Boba never
        // names a fixed one.
        cdp_port: 0,
        headed: true,
    }
}

/// Build the launch plan for `source_id` against the installed browser.
pub fn launch_plan(app_data: &Path, source_id: &str) -> Result<LaunchPlan, SessionError> {
    let profile = profile_dir(app_data, source_id)?;
    let executable = driver::chrome_executable().ok_or(SessionError::NoBrowser)?;
    Ok(session_plan(profile, executable))
}

/// Plain-filesystem [`SessionStore`]: one profile folder per Source.
pub struct ProfileSessions {
    app_data: PathBuf,
}

impl ProfileSessions {
    pub fn new(app_data: impl Into<PathBuf>) -> Self {
        Self {
            app_data: app_data.into(),
        }
    }
}

impl SessionStore for ProfileSessions {
    fn clear(&self, source_id: &str) -> Result<ClearReport, SessionError> {
        let dir = profile_dir(&self.app_data, source_id)?;
        let removed = match std::fs::symlink_metadata(&dir) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(SessionError::Io(error.to_string())),
            Ok(meta) => {
                // A symlink is removed as a link, never followed: `clear` must
                // never delete a directory the user pointed at.
                let result = if meta.file_type().is_symlink() {
                    std::fs::remove_file(&dir)
                } else {
                    std::fs::remove_dir_all(&dir)
                };
                result.map_err(|error| SessionError::Io(error.to_string()))?;
                true
            }
        };
        Ok(ClearReport {
            source_id: source_id.to_string(),
            removed,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "boba-session-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn every_source_gets_its_own_profile_under_the_sessions_root() {
        let root = Path::new("/data/boba");
        assert_eq!(
            profile_dir(root, "linkedin").unwrap(),
            Path::new("/data/boba/browser-sessions/linkedin")
        );
        assert_ne!(
            profile_dir(root, "linkedin").unwrap(),
            profile_dir(root, "indeed").unwrap(),
            "signing out of one Source must never touch another's cookies"
        );
    }

    #[test]
    fn a_source_id_that_could_leave_the_root_is_refused() {
        for bad in [
            "",
            ".",
            "..",
            "../linkedin",
            "a/b",
            "a\\b",
            "LinkedIn",
            "a b",
            "a\0b",
        ] {
            assert!(!valid_source_id(bad), "{bad:?} must be refused");
            assert!(
                profile_dir(Path::new("/data/boba"), bad).is_err(),
                "{bad:?} must not resolve to a path"
            );
        }
        let long = "a".repeat(MAX_SOURCE_ID_LEN + 1);
        assert!(!valid_source_id(&long));
        for good in ["linkedin", "indeed", "computrabajo", "occ-mx", "bumeran_ar"] {
            assert!(valid_source_id(good), "{good:?} is a source id");
        }
    }

    #[test]
    fn the_session_launch_plan_is_headed_on_the_source_profile_and_an_ephemeral_port() {
        let plan = session_plan(
            PathBuf::from("/data/boba/browser-sessions/linkedin"),
            PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
        );
        assert!(plan.headed, "the user watches the session (AD-10)");
        assert_eq!(
            plan.cdp_port, 0,
            "an ephemeral loopback CDP port, never a fixed one"
        );
        assert_eq!(
            plan.profile,
            Path::new("/data/boba/browser-sessions/linkedin")
        );
        // The plan is what the shared driver launches (S9.4).
        assert!(plan.executable.ends_with("Google Chrome"));
        let _ = driver::PROFILE_DIR;
    }

    #[test]
    fn clearing_one_session_removes_only_that_sources_profile() {
        let root = scratch("clear");
        let linkedin = root.join("browser-sessions").join("linkedin");
        let indeed = root.join("browser-sessions").join("indeed");
        std::fs::create_dir_all(&linkedin).unwrap();
        std::fs::create_dir_all(&indeed).unwrap();
        std::fs::write(linkedin.join("Cookies"), b"k=v").unwrap();
        std::fs::write(indeed.join("Cookies"), b"k=v").unwrap();

        let store = ProfileSessions::new(&root);
        let report = store.clear("linkedin").unwrap();

        assert_eq!(
            report,
            ClearReport {
                source_id: "linkedin".to_string(),
                removed: true
            }
        );
        assert!(!linkedin.exists(), "the source's cookies are gone");
        assert!(indeed.join("Cookies").exists(), "Indeed keeps its session");
        assert!(root.join("browser-sessions").exists(), "the root stays");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn clearing_twice_is_harmless() {
        let root = scratch("clear-twice");
        let store = ProfileSessions::new(&root);
        assert!(!store.clear("linkedin").unwrap().removed);
        assert!(!store.clear("linkedin").unwrap().removed);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn clearing_refuses_an_id_that_could_leave_the_root() {
        let root = scratch("clear-bad");
        std::fs::create_dir_all(root.join("browser-sessions")).unwrap();
        std::fs::write(root.join("browser-sessions").join("elsewhere"), b"x").unwrap();
        let store = ProfileSessions::new(&root);
        assert!(matches!(
            store.clear("../browser-sessions"),
            Err(SessionError::InvalidSourceId(_))
        ));
        assert!(root.join("browser-sessions").join("elsewhere").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn cookie_protection_is_platform_specific() {
        assert_eq!(
            protection_for(Os::Macos, false),
            CookieProtection::MacosKeychain
        );
        assert_eq!(
            protection_for(Os::Windows, false),
            CookieProtection::WindowsDpapi
        );
        assert_eq!(
            protection_for(Os::Linux, true),
            CookieProtection::LinuxKeyring
        );
        assert_eq!(
            protection_for(Os::Linux, false),
            CookieProtection::LinuxWeak
        );
        assert_eq!(protection_for(Os::Other, true), CookieProtection::LinuxWeak);
    }

    #[test]
    fn only_weak_linux_needs_the_extra_acknowledgement() {
        for protection in [
            CookieProtection::MacosKeychain,
            CookieProtection::WindowsDpapi,
            CookieProtection::LinuxKeyring,
        ] {
            assert!(!protection.requires_extra_ack(), "{protection:?}");
        }
        assert!(CookieProtection::LinuxWeak.requires_extra_ack());
    }

    #[test]
    fn the_protection_kind_has_a_stable_wire_name() {
        assert_eq!(CookieProtection::MacosKeychain.as_str(), "macos_keychain");
        assert_eq!(CookieProtection::WindowsDpapi.as_str(), "windows_dpapi");
        assert_eq!(CookieProtection::LinuxKeyring.as_str(), "linux_keyring");
        assert_eq!(CookieProtection::LinuxWeak.as_str(), "linux_weak");
    }
}
