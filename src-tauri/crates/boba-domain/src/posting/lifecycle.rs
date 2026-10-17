//! Posting lifecycle: seen and dismissed memory, repost and staleness signals,
//! and expiry (S4.4, FR-50, FR-51, FR-52).
//!
//! A posting is not a row that appears once and stays still: a board may re-send
//! the same job every day, drop it for a week and list it again, or leave a
//! dead opening up for months. This module holds the pure policy the app and the
//! store apply to one posting's [`PostingLifecycle`] row — no IO, no clock.
//!
//! - **Seen state (FR-50).** The user is told about a posting once. Seeing or
//!   dismissing it records a fingerprint of what they saw; the same posting
//!   coming back with the same fingerprint is not news, and one whose title,
//!   pay or text materially changed is.
//! - **Repost and staleness (FR-51).** A posting over [`STALE_AFTER_DAYS`] days
//!   old, or one that came back within [`REPOST_WINDOW_DAYS`] of disappearing,
//!   carries a signal. Both feed the Q4 legitimacy factor
//!   (`scoring/factors/legitimacy.rs`); nothing is deleted.
//! - **Expiry (FR-52).** Missing from its Source for
//!   [`MISSING_RUNS_BEFORE_CLOSED`] consecutive runs, or past its deadline, a
//!   posting is marked *Possibly closed*. The Applications saved against it are
//!   warned; the posting stays readable.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::dedup::{description_simhash, similarity};
use crate::posting::Offer;

/// Days a posting may sit before its age is the staleness signal (FR-51,
/// `offer-scoring.md` §5).
pub const STALE_AFTER_DAYS: i64 = 90;

/// How soon after disappearing a posting that comes back counts as a repost
/// (FR-51).
pub const REPOST_WINDOW_DAYS: i64 = 30;

/// Consecutive runs a posting must be missing from its Source before it is
/// marked *Possibly closed* (FR-52).
pub const MISSING_RUNS_BEFORE_CLOSED: u32 = 2;

/// Below this description similarity, a posting is saying something else
/// (FR-50's "materially"). It is deliberately far below dedup's 0.9 "same job"
/// bar: re-sending a posting with a sentence added or a paragraph trimmed is
/// not news, while a description rewritten for another role is. Measured on
/// the fixture text: one added sentence 0.89, one added word 0.88, an unrelated
/// description 0.44.
pub const DESCRIPTION_CHANGE_SIMILARITY: f64 = 0.6;

/// One posting's lifecycle, as the store keeps it (S4.4).
///
/// Every timestamp is UTC RFC 3339 (AD-17). `content_hash` and
/// `description_simhash` together are what the user was last told about: the
/// fingerprint is refreshed when they see or dismiss the posting, not when a
/// source merely re-sends it (FR-50).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostingLifecycle {
    pub posting_id: String,
    /// Fingerprint of the Offer's fields, [`material_hash`].
    pub content_hash: String,
    /// The description simhash at that same moment, when it was comparable.
    pub description_simhash: Option<u64>,
    /// When the user first saw it in the inbox; `None` until then (FR-50).
    pub seen_at: Option<String>,
    /// When the user dismissed it; `None` unless dismissed (FR-50).
    pub dismissed_at: Option<String>,
    /// Consecutive runs that did not see it (FR-52).
    pub missing_runs: u32,
    /// When it first stopped appearing in its Source, cleared on reappearance.
    pub disappeared_at: Option<String>,
    /// When it came back within the repost window (FR-51); a Q4 signal.
    pub repost_at: Option<String>,
    /// When its age passed the staleness threshold (FR-51); a Q4 signal.
    pub stale_at: Option<String>,
    /// When it was marked *Possibly closed* (FR-52).
    pub possibly_closed_at: Option<String>,
    /// When the row last changed.
    pub updated_at: String,
}

impl PostingLifecycle {
    /// The row a freshly ingested posting starts with: nothing seen, nothing
    /// missing, and the staleness check already run (a source can publish an
    /// old posting on the day it is first ingested).
    pub fn new(posting_id: &str, offer: &Offer, now: &str) -> Self {
        Self {
            posting_id: posting_id.to_string(),
            content_hash: material_hash(offer),
            description_simhash: description_simhash(&offer.description_text),
            seen_at: None,
            dismissed_at: None,
            missing_runs: 0,
            disappeared_at: None,
            repost_at: None,
            stale_at: stale_since(offer, now),
            possibly_closed_at: None,
            updated_at: now.to_string(),
        }
    }

