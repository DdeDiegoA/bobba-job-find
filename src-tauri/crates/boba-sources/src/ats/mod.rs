//! ATS board adapters (S4.5, FR-23, FR-27, addendum A.2).
//!
//! Five public job-board endpoints share one shape: a company token in, a list
//! of postings out, JSON, no auth. [`detect`] recognises a board from a careers
//! URL or an `ats:token` pair, each adapter implements the SDK's
//! [`SourceAdapter`], and [`test_fetch`] runs the "add a board" probe whose
//! count, sample and [`SourceRecord`] back the save in the Connections screen.
//!
//! Apply channel (S4.5 AC2, addendum A.3): Greenhouse, Lever, Ashby and
//! Workable expose a hosted form (`ats_form`); SmartRecruiters only links out
//! (`link_out`).

mod ashby;
mod greenhouse;
mod json;
mod lever;
mod smartrecruiters;
mod workable;

use boba_domain::source::{
    AccessClass, ApplyCapability, LegalTier, RateLimits, RawPosting, SearchQuery, SourceError,
    SourceManifest, SourceRecord,
};
use tokio_util::sync::CancellationToken;

use crate::http::HttpClient;
use crate::sdk::{SourceAdapter, SourceCtx};

pub use ashby::Ashby;
pub use greenhouse::Greenhouse;
pub use lever::Lever;
pub use smartrecruiters::SmartRecruiters;
pub use workable::Workable;

#[cfg(test)]
mod tests;

/// How many postings the add-board probe returns as its sample.
pub const SAMPLE_SIZE: usize = 5;

/// One supported ATS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AtsKind {
    Greenhouse,
    Lever,
    Ashby,
    Workable,
    SmartRecruiters,
}

impl AtsKind {
    /// Stable key used in source ids (`greenhouse:bobalabs`).
    pub fn key(self) -> &'static str {
        match self {
            Self::Greenhouse => "greenhouse",
            Self::Lever => "lever",
            Self::Ashby => "ashby",
            Self::Workable => "workable",
            Self::SmartRecruiters => "smartrecruiters",
        }
    }

    /// Display name for the connections UI.
    pub fn name(self) -> &'static str {
        match self {
            Self::Greenhouse => "Greenhouse",
            Self::Lever => "Lever",
            Self::Ashby => "Ashby",
            Self::Workable => "Workable",
            Self::SmartRecruiters => "SmartRecruiters",
        }
    }

    /// How an application can be sent for this board (S4.5 AC2, A.3).
    pub fn apply_capability(self) -> ApplyCapability {
        match self {
            Self::Greenhouse | Self::Lever | Self::Ashby | Self::Workable => {
                ApplyCapability::AtsForm
            }
            Self::SmartRecruiters => ApplyCapability::LinkOut,
        }
    }
}

/// One company board: which ATS hosts it and the company's token on that ATS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardRef {
    pub kind: AtsKind,
    /// The board token / company slug, exactly as the ATS spells it.
    pub token: String,
    /// Lever only: the EU host (`api.eu.lever.co`, A.2).
    pub eu: bool,
}

impl BoardRef {
    pub fn new(kind: AtsKind, token: &str) -> Result<Self, SourceError> {
        let token = token.trim();
        if !valid_token(token) {
            return Err(SourceError::InvalidValue {
                field: "ats board token".into(),
                value: token.to_string(),
            });
        }
        Ok(Self {
            kind,
            token: token.to_string(),
            eu: false,
        })
    }

    /// A Lever board on the EU host (A.2: `api.eu.lever.co`).
    pub fn lever_eu(token: &str) -> Result<Self, SourceError> {
        let mut board = Self::new(AtsKind::Lever, token)?;
        board.eu = true;
        Ok(board)
    }

