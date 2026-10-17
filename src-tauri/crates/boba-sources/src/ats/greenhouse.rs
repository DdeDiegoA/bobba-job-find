//! Greenhouse job-board adapter (S4.5, integrations.md §2).
//!
//! `GET /v1/boards/{token}/jobs?content=true` returns every live job for the
//! company; the description arrives HTML-escaped. Detail is the same object at
//! `/v1/boards/{token}/jobs/{id}`.

use boba_domain::source::{RawPosting, SearchQuery, SourceError, SourceManifest};
use serde_json::Value;

use super::BoardRef;
use super::json;
use crate::sdk::{BoxFuture, SourceAdapter, SourceCtx};

/// The public board API host.
pub const API: &str = "https://boards-api.greenhouse.io";

pub struct Greenhouse {
    board: BoardRef,
    base: String,
}

impl Greenhouse {
    pub fn new(board: BoardRef) -> Self {
        Self {
            board,
            base: API.to_string(),
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
        format!(
            "{}/v1/boards/{}/jobs?content=true",
            self.base, self.board.token
        )
    }

    fn detail_url(&self, external_id: &str) -> String {
        format!(
            "{}/v1/boards/{}/jobs/{}",
            self.base, self.board.token, external_id
        )
    }
}

/// One job object to a `RawPosting`; `None` when it has no usable identity.
fn parse_job(job: &Value, board: &BoardRef) -> Option<RawPosting> {
    let external_id = match job.get("id") {
        Some(Value::Number(id)) => id.to_string(),
        Some(Value::String(id)) if !id.trim().is_empty() => id.trim().to_string(),
        _ => return None,
    };
    let title = json::text(job, "title")?;
    let location = json::nested_text(job, &["location", "name"]);
    let url = json::text(job, "absolute_url").unwrap_or_else(|| {
        format!(
            "https://boards.greenhouse.io/{}/jobs/{external_id}",
            board.token
        )
    });
    let pay = job
        .get("pay_input_ranges")
        .and_then(Value::as_array)
        .and_then(|ranges| ranges.first());
    let salary_text = pay.and_then(|range| {
        json::salary_text(
            json::number(range, "min_cents").map(|cents| cents / 100.0),
            json::number(range, "max_cents").map(|cents| cents / 100.0),
            json::text(range, "currency_type").as_deref(),
            json::text(range, "interval").as_deref(),
        )
    });
    Some(RawPosting {
        external_id,
        title,
        company: json::text(job, "company_name").unwrap_or_else(|| board.token.clone()),
        url,
        remote: location.as_deref().map(json::is_remote),
        locations: location.into_iter().collect(),
        description_html: json::text(job, "content").map(|content| json::html_unescape(&content)),
        posted_at: json::text(job, "first_published").or_else(|| json::text(job, "updated_at")),
        salary_text,
        raw: json::provenance(job),
    })
}

impl SourceAdapter for Greenhouse {
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
            let jobs = json::array(&payload, "jobs")?;
            let postings = jobs
                .iter()
                .filter_map(|job| parse_job(job, &self.board))
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
            let url = self.detail_url(external_id);
            let body = json::get_json(ctx.http, &url, self.board.limits()).await?;
            let payload = json::parse(&body)?;
            parse_job(&payload, &self.board).ok_or_else(|| SourceError::InvalidPayload {
                message: format!("greenhouse job '{external_id}' has no title"),
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
        "/tests/fixtures/ats/greenhouse_jobs.json"
    ));
    const DETAIL: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ats/greenhouse_detail.json"
    ));

    fn client() -> HttpClient {
        HttpClient::new(HttpClientConfig {
            min_interval: Duration::from_millis(0),
            ..HttpClientConfig::default()
        })
        .expect("client")
    }

    fn board() -> BoardRef {
        BoardRef::new(AtsKind::Greenhouse, "bobalabs").expect("board")
    }

    async fn serve(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path("/v1/boards/bobalabs/jobs"))
            .and(query_param("content", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(LIST, "application/json"))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn search_reads_the_recorded_board() {
        let server = MockServer::start().await;
        serve(&server).await;
        let adapter = Greenhouse::with_base(server.uri(), board());
        let http = client();
        let ctx = SourceCtx {
            http: &http,
            cancel: tokio_util::sync::CancellationToken::new(),
        };

        let postings = adapter.search(&SearchQuery::default(), &ctx).await.unwrap();
        assert_eq!(postings.len(), 2);
        let first = &postings[0];
        assert_eq!(first.external_id, "4512345");
        assert_eq!(first.title, "Senior Backend Engineer");
        assert_eq!(first.company, "Boba Labs");
        assert_eq!(
            first.url,
            "https://boards.greenhouse.io/bobaboard/jobs/4512345"
        );
        assert_eq!(first.locations, vec!["Remote - Europe"]);
        assert_eq!(first.remote, Some(true));
        assert_eq!(
            first.description_html.as_deref(),
            Some(
                "<p>We are hiring a <b>Senior Backend Engineer</b> for our <em>payments</em> platform.</p>"
            )
        );
        assert_eq!(
            first.salary_text.as_deref(),
            Some("EUR 90000 – 120000 / yearly")
        );
        assert!(first.raw.contains("\"id\":4512345"));
        assert!(first.posted_at.is_some());
    }

    #[tokio::test]
    async fn search_honours_the_query() {
        let server = MockServer::start().await;
        serve(&server).await;
        let adapter = Greenhouse::with_base(server.uri(), board());
        let http = client();
        let ctx = SourceCtx {
            http: &http,
            cancel: tokio_util::sync::CancellationToken::new(),
        };
        let query = SearchQuery {
            roles: vec!["designer".into()],
            ..SearchQuery::default()
        };
        let postings = adapter.search(&query, &ctx).await.unwrap();
        assert_eq!(postings.len(), 1);
        assert_eq!(postings[0].title, "Product Designer");
    }

    #[tokio::test]
    async fn detail_reads_one_job() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/boards/bobalabs/jobs/4512345"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(DETAIL, "application/json"))
            .mount(&server)
            .await;
        let adapter = Greenhouse::with_base(server.uri(), board());
        let http = client();
        let ctx = SourceCtx {
            http: &http,
            cancel: tokio_util::sync::CancellationToken::new(),
        };
        let posting = adapter.detail("4512345", &ctx).await.unwrap();
        assert_eq!(posting.title, "Senior Backend Engineer");
        assert_eq!(
            posting.posted_at.as_deref(),
            Some("2026-09-12T10:00:00-04:00"),
            "first_published wins over updated_at"
        );
        assert!(
            posting
                .description_html
                .as_deref()
                .unwrap()
                .starts_with("<h2>About the role</h2>")
        );
    }

    #[tokio::test]
    async fn a_non_200_is_an_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404).set_body_string("nope"))
            .mount(&server)
            .await;
        let adapter = Greenhouse::with_base(server.uri(), board());
        let http = client();
        let ctx = SourceCtx {
            http: &http,
            cancel: tokio_util::sync::CancellationToken::new(),
        };
        let error = adapter
            .search(&SearchQuery::default(), &ctx)
            .await
            .expect_err("404 must fail");
        assert!(matches!(error, SourceError::Http { .. }));
    }
}
