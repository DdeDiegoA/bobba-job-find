//! The one outbound HTTP client (AD-10, architecture §6.3).
//!
//! Every source request goes through here. It applies a per-origin token bucket
//! (`governor`), honours each manifest's limits, checks robots.txt before any
//! HTML fetch, sends an honest User-Agent, retries 429/5xx with backoff only,
//! caps response size, and sanitises untrusted HTML with ammonia (AD-7).

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use boba_domain::source::{RateLimits, SourceError};
use governor::{DefaultDirectRateLimiter, Quota, RateLimiter};
use reqwest::header::{
    CONTENT_TYPE, ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED, LOCATION, RETRY_AFTER,
};
use texting_robots::Robot;

/// Honest identity: `BobaJobFind/<ver> (+repo URL)` (AD-10).
pub const REPO_URL: &str = "https://github.com/diegoarenas/boba-job-find";

/// Redirects a robots.txt fetch may follow before it is treated as broken
/// (RFC 9309 allows five). A broken chain fails closed (AD-10).
const MAX_ROBOTS_HOPS: u32 = 5;

/// What a robots.txt fetch produced.
enum RobotsFetch {
    Body(Vec<u8>),
    /// Missing or unreachable: nothing to enforce.
    Unavailable,
    /// Broken or endless redirect chain: the origin must fail closed.
    Broken,
}

/// The honest User-Agent string every request carries.
pub fn user_agent() -> String {
    format!("BobaJobFind/{} (+{})", env!("CARGO_PKG_VERSION"), REPO_URL)
}

/// What kind of resource is being fetched. `Html` triggers a robots.txt check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchKind {
    Html,
    Json,
    Feed,
    /// The robots.txt fetch itself; never robots-checked and never budgeted.
    Robots,
}

/// One request through the shared client.
#[derive(Debug, Clone)]
pub struct FetchRequest<'a> {
    pub url: &'a str,
    pub kind: FetchKind,
    pub limits: RateLimits,
    /// Stable key for the manifest's daily budget (normally the source id).
    pub budget_key: Option<&'a str>,
}

/// One POST through the shared client (S4.6: Jooble is POST-only).
#[derive(Debug, Clone)]
pub struct PostRequest<'a> {
    pub url: &'a str,
    pub limits: RateLimits,
    /// Stable key for the manifest's daily budget (normally the source id).
    pub budget_key: Option<&'a str>,
    pub body: &'a str,
    pub content_type: &'a str,
}

/// A response body plus enough metadata for adapters to parse it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
    pub from_cache: bool,
    /// `Location` on a 3xx; redirects are never followed (AD-10), so the target
    /// is handed to the adapter instead of being fetched behind its back.
    pub location: Option<String>,
}

