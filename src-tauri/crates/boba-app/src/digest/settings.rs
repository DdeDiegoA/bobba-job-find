//! The notification preferences (S5.3, FR-107): which events notify, how many
//! top matches the digest names, whether an empty run still speaks, and the
//! quiet hours the user does not want to hear from Boba in.
//!
//! Stored as one JSON row through [`SystemRepo`], like the other settings of the
//! run (S3.7). Reads are tolerant on purpose: a row a newer build wrote, or one
//! this build cannot read at all, falls back to the defaults instead of leaving
//! notifications undefined.

use serde::{Deserialize, Serialize};

/// `stored` laid over `defaults`: every field the row omits keeps its default,
/// and a field it has wins, so an older row and a partial one both read.
fn merged(defaults: serde_json::Value, stored: serde_json::Value) -> serde_json::Value {
    match (defaults, stored) {
        (serde_json::Value::Object(mut base), serde_json::Value::Object(stored)) => {
            for (key, value) in stored {
                let slot = base.entry(key).or_insert(serde_json::Value::Null);
                *slot = merged(std::mem::take(slot), value);
            }
            serde_json::Value::Object(base)
        }
        (_, stored) => stored,
    }
}

/// The `setting` row that holds the notification preferences.
pub const SETTINGS_KEY: &str = "notifications.settings";

/// How many top matches a digest names, at most.
pub const MAX_TOP_MATCHES: u8 = 10;

/// An event the user can silence (FR-107).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    /// The post-run digest (FR-45).
    Digest,
    /// An automatic send entering the Outbox hold (FR-85).
    Outbox,
    /// A Source that keeps failing (FR-31).
    Errors,
}

/// Which events notify (FR-107).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct NotifyEvents {
    pub digest: bool,
    pub outbox: bool,
    pub errors: bool,
}

impl Default for NotifyEvents {
    /// Everything notifies until the user says otherwise: a silent app is a bug,
    /// not a default.
    fn default() -> Self {
        Self {
            digest: true,
            outbox: true,
            errors: true,
        }
    }
}

/// The window the user does not want to hear from Boba in, local time (FR-107).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct QuietHours {
    pub enabled: bool,
    /// First quiet hour, inclusive (0–23).
    pub from: u8,
    /// First hour that is no longer quiet (0–23).
    pub to: u8,
    /// Keep notifying anyway: the user set the hours for other apps, not Boba.
    pub notify_during: bool,
}

impl Default for QuietHours {
    /// Off, with a night-time window ready for when it is switched on.
    fn default() -> Self {
        Self {
            enabled: false,
            from: 22,
            to: 7,
            notify_during: false,
        }
    }
}

impl QuietHours {
    /// Whether `hour` (local, 0–23) falls inside the window. A window that wraps
    /// midnight (`22 → 7`) covers both ends; `from == to` covers nothing, so
    /// switching the feature on can never mute the app by accident.
    pub fn covers(&self, hour: u8) -> bool {
        if !self.enabled || self.from == self.to || hour > 23 {
            return false;
        }
        if self.from < self.to {
            (self.from..self.to).contains(&hour)
        } else {
            hour >= self.from || hour < self.to
        }
    }
}

/// Everything FR-107 lets the user choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct NotificationSettings {
    pub events: NotifyEvents,
    /// How many top matches a digest names (0 = the counts alone).
    pub top_matches_shown: u8,
    /// Notify about a run that found nothing new.
    pub notify_when_empty: bool,
    pub quiet_hours: QuietHours,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            events: NotifyEvents::default(),
            top_matches_shown: 3,
            notify_when_empty: false,
            quiet_hours: QuietHours::default(),
        }
    }
}

impl NotificationSettings {
    /// The settings of a stored row, or the defaults when there is none (or it
    /// cannot be read). Values are clamped to what the app can honour.
    pub fn from_json(raw: Option<&str>) -> Self {
        let Some(raw) = raw else {
            return Self::default();
        };
        let Ok(stored) = serde_json::from_str::<serde_json::Value>(raw) else {
            return Self::default();
        };
        let defaults = serde_json::to_value(Self::default())
            .unwrap_or_else(|_| serde_json::Value::Object(Default::default()));
        serde_json::from_value::<Self>(merged(defaults, stored))
            .unwrap_or_default()
            .sanitized()
    }

