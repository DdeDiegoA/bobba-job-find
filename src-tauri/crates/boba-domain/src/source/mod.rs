//! Source-adapter vocabulary shared across the `SourcePort` and `SourcesRepo`
//! ports (AD-9, AD-10, AD-14).
//!
//! Pure: no IO. These types cross a port, so they live here and never in an
//! adapter crate (spine: every port type lives in `boba-domain`).

use serde::{Deserialize, Serialize};

/// How a source is accessed (AD-9, ADR-005 §1). The class drives the default
/// gate in the connections UI and the opt-in requirement in the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessClass {
    OfficialApi,
    PublicJson,
    Rss,
    HtmlScrape,
    SessionBrowser,
    ExternalProcess,
}

impl AccessClass {
    /// Gated classes cannot run until the user records a per-source opt-in (AD-9).
    pub fn requires_opt_in(self) -> bool {
        matches!(
            self,
            Self::HtmlScrape | Self::SessionBrowser | Self::ExternalProcess
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::OfficialApi => "official_api",
            Self::PublicJson => "public_json",
            Self::Rss => "rss",
            Self::HtmlScrape => "html_scrape",
            Self::SessionBrowser => "session_browser",
            Self::ExternalProcess => "external_process",
        }
    }
}

/// Legal posture declared by a source (AD-9, ADR-005 §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegalTier {
    Safe,
    Caution,
    /// Automated applying is prohibited by the platform's terms (e.g. LinkedIn).
    ProhibitedAutomation,
}

impl LegalTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Caution => "caution",
            Self::ProhibitedAutomation => "prohibited_automation",
        }
    }
}

/// How an application can be sent for a source (AD-9, AD-11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyCapability {
    None,
    LinkOut,
    AtsForm,
    SanctionedApi,
    Email,
    SessionForm,
}

impl ApplyCapability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::LinkOut => "link_out",
            Self::AtsForm => "ats_form",
            Self::SanctionedApi => "sanctioned_api",
            Self::Email => "email",
            Self::SessionForm => "session_form",
        }
    }
}

/// Parsing failures for the DB-backed `*_as_str` values.
macro_rules! parse_as_str {
    ($name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        impl std::str::FromStr for $name {
            type Err = SourceError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $($text => Ok(Self::$variant),)+
                    other => Err(SourceError::InvalidValue {
                        field: stringify!($name).to_string(),
                        value: other.to_string(),
                    }),
                }
            }
        }
    };
}

parse_as_str!(AccessClass {
    OfficialApi => "official_api",
    PublicJson => "public_json",
    Rss => "rss",
    HtmlScrape => "html_scrape",
    SessionBrowser => "session_browser",
    ExternalProcess => "external_process",
});
parse_as_str!(LegalTier {
    Safe => "safe",
    Caution => "caution",
    ProhibitedAutomation => "prohibited_automation",
});
parse_as_str!(ApplyCapability {
    None => "none",
    LinkOut => "link_out",
    AtsForm => "ats_form",
    SanctionedApi => "sanctioned_api",
    Email => "email",
    SessionForm => "session_form",
});

/// Rate limits a manifest declares; the shared client takes the stricter of
/// these and its global per-host default (AD-10, FR-32).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateLimits {
    /// Minimum interval between two requests to one host, in milliseconds.
    pub min_interval_ms: u64,
    /// Optional per-source daily request budget (e.g. Remotive ≤ 4/day).
    pub daily_cap: Option<u32>,
}

impl Default for RateLimits {
    fn default() -> Self {
        Self {
            min_interval_ms: 2000,
            daily_cap: None,
        }
    }
}

/// Everything a source declares about itself (AD-9). The registry gates on
/// `access_class`; the client enforces `rate_limits`; the UI shows `attribution`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceManifest {
    pub id: String,
    pub name: String,
    pub access_class: AccessClass,
    pub legal_tier: LegalTier,
    pub markets: Vec<String>,
    pub rate_limits: RateLimits,
    pub attribution: Option<String>,
    pub apply_capability: ApplyCapability,
    pub required_secrets: Vec<String>,
    /// JSON-Schema text for the source's user config, if any.
    pub config_schema: Option<String>,
}

impl SourceManifest {
    /// Whether this source needs a recorded opt-in before it may run (AD-9).
    pub fn requires_opt_in(&self) -> bool {
        self.access_class.requires_opt_in()
    }
}

/// A search request derived from the profile's preferences. Adapters map it
/// best-effort and report the fields they ignored (architecture §6.1).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchQuery {
    pub roles: Vec<String>,
    pub keywords: Vec<String>,
    pub locations: Vec<String>,
    pub remote: Option<bool>,
    pub languages: Vec<String>,
    pub markets: Vec<String>,
}

/// A posting exactly as a source returned it, before normalisation (AD-9/AD-19).
/// `description_html` is untrusted (AD-7): sanitise before any UI use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawPosting {
    pub external_id: String,
    pub title: String,
    pub company: String,
    pub url: String,
    pub locations: Vec<String>,
    pub remote: Option<bool>,
    pub description_html: Option<String>,
    pub posted_at: Option<String>,
    pub salary_text: Option<String>,
    /// The source payload as JSON text, kept as provenance (AD-19). Untrusted.
    pub raw: String,
}

