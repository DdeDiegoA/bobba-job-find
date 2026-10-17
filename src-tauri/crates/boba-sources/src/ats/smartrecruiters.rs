//! SmartRecruiters adapter (S4.5, integrations.md §2).
//!
//! `GET /v1/companies/{id}/postings` returns one page of postings (100 by
//! default) and `totalFound`; the list carries no description, so `detail`
//! reads `/postings/{id}` and joins its `jobAd` sections. SmartRecruiters has
//! no candidate apply endpoint, so the manifest links out (S4.5 AC2).

use boba_domain::source::{RawPosting, SearchQuery, SourceError, SourceManifest};
use serde_json::Value;

use super::BoardRef;
use super::json;
use crate::sdk::{BoxFuture, SourceAdapter, SourceCtx};

/// The public posting API host.
pub const API: &str = "https://api.smartrecruiters.com";

pub struct SmartRecruiters {
    board: BoardRef,
    base: String,
}

impl SmartRecruiters {
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
            "{}/v1/companies/{}/postings?limit=100",
            self.base, self.board.token
        )
    }

    fn detail_url(&self, external_id: &str) -> Option<String> {
        let id = json::id_segment(external_id)?;
        Some(format!(
            "{}/v1/companies/{}/postings/{id}",
            self.base, self.board.token
        ))
    }
}

