//! LinkedIn Session Board adapter tests (S11.1b, FR-30, AD-9, AD-10).

use std::sync::{Arc, Mutex};

use boba_app::ports::session::SessionPages;
use boba_domain::source::{AccessClass, ApplyCapability, LegalTier, SearchQuery, SourceError};

use super::*;
use crate::http::{HttpClient, HttpClientConfig};
use crate::sdk::{SourceAdapter, SourceCtx};
use crate::session::sdk::{BoardView, ChallengeKind, DEFAULT_PACING, PauseRecord, RunControl};

const SEARCH: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/session/linkedin_search.json"
));
const DETAIL: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/session/linkedin_detail.json"
));
const CAPTCHA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/session/linkedin_captcha.json"
));
const NOW: &str = "2026-10-17T06:00:00.000Z";

fn board() -> BoardView {
    BoardView {
        source_id: SOURCE_ID.to_string(),
        name: NAME.to_string(),
        enabled: true,
        opted_in: true,
        accepted_at: Some(NOW.to_string()),
        accepted_warning_version: Some("v1".to_string()),
        pause: None,
        weak_ack: false,
    }
}

fn query() -> SearchQuery {
    SearchQuery {
        roles: vec!["Rust Engineer".to_string()],
        keywords: vec!["backend".to_string()],
        locations: vec!["Madrid, Spain".to_string()],
        remote: None,
        languages: vec![],
        markets: vec![],
    }
}

#[test]
fn the_manifest_declares_a_prohibited_session_board() {
    let manifest = manifest();
    assert_eq!(manifest.id, "linkedin");
    assert_eq!(manifest.name, "LinkedIn");
    assert_eq!(manifest.access_class, AccessClass::SessionBrowser);
    // AC2: the manifest says exactly how risky applying here is.
    assert_eq!(manifest.legal_tier, LegalTier::ProhibitedAutomation);
    assert_eq!(manifest.apply_capability, ApplyCapability::SessionForm);
    // The cookies live in Chrome's own profile, never in a manifest secret.
    assert!(manifest.required_secrets.is_empty());
}

#[test]
fn the_defaults_are_low_volume_and_human_like() {
    // AC1: the conservative pacing every Session Board Source starts from.
    let limits = manifest().rate_limits;
    assert_eq!(limits.min_interval_ms, DEFAULT_PACING.min_interval_ms);
    assert!(limits.min_interval_ms >= 30_000);
    let cap = limits.daily_cap.expect("a session board has a daily cap");
    assert!((1..=120).contains(&cap), "low volume, not a crawl: {cap}");
}

#[test]
fn the_record_carries_the_manifest_and_the_defaults() {
    let record = record(NOW);
    assert_eq!(record.id, "linkedin");
    assert_eq!(record.access_class, AccessClass::SessionBrowser);
    assert_eq!(record.legal_tier, LegalTier::ProhibitedAutomation);
    assert_eq!(record.apply_capability, ApplyCapability::SessionForm);
    assert_eq!(record.min_interval_ms, DEFAULT_PACING.min_interval_ms);
    assert_eq!(record.daily_cap, manifest().rate_limits.daily_cap);
    assert_eq!(record.added_at, NOW);
    assert_eq!(record.updated_at, NOW);
    assert!(record.enabled, "a configured board starts enabled");
}

#[test]
fn a_source_without_an_opt_in_cannot_run() {
    let mut view = board();
    view.opted_in = false;
    assert_eq!(
        check_ready(&view),
        Err(BoardError::OptInRequired {
            source_id: SOURCE_ID.to_string()
        })
    );
}

#[test]
fn a_paused_source_cannot_run() {
    let mut view = board();
    view.enabled = false;
    view.pause = Some(PauseRecord::new(ChallengeKind::Captcha, NOW));
    assert_eq!(
        check_ready(&view),
        Err(BoardError::Paused {
            source_id: SOURCE_ID.to_string(),
            kind: ChallengeKind::Captcha,
        })
    );
}

#[test]
fn a_disabled_source_cannot_run() {
    let mut view = board();
    view.enabled = false;
    assert_eq!(
        check_ready(&view),
        Err(BoardError::OptInRequired {
            source_id: SOURCE_ID.to_string()
        })
    );
}

#[test]
fn an_opted_in_source_runs() {
    assert_eq!(check_ready(&board()), Ok(()));
}

#[test]
fn another_board_cannot_borrow_this_adapter() {
    // A run that mixed the boards up would read the wrong site.
    let mut view = board();
    view.source_id = "indeed".to_string();
    assert_eq!(
        check_ready(&view),
        Err(BoardError::WrongSource {
            expected: SOURCE_ID.to_string(),
            got: "indeed".to_string(),
        })
    );
}

#[test]
fn the_search_url_encodes_the_query() {
    let url = search_url(&query());
    assert_eq!(
        url,
        "https://www.linkedin.com/jobs/search/?keywords=Rust%20Engineer%20backend&location=Madrid%2C%20Spain"
    );
}

