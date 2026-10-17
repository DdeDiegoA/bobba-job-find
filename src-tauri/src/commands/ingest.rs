//! Manual Offer intake commands (S4.7, FR-25): paste a URL or paste the text.
//!
//! The command layer is the only place that can do the two things `boba-app`
//! cannot: fetch a page through the process-wide HTTP client (AD-10 — robots
//! rules, the per-host bucket and the honest User-Agent all apply) and read the
//! ports off the app handle (ADR-012). Everything after the fetch — reading the
//! page or the text, registering the Manual source, storing the Offer — is
//! [`ManualIntake`] in the core.
//!
//! A fetch that fails never stores anything: the screen shows the typed error
//! and asks for the text instead (AC1). What the page or the text contains is
//! untrusted (AD-7) and stays data: it is stored sanitised and never handed to
//! an agent prompt here.

use std::sync::Arc;

use boba_app::error::{AppError, AppResult, ErrorCode};
use boba_app::ingest::manual::{ManualError, ManualIntake, ManualOffer};
use boba_app::ports::agent::{AgentPort, RunBudget};
use boba_app::ports::repo::ingestion::IngestionRepo;
use boba_app::ports::repo::profile::ProfileRepo;
use boba_app::ports::repo::sources::SourcesRepo;
use boba_app::posting_features::{ExtractFeatures, ExtractRequest};
use boba_app::system::SystemClock;
use boba_sources::http::{FetchKind, FetchRequest, HttpClient};
use boba_sources::sdk::{RateLimits, SourceError};

use crate::wiring;

/// Redirects a pasted URL may follow before the fetch is reported as failed.
/// The shared client never follows one itself (AD-10: a cross-origin hop would
/// slip past both the robots check and the origin bucket), so this follows it by
/// hand — every hop goes back through the client and is therefore robots-checked
/// and paced for its own origin.
const MAX_HOPS: u8 = 5;

/// The Manual intake over the mounted ports (ADR-012).
fn intake(app: &tauri::AppHandle) -> AppResult<ManualIntake> {
    Ok(ManualIntake::new(
        wiring::port::<dyn IngestionRepo>(app)?,
        wiring::port::<dyn SourcesRepo>(app)?,
        Arc::new(SystemClock),
    ))
}

/// A use-case failure onto the typed IPC error (AD-3). The fetch failures are
/// `Unavailable`: the screen asks for the pasted text rather than retrying.
fn to_app(error: ManualError) -> AppError {
    let code = match &error {
        ManualError::EmptyInput | ManualError::TooLarge(_) => ErrorCode::InvalidInput,
        ManualError::FetchUnavailable | ManualError::FetchFailed(_) => ErrorCode::Unavailable,
        ManualError::Repo(_) | ManualError::Source(_) => ErrorCode::Internal,
    };
    let key = error.message_key();
    match error {
        // The reason is diagnostics: it never becomes the message (AD-3).
        ManualError::FetchFailed(reason) => AppError::new(code, key).with_detail("reason", reason),
        _ => AppError::new(code, key),
    }
}

/// Add an Offer from a pasted URL (AC1): fetch the page, then store what it
/// says. The user is asked to paste the text when the fetch fails.
#[tauri::command]
#[specta::specta]
pub async fn ingest_manual_url(app: tauri::AppHandle, url: String) -> AppResult<ManualOffer> {
    let url = pasted_url(&url)?;
    // The shared client is mounted by the composition root (port_list): an
    // unmounted one means no store was ever opened, so say so instead of
    // reporting a fetch failure.
    let http =
        wiring::port::<HttpClient>(&app).map_err(|_| to_app(ManualError::FetchUnavailable))?;

    let html = fetch_page(&http, &url)
        .await
        .map_err(|error| to_app(ManualError::FetchFailed(error.to_string())))?;
    let offer = intake(&app)?.from_page(&html, &url).map_err(to_app)?;
    trigger_extraction(&app, &offer.id);
    Ok(offer)
}

/// Add an Offer from pasted text (AC2).
#[tauri::command]
#[specta::specta]
pub fn ingest_manual_text(app: tauri::AppHandle, text: String) -> AppResult<ManualOffer> {
    let offer = intake(&app)?.from_text(&text).map_err(to_app)?;
    trigger_extraction(&app, &offer.id);
    Ok(offer)
}

