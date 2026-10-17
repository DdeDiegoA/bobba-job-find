//! Source commands (S4.8): the Connections catalogue, the custom RSS probe and
//! the custom feed add/remove actions (FR-23, FR-26, FR-31).
//!
//! The command layer is the only place that can do the three things `boba-app`
//! cannot: fetch a feed through the process-wide HTTP client (AD-10 — the
//! https-only SSRF guard, the per-host bucket and the honest User-Agent), read
//! the ports off the app handle (ADR-012), and assemble the built-in catalogue
//! from `boba-sources` (the core never depends on the adapters).
//!
//! What the user pastes is untrusted (AD-7): a feed URL is validated before it
//! is saved or fetched, and a posting URL that is not `https` never crosses IPC.

use boba_app::error::{AppError, AppResult, ErrorCode};
use boba_app::ports::clock::ClockPort;
use boba_app::ports::repo::sources::SourcesRepo;
use boba_app::ports::repo::system::SystemRepo;
use boba_app::sources::custom::{self, CustomFeedConfig};
use boba_app::sources::health::{self, SourceHealth, SourceStatus};
use boba_app::system::SystemClock;
use boba_domain::source::{SourceError, SourceManifest, SourceRecord};
use boba_sources::http::HttpClient;
use boba_sources::rss::{self, CustomFeed, FeedTest};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::wiring;

/// How many runs the tile's health fold reads (three failures plus context).
const HEALTH_WINDOW: usize = 5;

/// One Source tile on the Connections screen (AC1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct SourceTileView {
    /// Registry id and `source` primary key.
    pub id: String,
    pub name: String,
    /// Type shown on the tile: `api`, `ats_board`, `feed`, `session_board`,
    /// `manual` or `custom` (EXPERIENCE §9.7).
    pub source_type: String,
    /// The markets the source covers, as the manifest declares them.
    pub markets: Vec<String>,
    /// Risk tier: `safe`, `caution` or `prohibited_automation`.
    pub risk_tier: String,
    /// Whether the user added this source (`custom`).
    pub custom: bool,
    pub enabled: bool,
    /// Source health, folded from the newest runs (FR-31).
    pub status: SourceStatus,
    pub last_run: Option<String>,
    /// Postings found by the newest run (i32 keeps the binding numeric).
    pub offers_found: i32,
    pub last_error: Option<String>,
    pub consecutive_failures: u32,
    pub attribution: Option<String>,
}

/// One sampled posting in the add-feed preview (AC2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct RssSampleView {
    pub title: String,
    pub company: String,
    pub url: String,
    pub posted_at: Option<String>,
}

/// What the test fetch shows before the user saves (AC2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct RssTestView {
    /// The id the feed will be saved under (`rss:<slug>`).
    pub id: String,
    pub name: String,
    pub url: String,
    /// How many items the feed returned for this fetch.
    pub count: u32,
    /// Up to five items, for the preview.
    pub sample: Vec<RssSampleView>,
}

