//! RemoteOK feed adapter (S4.6, integrations.md §2, addendum A.2).
//!
//! `GET /api` returns an array whose **first element is a legal-notice object**,
//! not a job; the rest are the newest 100 postings. RemoteOK requires a
//! backlink, declared on the manifest as attribution. There is no detail
//! endpoint, so `detail` finds the id in the same list.

use boba_domain::source::{RateLimits, RawPosting, SearchQuery, SourceError, SourceManifest};
use serde_json::Value;

use super::FeedKind;
use super::api;
use crate::sdk::{BoxFuture, SourceAdapter, SourceCtx};

/// The public feed host (tests point the adapter at a mock origin).
pub const API: &str = "https://remoteok.com";

/// The feed path on that host.
const PATH: &str = "/api";

const NAME: &str = "RemoteOK";

pub struct RemoteOk {
    base: String,
    limits: RateLimits,
}

impl RemoteOk {
    pub fn new() -> Self {
        Self {
            base: API.to_string(),
            limits: FeedKind::RemoteOk.limits(),
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
            Some(FeedKind::RemoteOk.key()),
            None,
        )
        .await?;
        let payload = api::parse(&body)?;
        let jobs = payload
            .as_array()
            .ok_or_else(|| SourceError::InvalidPayload {
                message: "remoteok returned a non-array payload".into(),
            })?;
        Ok(jobs.iter().filter_map(parse_job).collect())
    }
}

impl Default for RemoteOk {
    fn default() -> Self {
        Self::new()
    }
}

/// One array element to a `RawPosting`. The legal-notice element has no
/// `position` and is skipped by the same check that guards malformed rows.
fn parse_job(job: &Value) -> Option<RawPosting> {
    let external_id = match job.get("id") {
        Some(Value::Number(id)) => id.to_string(),
        Some(Value::String(id)) if !id.trim().is_empty() => id.trim().to_string(),
        _ => return None,
    };
    let title = api::text(job, "position")?;
    let url = api::https_url(api::text(job, "url"))
        .or_else(|| api::https_url(api::text(job, "apply_url")))
        .unwrap_or_else(|| format!("https://remoteok.com/remote-jobs/{external_id}"));
    let location = api::text(job, "location");
    Some(RawPosting {
        external_id,
        title,
        company: api::text(job, "company").unwrap_or_else(|| NAME.to_string()),
        url,
        remote: Some(true),
        locations: location.into_iter().collect(),
        description_html: api::text(job, "description"),
        posted_at: api::integer(job, "epoch").and_then(api::rfc3339_from_secs),
        salary_text: salary_text(job),
        raw: api::provenance(job),
    })
}

/// RemoteOK reports an annual USD range; show it only when one bound exists.
fn salary_text(job: &Value) -> Option<String> {
    let min = api::number(job, "salary_min").filter(|value| *value > 0.0);
    let max = api::number(job, "salary_max").filter(|value| *value > 0.0);
    let range = match (min, max) {
        (Some(low), Some(high)) => format!("USD {low:.0} – {high:.0} / year"),
        (Some(low), None) => format!("USD from {low:.0} / year"),
        (None, Some(high)) => format!("USD up to {high:.0} / year"),
        (None, None) => return None,
    };
    Some(range)
}

impl SourceAdapter for RemoteOk {
    fn manifest(&self) -> SourceManifest {
        FeedKind::RemoteOk.manifest()
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
                    message: format!("remoteok has no job '{external_id}'"),
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
        "/tests/fixtures/feeds/remoteok_jobs.json"
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
            .and(path("/api"))
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
    async fn search_skips_the_legal_notice_and_keeps_https_urls() {
        let server = MockServer::start().await;
        serve(&server).await;
        let adapter = RemoteOk::test_with(server.uri(), RateLimits::default());
        let http = client();

        let postings = adapter
            .search(&SearchQuery::default(), &ctx(&http))
            .await
            .unwrap();
        assert_eq!(postings.len(), 2, "the legal-notice element is not a job");

        let first = &postings[0];
        assert_eq!(first.external_id, "1130001");
        assert_eq!(first.title, "Staff Platform Engineer");
        assert_eq!(first.company, "Boba Labs");
        assert_eq!(
            first.url,
            "https://remoteok.com/remote-jobs/staff-platform-engineer-1130001"
        );
        assert_eq!(first.remote, Some(true));
        assert_eq!(first.posted_at.as_deref(), Some("2026-10-01T00:00:00Z"));
        assert_eq!(
            first.salary_text.as_deref(),
            Some("USD 150000 – 190000 / year")
        );

        // The second job sends an `http:` URL: the adapter builds its own.
        assert_eq!(postings[1].url, "https://remoteok.com/remote-jobs/1130002");
    }

    #[tokio::test]
    async fn the_query_filters_titles() {
        let server = MockServer::start().await;
        serve(&server).await;
        let adapter = RemoteOk::test_with(server.uri(), RateLimits::default());
        let http = client();
        let query = SearchQuery {
            roles: vec!["technical writer".into()],
            ..SearchQuery::default()
        };
        let postings = adapter.search(&query, &ctx(&http)).await.unwrap();
        assert_eq!(postings.len(), 1);
        assert_eq!(postings[0].title, "Technical Writer");
    }

    #[tokio::test]
    async fn detail_finds_one_job() {
        let server = MockServer::start().await;
        serve(&server).await;
        let adapter = RemoteOk::test_with(server.uri(), RateLimits::default());
        let http = client();
        let posting = adapter.detail("1130002", &ctx(&http)).await.unwrap();
        assert_eq!(posting.title, "Technical Writer");
    }

    #[test]
    fn the_manifest_requires_a_backlink_and_has_no_daily_cap() {
        let manifest = FeedKind::RemoteOk.manifest();
        assert_eq!(manifest.id, "remoteok");
        assert_eq!(manifest.rate_limits.daily_cap, None, "not documented");
        assert!(manifest.attribution.unwrap().contains("remoteok.com"));
    }
}