/// A configured source as stored in the `source` table (AD-16).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceRecord {
    pub id: String,
    pub name: String,
    pub access_class: AccessClass,
    pub legal_tier: LegalTier,
    pub apply_capability: ApplyCapability,
    pub attribution: Option<String>,
    pub min_interval_ms: u64,
    pub daily_cap: Option<u32>,
    pub enabled: bool,
    pub added_at: String,
    pub updated_at: String,
}

impl SourceRecord {
    /// The client limits this source asks for.
    pub fn rate_limits(&self) -> RateLimits {
        RateLimits {
            min_interval_ms: self.min_interval_ms,
            daily_cap: self.daily_cap,
        }
    }
}

/// A recorded per-source opt-in (AD-9): which warning the user accepted, when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptInRecord {
    pub source_id: String,
    pub warning_version: String,
    pub accepted_at: String,
}

/// Lifecycle of one `source_run` row (AD-16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Ok,
    Failed,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Ok => "ok",
            Self::Failed => "failed",
        }
    }
}

parse_as_str!(RunStatus {
    Running => "running",
    Ok => "ok",
    Failed => "failed",
});

/// One execution of one source (AD-16).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRunRecord {
    pub id: String,
    pub source_id: String,
    pub status: RunStatus,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub postings_found: i64,
    pub error: Option<String>,
}

/// One source's contribution to a run. A failing source yields `Err` and never
/// aborts the rest of the run (S4.1 AC3).
#[derive(Debug, Clone, PartialEq)]
pub enum SourceFetch {
    Ok {
        source_id: String,
        postings: Vec<RawPosting>,
    },
    Err {
        source_id: String,
        error: SourceError,
    },
}

/// Errors from the source subsystem, shared by the client, registry and ports.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SourceError {
    #[error("source '{source_id}' requires an opt-in before it can run")]
    OptInRequired { source_id: String },
    #[error("unknown source '{source_id}'")]
    UnknownSource { source_id: String },
    #[error("a source with id '{source_id}' is already registered")]
    DuplicateSource { source_id: String },
    #[error("rate limited by '{host}'; retry in {retry_after_ms} ms")]
    RateLimited { host: String, retry_after_ms: u64 },
    #[error("daily request budget for '{budget_key}' is exhausted")]
    DailyCapExceeded { budget_key: String },
    #[error("robots.txt disallows {url}")]
    RobotsDisallowed { url: String },
    #[error("HTTP request for {url} failed: {message}")]
    Http { url: String, message: String },
    #[error("response for {url} exceeds the {limit} byte cap")]
    ResponseTooLarge { url: String, limit: usize },
    #[error("source returned an invalid payload: {message}")]
    InvalidPayload { message: String },
    #[error("invalid value '{value}' for {field}")]
    InvalidValue { field: String, value: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn only_gated_classes_require_opt_in() {
        assert!(!AccessClass::PublicJson.requires_opt_in());
        assert!(!AccessClass::Rss.requires_opt_in());
        assert!(!AccessClass::OfficialApi.requires_opt_in());
        assert!(AccessClass::HtmlScrape.requires_opt_in());
        assert!(AccessClass::SessionBrowser.requires_opt_in());
        assert!(AccessClass::ExternalProcess.requires_opt_in());
    }

    #[test]
    fn enums_round_trip_through_their_db_text() {
        for class in [
            AccessClass::OfficialApi,
            AccessClass::PublicJson,
            AccessClass::Rss,
            AccessClass::HtmlScrape,
            AccessClass::SessionBrowser,
            AccessClass::ExternalProcess,
        ] {
            assert_eq!(AccessClass::from_str(class.as_str()).unwrap(), class);
        }
        assert_eq!(
            ApplyCapability::from_str("session_form").unwrap(),
            ApplyCapability::SessionForm
        );
        assert_eq!(
            LegalTier::from_str("prohibited_automation").unwrap(),
            LegalTier::ProhibitedAutomation
        );
        assert!(AccessClass::from_str("telepathy").is_err());
    }

    #[test]
    fn manifest_gate_and_record_limits() {
        let manifest = SourceManifest {
            id: "linkedin".into(),
            name: "LinkedIn".into(),
            access_class: AccessClass::SessionBrowser,
            legal_tier: LegalTier::ProhibitedAutomation,
            markets: vec!["US".into()],
            rate_limits: RateLimits {
                min_interval_ms: 10_000,
                daily_cap: Some(3),
            },
            attribution: None,
            apply_capability: ApplyCapability::SessionForm,
            required_secrets: vec![],
            config_schema: None,
        };
        assert!(manifest.requires_opt_in());

        let record = SourceRecord {
            id: "remotive".into(),
            name: "Remotive".into(),
            access_class: AccessClass::PublicJson,
            legal_tier: LegalTier::Safe,
            apply_capability: ApplyCapability::LinkOut,
            attribution: Some("Jobs by Remotive".into()),
            min_interval_ms: 1000,
            daily_cap: Some(4),
            enabled: true,
            added_at: "2026-10-05T00:00:00Z".into(),
            updated_at: "2026-10-05T00:00:00Z".into(),
        };
        assert_eq!(record.rate_limits().daily_cap, Some(4));
        assert_eq!(RateLimits::default().min_interval_ms, 2000);
    }
}
