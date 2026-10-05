//! Typed settings over the `setting` table (FR-106; Consistency Conventions:
//! config). Each key has a compiled default and a validator, so an absent or
//! malformed row can never leave the app without a value, and no loose JSON
//! file is ever the source of truth.

use crate::ports::clock::ClockPort;
use crate::ports::repo::system::SystemRepo;
use crate::system::audit::SystemError;

/// A typed setting key. Adding a key is additive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingKey {
    /// UI language, BCP-47 (AD-17).
    UiLocale,
    /// Light, dark or follow-OS.
    Theme,
    /// Default content language for documents (AD-17).
    ContentLanguage,
    /// Launch the app at login.
    LaunchAtLogin,
}

impl SettingKey {
    /// Every key, for a full read or a reset.
    pub const ALL: [SettingKey; 4] = [
        SettingKey::UiLocale,
        SettingKey::Theme,
        SettingKey::ContentLanguage,
        SettingKey::LaunchAtLogin,
    ];

    /// The stable DB key.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UiLocale => "ui_locale",
            Self::Theme => "theme",
            Self::ContentLanguage => "content_language",
            Self::LaunchAtLogin => "launch_at_login",
        }
    }

    /// Parse a DB key.
    pub fn parse(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == key)
    }

    /// The compiled default, used when no row is stored.
    pub fn default_value(self) -> &'static str {
        match self {
            Self::UiLocale => "en",
            Self::Theme => "system",
            Self::ContentLanguage => "en",
            Self::LaunchAtLogin => "false",
        }
    }

    /// Reject out-of-range values (settings are validated in core, AD-11).
    pub fn validate(self, value: &str) -> Result<(), SystemError> {
        let ok = match self {
            Self::Theme => Theme::parse(value).is_some(),
            Self::LaunchAtLogin => matches!(value, "true" | "false"),
            Self::UiLocale | Self::ContentLanguage => is_bcp47(value),
        };
        if ok {
            Ok(())
        } else {
            Err(SystemError::InvalidSetting {
                key: self.as_str().to_string(),
                value: value.to_string(),
            })
        }
    }
}

/// App theme (FR-1, FR-106).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    Light,
    Dark,
    #[default]
    System,
}

impl Theme {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
            Self::System => "system",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            "system" => Some(Self::System),
            _ => None,
        }
    }
}

/// The resolved settings, compiled defaults merged with stored rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub ui_locale: String,
    pub theme: Theme,
    pub content_language: String,
    pub launch_at_login: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ui_locale: SettingKey::UiLocale.default_value().to_string(),
            theme: Theme::default(),
            content_language: SettingKey::ContentLanguage.default_value().to_string(),
            launch_at_login: false,
        }
    }
}

impl Settings {
    fn apply(&mut self, key: &str, value: &str) {
        match SettingKey::parse(key) {
            Some(SettingKey::UiLocale) => self.ui_locale = value.to_string(),
            Some(SettingKey::Theme) => {
                if let Some(theme) = Theme::parse(value) {
                    self.theme = theme;
                }
            }
            Some(SettingKey::ContentLanguage) => self.content_language = value.to_string(),
            Some(SettingKey::LaunchAtLogin) => self.launch_at_login = value == "true",
            // Forward-compatible: a key from a newer build is ignored.
            None => {}
        }
    }
}

/// Read and write typed settings.
pub struct SettingsService<R, C> {
    repo: R,
    clock: C,
}

impl<R: SystemRepo, C: ClockPort> SettingsService<R, C> {
    pub fn new(repo: R, clock: C) -> Self {
        Self { repo, clock }
    }

    /// Compiled defaults merged with every stored row.
    pub fn load(&self) -> Result<Settings, SystemError> {
        let mut settings = Settings::default();
        for record in self.repo.setting_list()? {
            settings.apply(&record.key, &record.value);
        }
        Ok(settings)
    }

    /// One setting's stored value, or its compiled default.
    pub fn get(&self, key: SettingKey) -> Result<String, SystemError> {
        Ok(self
            .repo
            .setting_get(key.as_str())?
            .unwrap_or_else(|| key.default_value().to_string()))
    }

    /// Validate and store one setting, stamped with the injected clock.
    pub fn set(&self, key: SettingKey, value: &str) -> Result<(), SystemError> {
        key.validate(value)?;
        self.repo
            .setting_put(key.as_str(), value, &self.clock.now_rfc3339())?;
        Ok(())
    }
}

/// A BCP-47-shaped tag: non-empty ASCII alphanumeric segments joined by `-`.
fn is_bcp47(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 35
        && value
            .split('-')
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::{FixedClock, testing::FakeSystemRepo};

    fn service(repo: FakeSystemRepo) -> SettingsService<FakeSystemRepo, FixedClock> {
        SettingsService::new(repo, FixedClock::new("2026-10-05T01:00:00.000Z"))
    }

    #[test]
    fn defaults_apply_when_no_rows_are_stored() {
        let settings = service(FakeSystemRepo::default()).load().unwrap();
        assert_eq!(settings, Settings::default());
        assert_eq!(settings.theme, Theme::System);
        assert!(!settings.launch_at_login);
    }

    #[test]
    fn stored_rows_override_defaults_and_round_trip() {
        let svc = service(FakeSystemRepo::default());
        svc.set(SettingKey::Theme, "dark").unwrap();
        svc.set(SettingKey::UiLocale, "es").unwrap();

        let settings = svc.load().unwrap();
        assert_eq!(settings.theme, Theme::Dark);
        assert_eq!(settings.ui_locale, "es");
        assert_eq!(svc.get(SettingKey::Theme).unwrap(), "dark");
    }

    #[test]
    fn invalid_values_are_rejected() {
        let svc = service(FakeSystemRepo::default());
        assert!(svc.set(SettingKey::Theme, "neon").is_err());
        assert!(svc.set(SettingKey::LaunchAtLogin, "yes").is_err());
        assert!(svc.set(SettingKey::UiLocale, "en US").is_err());
        assert!(svc.set(SettingKey::UiLocale, "es-MX").is_ok());
    }

    #[test]
    fn unknown_stored_keys_are_ignored() {
        let mut repo = FakeSystemRepo::default();
        repo.seed_setting("future_key", "whatever");
        assert_eq!(service(repo).load().unwrap(), Settings::default());
    }
}
