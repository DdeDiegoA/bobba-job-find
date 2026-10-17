//! LinkedIn Session Board adapter tests (S11.1b, FR-30, AD-9, AD-10).

use boba_domain::source::{AccessClass, ApplyCapability, LegalTier, SearchQuery};

use super::*;
use crate::session::sdk::{BoardView, ChallengeKind, DEFAULT_PACING, PauseRecord};

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
