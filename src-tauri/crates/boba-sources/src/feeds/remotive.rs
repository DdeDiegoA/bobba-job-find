//! Remotive feed adapter (S4.6, integrations.md §2, addendum A.2).
//!
//! `GET /api/remote-jobs` returns every live remote job as one JSON object;
//! there is no detail endpoint, so `detail` finds the id in the same list.
//! Remotive asks for at most **4 fetches a day** and requires attribution —
//! both are declared on the manifest, and the shared client enforces the cap
//! because every request carries the source id as its budget key.

use boba_domain::source::{RateLimits, RawPosting, SearchQuery, SourceError, SourceManifest};
use serde_json::Value;

use super::FeedKind;
use super::api;
use crate::sdk::{BoxFuture, SourceAdapter, SourceCtx};

/// The public feed host (tests point the adapter at a mock origin).
pub const API: &str = "https://remotive.com";

/// The feed path on that host.
const PATH: &str = "/api/remote-jobs";

const NAME: &str = "Remotive";

pub struct Remotive {
    base: String,
    limits: RateLimits,
}

impl Remotive {
    pub fn new() -> Self {
        Self {
            base: API.to_string(),
            limits: FeedKind::Remotive.limits(),
        }
    }

    #[cfg(test)]
    pub(crate) fn test_with(base: impl Into<String>, limits: RateLimits) -> Self {
        Self {
            base: base.into(),
            limits,
        }
    }

    /// The list URL, with the query as Remotive's own `search` parameter.
    fn list_url(&self, query: &SearchQuery) -> String {
        let terms: Vec<&str> = query
            .roles
            .iter()
            .chain(query.keywords.iter())
            .map(String::as_str)
            .map(str::trim)
            .filter(|term| !term.is_empty())
            .collect();
        let mut url = format!("{}{PATH}?limit=100", self.base);
        if !terms.is_empty() {
            url.push_str("&search=");
            url.push_str(&api::encode_query(&terms.join(" ")));
        }
        url
    }

    async fn list(
        &self,
        query: &SearchQuery,
        ctx: &SourceCtx<'_>,
    ) -> Result<Vec<RawPosting>, SourceError> {
        let body = api::get_json(
            ctx.http,
            &self.list_url(query),
            self.limits,
            Some(FeedKind::Remotive.key()),
            None,
        )
        .await?;
        let payload = api::parse(&body)?;
        let jobs = api::array(&payload, "jobs")?;
        Ok(jobs.iter().filter_map(parse_job).collect())
    }
}

impl Default for Remotive {
    fn default() -> Self {
        Self::new()
    }
}

/// One job object to a `RawPosting`; `None` when it has no usable identity.
fn parse_job(job: &Value) -> Option<RawPosting> {
    let external_id = match job.get("id") {
        Some(Value::Number(id)) => id.to_string(),
        Some(Value::String(id)) if !id.trim().is_empty() => id.trim().to_string(),
        _ => return None,
    };
    let title = api::text(job, "title")?;
    // The payload is untrusted and this URL becomes a clickable link: only an
    // absolute https URL survives (the S4.5 rule, applied to feeds too).
    let url = api::https_url(api::text(job, "url"))
        .unwrap_or_else(|| format!("https://remotive.com/remote-jobs/{external_id}"));
    let location = api::text(job, "candidate_required_location");
    Some(RawPosting {
        external_id,
        title,
        company: api::text(job, "company_name").unwrap_or_else(|| NAME.to_string()),
        url,
        // Remotive only carries remote jobs, whatever the location says.
        remote: Some(true),
        locations: location.into_iter().collect(),
        description_html: api::text(job, "description"),
        posted_at: api::text(job, "publication_date").and_then(|date| api::utc_timestamp(&date)),
        salary_text: api::text(job, "salary"),
        raw: api::provenance(job),
    })
}

impl SourceAdapter for Remotive {
    fn manifest(&self) -> SourceManifest {
        FeedKind::Remotive.manifest()
    }

