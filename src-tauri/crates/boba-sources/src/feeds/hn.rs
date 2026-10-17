//! Hacker News "Who is hiring?" adapter (S4.6, integrations.md §2, A.2).
//!
//! Two Algolia calls: the newest `Ask HN: Who is hiring?` story, then its
//! comments. Each top-level comment is one posting. HN comments are free prose,
//! so the mapping is deliberately best-effort — the deterministic extraction
//! (S6.2) and, where configured, the agent do the real parsing; the `Company:`
//! and `Location:` conventions are read when the comment follows them.

use boba_domain::source::{RateLimits, RawPosting, SearchQuery, SourceError, SourceManifest};
use serde_json::Value;

use super::FeedKind;
use super::api;
use crate::sdk::{BoxFuture, SourceAdapter, SourceCtx};

/// The Algolia API root.
pub const API: &str = "https://hn.algolia.com/api/v1";

const NAME: &str = "Hacker News";

pub struct HackerNews {
    base: String,
    limits: RateLimits,
}

impl HackerNews {
    pub fn new() -> Self {
        Self {
            base: API.to_string(),
            limits: FeedKind::HackerNews.limits(),
        }
    }

    #[cfg(test)]
    pub(crate) fn test_with(base: impl Into<String>, limits: RateLimits) -> Self {
        Self {
            base: base.into(),
            limits,
        }
    }

    fn story_url(&self) -> String {
        format!(
            "{}/search?query=who%20is%20hiring&tags=story,author_whoishiring&hitsPerPage=1",
            self.base
        )
    }

    fn item_url(&self, id: &str) -> Option<String> {
        if !valid_id(id) {
            return None;
        }
        Some(format!("{}/items/{id}", self.base))
    }

    async fn list(&self, ctx: &SourceCtx<'_>) -> Result<Vec<RawPosting>, SourceError> {
        let budget = Some(FeedKind::HackerNews.key());
        let story_body =
            api::get_json(ctx.http, &self.story_url(), self.limits, budget, None).await?;
        let story = api::parse(&story_body)?;
        let story_id = story
            .get("hits")
            .and_then(Value::as_array)
            .and_then(|hits| hits.first())
            .and_then(|hit| {
                hit.get("objectID")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .filter(|id| valid_id(id))
            .ok_or_else(|| SourceError::InvalidPayload {
                message: "HN has no 'Who is hiring?' story".into(),
            })?;
        let item_url = self
            .item_url(&story_id)
            .expect("the story id is already validated");
        let body = api::get_json(ctx.http, &item_url, self.limits, budget, None).await?;
        let thread = api::parse(&body)?;
        let children = thread
            .get("children")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        Ok(children.iter().filter_map(parse_comment).collect())
    }
}

impl Default for HackerNews {
    fn default() -> Self {
        Self::new()
    }
}

/// An HN id is numeric; anything else cannot become a path segment.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit())
}

/// One comment to a `RawPosting`; `None` for deleted/empty comments.
fn parse_comment(comment: &Value) -> Option<RawPosting> {
    let external_id = match comment.get("id") {
        Some(Value::Number(id)) => id.to_string(),
        Some(Value::String(id)) if valid_id(id.trim()) => id.trim().to_string(),
        _ => return None,
    };
    let html = api::text(comment, "text")?;
    let lines = plain_lines(&html);
    let first = lines.first()?.clone();
    let (mut company, mut role, mut locations) = (None, None, Vec::new());
    for line in lines.iter().take(6) {
        for segment in line.split('|') {
            let Some((key, value)) = segment.split_once(':') else {
                continue;
            };
            let (key, value) = (key.trim().to_ascii_lowercase(), value.trim());
            if value.is_empty() {
                continue;
            }
            match key.as_str() {
                "company" => company = Some(value.to_string()),
                "role" | "position" | "title" | "job" if role.is_none() => {
                    role = Some(value.to_string())
                }
                "location" => locations.push(value.to_string()),
                _ => {}
            }
        }
    }
    let lower = html.to_ascii_lowercase();
    Some(RawPosting {
        external_id: external_id.clone(),
        title: role.unwrap_or_else(|| truncate(&first, 160)),
        company: company.unwrap_or_else(|| NAME.to_string()),
        url: format!("https://news.ycombinator.com/item?id={external_id}"),
        remote: lower.contains("remote").then_some(true),
        locations,
        description_html: Some(html),
        posted_at: api::text(comment, "created_at").and_then(|date| api::utc_timestamp(&date)),
        salary_text: None,
        raw: api::provenance(comment),
    })
}

/// Non-empty, trimmed lines of an HN comment's HTML, tags removed.
fn plain_lines(html: &str) -> Vec<String> {
    let mut text = String::with_capacity(html.len());
    let mut in_tag = false;
    for chat in html.chars() {
        match chat {
            '<' => {
                in_tag = true;
                text.push(' ');
            }
            '>' => in_tag = false,
            _ if !in_tag => text.push(chat),
            _ => {}
        }
    }
    text.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let cut: String = text.chars().take(limit).collect();
    format!("{}…", cut.trim_end())
}

impl SourceAdapter for HackerNews {
    fn manifest(&self) -> SourceManifest {
        FeedKind::HackerNews.manifest()
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
            let url = self
                .item_url(external_id)
                .ok_or_else(|| SourceError::InvalidValue {
                    field: "hacker news item id".into(),
                    value: external_id.to_string(),
                })?;
            let body = api::get_json(
                ctx.http,
                &url,
                self.limits,
                Some(FeedKind::HackerNews.key()),
                None,
            )
            .await?;
            let item = api::parse(&body)?;
            parse_comment(&item).ok_or_else(|| SourceError::InvalidPayload {
                message: format!("hacker news item '{external_id}' has no text"),
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

    const STORY: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/feeds/hn_story.json"
    ));
    const THREAD: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/feeds/hn_thread.json"
    ));
    const ITEM: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/feeds/hn_item.json"
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

