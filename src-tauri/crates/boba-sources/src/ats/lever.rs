//! Lever postings adapter (S4.5, integrations.md §2).
//!
//! `GET /v0/postings/{company}?mode=json` returns a bare JSON array; the EU
//! board lives on `api.eu.lever.co` (addendum A.2). `createdAt` is epoch
//! milliseconds, which the store does not accept, so it is converted here.

use boba_domain::source::{RawPosting, SearchQuery, SourceError, SourceManifest};
use serde_json::Value;

use super::BoardRef;
use super::json;
use crate::sdk::{BoxFuture, SourceAdapter, SourceCtx};

/// The global postings API host.
pub const API: &str = "https://api.lever.co";
/// The EU postings API host (A.2).
pub const API_EU: &str = "https://api.eu.lever.co";

pub struct Lever {
    board: BoardRef,
    base: String,
}

impl Lever {
    pub fn new(board: BoardRef) -> Self {
        let base = if board.eu { API_EU } else { API };
        Self {
            board,
            base: base.to_string(),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_base(base: impl Into<String>, board: BoardRef) -> Self {
        Self {
            board,
            base: base.into(),
        }
    }

    fn list_url(&self) -> String {
        format!("{}/v0/postings/{}?mode=json", self.base, self.board.token)
    }

    fn detail_url(&self, external_id: &str) -> Option<String> {
        let id = json::id_segment(external_id)?;
        Some(format!(
            "{}/v0/postings/{}/{id}",
            self.base, self.board.token
        ))
    }
}

/// One posting object to a `RawPosting`.
fn parse_posting(posting: &Value, board: &BoardRef) -> Option<RawPosting> {
    let external_id = json::text(posting, "id")?;
    let title = json::text(posting, "text")?;
    let location = json::nested_text(posting, &["categories", "location"]);
    let fallback_host = if board.eu {
        "https://jobs.eu.lever.co"
    } else {
        "https://jobs.lever.co"
    };
    // The payload is untrusted and this URL becomes a clickable link (F1).
    let url = json::https_url(json::text(posting, "hostedUrl"))
        .or_else(|| json::https_url(json::text(posting, "applyUrl")))
        .unwrap_or_else(|| {
            format!(
                "{fallback_host}/{}/{}",
                board.token,
                json::encode_segment(&external_id)
            )
        });
    let salary = posting.get("salaryRange");
    let salary_text = salary.and_then(|range| {
        json::salary_text(
            json::number(range, "min"),
            json::number(range, "max"),
            json::text(range, "currency").as_deref(),
            json::text(range, "interval").as_deref(),
        )
    });
    let posted_at = match posting.get("createdAt").and_then(Value::as_i64) {
        Some(millis) => json::rfc3339_from_millis(millis),
        None => None,
    };
    Some(RawPosting {
        external_id,
        title,
        company: board.token.clone(),
        url,
        remote: location.as_deref().map(json::is_remote),
        locations: location.into_iter().collect(),
        description_html: json::text(posting, "description"),
        posted_at,
        salary_text,
        raw: json::provenance(posting),
    })
}

impl SourceAdapter for Lever {
    fn manifest(&self) -> SourceManifest {
        self.board.manifest()
    }

    fn search<'a>(
        &'a self,
        query: &'a SearchQuery,
        ctx: &'a SourceCtx<'_>,
    ) -> BoxFuture<'a, Result<Vec<RawPosting>, SourceError>> {
        Box::pin(async move {
            let url = self.list_url();
            let body = json::get_json(ctx.http, &url, self.board.limits()).await?;
            let payload = json::parse(&body)?;
            let postings: Vec<RawPosting> = payload
                .as_array()
                .ok_or_else(|| SourceError::InvalidPayload {
                    message: "lever postings is not an array".into(),
                })?
                .iter()
                .filter_map(|posting| parse_posting(posting, &self.board))
                .collect();
            Ok(json::retain_matching(postings, query))
        })
    }

