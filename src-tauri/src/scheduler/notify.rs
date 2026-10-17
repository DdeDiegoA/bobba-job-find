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

use boba_app::ports::notifier::{Notification, NotifierError, NotifierPort};

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
