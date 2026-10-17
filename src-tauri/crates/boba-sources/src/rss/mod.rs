//! Custom RSS/Atom feeds (S4.8, FR-26).
//!
//! The user pastes a feed URL; [`test_fetch`] fetches it once through the
//! shared client and returns a count plus a sample for the preview, and the
//! same [`CustomFeed`] runs in every Search Run once saved. Feeds are
//! `public_json`-grade, ungated, `link_out` sources: nothing about them is
//! automated beyond reading the publisher's own distribution channel.

use boba_domain::source::{
    AccessClass, ApplyCapability, LegalTier, RawPosting, SearchQuery, SourceError, SourceManifest,
};

use crate::feeds::api;
use crate::http::HttpClient;
use crate::sdk::{BoxFuture, SourceAdapter, SourceCtx};

#[cfg(test)]
mod tests;

/// How many postings the add-feed probe returns as its sample.
pub const SAMPLE_SIZE: usize = 5;

/// One user-supplied RSS or Atom feed (FR-26).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomFeed {
    /// Registry id and `SourcesRepo` primary key (`rss:<slug>`).
    pub id: String,
    /// The name the user gave it, or the feed's own title.
    pub name: String,
    /// The feed URL, already validated by [`validate_url`].
    pub url: String,
}

impl CustomFeed {
    /// Build a feed, validating the URL and its id.
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        url: &str,
    ) -> Result<Self, SourceError> {
        let id = id.into();
        let name = name.into().trim().to_string();
        if id.trim().is_empty() {
            return Err(SourceError::InvalidValue {
                field: "feed id".into(),
                value: id,
            });
        }
        Ok(Self {
            id,
            name: if name.is_empty() {
                "Custom feed".to_string()
            } else {
                name
            },
            url: validate_url(url)?,
        })
    }

    /// Tests point the feed at a mock origin, which is plain `http`.
    #[cfg(test)]
    pub(crate) fn test_with(id: &str, name: &str, url: impl Into<String>) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            url: url.into(),
        }
    }

    pub fn rate_limits(&self) -> boba_domain::source::RateLimits {
        boba_domain::source::RateLimits::default()
    }

    pub fn manifest(&self) -> SourceManifest {
        SourceManifest {
            id: self.id.clone(),
            name: self.name.clone(),
            access_class: AccessClass::Rss,
            legal_tier: LegalTier::Safe,
            markets: Vec::new(),
            rate_limits: self.rate_limits(),
            attribution: None,
            apply_capability: ApplyCapability::LinkOut,
            required_secrets: Vec::new(),
            config_schema: None,
        }
    }

    async fn list(&self, ctx: &SourceCtx<'_>) -> Result<Vec<RawPosting>, SourceError> {
        let body = api::get_feed(
            ctx.http,
            &self.url,
            self.rate_limits(),
            Some(self.id.as_str()),
        )
        .await?;
        parse_feed(&body)
    }
}

impl SourceAdapter for CustomFeed {
    fn manifest(&self) -> SourceManifest {
        CustomFeed::manifest(self)
    }

    fn search<'a>(
        &'a self,
        query: &'a SearchQuery,
        ctx: &'a SourceCtx<'_>,
    ) -> BoxFuture<'a, Result<Vec<RawPosting>, SourceError>> {
        Box::pin(async move {
            let postings = self.list(ctx).await?;
            Ok(api::retain_matching(postings, query))
        })
    }

    fn detail<'a>(
        &'a self,
        external_id: &'a str,
        ctx: &'a SourceCtx<'_>,
    ) -> BoxFuture<'a, Result<RawPosting, SourceError>> {
        Box::pin(async move {
            self.list(ctx)
                .await?
                .into_iter()
                .find(|posting| posting.external_id == external_id)
                .ok_or_else(|| SourceError::InvalidPayload {
                    message: format!("feed '{}' has no item '{external_id}'", self.id),
                })
        })
    }
}

/// What the "add a feed" test fetch shows before the user saves (AC2).
#[derive(Debug, Clone, PartialEq)]
pub struct FeedTest {
    pub feed: CustomFeed,
    /// Every posting the feed returned for this fetch.
    pub count: usize,
    /// The first [`SAMPLE_SIZE`] postings, for the preview.
    pub sample: Vec<RawPosting>,
}

