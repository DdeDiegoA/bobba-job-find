//! The native-notification adapter for [`NotifierPort`] (S5.1, AD-18, AD-21).
//!
//! The scheduler owns one instance, built from the app handle alone: a slot that
//! is skipped because the store is locked must notify even when the port list is
//! not mounted (ADR-009 §3). The wording lives in `locales/{en,es}/jobs.json` and
//! is embedded at compile time, the same way the tray embeds its menu labels, so
//! a notification reads the same in both languages (AD-21).

use std::collections::HashMap;
use std::sync::OnceLock;

use boba_app::i18n::Locale;
use serde_json::Value;
use tauri::{AppHandle, Runtime};
use tauri_plugin_notification::NotificationExt;

use boba_app::digest::settings::SETTINGS_KEY;
use boba_app::digest::{EventKind, NotificationSettings};
use boba_app::ports::clock::ClockPort;
use boba_app::ports::notifier::{Notification, NotifierError, NotifierPort};
use boba_app::ports::repo::system::SystemRepo;

const EN: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../locales/en/jobs.json"
));
const ES: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../locales/es/jobs.json"
));
const OUTBOX_EN: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../locales/en/outbox.json"
));
const OUTBOX_ES: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../locales/es/outbox.json"
));
const NUDGES_EN: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../locales/en/nudges.json"
));
const NUDGES_ES: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../locales/es/nudges.json"
));

/// One notification's catalog entry: its `jobs:<key>` message key, in the UI's
/// spelling, so `scripts/check-i18n.mjs` sees every key the core emits (AD-21).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub message_key: &'static str,
}

/// The texts of the one notification this slice sends (AC4, ADR-009 §3).
pub const LOCKED_TITLE: Entry = Entry {
    message_key: "jobs:locked.title",
};
pub const LOCKED_BODY: Entry = Entry {
    message_key: "jobs:locked.body",
};
/// The Session Board pause notice (S11.1a, FR-30): the Source stops, the user
/// decides when to resume it.
pub const SESSION_CHALLENGE_TITLE: Entry = Entry {
    message_key: "jobs:session_challenge.title",
};
pub const SESSION_CHALLENGE_BODY: Entry = Entry {
    message_key: "jobs:session_challenge.body",
};
/// The Source health notice (S4.8 AC3, FR-31): three failures in a row, one
/// notification.
pub const SOURCE_FAILING_TITLE: Entry = Entry {
    message_key: "jobs:source_failing.title",
};
pub const SOURCE_FAILING_BODY: Entry = Entry {
    message_key: "jobs:source_failing.body",
};
/// The top-matches notice (S5.2, FR-46): a run found Strong or Good Offers.
/// The body's plural is the catalog's: `body_one` for a single match.
pub const TOP_MATCHES_TITLE: Entry = Entry {
    message_key: "jobs:top_matches.title",
};
pub const TOP_MATCHES_BODY_ONE: Entry = Entry {
    message_key: "jobs:top_matches.body_one",
};
pub const TOP_MATCHES_BODY_MANY: Entry = Entry {
    message_key: "jobs:top_matches.body_many",
};

/// The Outbox hold notice (S9.7, FR-85): the wording lives in `outbox.json`,
/// the slice that owns the feature.
pub const OUTBOX_TITLE: Entry = Entry {
    message_key: "outbox:notification.title",
};
pub const OUTBOX_BODY: Entry = Entry {
    message_key: "outbox:notification.body",
};

/// The `jobs.json` message for `locale`, by its `jobs:<key>` (or bare `<key>`) key.
pub fn message(locale: Locale, key: &str) -> Option<&'static str> {
    let key = key.strip_prefix("jobs:").unwrap_or(key);
    catalog(locale).get(key).map(String::as_str)
}

/// The `outbox.json` message for `locale`, by its `outbox:<key>` key.
pub fn outbox_message(locale: Locale, key: &str) -> Option<&'static str> {
    let key = key.strip_prefix("outbox:").unwrap_or(key);
    outbox_catalog(locale).get(key).map(String::as_str)
}

/// The follow-up nudge notice (S9.5, FR-95): the wording lives in `nudges.json`,
/// the slice that owns the feature.
pub const NUDGE_CHECK_IN_TITLE: Entry = Entry {
    message_key: "nudges:notification.check_in.title",
};
pub const NUDGE_CHECK_IN_BODY: Entry = Entry {
    message_key: "nudges:notification.check_in.body",
};
pub const NUDGE_NO_RESPONSE_TITLE: Entry = Entry {
    message_key: "nudges:notification.no_response.title",
};
pub const NUDGE_NO_RESPONSE_BODY: Entry = Entry {
    message_key: "nudges:notification.no_response.body",
};

