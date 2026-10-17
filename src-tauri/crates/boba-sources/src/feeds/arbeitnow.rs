//! Arbeitnow feed adapter (S4.6, integrations.md §2, addendum A.2).
//!
//! `GET /api/job-board-api` returns `{ data: [...] }`, Europe-first. No limits
//! or attribution are documented for it. There is no detail endpoint, so
//! `detail` finds the slug in the same page.

use boba_domain::source::{RateLimits, RawPosting, SearchQuery, SourceError, SourceManifest};
use serde_json::Value;

use super::FeedKind;
use super::api;
use crate::sdk::{BoxFuture, SourceAdapter, SourceCtx};

/// The public feed host (tests point the adapter at a mock origin).
pub const API: &str = "https://www.arbeitnow.com";

/// The feed path on that host.
const PATH: &str = "/api/job-board-api";

const NAME: &str = "Arbeitnow";

pub struct Arbeitnow {
    base: String,
    limits: RateLimits,
}

impl Arbeitnow {
    pub fn new() -> Self {
        Self {
            base: API.to_string(),
            limits: FeedKind::Arbeitnow.limits(),
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
        let body = api::get_json(
            ctx.http,
            &format!("{}{PATH}", self.base),
            self.limits,
            Some(FeedKind::Arbeitnow.key()),
            None,
        )
        .await?;
        let payload = api::parse(&body)?;
        let jobs = api::array(&payload, "data")?;
        Ok(jobs.iter().filter_map(parse_job).collect())
    }
}

impl Default for Arbeitnow {
    fn default() -> Self {
        Self::new()
    }
}

/// One job object to a `RawPosting`; `None` when it has no usable identity.
fn parse_job(job: &Value) -> Option<RawPosting> {
    let external_id = api::text(job, "slug")?;
    let title = api::text(job, "title")?;
    let url = api::https_url(api::text(job, "url"))
        .unwrap_or_else(|| format!("https://www.arbeitnow.com/view/{external_id}"));
    let location = api::text(job, "location");
    Some(RawPosting {
        external_id,
        title,
        company: api::text(job, "company_name").unwrap_or_else(|| NAME.to_string()),
        url,
        remote: api::boolean(job, "remote"),
        locations: location.into_iter().collect(),
        description_html: api::text(job, "description"),
        posted_at: api::integer(job, "created_at").and_then(api::rfc3339_from_secs),
        salary_text: None,
        raw: api::provenance(job),
    })
}

impl SourceAdapter for Arbeitnow {
    fn manifest(&self) -> SourceManifest {
        FeedKind::Arbeitnow.manifest()
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
                    message: format!("arbeitnow has no job '{external_id}'"),
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

    const JOBS: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/feeds/arbeitnow_jobs.json"
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
            .and(path("/api/job-board-api"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(JOBS, "application/json"))
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
    async fn search_reads_the_data_array_and_keeps_https_urls() {
        let server = MockServer::start().await;
        serve(&server).await;
        let adapter = Arbeitnow::test_with(server.uri(), RateLimits::default());
        let http = client();

        let postings = adapter
            .search(&SearchQuery::default(), &ctx(&http))
            .await
            .unwrap();
        assert_eq!(postings.len(), 2);

        let first = &postings[0];
        assert_eq!(first.external_id, "senior-rust-engineer-berlin");
        assert_eq!(first.title, "Senior Rust Engineer");
        assert_eq!(first.company, "Boba Labs GmbH");
        assert_eq!(
            first.url,
            "https://www.arbeitnow.com/view/senior-rust-engineer-berlin"
        );
        assert_eq!(first.locations, vec!["Berlin"]);
        assert_eq!(first.remote, Some(true));
        assert_eq!(first.posted_at.as_deref(), Some("2026-10-03T00:00:00Z"));

        // A `javascript:` URL is refused; the adapter builds the board URL.
        assert_eq!(
            postings[1].url,
            "https://www.arbeitnow.com/view/office-manager-munich"
        );
    }

    #[tokio::test]
    async fn detail_finds_one_job() {
        let server = MockServer::start().await;
        serve(&server).await;
        let adapter = Arbeitnow::test_with(server.uri(), RateLimits::default());
        let http = client();
        let posting = adapter
            .detail("office-manager-munich", &ctx(&http))
            .await
            .unwrap();
        assert_eq!(posting.title, "Office Manager");
        assert_eq!(posting.remote, Some(false));
    }

    #[test]
    fn the_manifest_documents_no_limits() {
        let manifest = FeedKind::Arbeitnow.manifest();
        assert_eq!(manifest.id, "arbeitnow");
        assert_eq!(manifest.rate_limits, RateLimits::default());
        assert_eq!(manifest.attribution, None, "Arbeitnow asks for none");
    }
}