    /// The Q4 signals this posting carries (FR-51). Order is stable: staleness
    /// first, then repost.
    pub fn signals(&self) -> Vec<LifecycleSignal> {
        let mut signals = Vec::new();
        if let Some(since) = &self.stale_at {
            signals.push(LifecycleSignal::Stale {
                since: since.clone(),
            });
        }
        if let Some(at) = &self.repost_at {
            signals.push(LifecycleSignal::Repost { at: at.clone() });
        }
        signals
    }

    /// Whether the posting is marked *Possibly closed* (FR-52).
    pub fn is_possibly_closed(&self) -> bool {
        self.possibly_closed_at.is_some()
    }

    /// Whether the user has already been told about this posting once.
    pub fn was_seen_or_dismissed(&self) -> bool {
        self.seen_at.is_some() || self.dismissed_at.is_some()
    }

    /// Whether the Offer changed materially since the user saw or dismissed it
    /// (FR-50). A dismissal holds through a source merely re-sending the same
    /// job; it does not hold through a different job under the same id.
    pub fn has_material_change(&self, offer: &Offer) -> bool {
        if self.content_hash != material_hash(offer) {
            return true;
        }
        match (
            self.description_simhash,
            description_simhash(&offer.description_text),
        ) {
            (Some(stored), Some(current)) => {
                similarity(stored, current) < DESCRIPTION_CHANGE_SIMILARITY
            }
            // A description that appeared — or one that can no longer be
            // compared — is a change; two descriptions that were never
            // comparable say nothing either way.
            (Some(_), None) => true,
            (None, _) => false,
        }
    }

    /// Whether the posting is worth a notification (FR-50, AC1): anything the
    /// user has not seen yet, and a posting they saw or dismissed only when it
    /// changed materially.
    pub fn should_notify(&self, offer: &Offer) -> bool {
        if !self.was_seen_or_dismissed() {
            return true;
        }
        self.has_material_change(offer)
    }
}

/// The signals a posting carries for Q4 legitimacy (FR-51).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleSignal {
    /// The posting's age passed [`STALE_AFTER_DAYS`].
    Stale { since: String },
    /// It came back within [`REPOST_WINDOW_DAYS`] of disappearing.
    Repost { at: String },
}

/// A *Possibly closed* posting that has saved Applications, so the tracker can
/// warn on them (FR-52, AC3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClosedWarning {
    pub posting_id: String,
    /// Every saved Application on that posting.
    pub application_ids: Vec<String>,
}

/// The fingerprint of an Offer's substance: what makes a reappearance news
/// (FR-50). Tracking noise (the URL), provenance and fetch times are not in it;
/// the description is compared by simhash, separately.
pub fn material_hash(offer: &Offer) -> String {
    let mut text = String::new();
    for part in [
        normalise(&offer.title),
        normalise(&offer.company),
        locations_key(&offer.locations),
        offer.work_mode.as_str().to_string(),
        money_key(offer),
        offer.deadline.clone().unwrap_or_else(|| "-".to_string()),
    ] {
        text.push_str(&part);
        text.push('\u{1f}');
    }
    format!("{:016x}", fnv1a(text.as_bytes()))
}

/// The staleness signal for an Offer read at `now`, or `None` when it is young
/// enough (FR-51). A source with no posted date is judged by how long Boba has
/// held it, which is the only age it can know.
pub fn stale_since(offer: &Offer, now: &str) -> Option<String> {
    let age = days_between(now, offer.posted_at.as_deref().unwrap_or(&offer.first_seen))?;
    (age > STALE_AFTER_DAYS).then(|| now.to_string())
}

/// Whether a posting that disappeared at `disappeared_at` and is seen at `now`
/// counts as a repost (FR-51). A clock that cannot read either date says no,
/// never yes.
pub fn is_repost(disappeared_at: &str, now: &str) -> bool {
    days_between(now, disappeared_at).is_some_and(|days| (0..=REPOST_WINDOW_DAYS).contains(&days))
}

/// Whether an Offer's deadline has passed at `now` (FR-52). No deadline, or one
/// this build cannot read, says nothing.
pub fn is_past_deadline(offer: &Offer, now: &str) -> bool {
    let Some(deadline) = offer.deadline.as_deref() else {
        return false;
    };
    match (parse(deadline), parse(now)) {
        (Some(deadline), Some(now)) => deadline < now,
        _ => false,
    }
}

/// Whole days from `then` to `now`, both UTC RFC 3339. `None` when either is
/// not a timestamp this build can read.
pub fn days_between(now: &str, then: &str) -> Option<i64> {
    Some((parse(now)? - parse(then)?).whole_days())
}

fn parse(value: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(value, &Rfc3339).ok()
}

