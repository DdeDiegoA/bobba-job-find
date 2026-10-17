//! The one-shot "closing the window keeps Boba in the tray" explanation (AC1).
//!
//! The flag is a marker file rather than a settings row: the window can be
//! closed while the encrypted store is locked (idle timeout, ADR-009), and this
//! is a UI notice, not configuration.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// The marker file name inside the app config dir.
pub const MARKER: &str = "close-to-tray-explained";

/// The message keys of the explanation, in `locales/{en,es}/tray.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Notice {
    /// The dialog title.
    pub message_key: &'static str,
    /// The dialog body: why Boba stayed, and how to really quit.
    pub body_message_key: &'static str,
}

/// What Boba says the first time the window is closed (AC1).
pub const NOTICE: Notice = Notice {
    message_key: "tray:close.title",
    body_message_key: "tray:close.body",
};

/// Whether the explanation still owes the user a word.
#[derive(Debug)]
pub struct CloseNotice {
    /// Where the marker lives; `None` when the app config dir is unavailable.
    path: Option<PathBuf>,
    claimed: AtomicBool,
}

impl CloseNotice {
    /// A notice remembered by a marker file at `path`.
    pub fn new(path: PathBuf) -> Self {
        Self {
            path: Some(path),
            claimed: AtomicBool::new(false),
        }
    }

    /// A notice inside the app config dir `dir`.
    pub fn in_dir(dir: &Path) -> Self {
        Self::new(dir.join(MARKER))
    }

    /// A notice that cannot be remembered: it explains once per launch, which is
    /// what is left when the config dir is unavailable.
    pub fn ephemeral() -> Self {
        Self {
            path: None,
            claimed: AtomicBool::new(false),
        }
    }

    /// Whether to explain now. True at most once per launch, and never again
    /// once a marker is on disk — a marker that cannot be written is logged and
    /// degrades to once per launch rather than repeating on every close.
    pub fn claim(&self) -> bool {
        if self.claimed.swap(true, Ordering::SeqCst) {
            return false;
        }
        let Some(path) = &self.path else {
            return true;
        };
        if path.exists() {
            return false;
        }
        if let Err(error) = remember(path) {
            tracing::warn!(%error, path = %path.display(), "close-to-tray marker not written");
        }
        true
    }
}

/// Leave the marker behind, creating the config dir when this is the first run.
fn remember(path: &Path) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, b"1")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One temp dir per call: parallel tests in this process must not share one.
    fn temp_dir(tag: &str) -> PathBuf {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("boba-tray-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// AC1: the explanation is shown the first time the window is closed, and
    /// never twice in one launch.
    #[test]
    fn the_explanation_is_claimed_once() {
        let notice = CloseNotice::in_dir(&temp_dir("once"));
        assert!(notice.claim(), "the first close explains");
        assert!(!notice.claim(), "the second close does not");
        assert!(!notice.claim());
    }

    /// AC1: an explanation already given (marker from an earlier launch) is not
    /// repeated after a restart.
    #[test]
    fn a_marker_from_an_earlier_launch_silences_it() {
        let dir = temp_dir("earlier");
        std::fs::write(dir.join(MARKER), b"1").expect("seed the marker");
        assert!(!CloseNotice::in_dir(&dir).claim());
    }

    /// A marker that cannot be written (read-only or missing config dir) must not
    /// panic and must not nag on every close.
    #[test]
    fn an_unwritable_marker_explains_once_per_launch() {
        let dir = temp_dir("unwritable");
        let blocker = dir.join("not-a-dir");
        std::fs::write(&blocker, b"file, not a directory").expect("seed");
        let notice = CloseNotice::new(blocker.join(MARKER));

        assert!(notice.claim(), "an unwritable marker still explains once");
        assert!(!notice.claim(), "and not again in the same launch");
    }

    /// With no config dir to remember the marker, the explanation is still the
    /// first one of the launch.
    #[test]
    fn an_ephemeral_notice_explains_once_per_launch() {
        let notice = CloseNotice::ephemeral();
        assert!(notice.claim());
        assert!(!notice.claim());
    }

    /// AC1: the title and the body resolve in both shipped locales, and differ.
    #[test]
    fn the_notice_is_translated_in_en_and_es() {
        use crate::tray::menu;
        use boba_app::i18n::Locale;

        for key in [NOTICE.message_key, NOTICE.body_message_key] {
            let en = menu::message(Locale::En, key);
            let es = menu::message(Locale::Es, key);
            assert!(en.is_some(), "en/tray.json is missing {key}");
            assert!(es.is_some(), "es/tray.json is missing {key}");
            assert_ne!(en, es, "{key} is not translated");
        }
    }
}