/// Every Source, with its health: built-in feeds are registered on the first
/// open (the same lazy seam Manual intake uses), then every row is read back.
#[tauri::command]
#[specta::specta]
pub async fn sources_catalogue(app: tauri::AppHandle) -> AppResult<Vec<SourceTileView>> {
    let sources = wiring::port::<dyn SourcesRepo>(&app)?;
    let system = wiring::port::<dyn SystemRepo>(&app)?;
    let now = SystemClock.now_rfc3339();

    let manifests: Vec<SourceManifest> = boba_sources::feeds::builtin()
        .iter()
        .map(|adapter| adapter.manifest())
        .collect();
    for manifest in &manifests {
        if sources
            .get_source(&manifest.id)
            .map_err(store_error)?
            .is_none()
        {
            sources
                .upsert_source(&record_from_manifest(manifest, &now))
                .map_err(store_error)?;
        }
    }

    // The Session Board Sources are seeded the same lazy way, so Connections
    // and Settings → Privacy can list them before the per-Source opt-in
    // (S11.1b, FR-30). Their rows stay disabled until that opt-in.
    for manifest in [
        boba_sources::session::linkedin::manifest(),
        boba_sources::session::indeed::manifest(),
    ] {
        if sources
            .get_source(&manifest.id)
            .map_err(store_error)?
            .is_none()
        {
            let mut record = record_from_manifest(&manifest, &now);
            record.enabled = false;
            sources.upsert_source(&record).map_err(store_error)?;
        }
    }

    let custom: Vec<CustomFeedConfig> =
        custom::list_custom_feeds(system.as_ref()).map_err(to_app)?;
    for feed in &custom {
        if sources.get_source(&feed.id).map_err(store_error)?.is_none() {
            sources
                .upsert_source(&custom_record(feed, &now))
                .map_err(store_error)?;
        }
    }

    let mut tiles = Vec::new();
    for record in sources.list_sources().map_err(store_error)? {
        let is_custom = custom.iter().any(|feed| feed.id == record.id);
        // A removed custom feed leaves its `source` row behind (no delete in
        // the sources owner); it is hidden rather than shown as a dead tile.
        if record.id.starts_with("rss:") && !is_custom {
            continue;
        }
        let manifest = manifests.iter().find(|manifest| manifest.id == record.id);
        let runs = sources
            .recent_runs(&record.id, HEALTH_WINDOW)
            .map_err(store_error)?;
        tiles.push(tile(&record, manifest, is_custom, &runs));
    }
    tiles.sort_by_key(|tile| tile.name.to_lowercase());
    Ok(tiles)
}

/// Fetch a pasted feed once and return its count and a sample (AC2). Nothing
/// is saved here: the screen saves separately once the user accepts the preview.
#[tauri::command]
#[specta::specta]
pub async fn sources_test_rss(
    app: tauri::AppHandle,
    url: String,
    name: Option<String>,
) -> AppResult<RssTestView> {
    let feed = custom_feed(&url, name.as_deref())?;
    let http = wiring::port::<HttpClient>(&app).map_err(|_| {
        AppError::new(ErrorCode::Unavailable, "errors.sources.fetch")
            .with_detail("reason", "the HTTP client is not mounted")
    })?;
    let probe = rss::test_fetch(&feed, &http).await.map_err(source_error)?;
    Ok(rss_view(probe))
}

/// Save a custom feed that the user previewed; re-adding a URL updates it.
#[tauri::command]
#[specta::specta]
pub async fn sources_add_rss(
    app: tauri::AppHandle,
    url: String,
    name: Option<String>,
) -> AppResult<SourceTileView> {
    let feed = custom_feed(&url, name.as_deref())?;
    let sources = wiring::port::<dyn SourcesRepo>(&app)?;
    let system = wiring::port::<dyn SystemRepo>(&app)?;
    let now = SystemClock.now_rfc3339();
    let config = CustomFeedConfig {
        id: feed.id.clone(),
        name: feed.name.clone(),
        url: feed.url.clone(),
        added_at: now.clone(),
    };
    custom::save_custom_feed(system.as_ref(), &config, &now).map_err(to_app)?;
    let record = custom_record(&config, &now);
    sources.upsert_source(&record).map_err(store_error)?;
    let runs = sources
        .recent_runs(&record.id, HEALTH_WINDOW)
        .map_err(store_error)?;
    Ok(tile(&record, None, true, &runs))
}

/// Remove a custom feed; `true` when it was saved.
#[tauri::command]
#[specta::specta]
pub async fn sources_remove_rss(app: tauri::AppHandle, id: String) -> AppResult<bool> {
    let system = wiring::port::<dyn SystemRepo>(&app)?;
    custom::remove_custom_feed(system.as_ref(), &id, &SystemClock.now_rfc3339()).map_err(to_app)
}

/// Validate the pasted URL and build the feed the probe and the save share.
fn custom_feed(url: &str, name: Option<&str>) -> AppResult<CustomFeed> {
    let url = rss::validate_url(url).map_err(source_error)?;
    let id = custom::feed_id(&url);
    let name = name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| default_feed_name(&url));
    CustomFeed::new(id, name, &url).map_err(source_error)
}

