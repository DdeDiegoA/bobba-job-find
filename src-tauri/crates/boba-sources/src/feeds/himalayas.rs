//! Himalayas feed adapter (S4.6, integrations.md §2, addendum A.2).
//!
//! `GET /jobs/api?limit=20&offset=0` returns `{ jobs: [...], totalCount }`;
//! the limit is 20 per page, so the adapter reads the first page and lets the
//! query filter it locally. Attribution is required. There is no detail
//! endpoint, so `detail` finds the id in the same page.

use boba_domain::source::{RateLimits, RawPosting, SearchQuery, SourceError, SourceManifest};
use serde_json::Value;

use super::FeedKind;
use super::api;
use crate::sdk::{BoxFuture, SourceAdapter, SourceCtx};

/// The public feed host (tests point the adapter at a mock origin).
pub const API: &str = "https://himalayas.app";

/// The feed path on that host.
const PATH: &str = "/jobs/api";

/// The documented maximum page size.
pub const PAGE_SIZE: usize = 20;

const NAME: &str = "Himalayas";

pub struct Himalayas {
    base: String,
    limits: RateLimits,
}

impl Himalayas {
    pub fn new() -> Self {
        Self {
            base: API.to_string(),
            limits: FeedKind::Himalayas.limits(),
        }
    }

    #[cfg(test)]
    pub(crate) fn test_with(base: impl Into<String>, limits: RateLimits) -> Self {
        Self {
            base: base.into(),
            limits,
        }
    }

    fn list_url(&self) -> String {
        format!("{}{PATH}?limit={PAGE_SIZE}&offset=0", self.base)
    }

    async fn list(&self, ctx: &SourceCtx<'_>) -> Result<Vec<RawPosting>, SourceError> {
        let body = api::get_json(
            ctx.http,
            &self.list_url(),
            self.limits,
            Some(FeedKind::Himalayas.key()),
            None,
        )
        .await?;
        let payload = api::parse(&body)?;
        let jobs = api::array(&payload, "jobs")?;
        Ok(jobs.iter().filter_map(parse_job).collect())
    }
}

impl Default for Himalayas {
    fn default() -> Self {
        Self::new()
    }
}

/// One job object to a `RawPosting`; `None` when it has no usable identity.
fn parse_job(job: &Value) -> Option<RawPosting> {
    let external_id = api::text(job, "guid").or_else(|| match job.get("id") {
        Some(Value::Number(id)) => Some(id.to_string()),
        Some(Value::String(id)) if !id.trim().is_empty() => Some(id.trim().to_string()),
        _ => None,
    })?;
    let title = api::text(job, "title")?;
    let url = api::https_url(api::text(job, "applicationLink"))
        .or_else(|| api::https_url(api::text(job, "guid")))
        .unwrap_or_else(|| "https://himalayas.app/".to_string());
    let locations = locations(job);
    let location_text = locations.join(", ");
    Some(RawPosting {
        external_id,
        title,
        company: api::text(job, "companyName").unwrap_or_else(|| NAME.to_string()),
        url,
        remote: if location_text.is_empty() {
            None
        } else {
            Some(api::is_remote(&location_text))
        },
        locations,
        description_html: api::text(job, "description"),
        posted_at: api::integer(job, "pubDate").and_then(api::rfc3339_from_secs),
        salary_text: salary_text(job),
        raw: api::provenance(job),
    })
}

/// `locationRestrictions` is an array of country names; some rows only carry
/// the legacy `locationRestrictions` string.
fn locations(job: &Value) -> Vec<String> {
    let mut locations: Vec<String> = Vec::new();
    if let Some(entries) = job.get("locationRestrictions").and_then(Value::as_array) {
        for entry in entries {
            if let Some(name) = entry
                .as_str()
                .map(str::trim)
                .filter(|name| !name.is_empty())
            {
                locations.push(name.to_string());
            }
        }
    } else if let Some(name) = api::text(job, "locationRestrictions") {
        locations.push(name);
    }
    locations
}

fn salary_text(job: &Value) -> Option<String> {
    let min = api::number(job, "minSalary");
    let max = api::number(job, "maxSalary");
    let currency = api::text(job, "currency");
    let range = match (min, max) {
        (Some(low), Some(high)) => format!("{low:.0} – {high:.0}"),
        (Some(low), None) => format!("from {low:.0}"),
        (None, Some(high)) => format!("up to {high:.0}"),
        (None, None) => return None,
    };
    Some(match currency {
        Some(code) => format!("{code} {range}"),
        None => range,
    })
}

impl SourceAdapter for Himalayas {
    fn manifest(&self) -> SourceManifest {
        FeedKind::Himalayas.manifest()
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
                    message: format!("himalayas has no job '{external_id}'"),
                })
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::http::{HttpClient, HttpClientConfig};

    const JOBS: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/feeds/himalayas_jobs.json"
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
            .and(path("/jobs/api"))
            .and(query_param("limit", "20"))
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
    async fn search_reads_the_first_page_and_maps_its_fields() {
        let server = MockServer::start().await;
        serve(&server).await;
        let adapter = Himalayas::test_with(server.uri(), RateLimits::default());
        let http = client();

        let postings = adapter
            .search(&SearchQuery::default(), &ctx(&http))
            .await
            .unwrap();
        assert_eq!(postings.len(), 2);

        let first = &postings[0];
        assert_eq!(
            first.external_id,
            "https://himalayas.app/companies/boba/jobs/staff-sre"
        );
        assert_eq!(first.title, "Staff Site Reliability Engineer");
        assert_eq!(first.company, "Boba Labs");
        assert_eq!(
            first.url,
            "https://himalayas.app/companies/boba/jobs/staff-sre"
        );
        assert_eq!(first.locations, vec!["United States", "Canada"]);
        assert_eq!(
            first.remote,
            Some(false),
            "country restrictions are not remote"
        );
        assert_eq!(first.posted_at.as_deref(), Some("2026-10-02T00:00:00Z"));
        assert_eq!(first.salary_text.as_deref(), Some("USD 140000 – 180000"));
    }

    #[tokio::test]
    async fn a_non_https_link_falls_back_to_the_board() {
        let server = MockServer::start().await;
        serve(&server).await;
        let adapter = Himalayas::test_with(server.uri(), RateLimits::default());
        let http = client();
        let postings = adapter
            .search(&SearchQuery::default(), &ctx(&http))
            .await
            .unwrap();
        assert_eq!(postings[1].url, "https://himalayas.app/");
    }

    #[tokio::test]
    async fn detail_finds_one_job() {
        let server = MockServer::start().await;
        serve(&server).await;
        let adapter = Himalayas::test_with(server.uri(), RateLimits::default());
        let http = client();
        let posting = adapter
            .detail(
                "https://himalayas.app/companies/boba/jobs/staff-sre",
                &ctx(&http),
            )
            .await
            .unwrap();
        assert_eq!(posting.title, "Staff Site Reliability Engineer");
    }

    #[test]
    fn the_manifest_asks_for_attribution() {
        let manifest = FeedKind::Himalayas.manifest();
        assert!(manifest.attribution.unwrap().contains("himalayas.app"));
        assert_eq!(manifest.rate_limits.daily_cap, None);
    }
}
