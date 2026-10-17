//! Indeed Session Board adapter tests (S11.1b, FR-30, AD-9, AD-10).

use boba_domain::source::{AccessClass, ApplyCapability, LegalTier, SearchQuery};

use super::*;
use crate::session::sdk::{BoardView, ChallengeKind, DEFAULT_PACING, PauseRecord};

const SEARCH: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/session/indeed_search.json"
));
const DETAIL: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/session/indeed_detail.json"
));
const RATE_LIMITED: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/session/indeed_rate_limited.json"
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
    assert_eq!(manifest.id, "indeed");
    assert_eq!(manifest.name, "Indeed");
    assert_eq!(manifest.access_class, AccessClass::SessionBrowser);
    // AC2: Indeed is in the same prohibited-automation family as Easy Apply.
    assert_eq!(manifest.legal_tier, LegalTier::ProhibitedAutomation);
    assert_eq!(manifest.apply_capability, ApplyCapability::SessionForm);
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
    assert_eq!(record.id, "indeed");
    assert_eq!(record.access_class, AccessClass::SessionBrowser);
    assert_eq!(record.legal_tier, LegalTier::ProhibitedAutomation);
    assert_eq!(record.apply_capability, ApplyCapability::SessionForm);
    assert_eq!(record.min_interval_ms, DEFAULT_PACING.min_interval_ms);
    assert_eq!(record.daily_cap, manifest().rate_limits.daily_cap);
    assert_eq!(record.added_at, NOW);
    assert_eq!(record.updated_at, NOW);
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
    view.pause = Some(PauseRecord::new(ChallengeKind::UnusualActivity, NOW));
    assert_eq!(
        check_ready(&view),
        Err(BoardError::Paused {
            source_id: SOURCE_ID.to_string(),
            kind: ChallengeKind::UnusualActivity,
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
    view.source_id = "linkedin".to_string();
    assert_eq!(
        check_ready(&view),
        Err(BoardError::WrongSource {
            expected: SOURCE_ID.to_string(),
            got: "linkedin".to_string(),
        })
    );
}

#[test]
fn the_search_url_encodes_the_query() {
    assert_eq!(
        search_url(&query()),
        "https://www.indeed.com/jobs?q=Rust%20Engineer%20backend&l=Madrid%2C%20Spain"
    );
}

#[test]
fn the_search_url_asks_for_remote_only_when_wanted() {
    let mut remote = query();
    remote.remote = Some(true);
    remote.locations.clear();
    assert_eq!(
        search_url(&remote),
        "https://www.indeed.com/jobs?q=Rust%20Engineer%20backend&l=Remote"
    );

    let empty = SearchQuery::default();
    assert_eq!(search_url(&empty), "https://www.indeed.com/jobs");
}

#[test]
fn a_detail_url_refuses_an_id_that_could_leave_the_viewjob_page() {
    assert_eq!(
        detail_url("abc123def4567890").unwrap(),
        "https://www.indeed.com/viewjob?jk=abc123def4567890"
    );
    for id in [
        "",
        "..",
        "abc/../login",
        "abc&jk=other",
        "abc def",
        "abcdefghijklmnopq",
    ] {
        assert!(detail_url(id).is_err(), "{id}");
    }
}

#[test]
fn the_read_scripts_only_read() {
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
    assert_eq!(postings.len(), 2, "unsafe URLs and bad ids drop");
    let first = &postings[0];
    assert_eq!(first.external_id, "abc123def4567890");
    assert_eq!(first.title, "Senior Rust Engineer");
    assert_eq!(first.company, "Boba Labs");
    assert_eq!(first.locations, vec!["Remote in United States".to_string()]);
    assert_eq!(first.remote, Some(true));
    assert_eq!(first.salary_text.as_deref(), Some("$150,000 a year"));
    assert_eq!(
        first.url,
        "https://www.indeed.com/viewjob?jk=abc123def4567890"
    );
    assert!(first.raw.contains("\"jk\":\"abc123def4567890\""));
    assert!(first.description_html.is_none(), "the list carries no body");

    let second = &postings[1];
    assert_eq!(second.external_id, "fedcba0987654321");
    assert_eq!(second.remote, None);
    assert!(second.salary_text.is_none());
}

#[test]
fn the_rate_limit_page_pauses_instead_of_reading() {
    assert_eq!(
        parse_search(RATE_LIMITED),
        Err(BoardError::Challenge(ChallengeKind::RateLimited))
    );
}

#[test]
fn an_unusual_activity_warning_is_its_own_challenge() {
    let page =
        r#"{"title":"Indeed","text":"We noticed unusual activity on your account","cards":[]}"#;
    assert_eq!(
        parse_search(page),
        Err(BoardError::Challenge(ChallengeKind::UnusualActivity))
    );
}

#[test]
fn challenge_words_on_a_page_with_results_do_not_pause_it() {
    // R5: signed-in pages keep a "Sign in" link and salaries contain "429".
    let page = r#"{"title":"Rust jobs","text":"Sign in. A role paying 142900. 429 jobs.","cards":[{"jk":"abc123def4567890","title":"Rust Engineer","company":"Boba Labs","location":"Remote","url":"https://www.indeed.com/viewjob?jk=abc123def4567890","snippet":"","salary":""}]}"#;
    assert_eq!(parse_search(page).unwrap().len(), 1);
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
    assert_eq!(posting.external_id, "abc123def4567890");
    assert_eq!(posting.title, "Senior Rust Engineer");
    assert_eq!(posting.company, "Boba Labs");
    assert_eq!(posting.salary_text.as_deref(), Some("$150,000 a year"));
    assert_eq!(
        posting.description_html.as_deref(),
        Some("<p>Own the ingestion pipeline. Rust, SQLite, no cloud.</p>")
    );
    assert!(posting.raw.contains("\"descriptionHtml\""));
}

#[test]
fn a_detail_page_that_is_a_challenge_is_not_a_posting() {
    let page = r#"{"title":"Sign in","text":"Sign in to continue","loginWall":true,"job":null}"#;
    assert_eq!(
        parse_detail(page),
        Err(BoardError::Challenge(ChallengeKind::LoginWall))
    );
}
