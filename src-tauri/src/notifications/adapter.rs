//! The digest's adapter (S5.3, FR-45/FR-107): `boba_app::digest` decides, this
//! names the result in the UI locale and hands it to the OS.
//!
//! The wording lives in `locales/{en,es}/notifications.json` and is embedded at
//! compile time, the way the tray and the other notifications embed theirs
//! (AD-21). The body carries the first top match and how many more there are;
//! the counts are the title, so a glance is enough.

use std::collections::HashMap;
use std::sync::OnceLock;

use boba_app::digest::{Digest, DigestPort};
use boba_app::i18n::Locale;
use boba_app::ports::notifier::NotifierError;
use serde_json::Value;
use tauri::{AppHandle, Runtime};
use tauri_plugin_notification::NotificationExt;

const EN: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../locales/en/notifications.json"
));
const ES: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../locales/es/notifications.json"
));

/// One notification's catalog entry: its `notifications:<key>` message key, in
/// the UI's spelling, so `scripts/check-i18n.mjs` sees every key this core emits
/// (AD-21).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub message_key: &'static str,
}

/// The title with news: "3 Strong, 5 Good today" (FR-45).
pub const TITLE: Entry = Entry {
    message_key: "notifications:digest.title",
};
/// The title of an empty run the user asked to hear about.
pub const TITLE_EMPTY: Entry = Entry {
    message_key: "notifications:digest.title_empty",
};
/// The body naming the best match.
pub const BODY_TOP: Entry = Entry {
    message_key: "notifications:digest.body_top",
};
/// The body naming the best match and counting the rest.
pub const BODY_MORE: Entry = Entry {
    message_key: "notifications:digest.body_more",
};
/// The body of a digest that names no match.
pub const BODY_COUNTS: Entry = Entry {
    message_key: "notifications:digest.body_counts",
};
/// The body of an empty run.
pub const BODY_EMPTY: Entry = Entry {
    message_key: "notifications:digest.body_empty",
};

/// The `notifications.json` message for `locale`, by its `notifications:<key>`
/// (or bare `<key>`) key.
pub fn message(locale: Locale, key: &str) -> Option<&'static str> {
    let key = key.strip_prefix("notifications:").unwrap_or(key);
    catalog(locale).get(key).map(String::as_str)
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
    let json: Value = serde_json::from_str(raw).unwrap_or_else(|error| {
        panic!("locales/{lang}/notifications.json is not valid JSON: {error}")
    });
    json.as_object()
        .into_iter()
        .flatten()
        .map(|(key, value)| {
            let text = value.as_str().unwrap_or_else(|| {
                panic!("locales/{lang}/notifications.json: {key} is not a string")
            });
            (key.clone(), text.to_owned())
        })
        .collect()
}

/// The entry's text in `locale`; the key itself is the fallback, so a title is
/// never empty. A missing translation is a test failure, not a runtime one.
fn text(locale: Locale, entry: &Entry) -> &'static str {
    message(locale, entry.message_key).unwrap_or(entry.message_key)
}

/// Replace the `{name}` placeholders a catalog entry carries. The core has no
/// ICU runtime; these entries only ever interpolate counts and names, which is
/// exactly what a plain replace handles in both languages.
fn fill(template: &str, values: &[(&str, String)]) -> String {
    values
        .iter()
        .fold(template.to_string(), |text, (name, value)| {
            text.replace(&format!("{{{name}}}"), value)
        })
}

/// The title and body of `digest` in `locale` (FR-45).
pub fn texts(locale: Locale, digest: &Digest) -> (String, String) {
    let new = digest.strong + digest.good;
    if new == 0 {
        let body = fill(
            text(locale, &BODY_EMPTY),
            &[("sources", digest.sources_checked.to_string())],
        );
        return (text(locale, &TITLE_EMPTY).to_string(), body);
    }

    let title = fill(
        text(locale, &TITLE),
        &[
            ("strong", digest.strong.to_string()),
            ("good", digest.good.to_string()),
        ],
    );
    let body = match digest.top.split_first() {
        Some((first, [])) => fill(
            text(locale, &BODY_TOP),
            &[
                ("title", first.title.clone()),
                ("company", first.company.clone()),
            ],
        ),
        Some((first, rest)) => fill(
            text(locale, &BODY_MORE),
            &[
                ("title", first.title.clone()),
                ("company", first.company.clone()),
                ("more", rest.len().to_string()),
            ],
        ),
        None => text(locale, &BODY_COUNTS).to_string(),
    };
    (title, body)
}

