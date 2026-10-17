//! Ashby job-board adapter (S4.5, integrations.md §2).
//!
//! `GET /posting-api/job-board/{name}?includeCompensation=true` returns the
//! whole board; Ashby has no public per-posting endpoint, so `detail` re-reads
//! the board and picks the id out of it. Compensation only appears when the
//! query flag is set.

use boba_domain::source::{RawPosting, SearchQuery, SourceError, SourceManifest};
use serde_json::Value;

use super::BoardRef;
use super::json;
use crate::sdk::{BoxFuture, SourceAdapter, SourceCtx};

/// The public posting API host.
pub const API: &str = "https://api.ashbyhq.com";

pub struct Ashby {
    board: BoardRef,
    base: String,
}

impl Ashby {
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
            "{}/posting-api/job-board/{}?includeCompensation=true",
            self.base, self.board.token
        )
    }

    async fn fetch_board(&self, ctx: &SourceCtx<'_>) -> Result<Vec<RawPosting>, SourceError> {
        let url = self.list_url();
        let body = json::get_json(ctx.http, &url, self.board.limits()).await?;
        let payload = json::parse(&body)?;
        let jobs = json::array(&payload, "jobs")?;
        Ok(jobs
            .iter()
            .filter_map(|job| parse_job(job, &self.board))
            .collect())
    }
}

/// One job object to a `RawPosting`; `None` when it has no usable identity.
fn parse_job(job: &Value, board: &BoardRef) -> Option<RawPosting> {
    let external_id = json::text(job, "id")?;
    let title = json::text(job, "title")?;
    let url = json::text(job, "jobUrl")
        .or_else(|| json::text(job, "applyUrl"))
        .unwrap_or_else(|| format!("https://jobs.ashbyhq.com/{}/{external_id}", board.token));
    let mut locations = Vec::new();
    for location in std::iter::once(json::text(job, "location"))
        .chain(
            job.get("secondaryLocations")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|secondary| json::text(secondary, "location")),
        )
        .flatten()
    {
        if !locations.contains(&location) {
            locations.push(location);
        }
    }
    let remote = json::boolean(job, "isRemote").or_else(|| {
        json::text(job, "workplaceType").map(|workplace| workplace.eq_ignore_ascii_case("remote"))
    });
    Some(RawPosting {
        external_id,
        title,
        company: board.token.clone(),
        url,
        remote,
        locations,
        description_html: json::text(job, "descriptionHtml"),
        posted_at: json::text(job, "publishedAt"),
        salary_text: json::nested_text(job, &["compensation", "compensationTierSummary"]),
        raw: json::provenance(job),
    })
}

impl SourceAdapter for Ashby {
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
                    message: format!("ashby posting '{external_id}' is not on the board"),
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
        "/tests/fixtures/ats/ashby_jobs.json"
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
            .and(path("/posting-api/job-board/bobalabs"))
            .and(query_param("includeCompensation", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(BOARD, "application/json"))
            .mount(server)
            .await;
    }

    fn board() -> BoardRef {
        BoardRef::new(AtsKind::Ashby, "bobalabs").expect("board")
    }

    #[tokio::test]
    async fn search_reads_the_recorded_board() {
        let server = MockServer::start().await;
        serve(&server).await;
        let adapter = Ashby::with_base(server.uri(), board());
        let http = client();
        let ctx = SourceCtx {
            http: &http,
            cancel: tokio_util::sync::CancellationToken::new(),
        };
        let postings = adapter.search(&SearchQuery::default(), &ctx).await.unwrap();
        assert_eq!(postings.len(), 2);
        let first = &postings[0];
        assert_eq!(first.external_id, "0f1e2d3c-4b5a-6978-8a9b-0c1d2e3f4a5b");
        assert_eq!(first.title, "Full-stack Engineer");
        assert_eq!(first.company, "bobalabs");
        assert_eq!(
            first.url,
            "https://jobs.ashbyhq.com/bobalabs/0f1e2d3c-4b5a-6978-8a9b-0c1d2e3f4a5b"
        );
        assert_eq!(first.locations, vec!["Berlin", "Remote - Europe"]);
        assert_eq!(first.remote, Some(true));
        assert_eq!(first.salary_text.as_deref(), Some("€85K – €110K"));
        assert_eq!(
            first.posted_at.as_deref(),
            Some("2026-09-27T08:30:00.000+00:00")
        );
        assert_eq!(
            first.description_html.as_deref(),
            Some("<h2>About Boba</h2><p>Build the search experience end to end.</p>")
        );
        assert_eq!(postings[1].remote, Some(false));
    }

    #[tokio::test]
    async fn detail_picks_one_posting_off_the_board() {
        let server = MockServer::start().await;
        serve(&server).await;
        let adapter = Ashby::with_base(server.uri(), board());
        let http = client();
        let ctx = SourceCtx {
            http: &http,
            cancel: tokio_util::sync::CancellationToken::new(),
        };
        let posting = adapter
            .detail("9a8b7c6d-5e4f-3021-4c5d-6e7f8a9b0c1d", &ctx)
            .await
            .unwrap();
        assert_eq!(posting.title, "Marketing Lead");
        assert_eq!(posting.locations, vec!["London"]);

        let error = adapter
            .detail("missing", &ctx)
            .await
            .expect_err("an unknown id must fail");
        assert!(matches!(error, SourceError::InvalidPayload { .. }));
    }
}
