//! The tray menu (S1.6, FR-42, EXPERIENCE §9.9).
//!
//! The shape and the wording of the menu live in a plain table so they are
//! unit-tested without a windowing system; [`build`] is the thin Tauri glue that
//! turns the table into platform menu items.
//!
//! The labels are native, so they cannot come from the webview: `tray.json` is
//! embedded at compile time (AD-21) and looked up by the UI's own
//! `namespace:key` spelling, so a key reads the same on both sides.

use std::collections::HashMap;
use std::sync::OnceLock;

use boba_app::i18n::Locale;
use serde_json::Value;
use tauri::menu::{CheckMenuItemBuilder, Menu, MenuBuilder, MenuItemBuilder};
use tauri::{Manager, Runtime};

const EN: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../locales/en/tray.json"
));
const ES: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../locales/es/tray.json"
));

/// Menu action ids: what Tauri hands back when the user picks an item.
pub mod id {
    /// Show and focus the window.
    pub const OPEN: &str = "tray.open";
    /// Start a Search Run (S5.4).
    pub const RUN_NOW: &str = "tray.run_now";
    /// Pause or resume the schedule.
    pub const PAUSE_SCHEDULE: &str = "tray.pause_schedule";
    /// Stop every automatic action (wired in E9).
    pub const KILL_SWITCH: &str = "tray.kill_switch";
    /// Leave the app, tray included.
    pub const QUIT: &str = "tray.quit";
}

/// One menu entry: its action id, the catalog key of its label, and whether it
/// can be picked today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// The action id (see [`id`]).
    pub id: &'static str,
    /// The `tray.json` key of its label, in the UI's `tray:<key>` spelling.
    pub message_key: &'static str,
    /// Whether picking it does anything yet.
    pub enabled: bool,
}

/// The tray menu, in the order FR-42 lists it (AC2). Every entry is live: the
/// scheduler (S5.1) runs "Run now" since S5.4, and the Kill Switch since S9.8.
pub const ENTRIES: [Entry; 5] = [
    Entry {
        id: id::OPEN,
        message_key: "tray:menu.open",
        enabled: true,
    },
    Entry {
        id: id::RUN_NOW,
        message_key: "tray:menu.runNow",
        enabled: true,
    },
    Entry {
        id: id::PAUSE_SCHEDULE,
        message_key: "tray:menu.pauseSchedule",
        enabled: true,
    },
    Entry {
        id: id::KILL_SWITCH,
        message_key: "tray:menu.killSwitch",
        enabled: true,
    },
    Entry {
        id: id::QUIT,
        message_key: "tray:menu.quit",
        enabled: true,
    },
];

/// What picking a menu item does (AC2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Bring the window to the front.
    Open,
    /// Ask for a Search Run.
    RunNow,
    /// Pause or resume the schedule.
    ToggleSchedule,
    /// Stop every automatic action.
    KillSwitch,
    /// Leave the app.
    Quit,
    /// Nothing: an id from a newer build.
    Ignore,
}

/// Map a picked menu id to its action. [`super::attach`] is the only caller, so
/// a platform event never reaches the rest of the app untranslated.
pub fn action(picked: &str) -> Action {
    match picked {
        id::OPEN => Action::Open,
        id::RUN_NOW => Action::RunNow,
        id::PAUSE_SCHEDULE => Action::ToggleSchedule,
        id::KILL_SWITCH => Action::KillSwitch,
        id::QUIT => Action::Quit,
        _ => Action::Ignore,
    }
}

/// The label of `entry` in `locale`; the action id is the fallback, so a label
/// is never empty. A missing translation is a test failure, not a runtime one.
pub fn label(locale: Locale, entry: &Entry) -> &'static str {
    message(locale, entry.message_key).unwrap_or(entry.id)
}

/// Build the menu for `locale`; `paused` is what the "Pause schedule" check mark
/// shows (AC2).
pub fn build<R: Runtime, M: Manager<R>>(
    manager: &M,
    locale: Locale,
    paused: bool,
) -> tauri::Result<Menu<R>> {
    let mut builder = MenuBuilder::new(manager);
    for entry in ENTRIES {
        let text = label(locale, &entry);
        builder = if entry.id == id::PAUSE_SCHEDULE {
            let item = CheckMenuItemBuilder::with_id(entry.id, text)
                .checked(paused)
                .enabled(entry.enabled)
                .build(manager)?;
            builder.item(&item)
        } else {
            let item = MenuItemBuilder::with_id(entry.id, text)
                .enabled(entry.enabled)
                .build(manager)?;
            builder.item(&item)
        };
    }
    builder.build()
}

/// A `tray.json` message for `locale`, by its `tray:<key>` (or bare `<key>`) key.
pub fn message(locale: Locale, key: &str) -> Option<&'static str> {
    let key = key.strip_prefix("tray:").unwrap_or(key);
    catalog(locale).get(key).map(String::as_str)
}

