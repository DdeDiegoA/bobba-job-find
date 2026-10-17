//! Adzuna aggregator adapter (S4.6, AC2, addendum A.2).
//!
//! `GET /v1/api/jobs/{country}/search/{page}` with the user's `app_id`/`app_key`
//! (keychain-backed: the caller reads them from [`ADZUNA_APP_ID_REF`] and
//! [`ADZUNA_APP_KEY_REF`] and hands them in). Searches use the profile's
//! queries and locations; `country` comes from Preferences. The keys are
//! secrets: they travel in the query string, so every error is built from the
//! redacted URL and an invalid key is never echoed back.
//!
//! Adzuna documents 25 calls/minute and 250/day; both are on the manifest, and
//! the shared client enforces them through the source's daily budget.

use boba_domain::source::{RateLimits, RawPosting, SearchQuery, SourceError, SourceManifest};
use serde_json::Value;

use super::AggregatorKind;
use crate::feeds::api;
use crate::sdk::{BoxFuture, SourceAdapter, SourceCtx};

/// The public API host (tests point the adapter at a mock origin).
pub const API: &str = "https://api.adzuna.com";

/// Keychain references the app stores the user's Adzuna credentials under.
pub const ADZUNA_APP_ID_REF: &str = "source:adzuna:app_id";
pub const ADZUNA_APP_KEY_REF: &str = "source:adzuna:app_key";

/// The country Adzuna searches when Preferences names none.
pub const DEFAULT_COUNTRY: &str = "us";

/// Countries Adzuna serves (docs/research/integrations.md §2: 20+ countries).
pub const COUNTRIES: &[&str] = &[
    "at", "au", "be", "br", "ca", "ch", "de", "es", "fr", "gb", "in", "it", "mx", "nl", "nz", "pl",
    "sg", "us", "za",
];

/// Results per page (Adzuna's default is 10; a run wants more).
const RESULTS_PER_PAGE: u32 = 50;

/// The user's Adzuna credentials, read from the keychain by the caller.
#[derive(Clone)]
pub struct AdzunaKeys {
    app_id: String,
    app_key: String,
}

/// The credentials never appear in a debug dump.
impl std::fmt::Debug for AdzunaKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdzunaKeys")
            .field("app_id", &"<redacted>")
            .field("app_key", &"<redacted>")
            .finish()
    }
}

impl AdzunaKeys {
    pub fn new(app_id: impl Into<String>, app_key: impl Into<String>) -> Result<Self, SourceError> {
        let app_id = app_id.into();
        let app_key = app_key.into();
        if !valid_credential(&app_id) {
            return Err(invalid("adzuna app_id"));
        }
        if !valid_credential(&app_key) {
            return Err(invalid("adzuna app_key"));
        }
        Ok(Self { app_id, app_key })
    }
}

