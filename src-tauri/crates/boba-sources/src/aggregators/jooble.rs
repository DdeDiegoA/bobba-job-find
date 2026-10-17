//! Jooble aggregator adapter (S4.6, AC2, addendum A.2).
//!
//! `POST /api/{key}` with a JSON body (`keywords`, `location`, `page`) and the
//! user's key in the **path** — keychain-backed: the caller reads
//! [`JOOBLE_KEY_REF`] and hands the value in. Jooble grants only 500 requests
//! per key for its whole lifetime, so the manifest declares a two-a-day cap:
//! "use sparingly" is the documented limit, and one search plus at most one
//! detail per day is what sparing means here.
//!
//! The key is a secret: every error is built from a URL with the key masked,
//! and an invalid key is never echoed back.

use boba_domain::source::{RateLimits, RawPosting, SearchQuery, SourceError, SourceManifest};
use serde_json::Value;

use super::AggregatorKind;
use crate::feeds::api;
use crate::sdk::{BoxFuture, SourceAdapter, SourceCtx};

/// The public API host (tests point the adapter at a mock origin).
pub const API: &str = "https://jooble.org";

/// The path prefix; the key becomes its last segment.
const PATH: &str = "/api";

/// Keychain reference the app stores the user's Jooble key under.
pub const JOOBLE_KEY_REF: &str = "source:jooble:api_key";

pub struct Jooble {
    key: String,
    base: String,
    limits: RateLimits,
}

/// The key never appears in a debug dump.
impl std::fmt::Debug for Jooble {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Jooble")
            .field("key", &"<redacted>")
            .field("base", &self.base)
            .field("limits", &self.limits)
            .finish()
    }
}

impl Jooble {
    pub fn new(key: impl Into<String>) -> Result<Self, SourceError> {
        let key = key.into();
        // The key is one path segment: a bare token cannot reach another path.
        if !valid_key(&key) {
            return Err(SourceError::InvalidValue {
                field: "jooble api_key".to_string(),
                value: "<redacted>".to_string(),
            });
        }
        Ok(Self {
            key,
            base: API.to_string(),
            limits: AggregatorKind::Jooble.limits(),
        })
    }

    #[cfg(test)]
    pub(crate) fn test_with(key: &str, base: impl Into<String>, limits: RateLimits) -> Self {
        Self {
            key: key.to_string(),
            base: base.into(),
            limits,
        }
    }

    fn search_url(&self) -> String {
        format!("{}{PATH}/{}", self.base, self.key)
    }

    /// The URL errors carry; the key segment is masked.
    fn public_url(&self) -> String {
        format!("{}{PATH}/<key>", self.base)
    }

    /// The documented request body: the profile's queries and locations.
    fn body(query: &SearchQuery) -> String {
        let keywords = query
            .roles
            .iter()
            .chain(query.keywords.iter())
            .map(|term| term.trim())
            .filter(|term| !term.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let location = query
            .locations
            .iter()
            .map(|location| location.trim())
            .filter(|location| !location.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
        serde_json::json!({
            "keywords": keywords,
            "location": location,
            "page": "1",
        })
        .to_string()
    }

    async fn search_page(
        &self,
        query: &SearchQuery,
        ctx: &SourceCtx<'_>,
    ) -> Result<Vec<RawPosting>, SourceError> {
        let body = api::post_json(
            ctx.http,
            &self.search_url(),
            &self.public_url(),
            &Self::body(query),
            self.limits,
            Some(AggregatorKind::Jooble.key()),
        )
        .await?;
        let payload = api::parse(&body)?;
        let jobs = api::array(&payload, "jobs")?;
        Ok(jobs.iter().filter_map(parse_job).collect())
    }
}

fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

/// One job object to a `RawPosting`; `None` when it has no usable identity.
fn parse_job(job: &Value) -> Option<RawPosting> {
    let external_id = match job.get("id") {
        Some(Value::Number(id)) => id.to_string(),
        Some(Value::String(id)) if !id.trim().is_empty() => id.trim().to_string(),
        _ => return None,
    };
    let title = api::text(job, "title")?;
    let url =
        api::https_url(api::text(job, "link")).unwrap_or_else(|| "https://jooble.org/".to_string());
    let location = api::text(job, "location");
    Some(RawPosting {
        external_id,
        title,
        company: api::text(job, "company").unwrap_or_else(|| "Jooble".to_string()),
        url,
        remote: location.as_deref().map(api::is_remote),
        locations: location.into_iter().collect(),
        description_html: api::text(job, "snippet"),
        posted_at: api::text(job, "updated").and_then(|date| api::utc_timestamp(&date)),
        salary_text: api::text(job, "salary"),
        raw: api::provenance(job),
    })
}

impl SourceAdapter for Jooble {
    fn manifest(&self) -> SourceManifest {
        AggregatorKind::Jooble.manifest()
    }

    fn search<'a>(
        &'a self,
        query: &'a SearchQuery,
        ctx: &'a SourceCtx<'_>,
    ) -> BoxFuture<'a, Result<Vec<RawPosting>, SourceError>> {
        Box::pin(async move { self.search_page(query, ctx).await })
    }

