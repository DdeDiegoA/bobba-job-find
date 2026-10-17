//! The digest a finished run announces (S5.3, FR-45, FR-107).
//!
//! One run, at most one notification (FR-45): the counts of what it ranked, the
//! top matches worth naming, and the decisions the user made about their
//! notifications. Everything here is pure: the caller hands the run's numbers
//! and the local hour, and gets back either the digest to show or nothing at
//! all. Showing it and naming it in the UI locale belong to the adapter.

pub mod settings;

use serde::{Deserialize, Serialize};
pub use settings::{EventKind, NotificationSettings, QuietHours};

use crate::ports::notifier::NotifierError;

/// One Offer the digest may name (FR-45: the top of what the run found).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct TopMatch {
    pub title: String,
    pub company: String,
}

/// What a finished run hands the digest (FR-45).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestInput {
    /// New Offers the run ranked Strong.
    pub strong: u32,
    /// New Offers the run ranked Good.
    pub good: u32,
    /// Sources the run queried, failed ones included.
    pub sources_checked: u32,
    /// The top matches worth naming, best first; the digest caps the list.
    pub top: Vec<TopMatch>,
    /// The user's local hour (0–23) when the run ended, for quiet hours.
    pub hour: u8,
}

/// The one notification a finished run shows (FR-45).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Digest {
    pub strong: u32,
    pub good: u32,
    pub sources_checked: u32,
    /// The top matches to name, already capped to the user's choice.
    pub top: Vec<TopMatch>,
}

/// Decide whether this run speaks, and with what (FR-45, FR-107).
///
/// Silence when the user switched the digest off, when the run found nothing
/// new and they did not ask to hear about empty runs, or during quiet hours
/// they didn't opt out of. Otherwise the counts, and at most as many top
/// matches as they chose to see.
pub fn decide(settings: &NotificationSettings, input: DigestInput) -> Option<Digest> {
    if !settings.allows(EventKind::Digest) {
        return None;
    }
    if input.strong + input.good == 0 && !settings.notify_when_empty {
        return None;
    }
    if settings.quiet_hours.covers(input.hour) && !settings.quiet_hours.notify_during {
        return None;
    }
    Some(Digest {
        strong: input.strong,
        good: input.good,
        sources_checked: input.sources_checked,
        top: input
            .top
            .into_iter()
            .take(usize::from(settings.top_matches_shown))
            .collect(),
    })
}

/// The user's local hour at `now` (UTC RFC 3339), for quiet hours.
pub fn local_hour(now: &str) -> u8 {
    (crate::apply::rules::local_minute(now) / 60) as u8
}

/// The one way a digest reaches the OS: the adapter in `src-tauri` names it in
/// the UI locale and shows it. Kept beside the decision it renders so the two
/// cannot drift (FR-45).
pub trait DigestPort: Send + Sync {
    /// Show `digest` now. Failing to show one is never fatal: the run report is
    /// already stored, and the notification is the courtesy on top of it.
    fn show(&self, digest: &Digest) -> Result<(), NotifierError>;
}