#[test]
fn the_search_url_asks_for_remote_only_when_wanted() {
    let mut remote = query();
    remote.remote = Some(true);
    let url = search_url(&remote);
    assert!(url.contains("&f_WT=2"), "remote filter: {url}");
    assert!(!search_url(&query()).contains("f_WT"), "not by default");

    let empty = SearchQuery {
        remote: Some(true),
        ..SearchQuery::default()
    };
    assert_eq!(
        search_url(&empty),
        "https://www.linkedin.com/jobs/search/?f_WT=2"
    );
}

#[test]
fn a_detail_url_refuses_an_id_that_is_not_a_job_number() {
    assert_eq!(
        detail_url("4123456789").unwrap(),
        "https://www.linkedin.com/jobs/view/4123456789/"
    );
    for id in ["", "..", "4123/../authwall", "4123?x=1", "abc"] {
        assert!(detail_url(id).is_err(), "{id}");
    }
}

#[test]
fn the_read_scripts_only_read() {
    // AD-10: the page side of a session board never clicks, types, submits or
    // calls the network on its own.
    for script in [READ_SEARCH_JS, READ_DETAIL_JS] {
        for forbidden in [
            ".click(",
            ".submit(",
            "fetch(",
            "XMLHttpRequest",
            "sendBeacon",
            ".value",
            "document.cookie",
        ] {
            assert!(!script.contains(forbidden), "found {forbidden}");
        }
    }
}

#[test]
fn a_search_page_becomes_postings() {
    let postings = parse_search(SEARCH).unwrap();
    assert_eq!(
        postings.len(),
        2,
        "unsafe URLs, empty titles and bad ids drop"
    );
    let first = &postings[0];
    assert_eq!(first.external_id, "4123456789");
    assert_eq!(first.title, "Senior Rust Engineer");
    assert_eq!(first.company, "Boba Labs");
    assert_eq!(first.locations, vec!["Remote".to_string()]);
    assert_eq!(first.remote, Some(true));
    assert_eq!(
        first.url,
        "https://www.linkedin.com/jobs/view/4123456789/?refId=abc123"
    );
    assert!(first.raw.contains("\"id\":\"4123456789\""));
    assert!(first.description_html.is_none(), "the list carries no body");

    let second = &postings[1];
    assert_eq!(second.external_id, "4123456790");
    assert_eq!(second.remote, None, "an office location is not remote");
}

#[test]
fn a_captcha_page_is_a_challenge_not_postings() {
    assert_eq!(
        parse_search(CAPTCHA),
        Err(BoardError::Challenge(ChallengeKind::Captcha))
    );
}

#[test]
fn a_sign_in_wall_is_a_challenge() {
    let page = r#"{"url":"https://www.linkedin.com/authwall","title":"Sign in","text":"Sign in to continue","cards":[]}"#;
    assert_eq!(
        parse_search(page),
        Err(BoardError::Challenge(ChallengeKind::LoginWall))
    );
}

#[test]
fn challenge_words_on_a_page_with_results_do_not_pause_it() {
    // R5: "sign in" and "429" occur in real page chrome and salaries, so a
    // page that did return results is not a challenge page.
    let page = r#"{"title":"Rust jobs","text":"Sign in. A role paying 142900. 429 jobs.","cards":[{"id":"4123456789","title":"Rust Engineer","company":"Boba Labs","location":"Remote","url":"https://www.linkedin.com/jobs/view/4123456789/","posted":""}]}"#;
    let postings = parse_search(page).unwrap();
    assert_eq!(postings.len(), 1);
}

#[test]
fn a_rate_limit_on_an_empty_page_is_a_challenge() {
    let page = r#"{"title":"Access denied","text":"Too many requests","cards":[]}"#;
    assert_eq!(
        parse_search(page),
        Err(BoardError::Challenge(ChallengeKind::RateLimited))
    );
}

#[test]
fn an_unreadable_page_is_malformed() {
    assert!(matches!(
        parse_search("not json"),
        Err(BoardError::Malformed(_))
    ));
}

#[test]
fn a_detail_page_carries_its_description() {
    let posting = parse_detail(DETAIL).unwrap();
    assert_eq!(posting.external_id, "4123456789");
    assert_eq!(posting.title, "Senior Rust Engineer");
    assert_eq!(posting.company, "Boba Labs");
    assert_eq!(
        posting.url,
        "https://www.linkedin.com/jobs/view/4123456789/"
    );
    assert_eq!(
        posting.description_html.as_deref(),
        Some("<p>Build the core of a local-first job search app in Rust.</p>")
    );
    assert!(posting.raw.contains("\"descriptionHtml\""));
}

#[test]
fn a_detail_page_that_is_a_challenge_is_not_a_posting() {
    let page = r#"{"title":"Security check","text":"Please verify you are human","captcha":true,"job":null}"#;
    assert_eq!(
        parse_detail(page),
        Err(BoardError::Challenge(ChallengeKind::Captcha))
    );
}

// --- the run Source: pages go through the session, and the gate holds -------

/// The user's session, as the adapter sees it: the JSON a page answers with,
/// and every page it was asked to open.
struct FakePages {
    json: String,
    reads: Mutex<Vec<(String, String)>>,
}