    async fn serve(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path("/search"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(STORY, "application/json"))
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path("/items/45123456"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(THREAD, "application/json"))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn search_reads_the_latest_threads_top_level_comments() {
        let server = MockServer::start().await;
        serve(&server).await;
        let adapter = HackerNews::test_with(server.uri(), RateLimits::default());
        let http = client();

        let postings = adapter
            .search(&SearchQuery::default(), &ctx(&http))
            .await
            .unwrap();
        assert_eq!(postings.len(), 2, "an empty comment is not a posting");

        let first = &postings[0];
        assert_eq!(first.external_id, "45123500");
        assert_eq!(first.title, "Senior Rust Engineer");
        assert_eq!(first.company, "Boba Labs");
        assert_eq!(first.locations, vec!["Remote (EU)"]);
        assert_eq!(first.remote, Some(true));
        assert_eq!(first.url, "https://news.ycombinator.com/item?id=45123500");
        assert_eq!(first.posted_at.as_deref(), Some("2026-10-01T15:10:00.000Z"));
        assert!(first.description_html.as_deref().unwrap().contains("<p>"));

        // A comment without the `Company:` convention still yields a posting.
        assert_eq!(postings[1].company, "Hacker News");
        assert_eq!(postings[1].remote, None);
    }

    #[tokio::test]
    async fn detail_reads_one_comment() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/items/45123500"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(ITEM, "application/json"))
            .mount(&server)
            .await;
        let adapter = HackerNews::test_with(server.uri(), RateLimits::default());
        let http = client();
        let posting = adapter.detail("45123500", &ctx(&http)).await.unwrap();
        assert_eq!(posting.title, "Senior Rust Engineer");
    }

    #[tokio::test]
    async fn a_non_numeric_id_is_refused_before_any_request() {
        let server = MockServer::start().await;
        let adapter = HackerNews::test_with(server.uri(), RateLimits::default());
        let http = client();
        let error = adapter
            .detail("../../admin", &ctx(&http))
            .await
            .expect_err("refused");
        assert!(matches!(error, SourceError::InvalidValue { .. }));
    }

    #[test]
    fn an_oversized_first_line_is_truncated() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("a very long title", 6), "a very…");
    }
}