/// The location line: city, region and country, without repeats.
fn location_line(posting: &Value) -> Option<String> {
    let mut parts = Vec::new();
    for part in [
        json::nested_text(posting, &["location", "city"]),
        json::nested_text(posting, &["location", "region"]),
        json::nested_text(posting, &["location", "country"]),
    ]
    .into_iter()
    .flatten()
    {
        if !parts.contains(&part) {
            parts.push(part);
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

/// One posting object to a `RawPosting`; `description` is the detail payload's
/// `jobAd` when it was fetched.
fn parse_posting(
    posting: &Value,
    board: &BoardRef,
    description: Option<String>,
) -> Option<RawPosting> {
    let external_id = json::text(posting, "id")?;
    let title = json::text(posting, "name")?;
    let company_identifier = json::nested_text(posting, &["company", "identifier"])
        .unwrap_or_else(|| board.token.clone());
    Some(RawPosting {
        external_id: external_id.clone(),
        title,
        company: json::nested_text(posting, &["company", "name"])
            .unwrap_or_else(|| board.token.clone()),
        url: format!("https://jobs.smartrecruiters.com/{company_identifier}/{external_id}"),
        remote: posting
            .get("location")
            .and_then(|location| json::boolean(location, "remote")),
        locations: location_line(posting).into_iter().collect(),
        description_html: description,
        posted_at: json::text(posting, "releasedDate"),
        salary_text: None,
        raw: json::provenance(posting),
    })
}

/// The list is `{ content: [...], totalFound }`.
fn parse_list(payload: &Value, board: &BoardRef) -> Result<Vec<RawPosting>, SourceError> {
    Ok(json::array(payload, "content")?
        .iter()
        .filter_map(|posting| parse_posting(posting, board, None))
        .collect())
}

/// Join the detail payload's `jobAd` sections in reading order.
fn job_ad_html(posting: &Value) -> Option<String> {
    let sections = posting.get("jobAd")?.get("sections")?;
    let html: Vec<String> = [
        "companyDescription",
        "jobDescription",
        "qualifications",
        "additionalInformation",
    ]
    .into_iter()
    .filter_map(|section| json::nested_text(sections, &[section, "text"]))
    .collect();
    if html.is_empty() {
        None
    } else {
        Some(html.join("\n"))
    }
}

impl SourceAdapter for SmartRecruiters {
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
            let postings = parse_list(&payload, &self.board)?;
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
                    field: "smartrecruiters posting id".into(),
                    value: external_id.to_string(),
                })?;
            let body = json::get_json(ctx.http, &url, self.board.limits()).await?;
            let payload = json::parse(&body)?;
            let description = job_ad_html(&payload);
            parse_posting(&payload, &self.board, description).ok_or_else(|| {
                SourceError::InvalidPayload {
                    message: format!("smartrecruiters posting '{external_id}' has no name"),
                }
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
        "/tests/fixtures/ats/smartrecruiters_postings.json"
    ));
    const DETAIL: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ats/smartrecruiters_detail.json"
    ));

    fn client() -> HttpClient {
        HttpClient::new(HttpClientConfig {
            min_interval: Duration::from_millis(0),
            ..HttpClientConfig::default()
        })
        .expect("client")
    }

    fn board() -> BoardRef {
        BoardRef::new(AtsKind::SmartRecruiters, "Boba").expect("board")
    }

    async fn serve(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path("/v1/companies/Boba/postings"))
            .and(query_param("limit", "100"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(LIST, "application/json"))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn search_reads_the_recorded_page() {
        let server = MockServer::start().await;
        serve(&server).await;
        let adapter = SmartRecruiters::with_base(server.uri(), board());
        let http = client();
        let ctx = SourceCtx {
            http: &http,
            cancel: tokio_util::sync::CancellationToken::new(),
        };
        let postings = adapter.search(&SearchQuery::default(), &ctx).await.unwrap();
        assert_eq!(postings.len(), 2);
        let first = &postings[0];
        assert_eq!(first.external_id, "7440000123456789");
        assert_eq!(first.title, "Security Engineer");
        assert_eq!(first.company, "Boba Labs");
        assert_eq!(
            first.url,
            "https://jobs.smartrecruiters.com/Boba/7440000123456789"
        );
        assert_eq!(first.locations, vec!["Madrid, Comunidad de Madrid, es"]);
        assert_eq!(first.remote, Some(true));
        assert_eq!(first.posted_at.as_deref(), Some("2026-09-26T08:00:00.000Z"));
        assert!(
            first.description_html.is_none(),
            "the list carries no description"
        );
        assert_eq!(postings[1].remote, Some(false));
    }

    #[tokio::test]
    async fn detail_joins_the_job_ad_sections() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/companies/Boba/postings/7440000123456789"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(DETAIL, "application/json"))
            .mount(&server)
            .await;
        let adapter = SmartRecruiters::with_base(server.uri(), board());
        let http = client();
        let ctx = SourceCtx {
            http: &http,
            cancel: tokio_util::sync::CancellationToken::new(),
        };
        let posting = adapter.detail("7440000123456789", &ctx).await.unwrap();
        let html = posting.description_html.as_deref().unwrap();
        assert!(html.contains("Boba Labs makes job search calm."));
        assert!(html.contains("<li>Threat modelling</li>"));
        assert!(
            html.find("Boba Labs makes job search calm.") < html.find("<li>Threat modelling</li>"),
            "sections keep their reading order"
        );
    }
    #[test]
    fn a_detail_id_cannot_escape_the_company_path() {
        let adapter = SmartRecruiters::new(board());
        let url = adapter
            .detail_url("../evil")
            .expect("a mixed id is encoded");
        assert!(url.ends_with("/Boba/postings/..%2Fevil"), "{url}");
        assert!(!url.contains("/../"), "{url}");
        assert_eq!(adapter.detail_url(".."), None);
        assert_eq!(adapter.detail_url("."), None);
    }
    #[tokio::test]
    async fn a_detail_id_of_dots_is_refused_before_any_request() {
        let server = MockServer::start().await;
        let adapter = SmartRecruiters::with_base(server.uri(), board());
        let http = client();
        let ctx = SourceCtx {
            http: &http,
            cancel: tokio_util::sync::CancellationToken::new(),
        };
        let error = adapter.detail("..", &ctx).await.expect_err("refused");
        assert!(matches!(error, SourceError::InvalidValue { .. }));
    }
}