/// A readable name for a feed the user did not name: its host.
fn default_feed_name(url: &str) -> String {
    url.split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(url)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(url)
        .to_string()
}

fn record_from_manifest(manifest: &SourceManifest, now: &str) -> SourceRecord {
    SourceRecord {
        id: manifest.id.clone(),
        name: manifest.name.clone(),
        access_class: manifest.access_class,
        legal_tier: manifest.legal_tier,
        apply_capability: manifest.apply_capability,
        attribution: manifest.attribution.clone(),
        min_interval_ms: manifest.rate_limits.min_interval_ms,
        daily_cap: manifest.rate_limits.daily_cap,
        enabled: true,
        added_at: now.to_string(),
        updated_at: now.to_string(),
    }
}

fn custom_record(feed: &CustomFeedConfig, now: &str) -> SourceRecord {
    SourceRecord {
        id: feed.id.clone(),
        name: feed.name.clone(),
        access_class: boba_domain::source::AccessClass::Rss,
        legal_tier: boba_domain::source::LegalTier::Safe,
        apply_capability: boba_domain::source::ApplyCapability::LinkOut,
        attribution: None,
        min_interval_ms: boba_domain::source::RateLimits::default().min_interval_ms,
        daily_cap: None,
        enabled: true,
        added_at: feed.added_at.clone(),
        updated_at: now.to_string(),
    }
}

/// Fold one row and its history into the tile (AC1).
fn tile(
    record: &SourceRecord,
    manifest: Option<&SourceManifest>,
    custom: bool,
    runs: &[boba_domain::source::SourceRunRecord],
) -> SourceTileView {
    let health: SourceHealth = health::health_from_runs(runs);
    SourceTileView {
        id: record.id.clone(),
        name: record.name.clone(),
        source_type: source_type(record, custom).to_string(),
        markets: manifest.map(|m| m.markets.clone()).unwrap_or_default(),
        risk_tier: record.legal_tier.as_str().to_string(),
        custom,
        enabled: record.enabled,
        status: health.status,
        last_run: health.last_run,
        offers_found: i32::try_from(health.offers_found).unwrap_or(i32::MAX),
        last_error: health.last_error,
        consecutive_failures: health.consecutive_failures,
        attribution: record.attribution.clone(),
    }
}

/// The tile's Type (EXPERIENCE §9.7). The access class drives it; a custom
/// feed is `custom`, and the Manual intake keeps its own Type.
fn source_type(record: &SourceRecord, custom: bool) -> &'static str {
    if record.id == "manual" {
        return "manual";
    }
    if custom {
        return "custom";
    }
    match record.access_class {
        boba_domain::source::AccessClass::Rss => "feed",
        boba_domain::source::AccessClass::OfficialApi => "api",
        boba_domain::source::AccessClass::PublicJson => "ats_board",
        boba_domain::source::AccessClass::SessionBrowser => "session_board",
        boba_domain::source::AccessClass::HtmlScrape
        | boba_domain::source::AccessClass::ExternalProcess => "custom",
    }
}

/// What the preview shows: the probe's count and its sample, never more.
fn rss_view(probe: FeedTest) -> RssTestView {
    RssTestView {
        id: probe.feed.id,
        name: probe.feed.name,
        url: probe.feed.url,
        count: u32::try_from(probe.count).unwrap_or(u32::MAX),
        sample: probe
            .sample
            .into_iter()
            .map(|posting| RssSampleView {
                title: posting.title,
                company: posting.company,
                url: posting.url,
                posted_at: posting.posted_at,
            })
            .collect(),
    }
}

/// A feed failure as the screen shows it (AD-3): a bad URL is the user's input,
/// anything else is the fetch being unavailable right now.
struct FeedFailure(SourceError);

impl FeedFailure {
    fn code(&self) -> ErrorCode {
        match &self.0 {
            SourceError::InvalidValue { .. } | SourceError::InvalidPayload { .. } => {
                ErrorCode::InvalidInput
            }
            _ => ErrorCode::Unavailable,
        }
    }