    /// `lever`, or `lever-eu` when the board lives on the EU host.
    pub fn key(&self) -> &'static str {
        if self.kind == AtsKind::Lever && self.eu {
            "lever-eu"
        } else {
            self.kind.key()
        }
    }

    /// The registry id and `SourcesRepo` primary key (`greenhouse:bobalabs`).
    pub fn source_id(&self) -> String {
        format!("{}:{}", self.key(), self.token)
    }

    pub fn manifest(&self) -> SourceManifest {
        SourceManifest {
            id: self.source_id(),
            name: format!("{} · {}", self.kind.name(), self.token),
            access_class: AccessClass::PublicJson,
            legal_tier: LegalTier::Safe,
            markets: Vec::new(),
            rate_limits: RateLimits::default(),
            attribution: None,
            apply_capability: self.kind.apply_capability(),
            required_secrets: Vec::new(),
            config_schema: None,
        }
    }

    /// The row the caller persists with `SourcesRepo::upsert_source` once the
    /// user accepts the probe (S4.5 AC1).
    pub fn record(&self, now: &str) -> SourceRecord {
        let manifest = self.manifest();
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
}

/// Recognise the ATS from a careers URL, an API URL, or an `ats:token` pair.
///
/// A bare token is ambiguous (the same slug exists on several ATSs), so the
/// caller that already knows the ATS uses [`BoardRef::new`] instead.
pub fn detect(input: &str) -> Result<BoardRef, SourceError> {
    let input = input.trim();
    let invalid = || SourceError::InvalidValue {
        field: "ats board".into(),
        value: input.to_string(),
    };
    if input.is_empty() {
        return Err(invalid());
    }

    if !input.contains("://")
        && let Some((prefix, token)) = input.split_once(':')
        && let Some((kind, eu)) = kind_from_key(prefix)
    {
        let mut board = BoardRef::new(kind, token)?;
        board.eu = eu;
        return Ok(board);
    }

    let (host, segments) = split_url(input).ok_or_else(invalid)?;
    let (kind, eu, token) = match_ats(&host, &segments).ok_or_else(invalid)?;
    BoardRef::new(kind, token).map(|mut board| {
        board.eu = eu;
        board
    })
}

fn kind_from_key(prefix: &str) -> Option<(AtsKind, bool)> {
    match prefix.trim().to_ascii_lowercase().as_str() {
        "greenhouse" => Some((AtsKind::Greenhouse, false)),
        "lever" => Some((AtsKind::Lever, false)),
        "lever-eu" | "lever_eu" => Some((AtsKind::Lever, true)),
        "ashby" => Some((AtsKind::Ashby, false)),
        "workable" => Some((AtsKind::Workable, false)),
        "smartrecruiters" | "smart-recruiters" => Some((AtsKind::SmartRecruiters, false)),
        _ => None,
    }
}

/// Lower-cased host plus the path segments, scheme/query/fragment removed.
fn split_url(input: &str) -> Option<(String, Vec<&str>)> {
    let after_scheme = match input.split_once("://") {
        Some((_, rest)) => rest,
        None => input,
    };
    let host_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let host = &after_scheme[..host_end];
    let host = host.rsplit('@').next()?.split(':').next()?;
    let host = host
        .strip_prefix("www.")
        .unwrap_or(host)
        .to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    let path = after_scheme[host_end..]
        .split(['?', '#'])
        .next()
        .unwrap_or("");
    let segments: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    // A real careers URL has no dot segments; refusing them keeps a pasted path
    // from being read as a token.
    if segments.contains(&".") || segments.contains(&"..") {
        return None;
    }
    Some((host, segments))
}

