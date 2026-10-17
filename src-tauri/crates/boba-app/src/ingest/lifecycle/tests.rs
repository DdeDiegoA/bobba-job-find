//! Use-case tests for S4.4 (FR-50, FR-51, FR-52).
//!
//! The store fake mirrors the real adapter's lifecycle maintenance (a row per
//! posting on ingest, repost and reopen on the next observation), so these tests
//! drive the same states SQLite would.

use super::*;
use crate::ingest::dedup;
use crate::ingest::normalize::testing::FakeIngestionRepo;
use boba_domain::posting::{ApplyHints, Money, PayPeriod, Provenance, WorkMode};
use boba_domain::source::RawPosting;

/// The day the first run happens; every other instant is relative to it.
const DAY_ONE: &str = "2026-09-01T00:00:00Z";

const SOURCE: &str = "remotive";

fn raw(external_id: &str, title: &str, posted_at: Option<&str>) -> RawPosting {
    RawPosting {
        external_id: external_id.into(),
        title: title.into(),
        company: "Boba Labs".into(),
        url: format!("https://jobs.example.com/{external_id}"),
        locations: vec!["Remote".into()],
        remote: Some(true),
        description_html: Some("<p>Build APIs in Rust and PostgreSQL.</p>".into()),
        posted_at: posted_at.map(str::to_string),
        salary_text: None,
        raw: format!(r#"{{"id":"{external_id}"}}"#),
    }
}

fn ingest(repo: &FakeIngestionRepo, posting: &RawPosting, now: &str) -> String {
    let outcome = dedup::ingest_deduped(repo, posting, SOURCE, now).expect("ingested");
    outcome.posting_id().to_string()
}

fn window(started_at: &str, ended_at: &str) -> RunWindow {
    RunWindow {
        started_at: started_at.to_string(),
        ended_at: ended_at.to_string(),
        source_ids: vec![SOURCE.to_string()],
    }
}

/// An instant `days` after `2026-09-01T00:00:00Z`, in UTC RFC 3339.
fn day(days: i64) -> String {
    use time::format_description::well_known::Rfc3339;
    let base = time::OffsetDateTime::parse(DAY_ONE, &Rfc3339).expect("DAY_ONE parses");
    (base + time::Duration::days(days))
        .format(&Rfc3339)
        .expect("formats")
}

fn provenance(posting_id: &str, now: &str) -> Provenance {
    Provenance {
        posting_id: posting_id.to_string(),
        source_id: SOURCE.to_string(),
        external_id: "a1".into(),
        url: "https://jobs.example.com/a1".into(),
        raw: "{}".into(),
        fetched_at: now.to_string(),
    }
}

fn offer(posting_id: &str) -> Offer {
    Offer {
        id: posting_id.to_string(),
        source_id: SOURCE.into(),
        external_id: "a1".into(),
        title: "Backend Engineer".into(),
        company: "Boba Labs".into(),
        locations: vec!["Remote".into()],
        work_mode: WorkMode::Remote,
        money: Some(Money {
            amount_minor: 5_000_000,
            currency: "USD".into(),
            period: PayPeriod::Year,
        }),
        posted_at: Some(DAY_ONE.into()),
        deadline: None,
        first_seen: DAY_ONE.into(),
        last_seen: DAY_ONE.into(),
        lang: "en".into(),
        description_html: "<p>Build APIs.</p>".into(),
        description_text: "Build APIs in Rust and PostgreSQL.".into(),
        requirements: Vec::new(),
        apply: ApplyHints {
            url: "https://jobs.example.com/a1".into(),
            email: None,
        },
    }
}

// --- AC1: seen state --------------------------------------------------------

/// FR-50, AC1: a posting Boba has never shown the user is worth a
/// notification; once seen unchanged, it is not.
#[test]
fn a_seen_offer_is_not_notified_again_without_material_change() {
    let repo = FakeIngestionRepo::default();
    let posting = raw("a1", "Backend Engineer", None);
    let id = ingest(&repo, &posting, DAY_ONE);

    assert!(
        should_notify(&repo, &id).unwrap(),
        "never seen before: news"
    );

    mark_seen(&repo, &id, DAY_ONE).unwrap();
    assert!(!should_notify(&repo, &id).unwrap());

    // The same board re-sends the same job tomorrow: one posting, no news.
    let id_again = ingest(&repo, &posting, &day(1));
    assert_eq!(id_again, id, "the same job is one posting");
    assert!(!should_notify(&repo, &id).unwrap(), "unchanged: no news");
}

/// FR-50: a dismissal holds through a re-send.
#[test]
fn a_dismissed_offer_stays_dismissed() {
    let repo = FakeIngestionRepo::default();
    let posting = raw("a1", "Backend Engineer", None);
    let id = ingest(&repo, &posting, DAY_ONE);

    mark_dismissed(&repo, &id, DAY_ONE).unwrap();
    ingest(&repo, &posting, &day(2));

    assert!(!should_notify(&repo, &id).unwrap());
    let row = repo.lifecycle(&id).unwrap().expect("tracked");
    assert_eq!(row.dismissed_at.as_deref(), Some(DAY_ONE));
    assert_eq!(
        row.seen_at.as_deref(),
        Some(DAY_ONE),
        "dismissed implies seen"
    );
}

/// FR-50: a posting whose content materially changed is news again, even to a
/// user who dismissed it.
#[test]
fn a_material_change_brings_a_dismissed_offer_back() {
    let repo = FakeIngestionRepo::default();
    let posting = raw("a1", "Backend Engineer", None);
    let id = ingest(&repo, &posting, DAY_ONE);
    mark_dismissed(&repo, &id, DAY_ONE).unwrap();

    // The store now holds a different role under the same posting (the source
    // re-used its identifier): title and pay both moved.
    let mut changed = offer(&id);
    changed.title = "Staff Backend Engineer".into();
    changed.money.as_mut().unwrap().amount_minor = 8_000_000;
    repo.insert_offer(&changed, &provenance(&id, &day(3)))
        .expect("stored");

    assert!(should_notify(&repo, &id).unwrap());
}

/// A posting Boba has no lifecycle row for is one it has never shown.
#[test]
fn an_untracked_posting_is_notified() {
    let repo = FakeIngestionRepo::default();
    let id = "pst_01UNTRACKED";
    repo.insert_offer(&offer(id), &provenance(id, DAY_ONE))
        .expect("stored");

    assert!(should_notify(&repo, id).unwrap());
}

#[test]
fn a_posting_that_is_not_in_the_store_is_an_error() {
    let repo = FakeIngestionRepo::default();
    let error = mark_seen(&repo, "pst_01MISSING", DAY_ONE).expect_err("refused");

    assert_eq!(error.message_key(), "errors.lifecycle.missing_posting");
}

// --- AC2: staleness and repost ---------------------------------------------

/// FR-51, AC2: ingesting a posting more than 90 days old records the staleness
/// signal for Q4.
#[test]
fn an_old_posting_is_flagged_stale_on_ingest() {
    let repo = FakeIngestionRepo::default();
    let posting = raw("a1", "Backend Engineer", Some(&day(-100)));
    let id = ingest(&repo, &posting, DAY_ONE);

    assert_eq!(
        signals(&repo, &id).unwrap(),
        vec![LifecycleSignal::Stale {
            since: DAY_ONE.into()
        }]
    );
}

#[test]
fn a_fresh_posting_carries_no_signal() {
    let repo = FakeIngestionRepo::default();
    let posting = raw("a1", "Backend Engineer", Some(&day(-3)));
    let id = ingest(&repo, &posting, DAY_ONE);

    assert_eq!(signals(&repo, &id).unwrap(), Vec::new());
}

/// FR-51, AC2: gone for a week and back is a repost.
#[test]
fn coming_back_within_thirty_days_is_a_repost_signal() {
    let repo = FakeIngestionRepo::default();
    let posting = raw("a1", "Backend Engineer", None);
    let id = ingest(&repo, &posting, DAY_ONE);

    // Two runs that do not see it: it disappeared on the first.
    finish_run(&repo, &window(&day(2), &day(2))).unwrap();
    finish_run(&repo, &window(&day(5), &day(5))).unwrap();

    // Back a week after disappearing.
    let id_again = ingest(&repo, &posting, &day(12));
    assert_eq!(id_again, id);

    assert_eq!(
        signals(&repo, &id).unwrap(),
        vec![LifecycleSignal::Repost { at: day(12) }]
    );
    let row = repo.lifecycle(&id).unwrap().expect("tracked");
    assert_eq!(row.disappeared_at, None, "it is present again");
    assert!(!row.is_possibly_closed(), "a repost is not a closure");
}

/// FR-51: back after the window is a new listing, not a repost.
#[test]
fn coming_back_after_thirty_days_is_not_a_repost() {
    let repo = FakeIngestionRepo::default();
    let posting = raw("a1", "Backend Engineer", None);
    let id = ingest(&repo, &posting, DAY_ONE);

    finish_run(&repo, &window(&day(2), &day(2))).unwrap();
    ingest(&repo, &posting, &day(45));

    assert_eq!(signals(&repo, &id).unwrap(), Vec::new());
}

// --- AC3: expiry ------------------------------------------------------------

/// FR-52, AC3: missing for two consecutive runs marks it *Possibly closed* and
/// warns the Applications saved against it.
#[test]
fn two_missing_runs_mark_possibly_closed_and_warn_saved_applications() {
    let repo = FakeIngestionRepo::default();
    let posting = raw("a1", "Backend Engineer", None);
    let id = ingest(&repo, &posting, DAY_ONE);
    repo.add_saved_application(&id, "app_01");

    let first = finish_run(&repo, &window(&day(2), &day(2))).unwrap();
    assert_eq!(first.possibly_closed, Vec::<String>::new(), "one miss");
    let row = repo.lifecycle(&id).unwrap().expect("tracked");
    assert_eq!(row.missing_runs, 1);
    assert_eq!(row.disappeared_at.as_deref(), Some(day(2).as_str()));

    let second = finish_run(&repo, &window(&day(3), &day(3))).unwrap();
    assert_eq!(second.possibly_closed, vec![id.clone()]);
    assert_eq!(
        second.warnings,
        vec![ClosedWarning {
            posting_id: id.clone(),
            application_ids: vec!["app_01".into()],
        }]
    );
    assert_eq!(closed_warnings(&repo).unwrap(), second.warnings);

    let row = repo.lifecycle(&id).unwrap().expect("tracked");
    assert_eq!(row.possibly_closed_at.as_deref(), Some(day(3).as_str()));
}

/// A single miss followed by a sighting is not a closure.
#[test]
fn a_single_miss_is_not_a_closure() {
    let repo = FakeIngestionRepo::default();
    let posting = raw("a1", "Backend Engineer", None);
    let id = ingest(&repo, &posting, DAY_ONE);

    finish_run(&repo, &window(&day(2), &day(2))).unwrap();
    ingest(&repo, &posting, &day(3));

    let row = repo.lifecycle(&id).unwrap().expect("tracked");
    assert_eq!(row.missing_runs, 0, "seen again resets the count");
    assert!(!row.is_possibly_closed());
}

/// FR-52: a deadline that has passed closes the posting even while a Source
/// still lists it.
#[test]
fn a_posting_past_its_deadline_is_closed_even_when_seen() {
    let repo = FakeIngestionRepo::default();
    let id = "pst_01EXPIRED";
    let mut expired = offer(id);
    expired.deadline = Some(day(1));
    expired.last_seen = day(5);
    repo.insert_offer(&expired, &provenance(id, &day(5)))
        .expect("stored");

    let outcome = finish_run(&repo, &window(&day(5), &day(5))).unwrap();

    assert_eq!(outcome.possibly_closed, vec![id.to_string()]);
    let row = repo.lifecycle(id).unwrap().expect("tracked");
    assert_eq!(row.possibly_closed_at.as_deref(), Some(day(5).as_str()));
}

/// A posting past its deadline stays closed: a re-sighting does not reopen it,
/// and the next run neither reports it as newly closed nor moves its instant.
#[test]
fn a_deadline_closure_is_not_reopened_by_the_source_listing_it() {
    let repo = FakeIngestionRepo::default();
    let id = "pst_01EXPIRED";
    let mut expired = offer(id);
    expired.deadline = Some(day(1));
    expired.last_seen = day(5);
    repo.insert_offer(&expired, &provenance(id, &day(5)))
        .expect("stored");
    let first = finish_run(&repo, &window(&day(5), &day(5))).unwrap();
    assert_eq!(first.possibly_closed, vec![id.to_string()]);

    // The source lists it again: a re-sighting, not a new posting.
    repo.merge_provenance(id, &provenance(id, &day(6)), &day(6))
        .expect("re-seen");
    let outcome = finish_run(&repo, &window(&day(6), &day(6))).unwrap();

    assert_eq!(outcome.possibly_closed, Vec::<String>::new());
    let row = repo.lifecycle(id).unwrap().expect("tracked");
    assert!(row.is_possibly_closed());
    assert_eq!(
        row.possibly_closed_at.as_deref(),
        Some(day(5).as_str()),
        "the closure keeps the instant it was marked at"
    );
}

/// A posting that comes back after a closure is open again, and keeps the
/// repost the store recorded.
#[test]
fn a_reopened_posting_is_no_longer_closed() {
    let repo = FakeIngestionRepo::default();
    let posting = raw("a1", "Backend Engineer", None);
    let id = ingest(&repo, &posting, DAY_ONE);

    finish_run(&repo, &window(&day(2), &day(2))).unwrap();
    finish_run(&repo, &window(&day(3), &day(3))).unwrap();
    let closed = repo.lifecycle(&id).unwrap().expect("tracked");
    assert!(closed.is_possibly_closed(), "two misses close it");

    ingest(&repo, &posting, &day(4));

    let row = repo.lifecycle(&id).unwrap().expect("tracked");
    assert!(!row.is_possibly_closed());
    assert_eq!(row.missing_runs, 0);
    assert_eq!(row.repost_at.as_deref(), Some(day(4).as_str()));
    assert_eq!(closed_warnings(&repo).unwrap(), Vec::new());
}

/// A source the run did not cover says nothing about its postings.
#[test]
fn postings_of_untouched_sources_are_left_alone() {
    let repo = FakeIngestionRepo::default();
    let posting = raw("a1", "Backend Engineer", None);
    let id = ingest(&repo, &posting, DAY_ONE);

    let mut other = window(&day(2), &day(2));
    other.source_ids = vec!["himalayas".into()];
    let outcome = finish_run(&repo, &other).unwrap();

    assert_eq!(outcome, RunOutcome::default());
    let row = repo.lifecycle(&id).unwrap().expect("tracked");
    assert_eq!(row.missing_runs, 0);
}

/// A posting that sat past 90 days while still listed gains the staleness
/// signal at the run that notices it.
#[test]
fn a_posting_that_ages_while_listed_becomes_stale_at_run_end() {
    let repo = FakeIngestionRepo::default();
    let posting = raw("a1", "Backend Engineer", Some(DAY_ONE));
    let id = ingest(&repo, &posting, DAY_ONE);

    // The board keeps listing it; on the 95th day it is still the same posting.
    let id_again = ingest(&repo, &posting, &day(95));
    assert_eq!(id_again, id);

    let outcome = finish_run(&repo, &window(&day(95), &day(95))).unwrap();
    assert_eq!(outcome.stale, vec![id.clone()]);
    assert!(signals(&repo, &id).unwrap().iter().any(|signal| matches!(
        signal,
        LifecycleSignal::Stale { since } if since == &day(95)
    )));
}
