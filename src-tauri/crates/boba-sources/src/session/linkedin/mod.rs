//! LinkedIn Session Board adapter (S11.1b, FR-30, AD-9, AD-10).
//!
//! LinkedIn is browsed in the user's own logged-in session (S11.1a), never
//! through an API and never through an account Boba creates. The adapter is
//! deliberately split the way the ATS driver is (`boba-browser/src/forms`):
//!
//! * [`READ_SEARCH_JS`] and [`READ_DETAIL_JS`] are evaluated **in the page** by
//!   the session driver; they only read the DOM and return JSON. They never
//!   click, type, submit, create an account or answer a challenge.
//! * [`parse_search`] and [`parse_detail`] turn that JSON into [`RawPosting`]s
//!   here, pure and unit-tested against the fixtures in
//!   `tests/fixtures/session/`. The page payload is untrusted (AD-7): an
//!   unsafe URL or an unusable id drops one posting, never the board.
//!
//! A page that is a challenge — CAPTCHA, sign-in wall, "unusual activity",
//! rate limit — is reported as [`BoardError::Challenge`] and yields no
//! postings. The caller must hand the kind to
//! `SessionBoardService::report_challenge`, which pauses the Source and
//! notifies the user (AD-10); Boba never retries around a wall.
//!
//! Everything is opt-in and low volume: [`check_ready`] refuses a Source
//! without a live opt-in or with a recorded pause, and the manifest declares
//! the conservative pacing and daily cap ([`DAILY_CAP`]) every Session Board
//! Source starts from (FR-30).

mod page;

#[cfg(test)]
mod tests;

use boba_domain::source::{
    AccessClass, ApplyCapability, LegalTier, RateLimits, SearchQuery, SourceManifest, SourceRecord,
};

use crate::session::sdk::{BoardView, ChallengeKind, DEFAULT_PACING, RunError};

pub use page::{READ_DETAIL_JS, READ_SEARCH_JS, parse_detail, parse_search};

/// The registry id and `SourcesRepo` primary key.
pub const SOURCE_ID: &str = "linkedin";
/// The name the connections and privacy screens show.
pub const NAME: &str = "LinkedIn";
/// The site the session browses.
pub const BASE: &str = "https://www.linkedin.com";
/// The most page loads one day may spend here — a handful of searches and their
/// pages, never a crawl (FR-30: low volume).
pub const DAILY_CAP: u32 = 60;

/// Why a Session Board action was refused or a page could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BoardError {
    /// The Source has no recorded opt-in: nothing may run (AD-9).
    #[error("source '{source_id}' has no recorded opt-in")]
    OptInRequired { source_id: String },
    /// A challenge paused the Source; the user resumes it (AD-10).
    #[error("source '{source_id}' is paused after a {kind:?} challenge")]
    Paused {
        source_id: String,
        kind: ChallengeKind,
    },
    /// The page is a challenge: pause the Source and notify, never read on.
    #[error("the page asked for {0:?}; the Source must pause and the user be notified")]
    Challenge(ChallengeKind),
    /// The board record belongs to another Source, not this adapter's.
    #[error("this adapter drives '{expected}', not '{got}'")]
    WrongSource { expected: String, got: String },
    /// Today's page budget is spent (FR-30: low volume).
    #[error("source '{source_id}' already used its {cap} page loads today")]
    CapReached { source_id: String, cap: u32 },
    /// The page did not describe itself the way the script expects.
    #[error("the page could not be read: {0}")]
    Malformed(String),
}

/// Everything LinkedIn declares about itself (AC2).
pub fn manifest() -> SourceManifest {
    SourceManifest {
        id: SOURCE_ID.to_string(),
        name: NAME.to_string(),
        access_class: AccessClass::SessionBrowser,
        // LinkedIn's terms prohibit automated applying; Boba says so out loud.
        legal_tier: LegalTier::ProhibitedAutomation,
        markets: vec!["Global".to_string()],
        rate_limits: RateLimits {
            min_interval_ms: DEFAULT_PACING.min_interval_ms,
            daily_cap: Some(DAILY_CAP),
        },
        attribution: None,
        apply_capability: ApplyCapability::SessionForm,
        // The session lives in Chrome's profile encrypting its own cookies;
        // no secret ever reaches Boba's keychain or DB.
        required_secrets: Vec::new(),
        config_schema: None,
    }
}