impl HttpResponse {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// Client tunables; defaults follow architecture §6.3.
#[derive(Debug, Clone)]
pub struct HttpClientConfig {
    pub user_agent: String,
    /// Global per-host default: one request every 2 s.
    pub min_interval: Duration,
    pub timeout: Duration,
    pub max_body_bytes: usize,
    pub robots_cache_ttl: Duration,
    pub retry_base_delay: Duration,
    pub max_attempts: u32,
}

impl Default for HttpClientConfig {
    fn default() -> Self {
        Self {
            user_agent: user_agent(),
            min_interval: Duration::from_secs(2),
            timeout: Duration::from_secs(30),
            max_body_bytes: 4 * 1024 * 1024,
            robots_cache_ttl: Duration::from_secs(24 * 60 * 60),
            retry_base_delay: Duration::from_millis(500),
            max_attempts: 3,
        }
    }
}

#[derive(Clone)]
struct ConditionalEntry {
    etag: Option<String>,
    last_modified: Option<String>,
    body: Vec<u8>,
    content_type: Option<String>,
}

#[derive(Clone)]
struct CachedRobots {
    robot: Option<Arc<Robot>>,
    fetched: Instant,
}

/// One token bucket per origin (AD-10), paced at the strictest interval seen.
struct OriginBucket {
    interval: Duration,
    limiter: Arc<DefaultDirectRateLimiter>,
    /// Start of the last request on this origin; a rebuilt (idle) limiter is
    /// paced from here so tightening the interval cannot leak a free slot.
    last_start: Instant,
}

struct Budget {
    day: u64,
    count: u64,
}

/// The shared outbound client. One instance per core process.
pub struct HttpClient {
    client: reqwest::Client,
    config: HttpClientConfig,
    limiters: Mutex<HashMap<String, OriginBucket>>,
    robots: Mutex<HashMap<String, CachedRobots>>,
    budget: Mutex<HashMap<String, Budget>>,
    cache: Mutex<HashMap<String, ConditionalEntry>>,
}

impl HttpClient {
    pub fn new(config: HttpClientConfig) -> Result<Self, SourceError> {
        let client = reqwest::Client::builder()
            .user_agent(config.user_agent.clone())
            .timeout(config.timeout)
            // Never follow a redirect: a cross-origin hop would slip past both
            // the robots.txt check and the origin bucket (AD-10). The 3xx and
            // its `Location` are surfaced to the adapter instead.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| SourceError::Http {
                url: "<client build>".into(),
                message: e.to_string(),
            })?;
        Ok(Self {
            client,
            config,
            limiters: Mutex::new(HashMap::new()),
            robots: Mutex::new(HashMap::new()),
            budget: Mutex::new(HashMap::new()),
            cache: Mutex::new(HashMap::new()),
        })
    }

    /// A client with the production defaults and the honest User-Agent.
    pub fn honest() -> Result<Self, SourceError> {
        Self::new(HttpClientConfig::default())
    }

    /// GET through the shared client (AC2).
    pub async fn get(&self, request: FetchRequest<'_>) -> Result<HttpResponse, SourceError> {
        self.fetch(
            request.url,
            request.kind,
            request.limits,
            request.budget_key,
        )
        .await
    }

    /// POST through the shared client (S4.6). The same per-origin bucket, daily
    /// budget, timeout, body cap and no-redirect policy as [`Self::get`]; a POST
    /// is replayed only when doing so cannot double a side effect (S4.6 ruling:
    /// connect errors and 429/503 that carry `Retry-After`).
    pub async fn post(&self, request: PostRequest<'_>) -> Result<HttpResponse, SourceError> {
        if let (Some(key), Some(cap)) = (request.budget_key, request.limits.daily_cap) {
            self.check_budget(key, cap)?;
        }
        self.acquire_slot(request.url, &request.limits).await;
        self.send_post_with_retry(request.url, request.body, request.content_type)
            .await
    }

    async fn fetch(
        &self,
        url: &str,
        kind: FetchKind,
        limits: RateLimits,
        budget_key: Option<&str>,
    ) -> Result<HttpResponse, SourceError> {
        if let (Some(key), Some(cap)) = (budget_key, limits.daily_cap) {
            self.check_budget(key, cap)?;
        }
        if kind == FetchKind::Html {
            self.ensure_robots_allowed(url).await?;
        }
        self.acquire_slot(url, &limits).await;
        self.send_with_retry(url).await
    }

    /// Wait for the origin's token bucket, at the strictest interval seen for
    /// that origin, never below the global default (AD-10: one bucket per host).
    async fn acquire_slot(&self, url: &str, limits: &RateLimits) {
        let interval = Duration::from_millis(limits.min_interval_ms)
            .max(self.config.min_interval)
            .max(Duration::from_millis(1));
        let key = origin(url);
        let (limiter, carry_over) = {
            let mut map = self.limiters.lock().unwrap();
            let entry = map.entry(key.clone()).or_insert_with(|| OriginBucket {
                interval,
                limiter: Arc::new(RateLimiter::direct(quota_for(interval))),
                last_start: Instant::now(),
            });
            let mut carry_over = Duration::ZERO;
            if interval > entry.interval {
                entry.interval = interval;
                entry.limiter = Arc::new(RateLimiter::direct(quota_for(interval)));
                carry_over = interval.saturating_sub(entry.last_start.elapsed());
                // Tightening replaces the limiter, so a caller already waiting on
                // the replaced one can still slip through once.
                // shortcut: one in-flight request may beat a tightened interval;
                // upgrade to a queued limiter if that ever matters.
            }
            (entry.limiter.clone(), carry_over)
        };
        limiter.until_ready().await;
        if !carry_over.is_zero() {
            tokio::time::sleep(carry_over).await;
        }
        self.limiters
            .lock()
            .unwrap()
            .get_mut(&key)
            .expect("bucket stays inserted")
            .last_start = Instant::now();
    }

    fn check_budget(&self, key: &str, cap: u32) -> Result<(), SourceError> {
        let today = today_index();
        let mut map = self.budget.lock().unwrap();
        let entry = map.entry(key.to_string()).or_insert(Budget {
            day: today,
            count: 0,
        });
        if entry.day != today {
            entry.day = today;
            entry.count = 0;
        }
        if entry.count >= u64::from(cap) {
            return Err(SourceError::DailyCapExceeded {
                budget_key: key.to_string(),
            });
        }
        entry.count += 1;
        Ok(())
    }

    /// robots.txt check before HTML, cached for 24 h (AD-10, ADR-005 §3).
    async fn ensure_robots_allowed(&self, url: &str) -> Result<(), SourceError> {
        let key = origin(url);
        {
            let map = self.robots.lock().unwrap();
            if let Some(cached) = map.get(&key)
                && cached.fetched.elapsed() < self.config.robots_cache_ttl
            {
                return check_robot(cached.robot.as_deref(), url);
            }
        }
        let robot = match self.robots_url(url) {
            Some(robots_url) => match self.fetch_robots(&robots_url).await {
                RobotsFetch::Body(body) => Robot::new(&self.config.user_agent, &body)
                    .ok()
                    .map(Arc::new),
                // An absent or unreachable robots.txt means "allow all".
                RobotsFetch::Unavailable => None,
                // A broken redirect chain must never fall back to allow-all.
                RobotsFetch::Broken => Some(disallow_all(&self.config.user_agent)),
            },
            None => None,
        };
        {
            let mut map = self.robots.lock().unwrap();
            map.insert(
                key,
                CachedRobots {
                    robot: robot.clone(),
                    fetched: Instant::now(),
                },
            );
        }
        check_robot(robot.as_deref(), url)
    }

    /// Fetch robots.txt, following its redirects by hand: the client never
    /// follows any (AD-10), and a 3xx used to reach the real rules (http→https,
    /// apex→www). Every hop is paced for its own origin.
    async fn fetch_robots(&self, url: &str) -> RobotsFetch {
        // Host default only: a manifest's interval belongs to the adapter's own
        // request. This also stays out of the `fetch` recursion cycle.
        let host_default = RateLimits {
            min_interval_ms: 0,
            daily_cap: None,
        };
        let mut current = url.to_string();
        for _ in 0..=MAX_ROBOTS_HOPS {
            self.acquire_slot(&current, &host_default).await;
            match self.send_with_retry(&current).await {
                Ok(response) if response.status == 200 => return RobotsFetch::Body(response.body),
                Ok(response) if (300..400).contains(&response.status) => {
                    match response
                        .location
                        .as_deref()
                        .and_then(|location| redirect_target(&current, location))
                    {
                        Some(next) => current = next,
                        None => return RobotsFetch::Broken,
                    }
                }
                _ => return RobotsFetch::Unavailable,
            }
        }
        RobotsFetch::Broken
    }

    fn robots_url(&self, url: &str) -> Option<String> {
        let parsed = reqwest::Url::parse(url).ok()?;
        let host = parsed.host_str()?;
        Some(format!(
            "{}://{}/robots.txt",
            parsed.scheme(),
            authority(&parsed, host)
        ))
    }

    async fn send_with_retry(&self, url: &str) -> Result<HttpResponse, SourceError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let (if_none_match, if_modified_since) = {
                let cache = self.cache.lock().unwrap();
                match cache.get(url) {
                    Some(entry) => (entry.etag.clone(), entry.last_modified.clone()),
                    None => (None, None),
                }
            };
            let mut builder = self.client.get(url);
            if let Some(etag) = if_none_match {
                builder = builder.header(IF_NONE_MATCH, etag);
            }
            if let Some(last_modified) = if_modified_since {
                builder = builder.header(IF_MODIFIED_SINCE, last_modified);
            }

            let response = match builder.send().await {
                Ok(response) => response,
                Err(error) if attempt < self.config.max_attempts && retryable_transport(&error) => {
                    tokio::time::sleep(self.backoff(attempt)).await;
                    continue;
                }
                Err(error) => {
                    return Err(SourceError::Http {
                        url: url.to_string(),
                        message: error.to_string(),
                    });
                }
            };

            let status = response.status();
            if (status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error())
                && attempt < self.config.max_attempts
            {
                let delay = retry_after(&response).unwrap_or_else(|| self.backoff(attempt));
                tokio::time::sleep(delay).await;
                continue;
            }
            return self.to_response(url, response, true).await;
        }
    }

    async fn send_post_with_retry(
        &self,
        url: &str,
        body: &str,
        content_type: &str,
    ) -> Result<HttpResponse, SourceError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let response = match self
                .client
                .post(url)
                .header(CONTENT_TYPE, content_type)
                .body(body.to_string())
                .send()
                .await
            {
                Ok(response) => response,
                // A request that never connected cannot have been processed.
                Err(error) if attempt < self.config.max_attempts && error.is_connect() => {
                    tokio::time::sleep(self.backoff(attempt)).await;
                    continue;
                }
                Err(error) => {
                    return Err(SourceError::Http {
                        url: url.to_string(),
                        message: error.to_string(),
                    });
                }
            };

            // A POST is never replayed on a bare 5xx: the server may have acted
            // on it. Only a rate-limited 429/503 that says how long to wait is
            // safe (and required) to retry.
            let status = response.status();
            let safe_to_retry = status == reqwest::StatusCode::TOO_MANY_REQUESTS
                || status == reqwest::StatusCode::SERVICE_UNAVAILABLE;
            if safe_to_retry
                && attempt < self.config.max_attempts
                && let Some(delay) = retry_after(&response)
            {
                tokio::time::sleep(delay).await;
                continue;
            }
            // A POST's response is never fed to the conditional cache.
            return self.to_response(url, response, false).await;
        }
    }

    async fn to_response(
        &self,
        url: &str,
        response: reqwest::Response,
        store: bool,
    ) -> Result<HttpResponse, SourceError> {
        let status = response.status().as_u16();
        let content_type = header_str(&response, CONTENT_TYPE);
        let location = header_str(&response, LOCATION);

        if status == 304 {
            let cached = self.cache.lock().unwrap().get(url).cloned();
            return match cached {
                Some(entry) => Ok(HttpResponse {
                    status: 200,
                    content_type: entry.content_type,
                    body: entry.body,
                    from_cache: true,
                    location: None,
                }),
                None => Ok(HttpResponse {
                    status,
                    content_type,
                    body: Vec::new(),
                    from_cache: true,
                    location,
                }),
            };
        }

        if let Some(length) = response.content_length()
            && length as usize > self.config.max_body_bytes
        {
            return Err(SourceError::ResponseTooLarge {
                url: url.to_string(),
                limit: self.config.max_body_bytes,
            });
        }

        let etag = header_str(&response, ETAG);
        let last_modified = header_str(&response, LAST_MODIFIED);
        let body = response
            .bytes()
            .await
            .map_err(|e| SourceError::Http {
                url: url.to_string(),
                message: e.to_string(),
            })?
            .to_vec();
        if body.len() > self.config.max_body_bytes {
            return Err(SourceError::ResponseTooLarge {
                url: url.to_string(),
                limit: self.config.max_body_bytes,
            });
        }

        if status == 200 && store {
            self.cache.lock().unwrap().insert(
                url.to_string(),
                ConditionalEntry {
                    etag,
                    last_modified,
                    body: body.clone(),
                    content_type: content_type.clone(),
                },
            );
        }
        Ok(HttpResponse {
            status,
            content_type,
            body,
            from_cache: false,
            location,
        })
    }

    fn backoff(&self, attempt: u32) -> Duration {
        self.config.retry_base_delay * 2u32.pow(attempt.min(6))
    }
}