    fn search<'a>(
        &'a self,
        query: &'a SearchQuery,
        ctx: &'a SourceCtx<'_>,
    ) -> BoxFuture<'a, Result<Vec<RawPosting>, SourceError>> {
        Box::pin(async move {
            let postings = self.list(query, ctx).await?;
            Ok(api::retain_matching(postings, query))
        })
    }

    fn detail<'a>(
        &'a self,
        external_id: &'a str,
        ctx: &'a SourceCtx<'_>,
    ) -> BoxFuture<'a, Result<RawPosting, SourceError>> {
        Box::pin(async move {
            let postings = self.list(&SearchQuery::default(), ctx).await?;
            postings
                .into_iter()
                .find(|posting| posting.external_id == external_id)
                .ok_or_else(|| SourceError::InvalidPayload {
                    message: format!("remotive has no job '{external_id}'"),
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
        "/tests/fixtures/feeds/remotive_jobs.json"
    ));

    fn client() -> HttpClient {
        HttpClient::new(HttpClientConfig {
            min_interval: Duration::from_millis(0),
            retry_base_delay: Duration::from_millis(1),
            ..HttpClientConfig::default()
        })
        .expect("client")
    }

    fn limits(min_interval_ms: u64, daily_cap: Option<u32>) -> RateLimits {
        RateLimits {
            min_interval_ms,
            daily_cap,
        }
    }

    async fn serve(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path("/api/remote-jobs"))
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
    async fn search_reads_the_feed_and_keeps_only_https_urls() {
        let server = MockServer::start().await;
        serve(&server).await;
        let adapter = Remotive::test_with(server.uri(), limits(0, None));
        let http = client();

        let postings = adapter
            .search(&SearchQuery::default(), &ctx(&http))
            .await
            .unwrap();
        assert_eq!(postings.len(), 2);

        let first = &postings[0];
        assert_eq!(first.external_id, "1974000");
        assert_eq!(first.title, "Senior Backend Engineer");
        assert_eq!(first.company, "Boba Labs");
        assert_eq!(
            first.url,
            "https://remotive.com/remote/jobs/software-dev/senior-backend-engineer-1974000"
        );
        assert_eq!(first.locations, vec!["Worldwide"]);
        assert_eq!(first.remote, Some(true));
        assert!(
            first
                .description_html
                .as_deref()
                .unwrap()
                .contains("<b>payments</b>")
        );
        assert_eq!(
            first.posted_at.as_deref(),
            Some("2026-09-12T10:00:00Z"),
            "Remotive sends a naive timestamp; it is UTC"
        );
        assert_eq!(first.salary_text.as_deref(), Some("$120,000 - $160,000"));
        assert!(first.raw.contains("\"id\":1974000"));

        // The second job's `http:` URL is refused and replaced by the detail path.
        assert_eq!(postings[1].url, "https://remotive.com/remote-jobs/1974001");
    }

    #[tokio::test]
    async fn the_daily_budget_is_four_fetches() {
        let server = MockServer::start().await;
        serve(&server).await;
        let adapter = Remotive::test_with(server.uri(), limits(0, Some(4)));
        let http = client();
        for _ in 0..4 {
            adapter
                .search(&SearchQuery::default(), &ctx(&http))
                .await
                .expect("within the documented budget");
        }
        let error = adapter
            .search(&SearchQuery::default(), &ctx(&http))
            .await
            .expect_err("a fifth fetch exceeds Remotive's limit");
        assert!(matches!(
            error,
            SourceError::DailyCapExceeded { ref budget_key } if budget_key == "remotive"
        ));
    }

    #[tokio::test]
    async fn search_honours_the_query() {
        let server = MockServer::start().await;
        serve(&server).await;
        let adapter = Remotive::test_with(server.uri(), limits(0, None));
        let http = client();
        let query = SearchQuery {
            roles: vec!["designer".into()],
            ..SearchQuery::default()
        };
        let postings = adapter.search(&query, &ctx(&http)).await.unwrap();
        assert_eq!(postings.len(), 1);
        assert_eq!(postings[0].title, "Product Designer");
    }

    #[tokio::test]
    async fn detail_finds_one_job_in_the_feed() {
        let server = MockServer::start().await;
        serve(&server).await;
        let adapter = Remotive::test_with(server.uri(), limits(0, None));
        let http = client();
        let posting = adapter.detail("1974001", &ctx(&http)).await.unwrap();
        assert_eq!(posting.title, "Product Designer");

        let error = adapter
            .detail("nope", &ctx(&http))
            .await
            .expect_err("unknown");
        assert!(matches!(error, SourceError::InvalidPayload { .. }));
    }

    #[test]
    fn the_manifest_declares_the_documented_limits_and_attribution() {
        let manifest = FeedKind::Remotive.manifest();
        assert_eq!(manifest.id, "remotive");
        assert_eq!(manifest.rate_limits.daily_cap, Some(4));
        assert_eq!(manifest.rate_limits.min_interval_ms, 30_000);
        assert!(manifest.attribution.is_some());
        assert!(!manifest.requires_opt_in());
    }
}