/// The `nudges.json` message for `locale`, by its `nudges:<key>` key.
pub fn nudges_message(locale: Locale, key: &str) -> Option<&'static str> {
    let key = key.strip_prefix("nudges:").unwrap_or(key);
    nudges_catalog(locale).get(key).map(String::as_str)
}

/// The label of a nudge entry in `locale`; the key is the fallback.
pub fn nudges_text(locale: Locale, entry: &Entry) -> &'static str {
    nudges_message(locale, entry.message_key).unwrap_or(entry.message_key)
}

fn nudges_catalog(locale: Locale) -> &'static HashMap<String, String> {
    static CATALOG_EN: OnceLock<HashMap<String, String>> = OnceLock::new();
    static CATALOG_ES: OnceLock<HashMap<String, String>> = OnceLock::new();
    match locale {
        Locale::En => CATALOG_EN.get_or_init(|| parse(NUDGES_EN, "en")),
        Locale::Es => CATALOG_ES.get_or_init(|| parse(NUDGES_ES, "es")),
    }
}

/// The label of an outbox entry in `locale`; the key is the fallback.
pub fn outbox_text(locale: Locale, entry: &Entry) -> &'static str {
    outbox_message(locale, entry.message_key).unwrap_or(entry.message_key)
}

fn outbox_catalog(locale: Locale) -> &'static HashMap<String, String> {
    static CATALOG_EN: OnceLock<HashMap<String, String>> = OnceLock::new();
    static CATALOG_ES: OnceLock<HashMap<String, String>> = OnceLock::new();
    match locale {
        Locale::En => CATALOG_EN.get_or_init(|| parse(OUTBOX_EN, "en")),
        Locale::Es => CATALOG_ES.get_or_init(|| parse(OUTBOX_ES, "es")),
    }
}

/// The label of `entry` in `locale`; the key itself is the fallback, so a text
/// is never empty. A missing translation is a test failure, not a runtime one.
pub fn text(locale: Locale, entry: &Entry) -> &'static str {
    message(locale, entry.message_key).unwrap_or(entry.message_key)
}

fn catalog(locale: Locale) -> &'static HashMap<String, String> {
    static CATALOG_EN: OnceLock<HashMap<String, String>> = OnceLock::new();
    static CATALOG_ES: OnceLock<HashMap<String, String>> = OnceLock::new();
    match locale {
        Locale::En => CATALOG_EN.get_or_init(|| parse(EN, "en")),
        Locale::Es => CATALOG_ES.get_or_init(|| parse(ES, "es")),
    }
}

fn parse(raw: &str, lang: &str) -> HashMap<String, String> {
    let json: Value = serde_json::from_str(raw)
        .unwrap_or_else(|error| panic!("locales/{lang}/jobs.json is not valid JSON: {error}"));
    json.as_object()
        .into_iter()
        .flatten()
        .map(|(key, value)| {
            let text = value
                .as_str()
                .unwrap_or_else(|| panic!("locales/{lang}/jobs.json: {key} is not a string"));
            (key.clone(), text.to_owned())
        })
        .collect()
}

/// Whether the user wants this notification at this hour (FR-107, S5.3).
///
/// Two gates, in this order: the events they chose to hear about, then the
/// quiet hours they set. The security notices — a locked store, a paused
/// session board — pass both (orquestador, 2026-10-17): a user who forgot to
/// whitelist the night must still hear that Boba is stuck. A store that cannot
/// be read (still locked) reads as the defaults, which notify: a silent app is
/// the bug.
fn allows(settings: &NotificationSettings, notification: &Notification, hour: u8) -> bool {
    match notification {
        Notification::LockedSearch | Notification::SessionChallengePaused => true,
        Notification::OutboxHold => settings.allows(EventKind::Outbox) && awake(settings, hour),
        Notification::SourceFailing => settings.allows(EventKind::Errors) && awake(settings, hour),
        // A host without the digest port (S5.3) still shows the plain notice,
        // under the same "the daily digest" choice. Follow-up nudges (S9.5) are
        // the same everyday news, so the same switch and the same quiet hours
        // hold them.
        Notification::TopMatches { .. } | Notification::FollowUpNudge { .. } => {
            settings.allows(EventKind::Digest) && awake(settings, hour)
        }
    }
}