impl FakePages {
    fn answering(json: &str) -> Arc<Self> {
        Arc::new(Self {
            json: json.to_string(),
            reads: Mutex::new(Vec::new()),
        })
    }

    fn opened(&self) -> Vec<(String, String)> {
        self.reads.lock().unwrap().clone()
    }
}

impl SessionPages for FakePages {
    fn read(&self, source_id: &str, url: &str, _script: &str) -> Result<String, String> {
        self.reads
            .lock()
            .unwrap()
            .push((source_id.to_string(), url.to_string()));
        Ok(self.json.clone())
    }

    fn close(&self) -> Result<(), String> {
        Ok(())
    }
}

/// The core, as the adapter sees it: one board, today's counter, the audit and
/// the pause.
struct FakeCore {
    board: BoardView,
    used_today: u32,
    loads: Mutex<Vec<u32>>,
    paused: Mutex<Vec<ChallengeKind>>,
}

impl FakeCore {
    fn with(board: BoardView) -> Arc<Self> {
        Arc::new(Self {
            board,
            used_today: 0,
            loads: Mutex::new(Vec::new()),
            paused: Mutex::new(Vec::new()),
        })
    }
}

impl RunControl for FakeCore {
    fn board(&self, _source_id: &str) -> Result<(BoardView, u32), String> {
        Ok((self.board.clone(), self.used_today))
    }

    fn page_loaded(&self, _source_id: &str, _url: &str, loads_today: u32) -> Result<(), String> {
        self.loads.lock().unwrap().push(loads_today);
        Ok(())
    }

    fn pause(&self, _source_id: &str, kind: ChallengeKind) -> Result<(), String> {
        self.paused.lock().unwrap().push(kind);
        Ok(())
    }
}

fn http() -> HttpClient {
    HttpClient::new(HttpClientConfig {
        min_interval: std::time::Duration::from_millis(0),
        ..HttpClientConfig::default()
    })
    .expect("client")
}

#[tokio::test(start_paused = true)]
async fn a_search_reads_the_page_in_the_users_session() {
    let pages = FakePages::answering(SEARCH);
    let core = FakeCore::with(board());
    let http = http();
    let ctx = SourceCtx {
        http: &http,
        cancel: tokio_util::sync::CancellationToken::new(),
    };
    let adapter = LinkedIn::new(pages.clone(), core.clone());

    let postings = adapter.search(&query(), &ctx).await.unwrap();

    assert_eq!(postings.len(), 2);
    assert_eq!(
        pages.opened(),
        vec![(
            "linkedin".to_string(),
            "https://www.linkedin.com/jobs/search/?keywords=Rust%20Engineer%20backend&location=Madrid%2C%20Spain"
                .to_string()
        )],
        "the page is the query's search page"
    );
    assert_eq!(*core.loads.lock().unwrap(), vec![1], "the load is audited");
}

#[tokio::test(start_paused = true)]
async fn a_detail_reads_the_canonical_page() {
    let pages = FakePages::answering(DETAIL);
    let core = FakeCore::with(board());
    let http = http();
    let ctx = SourceCtx {
        http: &http,
        cancel: tokio_util::sync::CancellationToken::new(),
    };
    let adapter = LinkedIn::new(pages.clone(), core);

    let posting = adapter.detail("4123456789", &ctx).await.unwrap();

    assert_eq!(posting.external_id, "4123456789");
    assert_eq!(
        pages.opened()[0].1,
        "https://www.linkedin.com/jobs/view/4123456789/"
    );
}

#[tokio::test(start_paused = true)]
async fn a_challenge_pauses_the_source_and_yields_no_postings() {
    let pages = FakePages::answering(CAPTCHA);
    let core = FakeCore::with(board());
    let http = http();
    let ctx = SourceCtx {
        http: &http,
        cancel: tokio_util::sync::CancellationToken::new(),
    };
    let adapter = LinkedIn::new(pages.clone(), core.clone());

    let error = adapter.search(&query(), &ctx).await.unwrap_err();

    assert_eq!(
        error,
        SourceError::Paused {
            source_id: "linkedin".to_string(),
            kind: "captcha".to_string(),
        }
    );
    assert_eq!(*core.paused.lock().unwrap(), vec![ChallengeKind::Captcha]);
}

#[tokio::test(start_paused = true)]
async fn without_an_opt_in_no_page_is_read() {
    let pages = FakePages::answering(SEARCH);
    let mut view = board();
    view.opted_in = false;
    let core = FakeCore::with(view);
    let http = http();
    let ctx = SourceCtx {
        http: &http,
        cancel: tokio_util::sync::CancellationToken::new(),
    };
    let adapter = LinkedIn::new(pages.clone(), core.clone());

    let error = adapter.search(&query(), &ctx).await.unwrap_err();

    assert_eq!(
        error,
        SourceError::OptInRequired {
            source_id: "linkedin".to_string()
        }
    );
    assert!(pages.opened().is_empty(), "the session was never touched");
    assert!(core.loads.lock().unwrap().is_empty());
}