/// Extract one manually-added Offer through the stored engine, in the
/// background: pasted postings get the agent pass once an engine is chosen
/// (S3.2's seam; the intake screen answers the user before it runs).
///
/// Nothing happens without a stored choice — the deterministic features are
/// still stored by the normal read path — without mounted ports, or on a CLI
/// ADR-010's gate refuses: the runtime refuses untrusted runs on an unverified
/// CLI before it starts, and that refusal is logged here, never retried.
///
/// The background thread re-probes first: the pre-run gate reads the stored
/// `cli_install` evidence, and a CLI upgraded since the engine step must lose a
/// lockdown it no longer has before this untrusted run is dispatched.
fn trigger_extraction(app: &tauri::AppHandle, offer_id: &str) {
    let Some(choice) = crate::commands::agent::stored_choice(app) else {
        return;
    };
    let (Ok(ingestion), Ok(profile), Ok(agents), Ok(probe)) = (
        wiring::port::<dyn IngestionRepo>(app),
        wiring::port::<dyn ProfileRepo>(app),
        wiring::port::<dyn AgentPort>(app),
        wiring::port::<crate::wiring::agent::EngineProbe>(app),
    ) else {
        return;
    };
    let features = ExtractFeatures::new(ingestion, profile, agents, Arc::new(SystemClock));
    let offer_id = offer_id.to_string();
    std::thread::spawn(move || {
        probe.detect_and_store();
        let request = ExtractRequest {
            cli: choice.cli,
            model: choice.model,
            // A background run still needs a cap: a CLI that never answers must
            // not keep a thread (and a provider call) alive forever.
            budget: RunBudget {
                wall_clock: Some(std::time::Duration::from_secs(300)),
                ..RunBudget::default()
            },
        };
        match features.run(&offer_id, request) {
            Ok(_) => tracing::info!(offer = %offer_id, "manual intake extraction finished"),
            Err(error) => {
                tracing::warn!(offer = %offer_id, %error, "manual intake extraction failed")
            }
        }
    });
}

/// The pasted URL, validated: this app only fetches http(s), and the pasted link
/// is what the posting will point at.
fn pasted_url(url: &str) -> AppResult<String> {
    let url = url.trim();
    if url.is_empty() {
        return Err(AppError::invalid_input("errors.manual.missing_url"));
    }
    let parsed =
        url::Url::parse(url).map_err(|_| AppError::invalid_input("errors.manual.invalid_url"))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(AppError::invalid_input("errors.manual.invalid_url"));
    }
    Ok(parsed.to_string())
}

/// Fetch one page through the shared client, following a redirect by hand and
/// bounded. Each hop is a fresh request, so robots.txt gates the destination as
/// well as the origin.
async fn fetch_page(http: &HttpClient, url: &str) -> Result<String, SourceError> {
    let mut current = url.to_string();
    for _ in 0..=MAX_HOPS {
        let response = http
            .get(FetchRequest {
                url: &current,
                kind: FetchKind::Html,
                limits: RateLimits::default(),
                budget_key: None,
            })
            .await?;
        match response.status {
            200 => return Ok(response.text()),
            300..=399 => match next_hop(&current, response.location.as_deref()) {
                Some(next) => current = next,
                // A redirect we cannot resolve is a failed fetch, not a guess.
                None => {
                    return Err(SourceError::Http {
                        url: current,
                        message: "redirect without a usable Location".to_string(),
                    });
                }
            },
            status => {
                return Err(SourceError::Http {
                    url: current,
                    message: format!("HTTP {status}"),
                });
            }
        }
    }
    Err(SourceError::Http {
        url: url.to_string(),
        message: format!("more than {MAX_HOPS} redirects"),
    })
}

