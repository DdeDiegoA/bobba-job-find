//! Remote feed adapters (S4.6, FR-23/FR-24/FR-32, addendum A.2).
//!
//! Five public feeds and one Algolia-backed board, no auth: Remotive, RemoteOK,
//! Himalayas, Arbeitnow, WeWorkRemotely RSS and HN Who's Hiring. Each declares
//! its documented limits and attribution on the manifest (AC1) — the shared
//! client enforces the cap because every request carries the source id as its
//! budget key — and every feed is `link_out` (Manual only, A.3): none of them
//! sanctions submitting an application off its own site.

pub(crate) mod api;
mod arbeitnow;
mod himalayas;
mod hn;
mod remoteok;
mod remotive;
mod wwr;

use boba_domain::source::{AccessClass, ApplyCapability, LegalTier, RateLimits, SourceManifest};

use crate::sdk::SourceAdapter;

pub use arbeitnow::Arbeitnow;
pub use himalayas::Himalayas;
pub use hn::HackerNews;
pub use remoteok::RemoteOk;
pub use remotive::Remotive;
pub use wwr::WeWorkRemotely;

/// One built-in remote feed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FeedKind {
    Remotive,
    RemoteOk,
    Himalayas,
    Arbeitnow,
    WeWorkRemotely,
    HackerNews,
}

impl FeedKind {
    pub const ALL: [FeedKind; 6] = [
        Self::Remotive,
        Self::RemoteOk,
        Self::Himalayas,
        Self::Arbeitnow,
        Self::WeWorkRemotely,
        Self::HackerNews,
    ];

    /// Stable key used as the source id.
    pub fn key(self) -> &'static str {
        match self {
            Self::Remotive => "remotive",
            Self::RemoteOk => "remoteok",
            Self::Himalayas => "himalayas",
            Self::Arbeitnow => "arbeitnow",
            Self::WeWorkRemotely => "weworkremotely",
            Self::HackerNews => "hn-who-is-hiring",
        }
    }

    /// Display name for the connections UI.
    pub fn name(self) -> &'static str {
        match self {
            Self::Remotive => "Remotive",
            Self::RemoteOk => "RemoteOK",
            Self::Himalayas => "Himalayas",
            Self::Arbeitnow => "Arbeitnow",
            Self::WeWorkRemotely => "We Work Remotely",
            Self::HackerNews => "HN Who's Hiring",
        }
    }

    /// The documented limits (integrations.md §2, addendum A.2).
    pub fn limits(self) -> RateLimits {
        match self {
            // "asks ≤ 4 fetches/day; blocks > 2 req/min": one fetch every 30 s.
            Self::Remotive => RateLimits {
                min_interval_ms: 30_000,
                daily_cap: Some(4),
            },
            // HN needs two calls per run (story then comments).
            Self::HackerNews => RateLimits {
                min_interval_ms: 1_000,
                daily_cap: None,
            },
            Self::RemoteOk | Self::Himalayas | Self::Arbeitnow | Self::WeWorkRemotely => {
                RateLimits::default()
            }
        }
    }

    /// The markets the feed covers (A.2).
    pub fn markets(self) -> &'static [&'static str] {
        match self {
            Self::Arbeitnow => &["Europe"],
            Self::HackerNews => &["Global remote", "Tech"],
            _ => &["Global remote"],
        }
    }

    pub fn access_class(self) -> AccessClass {
        match self {
            Self::WeWorkRemotely => AccessClass::Rss,
            _ => AccessClass::PublicJson,
        }
    }

    /// The attribution the feed asks for, shown on its Offers (AC1, FR-32).
    pub fn attribution(self) -> Option<&'static str> {
        match self {
            Self::Remotive => Some("Jobs by Remotive — remotive.com"),
            Self::RemoteOk => Some("Remote OK — remoteok.com"),
            Self::Himalayas => Some("Jobs by Himalayas — himalayas.app"),
            Self::WeWorkRemotely => Some("We Work Remotely — weworkremotely.com"),
            Self::HackerNews => Some("Hacker News — news.ycombinator.com"),
            Self::Arbeitnow => None,
        }
    }

    pub fn manifest(self) -> SourceManifest {
        SourceManifest {
            id: self.key().to_string(),
            name: self.name().to_string(),
            access_class: self.access_class(),
            legal_tier: LegalTier::Safe,
            markets: self
                .markets()
                .iter()
                .map(|market| market.to_string())
                .collect(),
            rate_limits: self.limits(),
            attribution: self.attribution().map(str::to_string),
            // Manual only: a feed links out, Boba never submits for it (A.3).
            apply_capability: ApplyCapability::LinkOut,
            required_secrets: Vec::new(),
            config_schema: None,
        }
    }
}

/// Build the adapter for one feed with its production endpoint.
pub fn adapter(kind: FeedKind) -> Box<dyn SourceAdapter> {
    match kind {
        FeedKind::Remotive => Box::new(Remotive::new()),
        FeedKind::RemoteOk => Box::new(RemoteOk::new()),
        FeedKind::Himalayas => Box::new(Himalayas::new()),
        FeedKind::Arbeitnow => Box::new(Arbeitnow::new()),
        FeedKind::WeWorkRemotely => Box::new(WeWorkRemotely::new()),
        FeedKind::HackerNews => Box::new(HackerNews::new()),
    }
}

/// Every built-in remote feed, ready to register in the [`crate::registry`].
pub fn builtin() -> Vec<Box<dyn SourceAdapter>> {
    FeedKind::ALL.into_iter().map(adapter).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_feed_is_a_safe_link_out_source_with_its_id() {
        let adapters = builtin();
        assert_eq!(adapters.len(), FeedKind::ALL.len());
        for adapter in &adapters {
            let manifest = adapter.manifest();
            assert!(!manifest.access_class.requires_opt_in());
            assert_eq!(manifest.legal_tier, LegalTier::Safe);
            assert_eq!(manifest.apply_capability, ApplyCapability::LinkOut);
            assert!(manifest.required_secrets.is_empty());
            assert!(!manifest.id.is_empty());
            assert!(!manifest.name.is_empty());
        }
    }

    #[test]
    fn the_documented_limits_are_declared() {
        assert_eq!(FeedKind::Remotive.limits().daily_cap, Some(4));
        assert!(FeedKind::Remotive.limits().min_interval_ms >= 30_000);
        assert_eq!(FeedKind::WeWorkRemotely.access_class(), AccessClass::Rss);
        for kind in FeedKind::ALL {
            if kind != FeedKind::Remotive {
                assert_eq!(kind.limits().daily_cap, None, "{kind:?}");
            }
        }
    }

    #[test]
    fn feeds_that_ask_for_attribution_declare_it() {
        assert!(FeedKind::Remotive.attribution().is_some());
        assert!(FeedKind::RemoteOk.attribution().is_some());
        assert!(FeedKind::Himalayas.attribution().is_some());
        assert!(FeedKind::WeWorkRemotely.attribution().is_some());
        assert_eq!(FeedKind::Arbeitnow.attribution(), None);
    }
}
