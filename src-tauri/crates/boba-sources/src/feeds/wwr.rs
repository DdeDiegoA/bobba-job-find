//! WeWorkRemotely RSS adapter (S4.6, integrations.md §2, addendum A.2/A.3).
//!
//! `GET /remote-jobs.rss` is the publisher's own distribution channel, parsed
//! with `feed-rs` (tech-stack.md pins it as the one feed parser). The RSS title
//! is `Company: Role`; the item's guid carries the posting URL.
//!
//! WeWorkRemotely **forbids applying outside its own site**, so its manifest is
//! `link_out` (Manual channel only, AC3): Boba never submits for it.

use boba_domain::source::{RateLimits, RawPosting, SearchQuery, SourceError, SourceManifest};

use super::FeedKind;
use super::api;
use crate::sdk::{BoxFuture, SourceAdapter, SourceCtx};

/// The public feed host (tests point the adapter at a mock origin).
pub const API: &str = "https://weworkremotely.com";

/// The RSS path on that host.
const PATH: &str = "/remote-jobs.rss";

const NAME: &str = "We Work Remotely";

pub struct WeWorkRemotely {
    base: String,
    limits: RateLimits,
}

impl WeWorkRemotely {
    pub fn new() -> Self {
        Self {
            base: API.to_string(),
            limits: FeedKind::WeWorkRemotely.limits(),
        }
    }

    #[cfg(test)]
    pub(crate) fn test_with(base: impl Into<String>, limits: RateLimits) -> Self {
        Self {
            base: base.into(),
            limits,
        }
    }

    async fn list(&self, ctx: &SourceCtx<'_>) -> Result<Vec<RawPosting>, SourceError> {
        let body = api::get_feed(
            ctx.http,
            &format!("{}{PATH}", self.base),
            self.limits,
            Some(FeedKind::WeWorkRemotely.key()),
        )
        .await?;
        let feed = feed_rs::parser::parse(body.as_slice()).map_err(|error| {
            SourceError::InvalidPayload {
                message: format!("invalid RSS: {error}"),
            }
        })?;
        Ok(feed.entries.iter().filter_map(parse_entry).collect())
    }
}

impl Default for WeWorkRemotely {
    fn default() -> Self {
        Self::new()
    }
}

/// One RSS item to a `RawPosting`; `None` when it has no usable identity.
fn parse_entry(entry: &feed_rs::model::Entry) -> Option<RawPosting> {
    let title = entry.title.as_ref()?.content.trim();
    if title.is_empty() || entry.id.trim().is_empty() {
        return None;
    }
    // WWR titles are "Company: Role"; a title without the colon is the role.
    let (company, role) = match title.split_once(": ") {
        Some((company, role)) if !company.trim().is_empty() && !role.trim().is_empty() => {
            (company.trim().to_string(), role.trim().to_string())
        }
        _ => (NAME.to_string(), title.to_string()),
    };
    let url = entry
        .links
        .iter()
        .find_map(|link| api::https_url(Some(link.href.clone())))
        .or_else(|| api::https_url(Some(entry.id.clone())))
        .unwrap_or_else(|| "https://weworkremotely.com/".to_string());
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
        external_id: entry.id.clone(),
        title: role,
        company,
        url,
        remote: Some(true),
        locations: Vec::new(),
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

impl SourceAdapter for WeWorkRemotely {
    fn manifest(&self) -> SourceManifest {
        FeedKind::WeWorkRemotely.manifest()
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
            let postings = self.list(ctx).await?;
            postings
                .into_iter()
                .find(|posting| posting.external_id == external_id)
                .ok_or_else(|| SourceError::InvalidPayload {
                    message: format!("weworkremotely has no job '{external_id}'"),
                })
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::http::{HttpClient, HttpClientConfig};

    const FEED: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/feeds/weworkremotely.rss"
    ));

    fn client() -> HttpClient {
        HttpClient::new(HttpClientConfig {
            min_interval: Duration::from_millis(0),
            retry_base_delay: Duration::from_millis(1),
            ..HttpClientConfig::default()
        })
        .expect("client")
    }

    async fn serve(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path("/remote-jobs.rss"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(FEED, "application/rss+xml"))
            .mount(server)
            .await;
    }

    fn ctx(http: &HttpClient) -> SourceCtx<'_> {
        SourceCtx {
            http,
            cancel: tokio_util::sync::CancellationToken::new(),
        }
    }

    #[tokio::test]
    async fn search_parses_the_rss_and_splits_company_from_title() {
        let server = MockServer::start().await;
        serve(&server).await;
        let adapter = WeWorkRemotely::test_with(server.uri(), RateLimits::default());
        let http = client();

        let postings = adapter
            .search(&SearchQuery::default(), &ctx(&http))
            .await
            .unwrap();
        assert_eq!(postings.len(), 2);

        let first = &postings[0];
        assert_eq!(first.company, "Boba Labs");
        assert_eq!(first.title, "Senior Rust Engineer");
        assert_eq!(
            first.url,
            "https://weworkremotely.com/remote-jobs/boba-labs-senior-rust-engineer"
        );
        assert_eq!(first.remote, Some(true));
        assert_eq!(
            first.posted_at.as_deref(),
            Some("2026-10-12T08:00:00+00:00")
        );
        assert!(
            first
                .description_html
                .as_deref()
                .unwrap()
                .contains("Build <b>payments</b>")
        );
        assert!(first.raw.contains("Boba Labs"));

        // The second item's `http:` link and guid are refused: the board URL is used.
        assert_eq!(postings[1].url, "https://weworkremotely.com/");
    }

    #[tokio::test]
    async fn detail_finds_one_item() {
        let server = MockServer::start().await;
        serve(&server).await;
        let adapter = WeWorkRemotely::test_with(server.uri(), RateLimits::default());
        let http = client();
        let posting = adapter
            .detail(
                "https://weworkremotely.com/remote-jobs/boba-labs-senior-rust-engineer",
                &ctx(&http),
            )
            .await
            .unwrap();
        assert_eq!(posting.title, "Senior Rust Engineer");
    }

    #[test]
    fn the_manifest_is_manual_only() {
        let manifest = FeedKind::WeWorkRemotely.manifest();
        assert_eq!(manifest.id, "weworkremotely");
        assert_eq!(
            manifest.apply_capability,
            boba_domain::source::ApplyCapability::LinkOut,
            "WeWorkRemotely disallows applying anywhere else (AC3)"
        );
    }
}
