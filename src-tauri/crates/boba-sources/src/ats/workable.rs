//! Workable widget adapter (S4.5, integrations.md §2).
//!
//! `GET /api/v1/widget/accounts/{slug}?details=true` returns the account and
//! its jobs in one object; the endpoint is public but undocumented (addendum
//! A.2), so every field is read defensively. There is no per-posting endpoint:
//! `detail` re-reads the account and picks the shortcode out of it.

use boba_domain::source::{RawPosting, SearchQuery, SourceError, SourceManifest};
use serde_json::Value;

use super::BoardRef;
use super::json;
use crate::sdk::{BoxFuture, SourceAdapter, SourceCtx};

/// The public widget API host.
pub const API: &str = "https://apply.workable.com";

pub struct Workable {
    board: BoardRef,
    base: String,
}

impl Workable {
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
            "{}/api/v1/widget/accounts/{}?details=true",
            self.base, self.board.token
        )
    }

    async fn fetch_board(&self, ctx: &SourceCtx<'_>) -> Result<Vec<RawPosting>, SourceError> {
        let url = self.list_url();
        let body = json::get_json(ctx.http, &url, self.board.limits()).await?;
        let payload = json::parse(&body)?;
        let company = json::text(&payload, "name").unwrap_or_else(|| self.board.token.clone());
        let jobs = json::array(&payload, "jobs")?;
        Ok(jobs
            .iter()
            .filter_map(|job| parse_job(job, &self.board, &company))
            .collect())
    }
}

/// One job object to a `RawPosting`; `None` when it has no usable identity.
fn parse_job(job: &Value, board: &BoardRef, company: &str) -> Option<RawPosting> {
    let external_id = json::text(job, "shortcode").or_else(|| json::text(job, "id"))?;
    let title = json::text(job, "title")?;
    // The payload is untrusted and this URL becomes a clickable link (F1).
    let url = json::https_url(json::text(job, "url"))
        .or_else(|| json::https_url(json::text(job, "application_url")))
        .unwrap_or_else(|| {
            format!(
                "https://apply.workable.com/{}/j/{}/",
                board.token,
                json::encode_segment(&external_id)
            )
        });
    let mut parts = Vec::new();
    for part in [
        json::nested_text(job, &["location", "city"]),
        json::nested_text(job, &["location", "region"]),
        json::nested_text(job, &["location", "country"]),
    ]
    .into_iter()
    .flatten()
    {
        if !parts.contains(&part) {
            parts.push(part);
        }
    }
    Some(RawPosting {
        external_id,
        title,
        company: company.to_string(),
        url,
        remote: json::boolean(job, "telecommuting"),
        locations: if parts.is_empty() {
            Vec::new()
        } else {
            vec![parts.join(", ")]
        },
        description_html: json::text(job, "description").map(|html| json::html_unescape(&html)),
        posted_at: json::text(job, "published_on")
            .and_then(|date| json::date_to_rfc3339(&date))
            .or_else(|| json::text(job, "created_at")),
        salary_text: None,
        raw: json::provenance(job),
    })
}

impl SourceAdapter for Workable {
    fn manifest(&self) -> SourceManifest {
        self.board.manifest()
    }

    fn search<'a>(
        &'a self,
        query: &'a SearchQuery,
        ctx: &'a SourceCtx<'_>,
    ) -> BoxFuture<'a, Result<Vec<RawPosting>, SourceError>> {
        Box::pin(async move {
            let postings = self.fetch_board(ctx).await?;
            Ok(json::retain_matching(postings, query))
        })
    }

    fn detail<'a>(
        &'a self,
        external_id: &'a str,
        ctx: &'a SourceCtx<'_>,
    ) -> BoxFuture<'a, Result<RawPosting, SourceError>> {
        Box::pin(async move {
            self.fetch_board(ctx)
                .await?
                .into_iter()
                .find(|posting| posting.external_id == external_id)
                .ok_or_else(|| SourceError::InvalidPayload {
                    message: format!("workable posting '{external_id}' is not on the account"),
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

    const BOARD: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ats/workable_jobs.json"
    ));

    fn client() -> HttpClient {
        HttpClient::new(HttpClientConfig {
            min_interval: Duration::from_millis(0),
            ..HttpClientConfig::default()
        })
        .expect("client")
    }

    async fn serve(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path("/api/v1/widget/accounts/bobalabs"))
            .and(query_param("details", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(BOARD, "application/json"))
            .mount(server)
            .await;
    }

    fn board() -> BoardRef {
        BoardRef::new(AtsKind::Workable, "bobalabs").expect("board")
    }

    #[tokio::test]
    async fn search_reads_the_recorded_account() {
        let server = MockServer::start().await;
        serve(&server).await;
        let adapter = Workable::with_base(server.uri(), board());
        let http = client();
        let ctx = SourceCtx {
            http: &http,
            cancel: tokio_util::sync::CancellationToken::new(),
        };
        let postings = adapter.search(&SearchQuery::default(), &ctx).await.unwrap();
        assert_eq!(postings.len(), 2);
        let first = &postings[0];
        assert_eq!(first.external_id, "A1B2C3D4E5");
        assert_eq!(first.title, "Data Engineer");
        assert_eq!(first.company, "Boba Labs");
        assert_eq!(
            first.url,
            "https://apply.workable.com/bobalabs/j/A1B2C3D4E5/"
        );
        assert_eq!(first.locations, vec!["Berlin, Germany"]);
        assert_eq!(first.remote, Some(true));
        assert_eq!(first.posted_at.as_deref(), Some("2026-09-29T00:00:00Z"));
        assert_eq!(
            first.description_html.as_deref(),
            Some("<p>Own our pipelines and <strong>warehouse</strong>.</p>")
        );
        assert_eq!(postings[1].remote, Some(false));
    }

    #[tokio::test]
    async fn detail_picks_one_posting_off_the_account() {
        let server = MockServer::start().await;
        serve(&server).await;
        let adapter = Workable::with_base(server.uri(), board());
        let http = client();
        let ctx = SourceCtx {
            http: &http,
            cancel: tokio_util::sync::CancellationToken::new(),
        };
        let posting = adapter.detail("F6G7H8I9J0", &ctx).await.unwrap();
        assert_eq!(posting.title, "Office Manager");
        assert_eq!(posting.locations, vec!["Lisbon, Portugal"]);
    }
    const UNSAFE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ats/workable_unsafe_urls.json"
    ));

    #[tokio::test]
    async fn a_posting_url_that_is_not_https_falls_back_to_the_board_url() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/widget/accounts/bobalabs"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(UNSAFE, "application/json"))
            .mount(&server)
            .await;
        let adapter = Workable::with_base(server.uri(), board());
        let http = client();
        let ctx = SourceCtx {
            http: &http,
            cancel: tokio_util::sync::CancellationToken::new(),
        };
        let postings = adapter.search(&SearchQuery::default(), &ctx).await.unwrap();
        assert_eq!(
            postings[0].url,
            "https://apply.workable.com/bobalabs/j/SCRIPT0001/"
        );
        assert_eq!(
            postings[1].url,
            "https://apply.workable.com/bobalabs/j/PLAIN00002/"
        );
    }
}