    fn detail<'a>(
        &'a self,
        external_id: &'a str,
        ctx: &'a SourceCtx<'_>,
    ) -> BoxFuture<'a, Result<RawPosting, SourceError>> {
        Box::pin(async move {
            let url = self
                .detail_url(external_id)
                .ok_or_else(|| SourceError::InvalidValue {
                    field: "lever posting id".into(),
                    value: external_id.to_string(),
                })?;
            let body = json::get_json(ctx.http, &url, self.board.limits()).await?;
            let payload = json::parse(&body)?;
            parse_posting(&payload, &self.board).ok_or_else(|| SourceError::InvalidPayload {
                message: format!("lever posting '{external_id}' has no title"),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::super::{AtsKind, BoardRef};
    use super::*;
    use crate::http::{HttpClient, HttpClientConfig};

    const LIST: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ats/lever_postings.json"
    ));
    const DETAIL: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ats/lever_detail.json"
    ));

    fn client() -> HttpClient {
        HttpClient::new(HttpClientConfig {
            min_interval: Duration::from_millis(0),
            ..HttpClientConfig::default()
        })
        .expect("client")
    }

    #[tokio::test]
    async fn search_reads_the_recorded_postings() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v0/postings/bobalabs"))
            .and(query_param("mode", "json"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(LIST, "application/json"))
            .mount(&server)
            .await;
        let board = BoardRef::new(AtsKind::Lever, "bobalabs").unwrap();
        let adapter = Lever::with_base(server.uri(), board);
        let http = client();
        let ctx = SourceCtx {
            http: &http,
            cancel: tokio_util::sync::CancellationToken::new(),
        };

        let postings = adapter.search(&SearchQuery::default(), &ctx).await.unwrap();
        assert_eq!(postings.len(), 2);
        let first = &postings[0];
        assert_eq!(first.external_id, "6f2c1a44-0d2e-4f71-9c3e-2b8f0e1a7d10");
        assert_eq!(first.title, "Staff Platform Engineer");
        assert_eq!(first.company, "bobalabs");
        assert_eq!(
            first.url,
            "https://jobs.lever.co/bobalabs/6f2c1a44-0d2e-4f71-9c3e-2b8f0e1a7d10"
        );
        assert_eq!(first.locations, vec!["Remote - Europe"]);
        assert_eq!(first.remote, Some(true));
        assert_eq!(first.posted_at.as_deref(), Some("2024-10-01T00:00:00.000Z"));
        assert_eq!(
            first.salary_text.as_deref(),
            Some("EUR 120000 – 160000 / per-year-salary")
        );
        assert_eq!(
            first.description_html.as_deref(),
            Some("<div>Own the platform that ships Boba.</div>")
        );
        assert_eq!(postings[1].remote, Some(false));
    }

    #[tokio::test]
    async fn detail_reads_one_posting() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/v0/postings/bobalabs/6f2c1a44-0d2e-4f71-9c3e-2b8f0e1a7d10",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_raw(DETAIL, "application/json"))
            .mount(&server)
            .await;
        let board = BoardRef::new(AtsKind::Lever, "bobalabs").unwrap();
        let adapter = Lever::with_base(server.uri(), board);
        let http = client();
        let ctx = SourceCtx {
            http: &http,
            cancel: tokio_util::sync::CancellationToken::new(),
        };
        let posting = adapter
            .detail("6f2c1a44-0d2e-4f71-9c3e-2b8f0e1a7d10", &ctx)
            .await
            .unwrap();
        assert_eq!(posting.title, "Staff Platform Engineer");
        assert!(
            posting
                .description_html
                .as_deref()
                .unwrap()
                .contains("Rust and TypeScript")
        );
    }

    #[test]
    fn the_eu_board_uses_the_eu_host() {
        let eu = BoardRef::lever_eu("bobalabs").unwrap();
        let adapter = Lever::new(eu);
        assert!(adapter.list_url().starts_with(API_EU));
        let global = Lever::new(BoardRef::new(AtsKind::Lever, "bobalabs").unwrap());
        assert!(global.list_url().starts_with(API));
    }
    const UNSAFE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ats/lever_unsafe_urls.json"
    ));

    #[tokio::test]
    async fn a_posting_url_that_is_not_https_falls_back_to_the_board_url() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v0/postings/bobalabs"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(UNSAFE, "application/json"))
            .mount(&server)
            .await;
        let board = BoardRef::new(AtsKind::Lever, "bobalabs").unwrap();
        let adapter = Lever::with_base(server.uri(), board);
        let http = client();
        let ctx = SourceCtx {
            http: &http,
            cancel: tokio_util::sync::CancellationToken::new(),
        };
        let postings = adapter.search(&SearchQuery::default(), &ctx).await.unwrap();
        assert_eq!(
            postings[0].url,
            "https://jobs.lever.co/bobalabs/11111111-2222-3333-4444-555555555555"
        );
        assert_eq!(
            postings[1].url,
            "https://jobs.lever.co/bobalabs/66666666-7777-8888-9999-000000000000"
        );
    }

    #[test]
    fn a_detail_id_cannot_escape_the_board_path() {
        let adapter = Lever::new(BoardRef::new(AtsKind::Lever, "bobalabs").unwrap());
        let url = adapter
            .detail_url("../evil")
            .expect("a mixed id is encoded");
        assert!(url.ends_with("/bobalabs/..%2Fevil"), "{url}");
        assert!(!url.contains("/../"), "{url}");
        assert_eq!(adapter.detail_url(".."), None);
        assert_eq!(adapter.detail_url("."), None);
    }
    #[tokio::test]
    async fn a_detail_id_of_dots_is_refused_before_any_request() {
        let server = MockServer::start().await;
        let board = BoardRef::new(AtsKind::Lever, "bobalabs").unwrap();
        let adapter = Lever::with_base(server.uri(), board);
        let http = client();
        let ctx = SourceCtx {
            http: &http,
            cancel: tokio_util::sync::CancellationToken::new(),
        };
        let error = adapter.detail("..", &ctx).await.expect_err("refused");
        assert!(matches!(error, SourceError::InvalidValue { .. }));
    }
}
