//! Aggregator APIs with user-supplied keys (S4.6, FR-23/FR-24, addendum A.2).
//!
//! Adzuna and Jooble are `official_api` sources: they run only once the user
//! has entered their keys (AC2). The keys live in the OS keychain — the caller
//! reads them through `SecretPort` and hands them to [`Adzuna`]/[`Jooble`] — and
//! never appear in an error, a log or a `source_run` row: both adapters build
//! every error from a redacted URL, and both mask an invalid key.
//!
//! Searches use the profile's queries and locations (`SearchQuery`); the
//! country and the defaults themselves come from Preferences and are passed in.

mod adzuna;
mod jooble;

use boba_domain::source::{AccessClass, ApplyCapability, LegalTier, RateLimits, SourceManifest};

pub use adzuna::{
    ADZUNA_APP_ID_REF, ADZUNA_APP_KEY_REF, API as ADZUNA_API, Adzuna, AdzunaKeys, COUNTRIES,
    DEFAULT_COUNTRY,
};
pub use jooble::{API as JOOBLE_API, JOOBLE_KEY_REF, Jooble};

/// One supported aggregator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AggregatorKind {
    Adzuna,
    Jooble,
}

impl AggregatorKind {
    pub const ALL: [AggregatorKind; 2] = [Self::Adzuna, Self::Jooble];

    /// Stable key used as the source id.
    pub fn key(self) -> &'static str {
        match self {
            Self::Adzuna => "adzuna",
            Self::Jooble => "jooble",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Adzuna => "Adzuna",
            Self::Jooble => "Jooble",
        }
    }

    /// The documented limits (integrations.md §2, addendum A.2).
    pub fn limits(self) -> RateLimits {
        match self {
            // 25 calls/minute and 250/day.
            Self::Adzuna => RateLimits {
                min_interval_ms: 2_400,
                daily_cap: Some(250),
            },
            // 500 requests per key for its whole lifetime: "use sparingly" —
            // one search plus at most one detail a day.
            Self::Jooble => RateLimits {
                min_interval_ms: 5_000,
                daily_cap: Some(2),
            },
        }
    }

    pub fn attribution(self) -> &'static str {
        match self {
            Self::Adzuna => "Adzuna — adzuna.com",
            Self::Jooble => "Jooble — jooble.org",
        }
    }

    /// The keychain entries the user's credentials live under (AC2).
    pub fn secret_refs(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Adzuna => &[
                ("app_id", ADZUNA_APP_ID_REF),
                ("app_key", ADZUNA_APP_KEY_REF),
            ],
            Self::Jooble => &[("api_key", JOOBLE_KEY_REF)],
        }
    }

    pub fn manifest(self) -> SourceManifest {
        let required_secrets = self
            .secret_refs()
            .iter()
            .map(|(name, _)| (*name).to_string())
            .collect();
        let markets = match self {
            Self::Adzuna => COUNTRIES
                .iter()
                .map(|country| country.to_ascii_uppercase())
                .collect(),
            Self::Jooble => vec!["Global (60+ countries)".to_string()],
        };
        SourceManifest {
            id: self.key().to_string(),
            name: self.name().to_string(),
            access_class: AccessClass::OfficialApi,
            legal_tier: LegalTier::Safe,
            markets,
            rate_limits: self.limits(),
            attribution: Some(self.attribution().to_string()),
            // A keyed API links out; the application itself is the user's.
            apply_capability: ApplyCapability::LinkOut,
            required_secrets,
            config_schema: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_aggregators_are_safe_official_apis_behind_a_key() {
        for kind in AggregatorKind::ALL {
            let manifest = kind.manifest();
            assert_eq!(manifest.access_class, AccessClass::OfficialApi);
            assert!(
                !manifest.requires_opt_in(),
                "a key is the gate, not an opt-in"
            );
            assert_eq!(manifest.legal_tier, LegalTier::Safe);
            assert!(!manifest.required_secrets.is_empty());
            assert!(manifest.attribution.is_some());
        }
    }

    #[test]
    fn the_keychain_references_are_the_names_the_manifests_declare() {
        for kind in AggregatorKind::ALL {
            let manifest = kind.manifest();
            let declared: Vec<&str> = manifest
                .required_secrets
                .iter()
                .map(String::as_str)
                .collect();
            let refs: Vec<&str> = kind.secret_refs().iter().map(|(name, _)| *name).collect();
            assert_eq!(declared, refs, "{kind:?}");
            for (_, reference) in kind.secret_refs() {
                assert!(
                    boba_app::ports::secret::valid_reference(reference),
                    "{reference}"
                );
            }
        }
    }
}
