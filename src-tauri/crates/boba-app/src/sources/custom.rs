//! Custom RSS/Atom feeds the user saved (S4.8 AC2, FR-26).
//!
//! The `source` table has no config column, and this slice may not change the
//! schema, so a custom feed is one row in the settings store (`source` rows are
//! still written for id, name and health). The URL is validated twice: by the
//! command before it is saved and by `boba_sources::rss::CustomFeed::new` when
//! the feed is built for a run, so a hand-edited setting still fails closed.

use serde::{Deserialize, Serialize};

use crate::ports::repo::system::SystemRepo;

use super::SourcesError;

/// The one settings key the custom feeds live under.
pub const CUSTOM_RSS_KEY: &str = "sources.custom_rss";

/// One saved custom feed (the settings-store shape).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomFeedConfig {
    /// Registry id and `source` primary key (`rss:<slug>`).
    pub id: String,
    /// The name shown on the tile.
    pub name: String,
    /// The feed URL, as saved.
    pub url: String,
    /// When the user added it.
    pub added_at: String,
}

/// Every saved custom feed, in the order they were added.
pub fn list_custom_feeds(system: &dyn SystemRepo) -> Result<Vec<CustomFeedConfig>, SourcesError> {
    let Some(raw) = system
        .setting_get(CUSTOM_RSS_KEY)
        .map_err(|error| SourcesError::Store(error.to_string()))?
    else {
        return Ok(Vec::new());
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(raw).map_err(|error| SourcesError::Invalid(error.to_string()))
}

/// Insert or replace one feed by its id (a re-added URL updates in place).
pub fn save_custom_feed(
    system: &dyn SystemRepo,
    feed: &CustomFeedConfig,
    now: &str,
) -> Result<(), SourcesError> {
    if feed.id.trim().is_empty() || feed.url.trim().is_empty() {
        return Err(SourcesError::Invalid("empty feed id or url".to_string()));
    }
    let mut feeds = list_custom_feeds(system)?;
    match feeds.iter_mut().find(|saved| saved.id == feed.id) {
        Some(saved) => *saved = feed.clone(),
        None => feeds.push(feed.clone()),
    }
    store(system, &feeds, now)
}

/// Remove one feed; `true` when it was there.
pub fn remove_custom_feed(
    system: &dyn SystemRepo,
    id: &str,
    now: &str,
) -> Result<bool, SourcesError> {
    let mut feeds = list_custom_feeds(system)?;
    let before = feeds.len();
    feeds.retain(|feed| feed.id != id);
    if feeds.len() == before {
        return Ok(false);
    }
    store(system, &feeds, now)?;
    Ok(true)
}

fn store(
    system: &dyn SystemRepo,
    feeds: &[CustomFeedConfig],
    now: &str,
) -> Result<(), SourcesError> {
    let json =
        serde_json::to_string(feeds).map_err(|error| SourcesError::Invalid(error.to_string()))?;
    system
        .setting_put(CUSTOM_RSS_KEY, &json, now)
        .map_err(|error| SourcesError::Store(error.to_string()))
}

/// A stable id for a feed URL (`rss:<host-path-slug>`), so re-adding the same
/// URL updates the saved feed instead of duplicating it.
pub fn feed_id(url: &str) -> String {
    let rest = url
        .trim()
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(url.trim());
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    let mut slug = String::with_capacity(rest.len());
    for c in rest.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    let slug: String = slug.chars().take(64).collect();
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "rss:feed".to_string()
    } else {
        format!("rss:{slug}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::testing::FakeSystemRepo;

    fn feed(url: &str, name: &str) -> CustomFeedConfig {
        CustomFeedConfig {
            id: feed_id(url),
            name: name.to_string(),
            url: url.to_string(),
            added_at: "2026-10-12T08:00:00Z".to_string(),
        }
    }

    #[test]
    fn an_empty_store_has_no_feeds() {
        let system = FakeSystemRepo::default();
        assert_eq!(list_custom_feeds(&system).unwrap(), Vec::new());
    }

    #[test]
    fn a_saved_feed_round_trips() {
        let system = FakeSystemRepo::default();
        let saved = feed("https://jobs.example.com/feed.xml", "Boba Jobs");
        save_custom_feed(&system, &saved, "2026-10-12T08:00:00Z").unwrap();
        assert_eq!(list_custom_feeds(&system).unwrap(), vec![saved]);
    }

    #[test]
    fn re_adding_the_same_url_updates_in_place() {
        let system = FakeSystemRepo::default();
        let first = feed("https://jobs.example.com/feed.xml", "Jobs");
        save_custom_feed(&system, &first, "2026-10-12T08:00:00Z").unwrap();

        let mut renamed = first.clone();
        renamed.name = "Jobs (renamed)".to_string();
        save_custom_feed(&system, &renamed, "2026-10-13T08:00:00Z").unwrap();

        let feeds = list_custom_feeds(&system).unwrap();
        assert_eq!(feeds.len(), 1);
        assert_eq!(feeds[0].name, "Jobs (renamed)");
    }

    #[test]
    fn different_urls_get_different_ids() {
        assert_ne!(
            feed_id("https://jobs.example.com/feed.xml"),
            feed_id("https://other.example.com/feed.xml")
        );
        assert_eq!(
            feed_id("https://jobs.example.com/feed.xml"),
            feed_id("https://Jobs.Example.com/feed.xml?utm=x")
        );
        assert_eq!(feed_id("https://example.com"), "rss:example-com");
    }

    #[test]
    fn removing_a_feed_reports_whether_it_was_there() {
        let system = FakeSystemRepo::default();
        let saved = feed("https://jobs.example.com/feed.xml", "Jobs");
        save_custom_feed(&system, &saved, "2026-10-12T08:00:00Z").unwrap();

        assert!(remove_custom_feed(&system, &saved.id, "2026-10-13T08:00:00Z").unwrap());
        assert!(list_custom_feeds(&system).unwrap().is_empty());
        assert!(!remove_custom_feed(&system, &saved.id, "2026-10-13T08:00:00Z").unwrap());
    }

    #[test]
    fn a_hand_edited_value_fails_closed() {
        let mut system = FakeSystemRepo::default();
        system.seed_setting(CUSTOM_RSS_KEY, "{not json");
        assert!(matches!(
            list_custom_feeds(&system),
            Err(SourcesError::Invalid(_))
        ));
    }

    #[test]
    fn an_empty_or_blank_feed_is_refused() {
        let system = FakeSystemRepo::default();
        let mut bad = feed("https://jobs.example.com/feed.xml", "Jobs");
        bad.url = "  ".to_string();
        assert!(matches!(
            save_custom_feed(&system, &bad, "2026-10-12T08:00:00Z"),
            Err(SourcesError::Invalid(_))
        ));
    }
}