/// The row the caller persists once the Source is configured (AC1, AC2).
pub fn record(now: &str) -> SourceRecord {
    let manifest = manifest();
    SourceRecord {
        id: manifest.id,
        name: manifest.name,
        access_class: manifest.access_class,
        legal_tier: manifest.legal_tier,
        apply_capability: manifest.apply_capability,
        attribution: manifest.attribution,
        min_interval_ms: manifest.rate_limits.min_interval_ms,
        daily_cap: manifest.rate_limits.daily_cap,
        enabled: true,
        added_at: now.to_string(),
        updated_at: now.to_string(),
    }
}

/// The run gate: a Source runs only with a live opt-in and no pause.
///
/// Fail closed on anything else — a Source the user disabled, a Source paused
/// by a challenge, a Source whose opt-in is gone.
pub fn check_ready(board: &BoardView) -> Result<(), BoardError> {
    if board.source_id != SOURCE_ID {
        return Err(BoardError::WrongSource {
            expected: SOURCE_ID.to_string(),
            got: board.source_id.clone(),
        });
    }
    // One gate for every Session Board Source (S11.1b, S11.1a R1).
    crate::session::sdk::check(board).map_err(BoardError::from)
}

impl From<RunError> for BoardError {
    fn from(error: RunError) -> Self {
        match error {
            RunError::OptInRequired { source_id } => Self::OptInRequired { source_id },
            RunError::Paused { source_id, kind } => Self::Paused { source_id, kind },
            RunError::CapReached { source_id, cap } => Self::CapReached { source_id, cap },
        }
    }
}

/// The search page for a query: roles and keywords as `keywords`, the first
/// location, and LinkedIn's remote filter (`f_WT=2`) when the query asks for it.
pub fn search_url(query: &SearchQuery) -> String {
    let mut params = Vec::new();
    let terms = terms(query);
    if !terms.is_empty() {
        params.push(format!("keywords={}", encode_query(&terms)));
    }
    if let Some(location) = query
        .locations
        .iter()
        .map(String::as_str)
        .map(str::trim)
        .find(|location| !location.is_empty())
    {
        params.push(format!("location={}", encode_query(location)));
    }
    if query.remote == Some(true) {
        params.push("f_WT=2".to_string());
    }
    with_params(&format!("{BASE}/jobs/search/"), &params)
}

/// The canonical page for one posting. The id must be a job number: anything
/// else could address a different page (an auth wall, a search) instead.
pub fn detail_url(external_id: &str) -> Result<String, BoardError> {
    if !valid_id(external_id) {
        return Err(BoardError::Malformed(format!(
            "'{external_id}' is not a LinkedIn job number"
        )));
    }
    Ok(format!("{BASE}/jobs/view/{external_id}/"))
}

/// LinkedIn ids are decimal job numbers.
pub(crate) fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 20 && id.bytes().all(|byte| byte.is_ascii_digit())
}

/// The query's roles and keywords, space-joined, as LinkedIn's `keywords`.
pub(crate) fn terms(query: &SearchQuery) -> String {
    query
        .roles
        .iter()
        .chain(query.keywords.iter())
        .map(String::as_str)
        .map(str::trim)
        .filter(|term| !term.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Append `params` to `url`, leaving it untouched when there are none.
pub(crate) fn with_params(url: &str, params: &[String]) -> String {
    if params.is_empty() {
        url.to_string()
    } else {
        format!("{url}?{}", params.join("&"))
    }
}

pub(crate) use crate::feeds::api::encode_query;