/// Decide, then show. Returns what was shown (for callers that log it), and
/// swallows a port error the way every notification does (FR-45).
pub fn announce(
    port: Option<&dyn DigestPort>,
    settings: &NotificationSettings,
    input: DigestInput,
) -> Option<Digest> {
    let digest = decide(settings, input)?;
    if let Some(port) = port {
        let _ = port.show(&digest);
    }
    Some(digest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn defaults() -> NotificationSettings {
        NotificationSettings::default()
    }

    fn top(title: &str, company: &str) -> TopMatch {
        TopMatch {
            title: title.to_string(),
            company: company.to_string(),
        }
    }

    fn input(strong: u32, good: u32, hour: u8) -> DigestInput {
        DigestInput {
            strong,
            good,
            sources_checked: 4,
            top: vec![top("Rust Engineer", "Acme"), top("Backend Dev", "Globex")],
            hour,
        }
    }

    #[test]
    fn a_run_with_new_matches_summarises_the_bands() {
        let digest = decide(&defaults(), input(3, 5, 10)).expect("a digest");
        assert_eq!(digest.strong, 3);
        assert_eq!(digest.good, 5);
        assert_eq!(digest.sources_checked, 4);
    }

    #[test]
    fn an_empty_run_stays_quiet_by_default() {
        assert!(decide(&defaults(), input(0, 0, 10)).is_none());
    }

    #[test]
    fn an_empty_run_notifies_when_the_user_asked_for_it() {
        let mut settings = defaults();
        settings.notify_when_empty = true;
        let digest = decide(&settings, input(0, 0, 10)).expect("the user asked for it");
        assert_eq!((digest.strong, digest.good), (0, 0));
    }

    #[test]
    fn quiet_hours_suppress_a_digest() {
        let mut settings = defaults();
        settings.quiet_hours.enabled = true;
        assert!(decide(&settings, input(3, 5, 23)).is_none());
    }

    #[test]
    fn quiet_hours_that_wrap_midnight_cover_late_and_early_hours() {
        let mut settings = defaults();
        settings.quiet_hours.enabled = true;
        assert!(decide(&settings, input(3, 5, 0)).is_none());
        assert!(decide(&settings, input(3, 5, 6)).is_none());
    }

    #[test]
    fn quiet_hours_that_do_not_cover_the_hour_let_the_digest_through() {
        let mut settings = defaults();
        settings.quiet_hours.enabled = true;
        assert!(decide(&settings, input(3, 5, 8)).is_some());
    }

    #[test]
    fn a_user_who_keeps_notifications_during_quiet_hours_still_gets_it() {
        let mut settings = defaults();
        settings.quiet_hours.enabled = true;
        settings.quiet_hours.notify_during = true;
        assert!(decide(&settings, input(3, 5, 23)).is_some());
    }

    #[test]
    fn the_digest_names_at_most_the_top_matches_the_user_chose() {
        let mut settings = defaults();
        settings.top_matches_shown = 1;
        let digest = decide(&settings, input(3, 5, 10)).expect("a digest");
        assert_eq!(digest.top, [top("Rust Engineer", "Acme")]);
    }

    #[test]
    fn a_user_who_wants_no_names_gets_the_counts_alone() {
        let mut settings = defaults();
        settings.top_matches_shown = 0;
        let digest = decide(&settings, input(3, 5, 10)).expect("a digest");
        assert!(digest.top.is_empty());
    }

    #[test]
    fn digest_off_means_no_digest() {
        let mut settings = defaults();
        settings.events.digest = false;
        assert!(decide(&settings, input(3, 5, 10)).is_none());
    }

    /// A port that keeps what it was asked to show, and can refuse.
    #[derive(Default)]
    struct RecordingPort {
        shown: Mutex<Vec<Digest>>,
        refuses: bool,
    }

    impl DigestPort for RecordingPort {
        fn show(&self, digest: &Digest) -> Result<(), NotifierError> {
            self.shown.lock().unwrap().push(digest.clone());
            if self.refuses {
                return Err(NotifierError::new("the OS said no"));
            }
            Ok(())
        }
    }

    #[test]
    fn a_run_with_news_shows_one_digest() {
        let port = RecordingPort::default();
        let shown = announce(Some(&port), &defaults(), input(3, 5, 10));
        assert_eq!(shown.as_ref().map(|digest| digest.strong), Some(3));
        assert_eq!(port.shown.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_suppressed_digest_never_reaches_the_os() {
        let port = RecordingPort::default();
        let mut settings = defaults();
        settings.quiet_hours.enabled = true;
        assert!(announce(Some(&port), &settings, input(3, 5, 23)).is_none());
        assert!(port.shown.lock().unwrap().is_empty());
    }

    #[test]
    fn a_port_that_refuses_never_fails_the_run() {
        let port = RecordingPort {
            refuses: true,
            ..RecordingPort::default()
        };
        assert!(announce(Some(&port), &defaults(), input(3, 5, 10)).is_some());
    }

    #[test]
    fn a_host_without_a_port_decides_but_shows_nothing() {
        assert!(announce(None, &defaults(), input(3, 5, 10)).is_some());
    }
}