/// The [`DigestPort`] adapter: one native notification, then the click that
/// opens Today (FR-45).
pub struct DigestNotifier<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> DigestNotifier<R> {
    pub fn new(app: AppHandle<R>) -> Self {
        Self { app }
    }
}

impl<R: Runtime> DigestPort for DigestNotifier<R> {
    fn show(&self, digest: &Digest) -> Result<(), NotifierError> {
        let locale = crate::lifecycle::ui_locale(&self.app);
        let (title, body) = texts(locale, digest);
        // A host without the notification plugin panics inside Tauri's accessor
        // rather than returning an error; a missing digest never kills the run.
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.app
                .notification()
                .builder()
                .title(title)
                .body(body)
                .show()
        }))
        .map_err(|_| NotifierError::new("the notification plugin is not available"))?
        .map_err(|error| NotifierError::new(error.to_string()))?;
        // The click that opens Today: the plugin reports none on desktop, so the
        // focus the OS brings to the window is the click (S5.3).
        super::arm_open_today(&self.app);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use boba_app::digest::{Digest, TopMatch};

    fn digest(strong: u32, good: u32) -> Digest {
        Digest {
            strong,
            good,
            sources_checked: 4,
            top: Vec::new(),
        }
    }

    #[test]
    fn both_languages_resolve_every_digest_text() {
        for locale in [Locale::En, Locale::Es] {
            for entry in [
                TITLE,
                TITLE_EMPTY,
                BODY_TOP,
                BODY_MORE,
                BODY_COUNTS,
                BODY_EMPTY,
            ] {
                assert!(
                    message(locale, entry.message_key).is_some_and(|text| !text.is_empty()),
                    "{locale:?} is missing {}",
                    entry.message_key
                );
            }
        }
    }

    #[test]
    fn the_title_counts_the_bands_in_the_ui_language() {
        let (title, _body) = texts(Locale::En, &digest(3, 5));
        assert_eq!(title, "3 Strong, 5 Good today");

        let (title, _body) = texts(Locale::Es, &digest(3, 5));
        assert_eq!(title, "3 altas, 5 buenas hoy");
    }

    #[test]
    fn an_empty_run_says_nothing_new_and_counts_the_sources() {
        let (title, body) = texts(Locale::En, &digest(0, 0));
        assert_eq!(title, "Nothing new today");
        assert_eq!(body, "We checked 4 Sources. Nothing worth your time — yet.");
    }

    #[test]
    fn the_body_names_the_first_top_match() {
        let mut digest = digest(1, 0);
        digest.top = vec![TopMatch {
            title: "Rust Engineer".to_string(),
            company: "Acme".to_string(),
        }];
        let (_, body) = texts(Locale::En, &digest);
        assert_eq!(body, "Top: Rust Engineer · Acme");
    }

    #[test]
    fn the_body_counts_the_matches_it_did_not_name() {
        let mut digest = digest(2, 1);
        digest.top = vec![
            TopMatch {
                title: "Rust Engineer".to_string(),
                company: "Acme".to_string(),
            },
            TopMatch {
                title: "Backend Dev".to_string(),
                company: "Globex".to_string(),
            },
        ];
        let (_, body) = texts(Locale::En, &digest);
        assert_eq!(body, "Top: Rust Engineer · Acme +1 more");
    }

    #[test]
    fn a_digest_that_names_nothing_points_at_today() {
        let (_, body) = texts(Locale::En, &digest(3, 5));
        assert_eq!(body, "Open Today to see them.");
    }

    #[test]
    fn the_catalog_keeps_its_namespace_prefix() {
        assert_eq!(
            message(Locale::En, "notifications:digest.title"),
            message(Locale::En, "digest.title")
        );
        assert!(message(Locale::En, "digest.missing").is_none());
    }
}