/// The parsed catalog of one language, embedded once per process.
fn catalog(locale: Locale) -> &'static HashMap<String, String> {
    static CATALOG_EN: OnceLock<HashMap<String, String>> = OnceLock::new();
    static CATALOG_ES: OnceLock<HashMap<String, String>> = OnceLock::new();
    match locale {
        Locale::En => CATALOG_EN.get_or_init(|| parse(EN, "en")),
        Locale::Es => CATALOG_ES.get_or_init(|| parse(ES, "es")),
    }
}

/// Parse a flat `key -> message` catalog; a malformed shipped file fails loudly.
fn parse(raw: &str, lang: &str) -> HashMap<String, String> {
    let json: Value = serde_json::from_str(raw)
        .unwrap_or_else(|error| panic!("locales/{lang}/tray.json is not valid JSON: {error}"));
    json.as_object()
        .into_iter()
        .flatten()
        .map(|(key, value)| {
            let text = value
                .as_str()
                .unwrap_or_else(|| panic!("locales/{lang}/tray.json: {key} is not a string"));
            (key.clone(), text.to_owned())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AC2: Open, Run now, Pause schedule, Kill Switch, Quit — in that order.
    #[test]
    fn menu_is_the_five_items_of_fr42_in_order() {
        let ids: Vec<&str> = ENTRIES.iter().map(|entry| entry.id).collect();
        assert_eq!(
            ids,
            [
                id::OPEN,
                id::RUN_NOW,
                id::PAUSE_SCHEDULE,
                id::KILL_SWITCH,
                id::QUIT
            ]
        );
    }

    /// AC2: every entry can be picked; nothing on the menu is a dead click.
    #[test]
    fn every_entry_is_enabled() {
        let enabled = |id: &str| {
            ENTRIES
                .iter()
                .find(|entry| entry.id == id)
                .expect("entry")
                .enabled
        };
        assert!(enabled(id::OPEN));
        assert!(enabled(id::PAUSE_SCHEDULE));
        assert!(enabled(id::QUIT));
        assert!(enabled(id::KILL_SWITCH), "the Kill Switch is live (S9.8)");
        assert!(enabled(id::RUN_NOW), "Run now is live (S5.4)");
    }

    /// The labels the PRD keeps in English on purpose ("Kill Switch" is its
    /// binding vocabulary, voice-and-tone §5.3), so they are the one pair the
    /// translation check below cannot require to differ.
    const SAME_IN_BOTH_LOCALES: [&str; 1] = [id::KILL_SWITCH];

    /// AC2: every entry is labelled in both shipped locales, and the two labels
    /// differ — an English string left in `es` is a missing translation.
    #[test]
    fn every_label_is_translated_in_en_and_es() {
        for entry in ENTRIES {
            let en = message(Locale::En, entry.message_key);
            let es = message(Locale::Es, entry.message_key);
            assert!(
                en.is_some(),
                "en/tray.json is missing {}",
                entry.message_key
            );
            assert!(
                es.is_some(),
                "es/tray.json is missing {}",
                entry.message_key
            );
            if SAME_IN_BOTH_LOCALES.contains(&entry.id) {
                assert_eq!(en, es, "{} is the PRD's wording in both", entry.id);
            } else {
                assert_ne!(en, es, "{} is not translated", entry.message_key);
            }
            assert_ne!(
                label(Locale::En, &entry),
                entry.id,
                "{} falls back to its id",
                entry.message_key
            );
        }
    }

    /// The keys are spelled the way the UI spells them, so both sides of a
    /// catalog entry read the same.
    #[test]
    fn keys_carry_the_tray_namespace() {
        for entry in ENTRIES {
            assert!(
                entry.message_key.starts_with("tray:"),
                "{} is not namespaced",
                entry.message_key
            );
        }
    }

    /// Every id on the menu drives exactly one action, and an id from a newer
    /// build is ignored instead of doing something surprising.
    #[test]
    fn ids_map_to_their_action() {
        assert_eq!(action(id::OPEN), Action::Open);
        assert_eq!(action(id::RUN_NOW), Action::RunNow);
        assert_eq!(action(id::PAUSE_SCHEDULE), Action::ToggleSchedule);
        assert_eq!(action(id::KILL_SWITCH), Action::KillSwitch);
        assert_eq!(action(id::QUIT), Action::Quit);
        assert_eq!(action("tray.unknown"), Action::Ignore);
        assert_eq!(action(""), Action::Ignore);
    }

    /// An entry with no action arm would be a menu item that does nothing.
    #[test]
    fn every_entry_has_an_action() {
        for entry in ENTRIES {
            assert_ne!(
                action(entry.id),
                Action::Ignore,
                "{} does nothing",
                entry.id
            );
        }
    }

    /// A key that is not in the catalog resolves to `None` rather than to a
    /// guessed string.
    #[test]
    fn an_unknown_key_is_none() {
        assert_eq!(message(Locale::En, "tray:menu.nope"), None);
        assert_eq!(message(Locale::Es, "menu.nope"), None);
    }
}
