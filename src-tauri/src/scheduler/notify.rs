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

/// The `jobs.json` message for `locale`, by its `jobs:<key>` (or bare `<key>`) key.
pub fn message(locale: Locale, key: &str) -> Option<&'static str> {
    let key = key.strip_prefix("jobs:").unwrap_or(key);
    catalog(locale).get(key).map(String::as_str)
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
        let (title, body) = match notification {
            Notification::LockedSearch => (text(locale, &LOCKED_TITLE), text(locale, &LOCKED_BODY)),
        };
        self.app
            .notification()
            .builder()
            .title(title)
            .body(body)
            .show()
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
    fn the_entries_point_at_real_catalog_keys() {
        for entry in [LOCKED_TITLE, LOCKED_BODY] {
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