/// Sanitise untrusted posting HTML before any UI use (AD-7).
pub fn sanitize_html(html: &str) -> String {
    ammonia::clean(html)
}

fn quota_for(interval: Duration) -> Quota {
    Quota::with_period(interval)
        .expect("interval is non-zero")
        .allow_burst(NonZeroU32::new(1).expect("burst is non-zero"))
}

fn origin(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(parsed) => match parsed.host_str() {
            Some(host) => format!("{}://{}", parsed.scheme(), authority(&parsed, host)),
            None => url.to_string(),
        },
        Err(_) => url.to_string(),
    }
}

fn authority(parsed: &reqwest::Url, host: &str) -> String {
    match parsed.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    }
}

fn check_robot(robot: Option<&Robot>, url: &str) -> Result<(), SourceError> {
    match robot {
        Some(robot) if !robot.allowed(url) => Err(SourceError::RobotsDisallowed {
            url: url.to_string(),
        }),
        _ => Ok(()),
    }
}

/// The rules used when a robots.txt redirect chain is broken (AD-10).
fn disallow_all(user_agent: &str) -> Arc<Robot> {
    Arc::new(
        Robot::new(user_agent, b"User-Agent: *\nDisallow: /\n")
            .expect("a static disallow-all robots file parses"),
    )
}