/// Credentials travel in the query string, so they stay in a bare token set.
fn valid_credential(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

/// `InvalidValue` is a domain type that carries a value; for a secret that
/// value is always the redaction marker.
fn invalid(field: &str) -> SourceError {
    SourceError::InvalidValue {
        field: field.to_string(),
        value: "<redacted>".to_string(),
    }
}

#[derive(Debug)]
pub struct Adzuna {
    keys: AdzunaKeys,
    country: String,
    base: String,
    limits: RateLimits,
}

impl Adzuna {
    pub fn new(keys: AdzunaKeys, country: &str) -> Result<Self, SourceError> {
        let country = country.trim().to_ascii_lowercase();
        if !COUNTRIES.contains(&country.as_str()) {
            return Err(SourceError::InvalidValue {
                field: "adzuna country".to_string(),
                value: country,
            });
        }
        Ok(Self {
            keys,
            country,
            base: API.to_string(),
            limits: AggregatorKind::Adzuna.limits(),
        })
    }

    #[cfg(test)]
    pub(crate) fn test_with(
        keys: AdzunaKeys,
        country: &str,
        base: impl Into<String>,
        limits: RateLimits,
    ) -> Self {
        Self {
            keys,
            country: country.to_string(),
            base: base.into(),
            limits,
        }
    }

    /// The request URL; only this one carries the credentials.
    fn search_url(&self, query: &SearchQuery) -> String {
        let mut url = format!(
            "{}/v1/api/jobs/{}/search/1?app_id={}&app_key={}&results_per_page={RESULTS_PER_PAGE}&content-type=application/json",
            self.base,
            self.country,
            api::encode_query(&self.keys.app_id),
            api::encode_query(&self.keys.app_key),
        );
        let what = search_terms(query);
        if !what.is_empty() {
            url.push_str("&what=");
            url.push_str(&api::encode_query(&what));
        }
        let where_ = query.locations.join(", ");
        if !where_.trim().is_empty() {
            url.push_str("&where=");
            url.push_str(&api::encode_query(where_.trim()));
        }
        url
    }

    /// The URL errors carry; it never contains the keys.
    fn public_url(&self) -> String {
        format!("{}/v1/api/jobs/{}/search/1", self.base, self.country)
    }

    async fn search_page(
        &self,
        query: &SearchQuery,
        ctx: &SourceCtx<'_>,
    ) -> Result<Vec<RawPosting>, SourceError> {
        let body = api::get_json(
            ctx.http,
            &self.search_url(query),
            self.limits,
            Some(AggregatorKind::Adzuna.key()),
            Some(&self.public_url()),
        )
        .await?;
        let payload = api::parse(&body)?;
        let results = api::array(&payload, "results")?;
        Ok(results.iter().filter_map(parse_job).collect())
    }
}

/// The profile's roles and keywords as Adzuna's one `what` string.
fn search_terms(query: &SearchQuery) -> String {
    query
        .roles
        .iter()
        .chain(query.keywords.iter())
        .map(|term| term.trim())
        .filter(|term| !term.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// One result to a `RawPosting`; `None` when it has no usable identity.
fn parse_job(job: &Value) -> Option<RawPosting> {
    let external_id = match job.get("id") {
        Some(Value::Number(id)) => id.to_string(),
        Some(Value::String(id)) if !id.trim().is_empty() => id.trim().to_string(),
        _ => return None,
    };
    let title = api::text(job, "title")?;
    let url = api::https_url(api::text(job, "redirect_url"))
        .unwrap_or_else(|| "https://www.adzuna.com/".to_string());
    let location = api::nested_text(job, &["location", "display_name"]);
    Some(RawPosting {
        external_id,
        title,
        company: api::nested_text(job, &["company", "display_name"])
            .unwrap_or_else(|| "Adzuna".to_string()),
        url,
        remote: location.as_deref().map(api::is_remote),
        locations: location.into_iter().collect(),
        description_html: api::text(job, "description"),
        posted_at: api::text(job, "created").and_then(|date| api::utc_timestamp(&date)),
        salary_text: salary_text(job),
        raw: api::provenance(job),
    })
}

/// Adzuna sends bounds but no currency code, so the range stays unprefixed.
fn salary_text(job: &Value) -> Option<String> {
    let min = api::number(job, "salary_min").filter(|value| *value > 0.0);
    let max = api::number(job, "salary_max").filter(|value| *value > 0.0);
    match (min, max) {
        (Some(low), Some(high)) => Some(format!("{low:.0} – {high:.0}")),
        (Some(low), None) => Some(format!("from {low:.0}")),
        (None, Some(high)) => Some(format!("up to {high:.0}")),
        (None, None) => None,
    }
}

impl SourceAdapter for Adzuna {
    fn manifest(&self) -> SourceManifest {
        AggregatorKind::Adzuna.manifest()
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
                    message: format!("adzuna has no job '{external_id}'"),
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

    const RESULTS: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/feeds/adzuna_results.json"
    ));

    fn client() -> HttpClient {
        HttpClient::new(HttpClientConfig {
            min_interval: Duration::from_millis(0),
            retry_base_delay: Duration::from_millis(1),
            ..HttpClientConfig::default()
        })
        .expect("client")
    }

    fn keys() -> AdzunaKeys {
        AdzunaKeys::new("app-id-123", "super-secret-key").unwrap()
    }

    fn adapter(base: &str) -> Adzuna {
        Adzuna::test_with(keys(), "us", base, RateLimits::default())
    }

    fn ctx(http: &HttpClient) -> SourceCtx<'_> {
        SourceCtx {
            http,
            cancel: tokio_util::sync::CancellationToken::new(),
        }
    }

    #[tokio::test]
    async fn search_sends_the_query_and_the_credentials() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/api/jobs/us/search/1"))
            .and(query_param("app_id", "app-id-123"))
            .and(query_param("app_key", "super-secret-key"))
            .and(query_param("what", "rust backend"))
            .and(query_param("where", "Mexico City, Remote"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(RESULTS, "application/json"))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = adapter(&server.uri());
        let http = client();
        let query = SearchQuery {
            roles: vec!["rust".into()],
            keywords: vec!["backend".into()],
            locations: vec!["Mexico City".into(), "Remote".into()],
            ..SearchQuery::default()
        };
        let postings = adapter.search(&query, &ctx(&http)).await.unwrap();
        assert_eq!(postings.len(), 2);

        let first = &postings[0];
        assert_eq!(first.external_id, "3963446509");
        assert_eq!(first.title, "Senior Backend Engineer");
        assert_eq!(first.company, "Boba Labs");
        assert_eq!(first.url, "https://www.adzuna.com/land/ad/3963446509");
        assert_eq!(first.locations, vec!["Mexico City, Mexico"]);
        assert_eq!(first.remote, Some(false), "a city location is not remote");
        assert_eq!(first.posted_at.as_deref(), Some("2026-09-30T08:00:00Z"));
        assert_eq!(first.salary_text.as_deref(), Some("900000 – 1200000"));
    }

    #[tokio::test]
    async fn a_javascript_redirect_url_is_refused() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/api/jobs/us/search/1"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(RESULTS, "application/json"))
            .mount(&server)
            .await;
        let postings = adapter(&server.uri())
            .search(&SearchQuery::default(), &ctx(&client()))
            .await
            .unwrap();
        assert_eq!(postings[1].url, "https://www.adzuna.com/");
    }

    #[tokio::test]
    async fn an_error_never_carries_the_key() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/api/jobs/us/search/1"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server)
            .await;
        let error = adapter(&server.uri())
            .search(&SearchQuery::default(), &ctx(&client()))
            .await
            .expect_err("500");
        let text = error.to_string();
        assert!(!text.contains("super-secret-key"), "{text}");
        assert!(!text.contains("app-id-123"), "{text}");
        match error {
            SourceError::Http { url, .. } => {
                assert!(url.ends_with("/v1/api/jobs/us/search/1"), "{url}");
            }
            other => panic!("expected an Http error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn detail_refetches_the_page_and_finds_the_id() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/api/jobs/us/search/1"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(RESULTS, "application/json"))
            .mount(&server)
            .await;
        let posting = adapter(&server.uri())
            .detail("3963446510", &ctx(&client()))
            .await
            .unwrap();
        assert_eq!(posting.title, "Product Designer");
    }

    #[test]
    fn new_rejects_an_unknown_country_and_an_invalid_key_without_echoing_it() {
        let error = Adzuna::new(keys(), "atlantis").expect_err("country");
        assert!(matches!(error, SourceError::InvalidValue { .. }));

        let error = AdzunaKeys::new("app id", "k").expect_err("space in id");
        let text = error.to_string();
        assert!(!text.contains("app id"), "{text}");

        let error = AdzunaKeys::new("ok", "secret\nkey").expect_err("newline in key");
        assert!(!error.to_string().contains("secret"));
    }

    #[test]
    fn the_manifest_documents_250_calls_a_day() {
        let manifest = AggregatorKind::Adzuna.manifest();
        assert_eq!(manifest.rate_limits.daily_cap, Some(250));
        assert_eq!(manifest.required_secrets, vec!["app_id", "app_key"]);
        assert!(manifest.attribution.is_some());
    }
}