    fn message_key(&self) -> &'static str {
        match &self.0 {
            // `=> "key"` on one line: the i18n gate reads keys from these arms.
            SourceError::InvalidValue { .. } => "errors.sources.invalid_url",
            SourceError::InvalidPayload { .. } => "errors.sources.invalid_url",
            _ => "errors.sources.fetch",
        }
    }
}

fn source_error(error: SourceError) -> AppError {
    let failure = FeedFailure(error);
    let reason = failure.0.to_string();
    // F2: a host that resolves inside the network has its own explanation.
    if matches!(&failure.0, SourceError::InvalidValue { field, .. } if field == "feed host") {
        return AppError::invalid_input("errors.sources.private_host")
            .with_detail("reason", reason);
    }
    AppError::new(failure.code(), failure.message_key()).with_detail("reason", reason)
}

fn store_error<E: std::fmt::Display>(error: E) -> AppError {
    AppError::new(ErrorCode::Internal, "errors.sources.store")
        .with_detail("reason", error.to_string())
}

fn to_app(error: boba_app::sources::SourcesError) -> AppError {
    match error {
        boba_app::sources::SourcesError::Invalid(_) => {
            AppError::new(ErrorCode::InvalidInput, error.message_key())
        }
        boba_app::sources::SourcesError::Store(_) => {
            AppError::new(ErrorCode::Internal, error.message_key())
        }
    }
}

#[cfg(test)]
mod tests {
    use boba_domain::source::{
        AccessClass, ApplyCapability, LegalTier, RunStatus, SourceRunRecord,
    };

    use super::*;

    fn record(id: &str, class: AccessClass) -> SourceRecord {
        SourceRecord {
            id: id.into(),
            name: id.into(),
            access_class: class,
            legal_tier: LegalTier::Safe,
            apply_capability: ApplyCapability::LinkOut,
            attribution: None,
            min_interval_ms: 2000,
            daily_cap: None,
            enabled: true,
            added_at: "2026-10-12T08:00:00Z".into(),
            updated_at: "2026-10-12T08:00:00Z".into(),
        }
    }

    fn run(id: &str, status: RunStatus, found: i64) -> SourceRunRecord {
        SourceRunRecord {
            id: id.into(),
            source_id: "remotive".into(),
            status,
            started_at: format!("2026-10-12T08:0{id}:00Z"),
            finished_at: Some(format!("2026-10-12T08:0{id}:30Z")),
            postings_found: found,
            error: None,
        }
    }

    #[test]
    fn each_access_class_maps_to_its_tile_type() {
        assert_eq!(source_type(&record("r", AccessClass::Rss), false), "feed");
        assert_eq!(
            source_type(&record("a", AccessClass::OfficialApi), false),
            "api"
        );
        assert_eq!(
            source_type(&record("greenhouse:x", AccessClass::PublicJson), false),
            "ats_board"
        );
        assert_eq!(
            source_type(&record("linkedin", AccessClass::SessionBrowser), false),
            "session_board"
        );
        assert_eq!(
            source_type(&record("manual", AccessClass::PublicJson), false),
            "manual"
        );
        assert_eq!(
            source_type(&record("rss:x", AccessClass::Rss), true),
            "custom"
        );
    }

    #[test]
    fn the_tile_carries_the_manifest_markets_and_risk_tier() {
        let manifest = SourceManifest {
            id: "remotive".into(),
            name: "Remotive".into(),
            access_class: AccessClass::PublicJson,
            legal_tier: LegalTier::Safe,
            markets: vec!["Global remote".into()],
            rate_limits: boba_domain::source::RateLimits::default(),
            attribution: Some("Jobs by Remotive".into()),
            apply_capability: ApplyCapability::LinkOut,
            required_secrets: Vec::new(),
            config_schema: None,
        };
        let mut stored = record("remotive", AccessClass::PublicJson);
        stored.attribution = manifest.attribution.clone();
        let history = [run("2", RunStatus::Ok, 9)];
        let view = tile(&stored, Some(&manifest), false, &history);
        assert_eq!(view.markets, vec!["Global remote".to_string()]);
        assert_eq!(view.risk_tier, "safe");
        assert_eq!(view.status, SourceStatus::Ok);
        assert_eq!(view.offers_found, 9);
        assert_eq!(view.attribution.as_deref(), Some("Jobs by Remotive"));
        assert!(!view.custom);
    }

