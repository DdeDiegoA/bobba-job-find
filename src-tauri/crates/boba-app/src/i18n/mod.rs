//! Core-side i18n (AD-21, S1.4). The UI loads `locales/{en,es}/*.json` through
//! i18next; the core embeds the same catalogs at compile time so the tray and
//! native notifications are localized without a webview round-trip.
//!
//! Lookup uses the same flat dotted key as the UI (e.g. `errors.unknown`).

use std::sync::OnceLock;

use serde_json::Value;

/// A shipped UI locale (AD-17: BCP-47; the MVP ships EN and ES).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locale {
    /// English.
    En,
    /// Spanish.
    Es,
}

impl Locale {
    /// Resolve a BCP-47 tag: `es*` is Spanish, anything else English (AC1).
    pub fn from_tag(tag: &str) -> Self {
        if tag.get(..2).is_some_and(|p| p.eq_ignore_ascii_case("es")) {
            Locale::Es
        } else {
            Locale::En
        }
    }
}

const COMMON_EN: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../locales/en/common.json"
));
const COMMON_ES: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../locales/es/common.json"
));

fn catalog(locale: Locale) -> &'static Value {
    static EN: OnceLock<Value> = OnceLock::new();
    static ES: OnceLock<Value> = OnceLock::new();
    match locale {
        Locale::En => EN.get_or_init(|| parse(COMMON_EN, "en")),
        Locale::Es => ES.get_or_init(|| parse(COMMON_ES, "es")),
    }
}

fn parse(raw: &str, lang: &str) -> Value {
    serde_json::from_str(raw)
        .unwrap_or_else(|e| panic!("locales/{lang}/common.json is not valid JSON: {e}"))
}

/// Look up a message for `locale`; `None` when the key is absent.
pub fn translated(locale: Locale, key: &str) -> Option<&'static str> {
    catalog(locale).get(key).and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_os_tags_to_shipped_locales() {
        assert_eq!(Locale::from_tag("es"), Locale::Es);
        assert_eq!(Locale::from_tag("es-MX"), Locale::Es);
        assert_eq!(Locale::from_tag("ES-es"), Locale::Es);
        assert_eq!(Locale::from_tag("en-US"), Locale::En);
        assert_eq!(Locale::from_tag("fr"), Locale::En);
    }

    #[test]
    fn resolves_core_emitted_keys_in_both_locales() {
        for key in ["errors.unknown", "errors.task.missing_id"] {
            assert!(translated(Locale::En, key).is_some(), "en missing {key}");
            assert!(translated(Locale::Es, key).is_some(), "es missing {key}");
        }
    }

    #[test]
    fn unknown_key_is_none() {
        assert_eq!(translated(Locale::En, "errors.does_not_exist"), None);
    }
}