/// Whether `hour` is outside the quiet window (or the user keeps notifying in
/// it). The digest applies the same rule in `boba_app::digest`; this is the
/// other notifier's half of AC2.
fn awake(settings: &NotificationSettings, hour: u8) -> bool {
    !settings.quiet_hours.covers(hour) || settings.quiet_hours.notify_during
}

/// The stored notification settings; the defaults when the store is not open.
fn stored_settings<R: Runtime>(app: &AppHandle<R>) -> NotificationSettings {
    let raw = crate::wiring::port::<dyn SystemRepo>(app)
        .ok()
        .and_then(|repo| repo.setting_get(SETTINGS_KEY).ok().flatten());
    NotificationSettings::from_json(raw.as_deref())
}

/// The [`NotifierPort`] adapter: one native notification through the OS.
pub struct TauriNotifier<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> TauriNotifier<R> {
    pub fn new(app: AppHandle<R>) -> Self {
        Self { app }
    }
}

impl<R: Runtime> NotifierPort for TauriNotifier<R> {
    fn notify(&self, notification: Notification) -> Result<(), NotifierError> {
        let hour = boba_app::digest::local_hour(&boba_app::system::SystemClock.now_rfc3339());
        if !allows(&stored_settings(&self.app), &notification, hour) {
            return Ok(());
        }
        let locale = crate::lifecycle::ui_locale(&self.app);
        let (title, body): (&str, String) = match notification {
            Notification::LockedSearch => (
                text(locale, &LOCKED_TITLE),
                text(locale, &LOCKED_BODY).to_string(),
            ),
            Notification::SessionChallengePaused => (
                text(locale, &SESSION_CHALLENGE_TITLE),
                text(locale, &SESSION_CHALLENGE_BODY).to_string(),
            ),
            Notification::SourceFailing => (
                text(locale, &SOURCE_FAILING_TITLE),
                text(locale, &SOURCE_FAILING_BODY).to_string(),
            ),
            Notification::OutboxHold => (
                outbox_text(locale, &OUTBOX_TITLE),
                outbox_text(locale, &OUTBOX_BODY).to_string(),
            ),
            Notification::TopMatches { count } => {
                let entry = if count == 1 {
                    &TOP_MATCHES_BODY_ONE
                } else {
                    &TOP_MATCHES_BODY_MANY
                };
                (
                    text(locale, &TOP_MATCHES_TITLE),
                    // `{count}` is the only placeholder these entries carry;
                    // a plain replace keeps the catalog readable in both
                    // languages without an ICU runtime in the core.
                    text(locale, entry).replace("{count}", &count.to_string()),
                )
            }
            Notification::FollowUpNudge {
                title,
                company,
                days,
                no_response,
            } => {
                let (title_entry, body_entry) = if no_response {
                    (&NUDGE_NO_RESPONSE_TITLE, &NUDGE_NO_RESPONSE_BODY)
                } else {
                    (&NUDGE_CHECK_IN_TITLE, &NUDGE_CHECK_IN_BODY)
                };
                (
                    nudges_text(locale, title_entry),
                    nudges_text(locale, body_entry)
                        .replace("{title}", &title)
                        .replace("{company}", &company)
                        .replace("{days}", &days.to_string()),
                )
            }
        };
        // A host without the notification plugin panics inside Tauri's accessor
        // rather than returning an error; a missing toast never kills the tick.
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.app
                .notification()
                .builder()
                .title(title)
                .body(body)
                .show()
        }))
        .map_err(|_| NotifierError::new("the notification plugin is not available"))?
        .map_err(|error| NotifierError::new(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_languages_resolve_the_locked_notification() {
        for (locale, title, body) in [
            (Locale::En, "locked.title", "locked.body"),
            (Locale::Es, "locked.title", "locked.body"),
        ] {
            let title = message(locale, title).expect("title in jobs.json");
            let body = message(locale, body).expect("body in jobs.json");
            assert!(!title.is_empty());
            assert!(!body.is_empty());
        }
    }

    #[test]
    fn the_catalog_key_keeps_its_namespace_prefix() {
        assert_eq!(
            message(Locale::En, "jobs:locked.title"),
            message(Locale::En, "locked.title")
        );
        assert!(message(Locale::En, "jobs:does.not.exist").is_none());
    }

    #[test]
    fn both_languages_resolve_the_outbox_hold_notification() {
        for locale in [Locale::En, Locale::Es] {
            for entry in [OUTBOX_TITLE, OUTBOX_BODY] {
                let text = outbox_message(locale, entry.message_key);
                assert!(
                    text.is_some_and(|text| !text.is_empty()),
                    "{locale:?} is missing {}",
                    entry.message_key
                );
            }
        }
    }

    /// S5.3 (FR-107): the user's choices gate the events they can silence —
    /// and never the security ones, which also ignore quiet hours.
    #[test]
    fn the_users_choices_gate_what_they_can_silence() {
        use boba_app::digest::NotificationSettings;

        let mut off = NotificationSettings::default();
        off.events.outbox = false;
        off.events.errors = false;
        off.events.digest = false;

        // Silenced when asked.
        assert!(!allows(&off, &Notification::OutboxHold, 10));
        assert!(!allows(&off, &Notification::SourceFailing, 10));
        assert!(!allows(&off, &Notification::TopMatches { count: 3 }, 10));
        assert!(!allows(&off, &nudge(), 10));

        // Never silenced: a locked store and a paused board need the user.
        assert!(allows(&off, &Notification::LockedSearch, 10));
        assert!(allows(&off, &Notification::SessionChallengePaused, 10));

        // Everything on by default.
        let on = NotificationSettings::default();
        for notification in [
            Notification::OutboxHold,
            Notification::SourceFailing,
            Notification::TopMatches { count: 1 },
            nudge(),
        ] {
            assert!(allows(&on, &notification, 10));
        }
    }

    /// S5.3 AC2 (review F3): quiet hours silence the non-security notices too,
    /// not only the digest — and never the security ones.
    #[test]
    fn quiet_hours_silence_the_silenceable_notices() {
        use boba_app::digest::NotificationSettings;

        let mut night = NotificationSettings::default();
        night.quiet_hours.enabled = true;
        night.quiet_hours.from = 22;
        night.quiet_hours.to = 7;

        assert!(!allows(&night, &Notification::OutboxHold, 23));
        assert!(!allows(&night, &Notification::SourceFailing, 2));
        assert!(!allows(&night, &Notification::TopMatches { count: 2 }, 6));
        assert!(!allows(&night, &nudge(), 23));
        assert!(allows(&night, &Notification::OutboxHold, 9));

        // Security notices are exempt from quiet hours.
        assert!(allows(&night, &Notification::LockedSearch, 23));
        assert!(allows(&night, &Notification::SessionChallengePaused, 23));

        // And a user who asked to keep hearing Boba at night still does.
        night.quiet_hours.notify_during = true;
        assert!(allows(&night, &Notification::OutboxHold, 23));
    }

    /// The S9.5 nudge notice, as the sweep hands it over.
    fn nudge() -> Notification {
        Notification::FollowUpNudge {
            title: "Backend Engineer".into(),
            company: "Nimbus".into(),
            days: 12,
            no_response: false,
        }
    }

    /// S9.5: both nudge notices resolve in both languages.
    #[test]
    fn both_languages_resolve_the_follow_up_nudges() {
        for locale in [Locale::En, Locale::Es] {
            for entry in [
                NUDGE_CHECK_IN_TITLE,
                NUDGE_CHECK_IN_BODY,
                NUDGE_NO_RESPONSE_TITLE,
                NUDGE_NO_RESPONSE_BODY,
            ] {
                assert!(
                    nudges_message(locale, entry.message_key).is_some_and(|text| !text.is_empty()),
                    "{locale:?} is missing {}",
                    entry.message_key
                );
            }
        }
    }

    #[test]
    fn the_entries_point_at_real_catalog_keys() {
        for entry in [
            LOCKED_TITLE,
            LOCKED_BODY,
            SESSION_CHALLENGE_TITLE,
            SESSION_CHALLENGE_BODY,
            SOURCE_FAILING_TITLE,
            SOURCE_FAILING_BODY,
            TOP_MATCHES_TITLE,
            TOP_MATCHES_BODY_ONE,
            TOP_MATCHES_BODY_MANY,
        ] {
            assert!(
                message(Locale::En, entry.message_key).is_some(),
                "en is missing {}",
                entry.message_key
            );
            assert!(
                message(Locale::Es, entry.message_key).is_some(),
                "es is missing {}",
                entry.message_key
            );
        }
    }
}