/// Map one parsed URL to (ATS, Lever-EU, token).
fn match_ats<'a>(host: &'a str, segments: &[&'a str]) -> Option<(AtsKind, bool, &'a str)> {
    let first = |segments: &[&'a str]| segments.first().copied();
    match host {
        "boards.greenhouse.io" | "job-boards.greenhouse.io" => {
            first(segments).map(|token| (AtsKind::Greenhouse, false, token))
        }
        "boards-api.greenhouse.io" => match segments {
            ["v1", "boards", token, ..] => Some((AtsKind::Greenhouse, false, token)),
            _ => None,
        },
        "jobs.lever.co" => first(segments).map(|token| (AtsKind::Lever, false, token)),
        "api.lever.co" => match segments {
            ["v0", "postings", token, ..] => Some((AtsKind::Lever, false, token)),
            _ => None,
        },
        "jobs.eu.lever.co" => first(segments).map(|token| (AtsKind::Lever, true, token)),
        "api.eu.lever.co" => match segments {
            ["v0", "postings", token, ..] => Some((AtsKind::Lever, true, token)),
            _ => None,
        },
        "jobs.ashbyhq.com" => first(segments).map(|token| (AtsKind::Ashby, false, token)),
        "api.ashbyhq.com" => match segments {
            ["posting-api", "job-board", token, ..] => Some((AtsKind::Ashby, false, token)),
            _ => None,
        },
        "apply.workable.com" => match segments {
            ["api", "v1", "widget", "accounts", token, ..] => {
                Some((AtsKind::Workable, false, token))
            }
            _ => first(segments).map(|token| (AtsKind::Workable, false, token)),
        },
        "jobs.smartrecruiters.com" | "careers.smartrecruiters.com" => {
            first(segments).map(|token| (AtsKind::SmartRecruiters, false, token))
        }
        "api.smartrecruiters.com" => match segments {
            ["v1", "companies", token, ..] => Some((AtsKind::SmartRecruiters, false, token)),
            _ => None,
        },
        other => other
            .strip_suffix(".workable.com")
            .filter(|subdomain| !subdomain.is_empty() && !subdomain.contains('.'))
            .map(|subdomain| (AtsKind::Workable, false, subdomain)),
    }
}

/// Tokens become URL path segments, so they stay inside a conservative set.
fn valid_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 128
        && !token.starts_with('.')
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~'))
}

/// Build the adapter for a board with its production endpoint.
pub fn adapter(board: &BoardRef) -> Box<dyn SourceAdapter> {
    match board.kind {
        AtsKind::Greenhouse => Box::new(Greenhouse::new(board.clone())),
        AtsKind::Lever => Box::new(Lever::new(board.clone())),
        AtsKind::Ashby => Box::new(Ashby::new(board.clone())),
        AtsKind::Workable => Box::new(Workable::new(board.clone())),
        AtsKind::SmartRecruiters => Box::new(SmartRecruiters::new(board.clone())),
    }
}

/// What the "add a board" test fetch shows before the user saves (S4.5 AC1).
#[derive(Debug, Clone, PartialEq)]
pub struct TestFetch {
    pub board: BoardRef,
    /// Every posting the board returned for this fetch.
    pub count: usize,
    /// The first [`SAMPLE_SIZE`] postings, for the preview.
    pub sample: Vec<RawPosting>,
}

impl TestFetch {
    /// The `SourceRecord` to save once the user accepts the preview.
    pub fn source_record(&self, now: &str) -> SourceRecord {
        self.board.record(now)
    }
}

/// Fetch the board once and return its count and a sample (S4.5 AC1).
pub async fn test_fetch(board: &BoardRef, http: &HttpClient) -> Result<TestFetch, SourceError> {
    probe(adapter(board).as_ref(), board, http).await
}

/// The probe over an already-built adapter; tests point it at a mock host.
pub(crate) async fn probe(
    adapter: &dyn SourceAdapter,
    board: &BoardRef,
    http: &HttpClient,
) -> Result<TestFetch, SourceError> {
    let ctx = SourceCtx {
        http,
        cancel: CancellationToken::new(),
    };
    let postings = adapter.search(&SearchQuery::default(), &ctx).await?;
    let count = postings.len();
    let sample = postings.into_iter().take(SAMPLE_SIZE).collect();
    Ok(TestFetch {
        board: board.clone(),
        count,
        sample,
    })
}

impl BoardRef {
    /// Board limits used on every request (the manifest's).
    pub(crate) fn limits(&self) -> RateLimits {
        self.manifest().rate_limits
    }
}