/// The next URL of a redirect: absolute or relative to the current one, http(s)
/// only. A `javascript:` or `mailto:` target is not followed.
fn next_hop(current: &str, location: Option<&str>) -> Option<String> {
    let next = url::Url::parse(current).ok()?.join(location?).ok()?;
    matches!(next.scheme(), "http" | "https").then(|| next.to_string())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use boba_sources::http::HttpClientConfig;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    /// A client that does not wait between requests: the pacing itself is S4.1h's
    /// subject, not this test's.
    fn client() -> HttpClient {
        HttpClient::new(HttpClientConfig {
            min_interval: Duration::from_millis(1),
            retry_base_delay: Duration::from_millis(1),
            ..HttpClientConfig::default()
        })
        .expect("client")
    }

    /// AC1: the page the user pasted is fetched, and its text is what comes back.
    #[tokio::test]
    async fn a_pasted_url_is_fetched() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/job/1"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<h1>Engineer</h1>"))
            .mount(&server)
            .await;

        let html = fetch_page(&client(), &format!("{}/job/1", server.uri()))
            .await
            .expect("fetched");
        assert!(html.contains("Engineer"));
    }

    /// AC1: robots rules are respected — a disallowed page is a fetch failure,
    /// and the page is never requested at all.
    #[tokio::test]
    async fn robots_rules_block_the_fetch() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/robots.txt"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("User-Agent: *\nDisallow: /private"),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/private/job"))
            .respond_with(ResponseTemplate::new(200).set_body_string("nope"))
            .expect(0)
            .mount(&server)
            .await;

        let error = fetch_page(&client(), &format!("{}/private/job", server.uri()))
            .await
            .expect_err("robots disallow is a failure");
        assert!(matches!(error, SourceError::RobotsDisallowed { .. }));
    }

    /// A redirect is followed by hand, so the user's normal link (http → https,
    /// apex → www) still works.
    #[tokio::test]
    async fn a_redirect_is_followed_to_the_page() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/robots.txt"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/old"))
            .respond_with(
                ResponseTemplate::new(301)
                    .insert_header("Location", format!("{}/new", server.uri()).as_str()),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/new"))
            .respond_with(ResponseTemplate::new(200).set_body_string("the real page"))
            .mount(&server)
            .await;

        let html = fetch_page(&client(), &format!("{}/old", server.uri()))
            .await
            .expect("followed");
        assert_eq!(html, "the real page");
    }

    #[tokio::test]
    async fn a_redirect_loop_stops_instead_of_fetching_forever() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/robots.txt"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let itself = format!("{}/loop", server.uri());
        Mock::given(method("GET"))
            .and(path("/loop"))
            .respond_with(ResponseTemplate::new(302).insert_header("Location", itself.as_str()))
            .expect(u64::from(MAX_HOPS) + 1)
            .mount(&server)
            .await;

        let error = fetch_page(&client(), &itself).await.expect_err("bounded");
        assert!(error.to_string().contains("redirects"), "{error}");
    }

    /// An error status is a fetch failure (the screen asks for the text), never
    /// an Offer built from an error page.
    #[tokio::test]
    async fn an_error_status_is_a_fetch_failure() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/robots.txt"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/gone"))
            .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
            .mount(&server)
            .await;

        let error = fetch_page(&client(), &format!("{}/gone", server.uri()))
            .await
            .expect_err("404 is a failure");
        assert!(error.to_string().contains("404"), "{error}");
    }

    #[test]
    fn only_http_locations_are_followed() {
        assert_eq!(
            next_hop("https://a.example/job", Some("/next")).as_deref(),
            Some("https://a.example/next")
        );
        assert_eq!(
            next_hop("https://a.example/job", Some("https://b.example/x")).as_deref(),
            Some("https://b.example/x")
        );
        assert_eq!(
            next_hop("https://a.example/job", Some("javascript:alert(1)")),
            None
        );
        assert_eq!(
            next_hop("https://a.example/job", Some("mailto:x@y.z")),
            None
        );
        assert_eq!(next_hop("https://a.example/job", None), None);
    }

    #[test]
    fn only_a_pasted_http_url_is_accepted() {
        assert_eq!(
            pasted_url("  https://boards.example.com/job/1  ").unwrap(),
            "https://boards.example.com/job/1"
        );
        for bad in [
            "",
            "   ",
            "not a url",
            "file:///etc/passwd",
            "javascript:alert(1)",
        ] {
            let error = pasted_url(bad).unwrap_err();
            assert!(
                error.message_key == "errors.manual.missing_url"
                    || error.message_key == "errors.manual.invalid_url",
                "{bad}: {}",
                error.message_key
            );
        }
    }

    /// The end-to-end store path: the Manual source row is created, the Offer is
    /// written under it (the `posting.source_id` foreign key needs that row) and
    /// the paste is kept as provenance.
    #[test]
    fn the_intake_stores_an_offer_under_the_manual_source() {
        let store = store("manual-intake");
        let ingestion: Arc<dyn IngestionRepo> = store.clone();
        let sources: Arc<dyn SourcesRepo> = store.clone();
        let intake = ManualIntake::new(
            ingestion,
            sources,
            Arc::new(boba_app::system::FixedClock::new("2026-10-16T12:00:00Z")),
        );

        let offer = intake
            .from_text("Platform Engineer\nAcme\nRemote, USD 90,000 per year")
            .expect("stored");

        let stored = store.get_offer(&offer.id).expect("read").expect("row");
        assert_eq!(stored.source_id, "manual");
        assert_eq!(
            store.get_source("manual").expect("read").expect("row").name,
            "Manual"
        );
        assert_eq!(store.provenance(&offer.id).expect("read").len(), 1);
    }

    /// An open store with every migration applied (the wiring tests' fixture).
    fn store(tag: &str) -> Arc<boba_store::db::Store> {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir: std::path::PathBuf =
            std::env::temp_dir().join(format!("boba-ingest-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let key = boba_store::keys::DbKey::from_hex(
            "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff",
        )
        .expect("test key");
        let config = boba_store::db::StoreConfig::new(dir.join("boba.db"), dir.join("snapshots"));
        Arc::new(boba_store::db::Store::open_with_key(config, key).expect("store opens"))
    }
}