    fn detail<'a>(
        &'a self,
        external_id: &'a str,
        ctx: &'a SourceCtx<'_>,
    ) -> BoxFuture<'a, Result<RawPosting, SourceError>> {
        Box::pin(async move {
            let postings = self.search_page(&SearchQuery::default(), ctx).await?;
            postings
                .into_iter()
                .find(|posting| posting.external_id == external_id)
                .ok_or_else(|| SourceError::InvalidPayload {
                    message: format!("jooble has no job '{external_id}'"),
                })
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::http::{HttpClient, HttpClientConfig};

    const JOBS: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/feeds/jooble_jobs.json"
    ));

    fn client() -> HttpClient {
        HttpClient::new(HttpClientConfig {
            min_interval: Duration::from_millis(0),
            retry_base_delay: Duration::from_millis(1),
            ..HttpClientConfig::default()
        })
        .expect("client")
    }

    fn ctx(http: &HttpClient) -> SourceCtx<'_> {
        SourceCtx {
            http,
            cancel: tokio_util::sync::CancellationToken::new(),
        }
    }

    fn adapter(base: &str) -> Jooble {
        Jooble::test_with("secret-key-123", base, RateLimits::default())
    }

    #[tokio::test]
    async fn search_posts_the_query_and_maps_the_jobs() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/secret-key-123"))
            .and(body_string_contains("\"keywords\":\"rust backend\""))
            .and(body_string_contains("\"location\":\"Mexico City\""))
            .respond_with(ResponseTemplate::new(200).set_body_raw(JOBS, "application/json"))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = adapter(&server.uri());
        let http = client();
        let query = SearchQuery {
            roles: vec!["rust".into()],
            keywords: vec!["backend".into()],
            locations: vec!["Mexico City".into()],
            ..SearchQuery::default()
        };
        let postings = adapter.search(&query, &ctx(&http)).await.unwrap();
        assert_eq!(postings.len(), 2);

        let first = &postings[0];
        assert_eq!(first.external_id, "4410999");
        assert_eq!(first.title, "Backend Engineer (Rust)");
        assert_eq!(first.company, "Boba Labs");
        assert_eq!(first.url, "https://jooble.org/jdp/4410999");
        assert_eq!(first.locations, vec!["Ciudad de México"]);
        assert_eq!(first.remote, Some(false));
        assert_eq!(first.posted_at.as_deref(), Some("2026-09-28T00:00:00Z"));
        assert_eq!(first.salary_text.as_deref(), Some("MXN 60,000 - 80,000"));
    }

    #[tokio::test]
    async fn a_non_https_link_is_refused() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/secret-key-123"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(JOBS, "application/json"))
            .mount(&server)
            .await;
        let postings = adapter(&server.uri())
            .search(&SearchQuery::default(), &ctx(&client()))
            .await
            .unwrap();
        assert_eq!(postings[1].url, "https://jooble.org/");
    }

    #[tokio::test]
    async fn an_error_never_carries_the_key() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/secret-key-123"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server)
            .await;
        let error = adapter(&server.uri())
            .search(&SearchQuery::default(), &ctx(&client()))
            .await
            .expect_err("500");
        let text = error.to_string();
        assert!(!text.contains("secret-key-123"), "{text}");
        assert!(text.contains("<key>"), "{text}");
    }

    #[tokio::test]
    async fn detail_refetches_and_finds_the_id() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/secret-key-123"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(JOBS, "application/json"))
            .mount(&server)
            .await;
        let posting = adapter(&server.uri())
            .detail("4411000", &ctx(&client()))
            .await
            .unwrap();
        assert_eq!(posting.title, "Product Designer");
    }

    #[test]
    fn new_rejects_an_unsafe_key_without_echoing_it() {
        for key in ["", "a/b", "a b", "a\nb", "../../admin"] {
            let error = Jooble::new(key).expect_err(key);
            let text = error.to_string();
            assert!(!text.contains("admin"), "{text}");
            assert!(text.contains("<redacted>"), "{text}");
        }
        assert!(Jooble::new("hex-key_123").is_ok());
    }

    #[test]
    fn the_manifest_documents_a_sparing_daily_cap() {
        let manifest = AggregatorKind::Jooble.manifest();
        assert_eq!(
            manifest.rate_limits.daily_cap,
            Some(2),
            "500 requests per key lifetime; one search plus one detail a day"
        );
        assert_eq!(manifest.required_secrets, vec!["api_key"]);
    }
}