    /// Keep every value inside the range the UI can render: a count above
    /// [`MAX_TOP_MATCHES`] and an hour past the clock are capped, never refused.
    pub fn sanitized(self) -> Self {
        Self {
            events: self.events,
            top_matches_shown: self.top_matches_shown.min(MAX_TOP_MATCHES),
            notify_when_empty: self.notify_when_empty,
            quiet_hours: QuietHours {
                enabled: self.quiet_hours.enabled,
                from: self.quiet_hours.from.min(23),
                to: self.quiet_hours.to.min(23),
                notify_during: self.quiet_hours.notify_during,
            },
        }
    }

    /// Whether the user wants `kind` to notify at all (FR-107).
    pub fn allows(&self, kind: EventKind) -> bool {
        match kind {
            EventKind::Digest => self.events.digest,
            EventKind::Outbox => self.events.outbox,
            EventKind::Errors => self.events.errors,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_notify_every_event() {
        let settings = NotificationSettings::default();
        for kind in [EventKind::Digest, EventKind::Outbox, EventKind::Errors] {
            assert!(settings.allows(kind), "{kind:?} is on by default");
        }
    }

    #[test]
    fn an_event_the_user_switched_off_stays_quiet() {
        let mut settings = NotificationSettings::default();
        settings.events.outbox = false;
        assert!(!settings.allows(EventKind::Outbox));
        assert!(settings.allows(EventKind::Digest));
    }

    #[test]
    fn the_top_match_count_is_capped() {
        let settings = NotificationSettings {
            top_matches_shown: 99,
            ..NotificationSettings::default()
        };
        assert_eq!(
            settings.sanitized().top_matches_shown,
            super::MAX_TOP_MATCHES
        );
    }

    #[test]
    fn quiet_hours_are_off_by_default() {
        assert!(!NotificationSettings::default().quiet_hours.enabled);
    }

    #[test]
    fn a_quiet_window_that_wraps_midnight_covers_the_night() {
        let hours = QuietHours {
            enabled: true,
            from: 22,
            to: 7,
            notify_during: false,
        };
        assert!(hours.covers(22));
        assert!(hours.covers(23));
        assert!(hours.covers(0));
        assert!(hours.covers(6));
        assert!(!hours.covers(7));
        assert!(!hours.covers(12));
    }

    #[test]
    fn a_quiet_window_inside_one_day_covers_only_its_hours() {
        let hours = QuietHours {
            enabled: true,
            from: 9,
            to: 12,
            notify_during: false,
        };
        assert!(!hours.covers(8));
        assert!(hours.covers(9));
        assert!(hours.covers(11));
        assert!(!hours.covers(12));
    }

    #[test]
    fn a_disabled_or_empty_window_covers_nothing() {
        let mut hours = QuietHours {
            enabled: false,
            from: 22,
            to: 7,
            notify_during: false,
        };
        assert!(!hours.covers(23));
        hours.enabled = true;
        hours.to = hours.from;
        assert!(!hours.covers(23), "from == to is not a whole day of quiet");
    }

    #[test]
    fn a_stored_row_this_build_cannot_read_falls_back_to_the_defaults() {
        assert_eq!(
            NotificationSettings::from_json(Some("not json")),
            NotificationSettings::default()
        );
        assert_eq!(
            NotificationSettings::from_json(None),
            NotificationSettings::default()
        );
    }

    #[test]
    fn a_partial_stored_row_keeps_the_defaults_it_omits() {
        let settings = NotificationSettings::from_json(Some(r#"{"notify_when_empty":true}"#));
        assert!(settings.notify_when_empty);
        assert_eq!(settings.top_matches_shown, 3);
        assert!(settings.events.digest);
    }

    #[test]
    fn a_stored_row_round_trips() {
        let mut settings = NotificationSettings::default();
        settings.events.errors = false;
        settings.quiet_hours.enabled = true;
        settings.top_matches_shown = 5;
        let raw = serde_json::to_string(&settings).expect("serialize");
        assert_eq!(NotificationSettings::from_json(Some(&raw)), settings);
    }

    #[test]
    fn settings_from_a_newer_build_ignore_the_fields_this_build_lacks() {
        let settings = NotificationSettings::from_json(Some(
            r#"{"notify_when_empty":true,"future_field":{"a":1}}"#,
        ));
        assert!(settings.notify_when_empty);
    }
}