/// Fetch the feed once and return its count and a sample (AC2).
pub async fn test_fetch(feed: &CustomFeed, http: &HttpClient) -> Result<FeedTest, SourceError> {
    let postings = feed
        .list(&SourceCtx {
            http,
            cancel: tokio_util::sync::CancellationToken::new(),
        })
        .await?;
    let count = postings.len();
    Ok(FeedTest {
        feed: feed.clone(),
        count,
        sample: postings.into_iter().take(SAMPLE_SIZE).collect(),
    })
}

/// Longest feed URL Boba accepts; anything longer is a paste accident.
const MAX_URL_LEN: usize = 2048;

/// A pasted feed URL must be `https`, carry no credentials and name a host.
pub fn validate_url(input: &str) -> Result<String, SourceError> {
    let input = input.trim();
    let invalid = || SourceError::InvalidValue {
        field: "feed url".into(),
        value: input.to_string(),
    };
    if input.is_empty()
        || input.len() > MAX_URL_LEN
        || input.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(invalid());
    }
    let rest = input
        .strip_prefix("https://")
        .or_else(|| input.strip_prefix("HTTPS://"))
        .ok_or_else(invalid)?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    // A password in the URL would be stored and shown; a feed never needs one.
    if authority.is_empty() || authority.contains('@') || authority.contains('\\') {
        return Err(invalid());
    }
    let host = authority.split(':').next().unwrap_or_default();
    if host.is_empty() || host.starts_with('.') || !public_host(host) {
        return Err(invalid());
    }
    Ok(input.to_string())
}

/// The SSRF guard (S4.1h hardening): a saved feed must name a public DNS host.
/// Loopback, private, link-local and literal IP hosts are refused, so a pasted
/// URL can never make Boba fetch its own machine or the local network.
fn public_host(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") {
        return false;
    }
    // A literal IPv6 host (or a bracketed anything) never names a public feed.
    if host.contains(':') || host.contains('[') || host.contains(']') {
        return false;
    }
    // A literal IPv4 host: only a public, routable address would pass, and a
    // feed URL never needs one.
    if host.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return false;
    }
    // Public DNS names have a dot; a dotless host is an intranet name.
    host.contains('.')
}

/// Parse an RSS or Atom document into postings.
///
/// Malformed documents are an `InvalidPayload`; an item without a title or a
/// usable identity is skipped rather than made up.
pub fn parse_feed(body: &[u8]) -> Result<Vec<RawPosting>, SourceError> {
    let feed = feed_rs::parser::parse(body).map_err(|error| SourceError::InvalidPayload {
        message: format!("invalid RSS/Atom: {error}"),
    })?;
    let feed_title = feed
        .title
        .as_ref()
        .map(|title| title.content.trim().to_string())
        .filter(|title| !title.is_empty());
    Ok(feed
        .entries
        .iter()
        .filter_map(|entry| entry_to_posting(entry, feed_title.as_deref()))
        .collect())
}

/// One feed entry to a `RawPosting`; `None` when it has no usable identity.
fn entry_to_posting(entry: &feed_rs::model::Entry, feed_title: Option<&str>) -> Option<RawPosting> {
    let title = entry.title.as_ref()?.content.trim();
    if title.is_empty() {
        return None;
    }
    let url = entry
        .links
        .iter()
        .find_map(|link| api::https_url(Some(link.href.clone())))
        .or_else(|| api::https_url(Some(entry.id.clone())))?;
    let external_id = if entry.id.trim().is_empty() {
        url.clone()
    } else {
        entry.id.clone()
    };
    let company = entry
        .authors
        .first()
        .and_then(|author| author.name.clone())
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .or_else(|| feed_title.map(str::to_string))
        .unwrap_or_else(|| "Custom feed".to_string());
    let description_html = entry
        .content
        .as_ref()
        .and_then(|content| content.body.clone())
        .or_else(|| {
            entry
                .summary
                .as_ref()
                .map(|summary| summary.content.clone())
        });
    Some(RawPosting {
        external_id,
        title: title.to_string(),
        company,
        url,
        locations: Vec::new(),
        remote: None,
        description_html,
        posted_at: entry
            .published
            .or(entry.updated)
            .map(|date| date.to_rfc3339()),
        salary_text: None,
        // The entry in feed-rs' own shape is the provenance (AD-19).
        raw: serde_json::to_string(entry).unwrap_or_default(),
    })
}
