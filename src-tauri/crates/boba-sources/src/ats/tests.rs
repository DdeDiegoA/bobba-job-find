//! Detection and probe tests for the ATS adapters (S4.5).

use super::*;

fn detect_board(input: &str) -> BoardRef {
    detect(input).unwrap_or_else(|error| panic!("{input} must be recognised: {error}"))
}

#[test]
fn recognises_each_ats_from_a_public_careers_url() {
    let greenhouse = detect_board("https://boards.greenhouse.io/bobalabs");
    assert_eq!(greenhouse.kind, AtsKind::Greenhouse);
    assert_eq!(greenhouse.token, "bobalabs");
    assert!(!greenhouse.eu);

    let lever = detect_board("https://jobs.lever.co/bobalabs");
    assert_eq!(lever.kind, AtsKind::Lever);
    assert_eq!(lever.token, "bobalabs");
    assert!(!lever.eu);

    let ashby = detect_board("https://jobs.ashbyhq.com/bobalabs");
    assert_eq!(ashby.kind, AtsKind::Ashby);

    let workable = detect_board("https://apply.workable.com/bobalabs/");
    assert_eq!(workable.kind, AtsKind::Workable);
    assert_eq!(workable.token, "bobalabs");

    let sr = detect_board("https://jobs.smartrecruiters.com/Boba/7440000123456789");
    assert_eq!(sr.kind, AtsKind::SmartRecruiters);
    assert_eq!(sr.token, "Boba");
}

#[test]
fn recognises_the_lever_eu_host() {
    let board = detect_board("https://jobs.eu.lever.co/bobalabs");
    assert_eq!(board.kind, AtsKind::Lever);
    assert!(board.eu, "jobs.eu.lever.co must select api.eu.lever.co");
    assert_eq!(board.key(), "lever-eu");

    let api = detect_board("https://api.eu.lever.co/v0/postings/bobalabs?mode=json");
    assert!(api.eu);
}

#[test]
fn recognises_the_board_api_urls_users_paste() {
    let greenhouse =
        detect_board("https://boards-api.greenhouse.io/v1/boards/bobalabs/jobs?content=true");
    assert_eq!(greenhouse.kind, AtsKind::Greenhouse);
    assert_eq!(greenhouse.token, "bobalabs");

    let ashby = detect_board(
        "https://api.ashbyhq.com/posting-api/job-board/bobalabs?includeCompensation=true",
    );
    assert_eq!(ashby.kind, AtsKind::Ashby);
    assert_eq!(ashby.token, "bobalabs");

    let workable =
        detect_board("https://apply.workable.com/api/v1/widget/accounts/bobalabs?details=true");
    assert_eq!(workable.kind, AtsKind::Workable);
    assert_eq!(workable.token, "bobalabs");

    let sr = detect_board("https://api.smartrecruiters.com/v1/companies/Boba/postings");
    assert_eq!(sr.kind, AtsKind::SmartRecruiters);
    assert_eq!(sr.token, "Boba");
}

#[test]
fn recognises_a_workable_subdomain_board() {
    let board = detect_board("https://bobalabs.workable.com/");
    assert_eq!(board.kind, AtsKind::Workable);
    assert_eq!(board.token, "bobalabs");
}

#[test]
fn accepts_a_bare_token_when_the_ats_is_given() {
    let board = BoardRef::new(AtsKind::Greenhouse, "bobalabs").expect("valid token");
    assert_eq!(board.kind, AtsKind::Greenhouse);
    assert_eq!(board.source_id(), "greenhouse:bobalabs");

    let eu = BoardRef::lever_eu("bobalabs").expect("valid token");
    assert_eq!(eu.source_id(), "lever-eu:bobalabs");
}

#[test]
fn accepts_an_ats_prefixed_token() {
    let board = detect_board("greenhouse:bobalabs");
    assert_eq!(board.kind, AtsKind::Greenhouse);
    assert_eq!(board.token, "bobalabs");

    let eu = detect_board("lever-eu:bobalabs");
    assert_eq!(eu.kind, AtsKind::Lever);
    assert!(eu.eu);
}

#[test]
fn rejects_an_ambiguous_token_an_unknown_host_and_a_bad_token() {
    assert!(detect("bobalabs").is_err(), "a bare token needs its ATS");
    assert!(detect("https://example.com/careers").is_err());
    assert!(detect("https://boards.greenhouse.io/").is_err());
    assert!(detect("https://boards.greenhouse.io/acme/../evil").is_err());
    assert!(BoardRef::new(AtsKind::Lever, "has space").is_err());
    assert!(BoardRef::new(AtsKind::Lever, "").is_err());
}

#[test]
fn manifests_declare_the_apply_channel_from_the_addendum() {
    let form = [
        AtsKind::Greenhouse,
        AtsKind::Lever,
        AtsKind::Ashby,
        AtsKind::Workable,
    ];
    for kind in form {
        let manifest = BoardRef::new(kind, "bobalabs").unwrap().manifest();
        assert_eq!(
            manifest.apply_capability,
            ApplyCapability::AtsForm,
            "{kind:?} is filled in a browser"
        );
        assert_eq!(manifest.access_class, AccessClass::PublicJson);
        assert_eq!(manifest.legal_tier, LegalTier::Safe);
    }
    let manifest = BoardRef::new(AtsKind::SmartRecruiters, "Boba")
        .unwrap()
        .manifest();
    assert_eq!(manifest.apply_capability, ApplyCapability::LinkOut);
    assert_eq!(manifest.access_class, AccessClass::PublicJson);
}

#[test]
fn a_board_builds_the_source_record_the_save_persists() {
    let record = BoardRef::new(AtsKind::Greenhouse, "bobalabs")
        .unwrap()
        .record("2026-10-17T00:00:00Z");
    assert_eq!(record.id, "greenhouse:bobalabs");
    assert_eq!(record.access_class, AccessClass::PublicJson);
    assert_eq!(record.apply_capability, ApplyCapability::AtsForm);
    assert!(record.enabled);
    assert_eq!(record.added_at, "2026-10-17T00:00:00Z");
    assert_eq!(
        record.rate_limits().min_interval_ms,
        RateLimits::default().min_interval_ms
    );
}

#[tokio::test]
async fn the_add_board_probe_shows_a_count_a_sample_and_the_record() {
    use std::time::Duration;

    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::http::{HttpClient, HttpClientConfig};

    const LIST: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ats/greenhouse_jobs.json"
    ));

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(LIST, "application/json"))
        .mount(&server)
        .await;
    let http = HttpClient::new(HttpClientConfig {
        min_interval: Duration::from_millis(0),
        ..HttpClientConfig::default()
    })
    .expect("client");

    let board = BoardRef::new(AtsKind::Greenhouse, "bobalabs").unwrap();
    let adapter = greenhouse::Greenhouse::with_base(server.uri(), board.clone());
    let probe = probe(&adapter, &board, &http).await.unwrap();

    assert_eq!(
        probe.count, 2,
        "the preview shows how many jobs the board has"
    );
    assert_eq!(
        probe.sample.len(),
        2,
        "a small board fits whole in the sample"
    );
    assert_eq!(probe.board, board);
    let record = probe.source_record("2026-10-17T00:00:00Z");
    assert_eq!(record.id, "greenhouse:bobalabs");
    assert_eq!(record.apply_capability, ApplyCapability::AtsForm);
}