/// Resolve a `Location` (absolute or relative) to the next http(s) URL.
fn redirect_target(current: &str, location: &str) -> Option<String> {
    let next = reqwest::Url::parse(current).ok()?.join(location).ok()?;
    matches!(next.scheme(), "http" | "https").then(|| next.to_string())
}

fn header_str(response: &reqwest::Response, name: reqwest::header::HeaderName) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

fn retry_after(response: &reqwest::Response) -> Option<Duration> {
    response
        .headers()
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

fn retryable_transport(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect()
}

fn today_index() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client(interval: Duration) -> HttpClient {
        HttpClient::new(HttpClientConfig {
            min_interval: interval,
            retry_base_delay: Duration::from_millis(1),
            robots_cache_ttl: Duration::from_secs(60),
            ..HttpClientConfig::default()
        })
        .expect("client")
    }

    fn request(url: &str, kind: FetchKind) -> FetchRequest<'_> {
        FetchRequest {
            url,
            kind,
            limits: RateLimits::default(),
            budget_key: None,
        }
    }

    /// A JSON request with an explicit host interval.
    fn limited(url: &str, min_interval_ms: u64) -> FetchRequest<'_> {
        FetchRequest {
            url,
            kind: FetchKind::Json,
            limits: RateLimits {
                min_interval_ms,
                daily_cap: None,
            },
            budget_key: None,
        }
    }

    #[tokio::test]
    async fn sends_honest_user_agent() {
        // The mock only matches when the honest UA is present.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/jobs"))
            .and(header("user-agent", user_agent().as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .expect(1)
            .mount(&server)
            .await;

        let response = client(Duration::from_millis(1))
            .get(request(&format!("{}/jobs", server.uri()), FetchKind::Json))
            .await
            .expect("fetch");
        assert_eq!(response.status, 200);
        assert_eq!(response.text(), "ok");
        assert!(user_agent().starts_with("BobaJobFind/"));
        assert!(user_agent().contains(REPO_URL));
    }

    #[tokio::test]
    async fn robots_disallow_blocks_html_and_never_fetches_it() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/robots.txt"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("User-Agent: *\nDisallow: /secret"),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/secret"))
            .respond_with(ResponseTemplate::new(200).set_body_string("nope"))
            .expect(0)
            .mount(&server)
            .await;

        let error = client(Duration::from_millis(1))
            .get(request(
                &format!("{}/secret", server.uri()),
                FetchKind::Html,
            ))
            .await
            .expect_err("robots disallow");
        assert!(matches!(error, SourceError::RobotsDisallowed { .. }));
    }

    #[tokio::test]
    async fn absent_robots_allows_html() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/robots.txt"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/page"))
            .respond_with(ResponseTemplate::new(200).set_body_string("hi"))
            .mount(&server)
            .await;

        let response = client(Duration::from_millis(1))
            .get(request(&format!("{}/page", server.uri()), FetchKind::Html))
            .await
            .expect("fetch");
        assert_eq!(response.text(), "hi");
    }

    #[tokio::test]
    async fn robots_is_cached_across_html_fetches() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/robots.txt"))
            .respond_with(ResponseTemplate::new(200).set_body_string("User-Agent: *\nAllow: /"))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/one"))
            .respond_with(ResponseTemplate::new(200).set_body_string("1"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/two"))
            .respond_with(ResponseTemplate::new(200).set_body_string("2"))
            .mount(&server)
            .await;

        let http = client(Duration::from_millis(1));
        http.get(request(&format!("{}/one", server.uri()), FetchKind::Html))
            .await
            .expect("first");
        http.get(request(&format!("{}/two", server.uri()), FetchKind::Html))
            .await
            .expect("second");
    }

    #[tokio::test]
    async fn manifest_daily_cap_is_enforced() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/feed"))
            .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
            .mount(&server)
            .await;

        let http = client(Duration::from_millis(1));
        let url = format!("{}/feed", server.uri());
        let limits = RateLimits {
            min_interval_ms: 1,
            daily_cap: Some(1),
        };
        let first = http
            .get(FetchRequest {
                url: &url,
                kind: FetchKind::Json,
                limits,
                budget_key: Some("remotive"),
            })
            .await
            .expect("first");
        assert_eq!(first.status, 200);
        let second = http
            .get(FetchRequest {
                url: &url,
                kind: FetchKind::Json,
                limits,
                budget_key: Some("remotive"),
            })
            .await
            .expect_err("cap");
        assert!(matches!(second, SourceError::DailyCapExceeded { .. }));
    }

    #[tokio::test]
    async fn per_origin_buckets_do_not_block_each_other() {
        // A 30 s interval would make a shared bucket block for 30 s. Different
        // origins must still return within the 2 s test timeout.
        let first = MockServer::start().await;
        let second = MockServer::start().await;
        for server in [&first, &second] {
            Mock::given(method("GET"))
                .and(path("/x"))
                .respond_with(ResponseTemplate::new(200).set_body_string("x"))
                .mount(server)
                .await;
        }

        let http = client(Duration::from_secs(30));
        tokio::time::timeout(
            Duration::from_secs(2),
            http.get(request(&format!("{}/x", first.uri()), FetchKind::Json)),
        )
        .await
        .expect("first not blocked")
        .expect("first ok");
        tokio::time::timeout(
            Duration::from_secs(2),
            http.get(request(&format!("{}/x", second.uri()), FetchKind::Json)),
        )
        .await
        .expect("second origin not blocked by the first")
        .expect("second ok");
    }

    /// Two sources on one host share one bucket: the stricter interval wins (AD-10).
    #[tokio::test]
    async fn same_host_sources_share_one_bucket_at_the_strictest_interval() {
        let server = MockServer::start().await;
        for p in ["/lax", "/strict"] {
            Mock::given(method("GET"))
                .and(path(p))
                .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
                .mount(&server)
                .await;
        }
        let lax = format!("{}/lax", server.uri());
        let strict = format!("{}/strict", server.uri());

        let http = client(Duration::from_millis(1));
        // The lax source seeds the bucket; the strict source must tighten it.
        http.get(limited(&lax, 1)).await.expect("lax source");
        let started = Instant::now();
        http.get(limited(&strict, 300))
            .await
            .expect("strict source");
        assert!(
            started.elapsed() >= Duration::from_millis(250),
            "the stricter interval must pace the shared host bucket, waited {:?}",
            started.elapsed()
        );
    }

    /// The strictest interval seen sticks, even when a laxer source follows (AD-10).
    #[tokio::test]
    async fn a_laxer_source_keeps_the_strictest_host_interval() {
        let server = MockServer::start().await;
        for p in ["/strict", "/lax"] {
            Mock::given(method("GET"))
                .and(path(p))
                .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
                .mount(&server)
                .await;
        }
        let strict = format!("{}/strict", server.uri());
        let lax = format!("{}/lax", server.uri());

        let http = client(Duration::from_millis(1));
        http.get(limited(&strict, 300))
            .await
            .expect("strict source");
        let started = Instant::now();
        http.get(limited(&lax, 1)).await.expect("lax source");
        assert!(
            started.elapsed() >= Duration::from_millis(250),
            "the host keeps the strictest interval seen, waited {:?}",
            started.elapsed()
        );
    }

    /// A cross-origin redirect is surfaced, never followed (AD-10): the target
    /// host is not contacted at all, so its robots.txt can never be bypassed.
    #[tokio::test]
    async fn cross_origin_redirect_to_a_robots_disallowed_path_is_not_followed() {
        let target = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/robots.txt"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("User-Agent: *\nDisallow: /blocked"),
            )
            .expect(0)
            .mount(&target)
            .await;
        Mock::given(method("GET"))
            .and(path("/blocked"))
            .respond_with(ResponseTemplate::new(200).set_body_string("must never be fetched"))
            .expect(0)
            .mount(&target)
            .await;

        let origin = MockServer::start().await;
        let location = format!("{}/blocked", target.uri());
        Mock::given(method("GET"))
            .and(path("/go"))
            .respond_with(ResponseTemplate::new(302).insert_header("Location", location.as_str()))
            .mount(&origin)
            .await;

        let response = client(Duration::from_millis(1))
            .get(request(&format!("{}/go", origin.uri()), FetchKind::Json))
            .await
            .expect("the redirect is surfaced as a response");
        assert_eq!(response.status, 302);
        assert_eq!(response.location.as_deref(), Some(location.as_str()));
        assert!(!response.text().contains("must never be fetched"));
    }

    /// A redirected robots.txt is followed by hand (RFC 9309), so its rules
    /// still gate the origin that served the redirect.
    #[tokio::test]
    async fn redirected_robots_is_followed_and_still_blocks() {
        let target = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/robots.txt"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("User-Agent: *\nDisallow: /blocked"),
            )
            .mount(&target)
            .await;
        Mock::given(method("GET"))
            .and(path("/blocked"))
            .respond_with(ResponseTemplate::new(200).set_body_string("must never be fetched"))
            .expect(0)
            .mount(&target)
            .await;

        let origin = MockServer::start().await;
        let robots_target = format!("{}/robots.txt", target.uri());
        Mock::given(method("GET"))
            .and(path("/robots.txt"))
            .respond_with(
                ResponseTemplate::new(301).insert_header("Location", robots_target.as_str()),
            )
            .mount(&origin)
            .await;
        Mock::given(method("GET"))
            .and(path("/blocked"))
            .respond_with(ResponseTemplate::new(200).set_body_string("must never be fetched"))
            .expect(0)
            .mount(&origin)
            .await;

        let error = client(Duration::from_millis(1))
            .get(request(
                &format!("{}/blocked", origin.uri()),
                FetchKind::Html,
            ))
            .await
            .expect_err("the redirected robots.txt still disallows /blocked");
        assert!(matches!(error, SourceError::RobotsDisallowed { .. }));
    }

    /// A robots.txt redirect without a usable `Location` fails closed (AD-10).
    #[tokio::test]
    async fn robots_redirect_without_a_location_fails_closed() {
        let origin = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/robots.txt"))
            .respond_with(ResponseTemplate::new(301))
            .mount(&origin)
            .await;
        Mock::given(method("GET"))
            .and(path("/page"))
            .respond_with(ResponseTemplate::new(200).set_body_string("must never be fetched"))
            .expect(0)
            .mount(&origin)
            .await;

        let error = client(Duration::from_millis(1))
            .get(request(&format!("{}/page", origin.uri()), FetchKind::Html))
            .await
            .expect_err("a broken robots redirect must not allow all");
        assert!(matches!(error, SourceError::RobotsDisallowed { .. }));
    }

    /// A robots.txt redirect loop stops after five hops and fails closed.
    #[tokio::test]
    async fn robots_redirect_loop_fails_closed() {
        let origin = MockServer::start().await;
        let itself = format!("{}/robots.txt", origin.uri());
        Mock::given(method("GET"))
            .and(path("/robots.txt"))
            .respond_with(ResponseTemplate::new(302).insert_header("Location", itself.as_str()))
            .expect(6) // the original fetch plus the five allowed hops
            .mount(&origin)
            .await;

        let error = client(Duration::from_millis(1))
            .get(request(&format!("{}/page", origin.uri()), FetchKind::Html))
            .await
            .expect_err("a redirect loop must not allow all");
        assert!(matches!(error, SourceError::RobotsDisallowed { .. }));
    }

    #[tokio::test]
    async fn retries_429_then_succeeds() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/flaky"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/flaky"))
            .respond_with(ResponseTemplate::new(200).set_body_string("recovered"))
            .mount(&server)
            .await;

        let response = client(Duration::from_millis(1))
            .get(request(&format!("{}/flaky", server.uri()), FetchKind::Json))
            .await
            .expect("fetch");
        assert_eq!(response.status, 200);
        assert_eq!(response.text(), "recovered");
    }

    fn post_request<'a>(url: &'a str, body: &'a str) -> PostRequest<'a> {
        PostRequest {
            url,
            limits: RateLimits {
                min_interval_ms: 0,
                daily_cap: None,
            },
            budget_key: None,
            body,
            content_type: "application/json",
        }
    }

    #[tokio::test]
    async fn post_sends_the_json_body_and_returns_the_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/key"))
            .and(header("content-type", "application/json"))
            .and(body_string_contains("\"keywords\":\"rust\""))
            .respond_with(ResponseTemplate::new(200).set_body_string("{\"jobs\":[]}"))
            .expect(1)
            .mount(&server)
            .await;

        let response = client(Duration::from_millis(1))
            .post(post_request(
                &format!("{}/api/key", server.uri()),
                "{\"keywords\":\"rust\"}",
            ))
            .await
            .expect("post");
        assert_eq!(response.status, 200);
        assert_eq!(response.text(), "{\"jobs\":[]}");
    }

    #[tokio::test]
    async fn post_never_follows_a_redirect() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/key"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", "https://elsewhere.test/collect"),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/collect"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        let response = client(Duration::from_millis(1))
            .post(post_request(&format!("{}/api/key", server.uri()), "{}"))
            .await
            .expect("post");
        assert_eq!(response.status, 302);
        assert_eq!(
            response.location.as_deref(),
            Some("https://elsewhere.test/collect")
        );
    }

    #[tokio::test]
    async fn post_retries_a_429_only_when_the_server_says_when() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/key"))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "0"))
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/key"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .expect(1)
            .mount(&server)
            .await;

        let response = client(Duration::from_millis(1))
            .post(post_request(&format!("{}/api/key", server.uri()), "{}"))
            .await
            .expect("post");
        assert_eq!(response.status, 200);
    }

    #[tokio::test]
    async fn post_does_not_retry_a_5xx_without_retry_after() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/key"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .expect(1)
            .mount(&server)
            .await;

        let response = client(Duration::from_millis(1))
            .post(post_request(&format!("{}/api/key", server.uri()), "{}"))
            .await
            .expect("post");
        assert_eq!(response.status, 500, "a POST is not replayed blindly");
    }

    #[tokio::test]
    async fn post_counts_against_the_daily_budget() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/key"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .mount(&server)
            .await;
        let http = client(Duration::from_millis(1));
        let url = format!("{}/api/key", server.uri());
        for _ in 0..2 {
            let mut request = post_request(&url, "{}");
            request.limits.daily_cap = Some(2);
            request.budget_key = Some("jooble");
            http.post(request).await.expect("within budget");
        }
        let mut request = post_request(&url, "{}");
        request.limits.daily_cap = Some(2);
        request.budget_key = Some("jooble");
        let error = http.post(request).await.expect_err("budget spent");
        assert!(matches!(
            error,
            SourceError::DailyCapExceeded { ref budget_key } if budget_key == "jooble"
        ));
    }

    #[tokio::test]
    async fn post_response_over_the_cap_is_rejected() {
        let server = MockServer::start().await;
        let body = "x".repeat(HttpClientConfig::default().max_body_bytes + 1);
        Mock::given(method("POST"))
            .and(path("/api/key"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;

        let error = client(Duration::from_millis(1))
            .post(post_request(&format!("{}/api/key", server.uri()), "{}"))
            .await
            .expect_err("too large");
        assert!(matches!(error, SourceError::ResponseTooLarge { .. }));
    }

    #[tokio::test]
    async fn a_post_response_is_not_cached() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/key"))
            .respond_with(ResponseTemplate::new(200).set_body_string("first"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/key"))
            .respond_with(ResponseTemplate::new(200).set_body_string("second"))
            .expect(1)
            .mount(&server)
            .await;

        let http = client(Duration::from_millis(1));
        let url = format!("{}/api/key", server.uri());
        http.post(post_request(&url, "{}")).await.expect("post");
        let response = http.get(request(&url, FetchKind::Json)).await.expect("get");
        assert_eq!(response.text(), "second");
        assert!(!response.from_cache);
    }

    #[tokio::test]
    async fn response_over_the_cap_is_rejected() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/big"))
            .respond_with(ResponseTemplate::new(200).set_body_string("x".repeat(4096)))
            .mount(&server)
            .await;

        let http = HttpClient::new(HttpClientConfig {
            max_body_bytes: 64,
            min_interval: Duration::from_millis(1),
            ..HttpClientConfig::default()
        })
        .expect("client");
        let error = http
            .get(request(&format!("{}/big", server.uri()), FetchKind::Json))
            .await
            .expect_err("too large");
        assert!(matches!(error, SourceError::ResponseTooLarge { .. }));
    }

    #[test]
    fn sanitize_html_strips_scripts_and_handlers() {
        let cleaned =
            sanitize_html("<p>hi<script>alert(1)</script><a href=\"#\" onclick=\"x()\">l</a></p>");
        assert!(!cleaned.contains("<script"));
        assert!(!cleaned.contains("onclick"));
        assert!(cleaned.contains("hi"));
    }

    #[test]
    fn quota_matches_the_interval() {
        let quota = quota_for(Duration::from_millis(250));
        assert_eq!(quota.replenish_interval(), Duration::from_millis(250));
        assert_eq!(quota.burst_size().get(), 1);
    }

    #[test]
    fn direct_limiter_allows_one_then_blocks() {
        let limiter = RateLimiter::direct(quota_for(Duration::from_secs(30)));
        assert!(limiter.check().is_ok());
        assert!(limiter.check().is_err());
    }
}