fn normalise(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn locations_key(locations: &[String]) -> String {
    let mut key: Vec<String> = locations.iter().map(|l| normalise(l)).collect();
    key.sort();
    key.join("\u{1f}")
}

fn money_key(offer: &Offer) -> String {
    match &offer.money {
        Some(money) => format!(
            "{}/{}/{}",
            money.amount_minor,
            money.currency,
            money.period.as_str()
        ),
        None => "-".to_string(),
    }
}

/// FNV-1a 64-bit: stable across builds (unlike `DefaultHasher`), which a stored
/// fingerprint has to be.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::posting::{ApplyHints, Money, PayPeriod, WorkMode};

    const NOW: &str = "2026-10-16T00:00:00Z";

    fn offer() -> Offer {
        Offer {
            id: "pst_01LIFECYCLE".into(),
            source_id: "remotive".into(),
            external_id: "abc-1".into(),
            title: "Backend Engineer".into(),
            company: "Boba Labs".into(),
            locations: vec!["Remote".into(), "Bogotá, CO".into()],
            work_mode: WorkMode::Remote,
            money: Some(Money {
                amount_minor: 5_000_000,
                currency: "USD".into(),
                period: PayPeriod::Year,
            }),
            posted_at: Some("2026-10-01T00:00:00Z".into()),
            deadline: None,
            first_seen: NOW.into(),
            last_seen: NOW.into(),
            lang: "en".into(),
            description_html: "<p>Build APIs.</p>".into(),
            description_text: "Build APIs in Rust and PostgreSQL for a growing team.".into(),
            requirements: Vec::new(),
            apply: ApplyHints {
                url: "https://jobs.example.com/abc-1".into(),
                email: None,
            },
        }
    }

    fn long_text(extra: &str) -> String {
        format!(
            "We are hiring a backend engineer to design, build and operate the services behind \
             our marketplace. You will own Rust services, PostgreSQL schemas, migrations, \
             observability, incident response and the developer experience around them. \
             Experience with distributed systems and event-driven architecture is welcome. {extra}"
        )
    }

    fn lifecycle() -> PostingLifecycle {
        PostingLifecycle::new("pst_01LIFECYCLE", &offer(), NOW)
    }

    #[test]
    fn a_fresh_posting_starts_unseen_open_and_with_no_signals() {
        let row = lifecycle();

        assert_eq!(row.posting_id, "pst_01LIFECYCLE");
        assert_eq!(row.seen_at, None);
        assert_eq!(row.dismissed_at, None);
        assert_eq!(row.missing_runs, 0);
        assert!(!row.is_possibly_closed());
        assert_eq!(row.signals(), Vec::new());
        assert_eq!(row.updated_at, NOW);
    }

    #[test]
    fn the_fingerprint_ignores_case_location_order_and_provenance_noise() {
        let mut same = offer();
        same.title = "  backend   ENGINEER ".into();
        same.locations = vec!["bogotá, co".into(), "remote".into()];
        same.apply.url = "https://jobs.example.com/abc-1?utm_source=linkedin".into();
        same.first_seen = "2026-10-15T00:00:00Z".into();

        assert_eq!(material_hash(&offer()), material_hash(&same));
    }

    #[test]
    fn the_fingerprint_changes_when_the_substance_does() {
        let base = material_hash(&offer());

        let mut retitled = offer();
        retitled.title = "Staff Backend Engineer".into();
        assert_ne!(base, material_hash(&retitled));

        let mut repriced = offer();
        repriced.money.as_mut().unwrap().amount_minor = 6_000_000;
        assert_ne!(base, material_hash(&repriced));

        let mut moved = offer();
        moved.locations = vec!["Berlin, DE".into()];
        assert_ne!(base, material_hash(&moved));

        let mut closed = offer();
        closed.deadline = Some("2026-12-01T00:00:00Z".into());
        assert_ne!(base, material_hash(&closed));
    }

    /// FR-50, AC1: a posting the user never saw is news.
    #[test]
    fn an_unseen_posting_is_worth_a_notification() {
        assert!(lifecycle().should_notify(&offer()));
    }

    /// FR-50, AC1: seen once, re-sent unchanged, not notified again.
    #[test]
    fn a_seen_posting_is_not_news_again_until_it_changes() {
        let mut row = lifecycle();
        row.seen_at = Some(NOW.into());

        let mut resent = offer();
        resent.first_seen = "2026-10-20T00:00:00Z".into();
        resent.last_seen = "2026-10-20T00:00:00Z".into();

        assert!(!row.has_material_change(&resent));
        assert!(!row.should_notify(&resent));

        let mut changed = resent.clone();
        changed.title = "Staff Backend Engineer".into();
        assert!(row.has_material_change(&changed));
        assert!(row.should_notify(&changed));
    }

    /// FR-50: a dismissal holds through a re-send, and is broken by a real
    /// change.
    #[test]
    fn a_dismissed_posting_stays_dismissed_until_it_changes() {
        let mut row = lifecycle();
        row.dismissed_at = Some(NOW.into());

        assert!(!row.should_notify(&offer()));

        let mut changed = offer();
        changed.money = None;
        assert!(row.should_notify(&changed));
    }

    /// A rewritten description is a material change; a typo-level edit is not.
    #[test]
    fn the_description_is_compared_by_simhash() {
        let mut row = lifecycle();
        let original = long_text("");
        let mut first = offer();
        first.description_text = original.clone();
        row.content_hash = material_hash(&first);
        row.description_simhash = description_simhash(&original);

        let mut edited = first.clone();
        edited.description_text = long_text("We offer a generous learning budget.");
        assert!(
            !row.has_material_change(&edited),
            "one added sentence is still the same posting"
        );

        let mut rewritten = first;
        rewritten.description_text = "Pastry operations manager for our bakery: sourdough \
             starters, laminated doughs, oven schedules, inventory rotation, allergen \
             controls, supplier negotiations and weekend production planning. You will \
             lead a small team and own quality."
            .to_string();
        assert!(row.has_material_change(&rewritten));
    }

    /// FR-51: a posting over 90 days old carries the staleness signal.
    #[test]
    fn a_posting_over_ninety_days_old_is_stale() {
        let mut old = offer();
        old.posted_at = Some("2026-07-01T00:00:00Z".into());

        let row = PostingLifecycle::new("pst_01LIFECYCLE", &old, NOW);
        assert_eq!(row.stale_at.as_deref(), Some(NOW));
        assert_eq!(
            row.signals(),
            vec![LifecycleSignal::Stale { since: NOW.into() }]
        );
    }

    #[test]
    fn a_young_posting_is_not_stale() {
        let mut fresh = offer();
        fresh.posted_at = Some("2026-08-01T00:00:00Z".into());
        assert_eq!(stale_since(&fresh, NOW), None, "76 days");

        // Exactly 90 days is not *more than* 90.
        fresh.posted_at = Some("2026-07-18T00:00:00Z".into());
        assert_eq!(stale_since(&fresh, NOW), None);
    }

    /// A source that sends no posted date is judged by how long Boba held it.
    #[test]
    fn a_posting_without_a_date_ages_from_first_seen() {
        let mut undated = offer();
        undated.posted_at = None;
        undated.first_seen = "2026-06-01T00:00:00Z".into();

        assert_eq!(stale_since(&undated, NOW).as_deref(), Some(NOW));
    }

    #[test]
    fn a_date_that_cannot_be_read_fires_no_signal() {
        let mut broken = offer();
        broken.posted_at = Some("last Tuesday".into());
        broken.first_seen = "not a timestamp".into();

        assert_eq!(stale_since(&broken, NOW), None);
        assert!(!is_past_deadline(&broken, "also not one"));
    }

    /// FR-51: back within 30 days of disappearing is a repost.
    #[test]
    fn coming_back_within_thirty_days_is_a_repost() {
        assert!(is_repost("2026-10-01T00:00:00Z", NOW), "15 days");
        assert!(is_repost("2026-09-16T00:00:00Z", NOW), "exactly 30");

        assert!(!is_repost("2026-09-15T00:00:00Z", NOW), "31 days");
        assert!(
            !is_repost("2026-10-20T00:00:00Z", NOW),
            "a disappearance after now is not history"
        );
        assert!(!is_repost("whenever", NOW));
    }

    /// FR-52: a deadline in the past means expired, whatever the source says.
    #[test]
    fn a_past_deadline_is_expired() {
        let mut expired = offer();
        expired.deadline = Some("2026-10-15T00:00:00Z".into());
        assert!(is_past_deadline(&expired, NOW));

        expired.deadline = Some("2026-10-17T00:00:00Z".into());
        assert!(!is_past_deadline(&expired, NOW));

        expired.deadline = None;
        assert!(!is_past_deadline(&expired, NOW));
    }

    #[test]
    fn signals_are_listed_stable_and_both_fire_together() {
        let mut row = lifecycle();
        row.stale_at = Some("2026-10-10T00:00:00Z".into());
        row.repost_at = Some("2026-10-12T00:00:00Z".into());

        assert_eq!(
            row.signals(),
            vec![
                LifecycleSignal::Stale {
                    since: "2026-10-10T00:00:00Z".into()
                },
                LifecycleSignal::Repost {
                    at: "2026-10-12T00:00:00Z".into()
                },
            ]
        );
    }
}