    #[test]
    fn a_failing_source_tile_shows_the_streak_and_the_error() {
        let mut failing = record("remotive", AccessClass::PublicJson);
        failing.legal_tier = LegalTier::Caution;
        let mut newest = run("1", RunStatus::Failed, 0);
        newest.error = Some("status 503".into());
        let view = tile(
            &failing,
            None,
            false,
            &[
                newest,
                run("2", RunStatus::Failed, 0),
                run("3", RunStatus::Failed, 0),
            ],
        );
        assert_eq!(view.status, SourceStatus::Failing);
        assert_eq!(view.risk_tier, "caution");
        assert_eq!(view.consecutive_failures, 3);
        assert_eq!(view.last_error.as_deref(), Some("status 503"));
        assert!(view.markets.is_empty());
    }

    #[test]
    fn the_preview_keeps_the_probe_count_and_sample() {
        let probe = FeedTest {
            feed: CustomFeed::new(
                "rss:jobs-example-com-feed-xml",
                "Jobs",
                "https://jobs.example.com/feed.xml",
            )
            .unwrap(),
            count: 7,
            sample: vec![boba_domain::source::RawPosting {
                external_id: "1".into(),
                title: "Rust Engineer".into(),
                company: "Boba Labs".into(),
                url: "https://jobs.example.com/1".into(),
                locations: Vec::new(),
                remote: None,
                description_html: None,
                posted_at: Some("2026-10-12T08:00:00+00:00".into()),
                salary_text: None,
                raw: "{}".into(),
            }],
        };
        let view = rss_view(probe);
        assert_eq!(view.count, 7);
        assert_eq!(view.sample.len(), 1);
        assert_eq!(view.sample[0].title, "Rust Engineer");
        assert_eq!(view.sample[0].url, "https://jobs.example.com/1");
    }

    #[test]
    fn a_bad_url_is_invalid_input_and_a_fetch_failure_is_unavailable() {
        let invalid = source_error(SourceError::InvalidValue {
            field: "feed url".into(),
            value: "http://x".into(),
        });
        assert_eq!(invalid.code, ErrorCode::InvalidInput);
        assert_eq!(invalid.message_key, "errors.sources.invalid_url");

        let unavailable = source_error(SourceError::Http {
            url: "https://jobs.example.com/feed.xml".into(),
            message: "status 503".into(),
        });
        assert_eq!(unavailable.code, ErrorCode::Unavailable);
        assert_eq!(unavailable.message_key, "errors.sources.fetch");

        // F2: a name that resolves inside the network has its own copy.
        let private = source_error(SourceError::InvalidValue {
            field: "feed host".into(),
            value: "127.0.0.1".into(),
        });
        assert_eq!(private.code, ErrorCode::InvalidInput);
        assert_eq!(private.message_key, "errors.sources.private_host");
    }

    #[test]
    fn a_manifest_becomes_the_row_the_store_keeps() {
        let manifest = SourceManifest {
            id: "remotive".into(),
            name: "Remotive".into(),
            access_class: AccessClass::PublicJson,
            legal_tier: LegalTier::Safe,
            markets: vec![],
            rate_limits: boba_domain::source::RateLimits {
                min_interval_ms: 30_000,
                daily_cap: Some(4),
            },
            attribution: Some("Jobs by Remotive".into()),
            apply_capability: ApplyCapability::LinkOut,
            required_secrets: Vec::new(),
            config_schema: None,
        };
        let stored = record_from_manifest(&manifest, "2026-10-12T08:00:00Z");
        assert_eq!(stored.id, "remotive");
        assert_eq!(stored.min_interval_ms, 30_000);
        assert_eq!(stored.daily_cap, Some(4));
        assert!(stored.enabled);
    }

    #[test]
    fn an_unnamed_feed_is_named_after_its_host() {
        assert_eq!(
            default_feed_name("https://jobs.example.com/feed.xml?src=x"),
            "jobs.example.com"
        );
    }
}
