//! Core-side i18n (AD-21, S1.4). The UI loads `locales/{en,es}/*.json` through
//! i18next; the core embeds the same catalogs at compile time so the tray and
//! native notifications are localized without a webview round-trip.
//!
//! Lookup uses the same flat dotted key as the UI (e.g. `errors.unknown`).
//!
//! CH7: core error keys live one file per area in `locales/{en,es}/errors/<area>.json`
//! so two slices never edit the same catalog. Keys keep their `errors.<area>.<x>` form
//! and the catalogs are merged, so lookups and call sites are unchanged.

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

/// The per-area error fragments (CH7), embedded alongside `common.json`.
/// A new area is one line here (`locales/{en,es}/errors/<area>.json`).
macro_rules! error_fragments {
    ($($area:literal),* $(,)?) => {
        &[$(
            (
                $area,
                include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../locales/en/errors/", $area, ".json")),
                include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../locales/es/errors/", $area, ".json")),
            ),
        )*]
    };
}

const ERROR_FRAGMENTS: &[(&str, &str, &str)] = error_fragments![
    "agent",
    "apply",
    "backup",
    "email",
    "inbox_actions",
    "lifecycle",
    "manual",
    "narration",
    "offers",
    "posting_features",
    "preferences",
    "profile",
    "profile_import",
    "run_report",
    "scoring",
    "session",
    "sources",
    "store",
    "studio",
    "task",
    "tracker",
    "unlock",
    "unknown",
];

fn catalog(locale: Locale) -> &'static Value {
    static EN: OnceLock<Value> = OnceLock::new();
    static ES: OnceLock<Value> = OnceLock::new();
    match locale {
        Locale::En => EN.get_or_init(|| merged(Locale::En, COMMON_EN)),
        Locale::Es => ES.get_or_init(|| merged(Locale::Es, COMMON_ES)),
    }
}

/// `common.json` plus every error fragment, as one flat key -> message object.
fn merged(locale: Locale, common: &str) -> Value {
    let lang = match locale {
        Locale::En => "en",
        Locale::Es => "es",
    };
    let mut map = parse(common, lang, "common.json")
        .as_object()
        .cloned()
        .unwrap_or_default();
    for (area, en, es) in ERROR_FRAGMENTS {
        let raw = match locale {
            Locale::En => en,
            Locale::Es => es,
        };
        let fragment = parse(raw, lang, &format!("errors/{area}.json"));
        map.extend(fragment.as_object().cloned().unwrap_or_default());
    }
    Value::Object(map)
}

fn parse(raw: &str, lang: &str, file: &str) -> Value {
    serde_json::from_str(raw)
        .unwrap_or_else(|e| panic!("locales/{lang}/{file} is not valid JSON: {e}"))
}

/// Look up a message for `locale`; `None` when the key is absent.
pub fn translated(locale: Locale, key: &str) -> Option<&'static str> {
    catalog(locale).get(key).and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

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

    /// CH7: every key of every error fragment resolves, in both locales. A new
    /// fragment is covered without touching this test.
    #[test]
    fn resolves_every_error_fragment_key_in_both_locales() {
        assert!(!ERROR_FRAGMENTS.is_empty());
        for (area, en, es) in ERROR_FRAGMENTS {
            let keys = parse(en, "en", &format!("errors/{area}.json"));
            let keys_es = parse(es, "es", &format!("errors/{area}.json"));
            assert_eq!(
                keys.as_object().map(|m| m.len()),
                keys_es.as_object().map(|m| m.len()),
                "errors/{area}.json: EN/ES key count differs"
            );
            for key in keys.as_object().expect("fragment is an object").keys() {
                assert!(
                    key == &format!("errors.{area}") || key.starts_with(&format!("errors.{area}.")),
                    "errors/{area}.json: key {key} is not in this file's area"
                );
                assert!(translated(Locale::En, key).is_some(), "en missing {key}");
                assert!(translated(Locale::Es, key).is_some(), "es missing {key}");
            }
        }
    }

    #[test]
    fn unknown_key_is_none() {
        assert_eq!(translated(Locale::En, "errors.does_not_exist"), None);
    }

    /// CH7: `ERROR_FRAGMENTS` is hand-listed, so a fragment file it forgets would
    /// ship an unresolvable key. The list must match the directory, both languages.
    #[test]
    fn error_fragment_list_matches_the_locales_directory() {
        let mut listed: Vec<String> = ERROR_FRAGMENTS
            .iter()
            .map(|(area, ..)| (*area).to_owned())
            .collect();
        listed.sort();
        for lang in ["en", "es"] {
            let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../locales")
                .join(lang)
                .join("errors");
            let mut found: Vec<String> = std::fs::read_dir(&dir)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
                .map(|entry| {
                    entry
                        .expect("fragment entry")
                        .file_name()
                        .to_string_lossy()
                        .into_owned()
                })
                .filter(|name| name.ends_with(".json"))
                .map(|name| name.trim_end_matches(".json").to_owned())
                .collect();
            found.sort();
            assert_eq!(
                found, listed,
                "locales/{lang}/errors/*.json does not match ERROR_FRAGMENTS"
            );
        }
    }
}
