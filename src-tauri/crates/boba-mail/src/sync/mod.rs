//! Scoped local reading (S8.3, FR-38, FR-104, FR-113).
//!
//! In read mode the app never mirrors a mailbox. A sync pass walks the
//! messages the adapter fetched since the last cursor and keeps only those that
//! plausibly relate to an Application (FR-38):
//!
//! * a sender on a domain of a company the user applied to,
//! * a known applicant-tracking-system sender (Greenhouse, Lever, Workday…), or
//! * a reply in a thread the app started (`In-Reply-To`/`References` names one
//!   of our sent `Message-ID`s).
//!
//! Everything else is dropped **before** it reaches the sink, so unrelated
//! inbox content is never written to disk. A manual `.eml` import (FR-104) runs
//! through the same matcher and the same sink, and so does any future read
//! adapter (Gmail BYO in S8.5).
//!
//! The pass is incremental: the caller passes the cursor it stored, and every
//! message older than it is skipped without being stored again. The 60-day
//! lookback default bounds how far back a pass reads (FR-38); a manual import
//! is user-initiated and bypasses the window, while retention (FR-113) still
//! applies to what is stored.
//!
//! ## Wiring
//!
//! [`MessageSink`] is implemented by `boba-store` over `email_message`
//! (migration `0025_email_messages.sql`) and mounted as a port; the scheduler's
//! `email_sync` handler builds the scope from the Tracker (the addresses
//! applications were sent to, the Offer hosts and the sent `Message-ID`s),
//! reads each read+send connection (`graph::fetch_inbox`, `imap::fetch_inbox`,
//! both read-only and capped) and calls [`run`] with the cursor it stored. S8.4
//! then reads the stored rows for classification; S10.3 sweeps them by
//! `received_at`.

use std::collections::BTreeSet;

pub use boba_app::ports::mail::MessageSink;
pub use boba_domain::mail::{ImportOrigin, MatchReason, RawMessage, StoredMessage, SyncError};

/// The default lookback window: messages older than this are not read during a
/// sync (FR-38).
pub const DEFAULT_LOOKBACK_DAYS: u32 = 60;

const SECONDS_PER_DAY: i64 = 24 * 60 * 60;

/// Sender domains of the applicant-tracking systems that mail on a company's
/// behalf (FR-38, "known ATS senders").
///
/// A match is on the domain or a subdomain of it, so `no-reply@greenhouse.io`
/// and `jobs@mail.greenhouse.io` both match. The list is deliberately data, not
/// policy: a caller may add a board's domain to its [`MessageScope`] without
/// touching this code.
pub const ATS_SENDER_DOMAINS: &[&str] = &[
    "greenhouse.io",
    "greenhouse-mail.io",
    "lever.co",
    "ashbyhq.com",
    "smartrecruiters.com",
    "workday.com",
    "myworkdayjobs.com",
    "myworkday.com",
    "icims.com",
    "taleo.net",
    "successfactors.com",
    "bamboohr.com",
    "workable.com",
    "recruitee.com",
    "jobvite.com",
    "applytojob.com",
    "breezy.hr",
    "teamtailor.com",
    "personio.de",
    "personio.com",
    "join.com",
    "recruiterbox.com",
];

/// What the caller knows about the Applications before a pass runs. It is
/// built by the core from the Tracker, never by an adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageScope {
    company_domains: Vec<String>,
    ats_domains: Vec<String>,
    sent_message_ids: Vec<String>,
    lookback_seconds: i64,
}

impl MessageScope {
    /// A scope over the given applied-company domains, additional ATS domains
    /// and the `Message-ID`s of mail the app sent, with the 60-day default
    /// lookback (FR-38).
    ///
    /// `sent_message_ids` must be the *internet* `Message-ID`s the provider put
    /// on the sent messages (what a reply's `In-Reply-To` names), not the
    /// provider's internal row id.
    pub fn new(
        company_domains: impl IntoIterator<Item = impl Into<String>>,
        ats_domains: impl IntoIterator<Item = impl Into<String>>,
        sent_message_ids: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        let mut ats: Vec<String> = ATS_SENDER_DOMAINS
            .iter()
            .map(|d| (*d).to_string())
            .collect();
        ats.extend(ats_domains.into_iter().map(Into::into));
        Self {
            company_domains: company_domains.into_iter().map(Into::into).collect(),
            ats_domains: ats,
            sent_message_ids: sent_message_ids
                .into_iter()
                .map(Into::into)
                .map(|id| normalize_message_id(&id))
                .filter(|id| !id.is_empty())
                .collect(),
            lookback_seconds: DEFAULT_LOOKBACK_DAYS as i64 * SECONDS_PER_DAY,
        }
    }

    /// Replace the lookback window (FR-38 default: [`DEFAULT_LOOKBACK_DAYS`]).
    /// Zero means "today only".
    #[must_use]
    pub fn with_lookback_days(mut self, days: u32) -> Self {
        self.lookback_seconds = i64::from(days) * SECONDS_PER_DAY;
        self
    }

    /// The oldest `received_at` (Unix seconds) a sync pass still reads at
    /// `now`.
    pub fn cutoff(&self, now: i64) -> i64 {
        now - self.lookback_seconds
    }

    /// Whether `message` falls inside the lookback window ending at `now`.
    pub fn in_window(&self, message: &RawMessage, now: i64) -> bool {
        message.received_at >= self.cutoff(now)
    }

    /// Why `message` relates to an Application, or `None` when it does not.
    /// A thread we started is the strongest signal, then a company domain, then
    /// a known ATS.
    pub fn matches(&self, message: &RawMessage) -> Option<MatchReason> {
        if self.is_sent_thread(message) {
            return Some(MatchReason::SentThread);
        }
        let domain = message.sender_domain()?;
        if self
            .company_domains
            .iter()
            .any(|d| domain_within(&domain, d))
        {
            return Some(MatchReason::CompanyDomain);
        }
        if self.ats_domains.iter().any(|d| domain_within(&domain, d)) {
            return Some(MatchReason::AtsSender);
        }
        None
    }

    fn is_sent_thread(&self, message: &RawMessage) -> bool {
        let references = message.in_reply_to.iter().chain(message.references.iter());
        references
            .map(|id| normalize_message_id(id))
            .any(|id| self.sent_message_ids.iter().any(|sent| sent == &id))
    }
}

/// One sync pass: when it runs, how far back to read, and what asked for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncRequest {
    /// The pass's clock, in Unix seconds.
    pub now: i64,
    /// The cursor from the previous pass: messages older than it were already
    /// scanned and are skipped (incremental sync).
    pub after: Option<i64>,
    /// Whether a mailbox adapter or the manual importer asked for the pass.
    pub origin: ImportOrigin,
    /// The connection a mailbox pass belongs to; `None` for an import.
    pub account: Option<String>,
}

impl SyncRequest {
    /// An incremental pass over one connected mailbox (FR-38).
    pub fn incremental(account: impl Into<String>, now: i64, after: Option<i64>) -> Self {
        Self {
            now,
            after,
            origin: ImportOrigin::Sync,
            account: Some(account.into()),
        }
    }

    /// A manual import (FR-104): no connection, no cursor and no lookback
    /// window.
    pub fn import(now: i64) -> Self {
        Self {
            now,
            after: None,
            origin: ImportOrigin::Eml,
            account: None,
        }
    }
}

/// What one pass did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// How many messages the adapter handed over.
    pub scanned: usize,
    /// How many were stored.
    pub stored: usize,
    /// How many matched nothing and were dropped after matching.
    pub discarded: usize,
    /// How many fell outside the lookback window.
    pub too_old: usize,
    /// How many were older than the cursor, or a duplicate delivery.
    pub already_seen: usize,
    /// The cursor to store for the next pass: the newest `received_at` seen,
    /// never later than the pass's `now`.
    pub next_after: Option<i64>,
}

/// Run one scoped pass over `messages`.
///
/// The order of checks is what makes the privacy promise cheap and exact:
/// the lookback window first (a sync never reads old mail), then the cursor and
/// duplicate ids (an incremental pass is idempotent), then the scope match.
/// Only a matched message calls [`MessageSink::store`], so nothing else is ever
/// written.
///
/// The cursor is deliberately conservative: a message is skipped only when it
/// is **older** than the cursor (two messages can share a timestamp, and the
/// sink's unique key makes re-storing one harmless), and a message dated in the
/// future cannot push the cursor past `now`.
pub fn run(
    messages: impl IntoIterator<Item = RawMessage>,
    scope: &MessageScope,
    request: &SyncRequest,
    sink: &dyn MessageSink,
) -> Result<SyncReport, SyncError> {
    let mut report = SyncReport {
        next_after: request.after,
        ..SyncReport::default()
    };
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for message in messages {
        report.scanned += 1;
        let stamp = message.received_at.min(request.now);
        report.next_after = Some(report.next_after.map_or(stamp, |cursor| cursor.max(stamp)));
        if request.origin == ImportOrigin::Sync && !scope.in_window(&message, request.now) {
            report.too_old += 1;
            continue;
        }
        if request
            .after
            .is_some_and(|cursor| message.received_at < cursor)
        {
            report.already_seen += 1;
            continue;
        }
        if !seen.insert(message.provider_id.clone()) {
            report.already_seen += 1;
            continue;
        }
        let Some(matched_by) = scope.matches(&message) else {
            report.discarded += 1;
            continue;
        };
        sink.store(StoredMessage {
            message: &message,
            matched_by,
            origin: request.origin,
            account_id: request.account.as_deref(),
        })?;
        report.stored += 1;
    }
    // A cursor left in the future by an earlier skewed clock is pulled back to
    // the pass's clock, so it cannot hide real mail from now on.
    report.next_after = report.next_after.map(|cursor| cursor.min(request.now));
    Ok(report)
}

/// Lowercase a domain and drop a trailing root dot, so `Mail.Example.COM.`
/// and `mail.example.com` are the same domain.
fn normalize_domain(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('@')
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

/// `Message-ID`s are compared without their angle brackets and case-folded:
/// transporters re-wrap and re-case them in practice.
pub(crate) fn normalize_message_id(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim()
        .to_ascii_lowercase()
}

/// Whether `domain` is `candidate` or a subdomain of it. Label-aware, so
/// `notexample.com` never matches `example.com`.
fn domain_within(domain: &str, candidate: &str) -> bool {
    let candidate = normalize_domain(candidate);
    if candidate.is_empty() {
        return false;
    }
    domain == candidate
        || domain
            .strip_suffix(&candidate)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_760_000_000;
    const DAY: i64 = 24 * 60 * 60;
    const ACCOUNT: &str = "acc-1";

    fn message(provider_id: &str, from: &str, received_at: i64) -> RawMessage {
        RawMessage {
            provider_id: provider_id.to_string(),
            from_address: from.to_string(),
            received_at,
            subject: "hello".to_string(),
            body_text: "body".to_string(),
            ..RawMessage::default()
        }
    }

    #[derive(Default)]
    struct Collector {
        stored: std::sync::Mutex<Vec<RawMessage>>,
        reasons: std::sync::Mutex<Vec<MatchReason>>,
        origins: std::sync::Mutex<Vec<ImportOrigin>>,
    }

    impl Collector {
        fn stored(&self) -> Vec<RawMessage> {
            self.stored.lock().unwrap().clone()
        }

        fn reasons(&self) -> Vec<MatchReason> {
            self.reasons.lock().unwrap().clone()
        }

        fn origins(&self) -> Vec<ImportOrigin> {
            self.origins.lock().unwrap().clone()
        }
    }

    impl MessageSink for Collector {
        fn store(&self, message: StoredMessage<'_>) -> Result<(), SyncError> {
            self.stored.lock().unwrap().push(message.message.clone());
            self.reasons.lock().unwrap().push(message.matched_by);
            self.origins.lock().unwrap().push(message.origin);
            Ok(())
        }
    }

    struct BrokenSink;

    impl MessageSink for BrokenSink {
        fn store(&self, _message: StoredMessage<'_>) -> Result<(), SyncError> {
            Err(SyncError::Store("disk full".to_string()))
        }
    }

    fn scope() -> MessageScope {
        MessageScope::new(
            ["acme.example".to_string()],
            Vec::<String>::new(),
            ["<sent-1@boba.local>".to_string()],
        )
    }

    #[test]
    fn lookback_defaults_to_sixty_days() {
        assert_eq!(DEFAULT_LOOKBACK_DAYS, 60);
        let scope = scope();
        assert_eq!(scope.cutoff(NOW), NOW - 60 * DAY);
    }

    #[test]
    fn an_applied_company_domain_matches() {
        let kept = message("m1", "talent@acme.example", NOW - DAY);
        assert_eq!(scope().matches(&kept), Some(MatchReason::CompanyDomain));
    }

    #[test]
    fn a_subdomain_of_an_applied_company_matches() {
        let kept = message("m1", "no-reply@mail.acme.example", NOW - DAY);
        assert_eq!(scope().matches(&kept), Some(MatchReason::CompanyDomain));
    }

    #[test]
    fn a_lookalike_domain_does_not_match() {
        let other = message("m1", "talent@notacme.example", NOW - DAY);
        assert_eq!(scope().matches(&other), None);
        let parent = message("m2", "talent@example", NOW - DAY);
        assert_eq!(scope().matches(&parent), None);
    }

    #[test]
    fn matching_is_case_insensitive_and_ignores_a_root_dot() {
        let kept = message("m1", "Talent@ACME.Example.", NOW - DAY);
        assert_eq!(scope().matches(&kept), Some(MatchReason::CompanyDomain));
    }

    #[test]
    fn a_known_ats_sender_matches() {
        for sender in ["no-reply@greenhouse.io", "jobs@mail.greenhouse.io"] {
            let kept = message("m1", sender, NOW - DAY);
            assert_eq!(
                scope().matches(&kept),
                Some(MatchReason::AtsSender),
                "{sender}"
            );
        }
    }

    #[test]
    fn a_caller_can_add_its_own_ats_domain() {
        let scope = MessageScope::new(
            Vec::<String>::new(),
            ["hire.example".to_string()],
            Vec::<String>::new(),
        );
        let kept = message("m1", "jobs@hire.example", NOW - DAY);
        assert_eq!(scope.matches(&kept), Some(MatchReason::AtsSender));
    }

    #[test]
    fn a_reply_to_a_sent_message_matches_by_its_thread() {
        let mut reply = message("m1", "someone@unknown.example", NOW - DAY);
        reply.in_reply_to = vec!["<SENT-1@boba.local>".to_string()];
        assert_eq!(scope().matches(&reply), Some(MatchReason::SentThread));
    }

    #[test]
    fn a_references_chain_to_a_sent_message_matches_by_its_thread() {
        let mut reply = message("m1", "someone@unknown.example", NOW - DAY);
        reply.references = vec![
            "<older@elsewhere.example>".to_string(),
            "<sent-1@boba.local>".to_string(),
        ];
        assert_eq!(scope().matches(&reply), Some(MatchReason::SentThread));
    }

    #[test]
    fn an_unrelated_message_is_discarded_before_the_sink() {
        let collector = Collector::default();
        let report = run(
            [message("m1", "friend@personal.example", NOW - DAY)],
            &scope(),
            &SyncRequest::incremental(ACCOUNT, NOW, None),
            &collector,
        )
        .expect("pass");
        assert_eq!(report.discarded, 1);
        assert_eq!(report.stored, 0);
        assert!(collector.stored().is_empty());
    }

    #[test]
    fn a_message_older_than_the_lookback_is_not_even_matched() {
        let collector = Collector::default();
        let report = run(
            [message("m1", "talent@acme.example", NOW - 61 * DAY)],
            &scope(),
            &SyncRequest::incremental(ACCOUNT, NOW, None),
            &collector,
        )
        .expect("pass");
        assert_eq!(report.too_old, 1);
        assert_eq!(report.stored, 0);
        assert!(collector.stored().is_empty());
    }

    #[test]
    fn the_lookback_window_follows_the_message_date() {
        let collector = Collector::default();
        let report = run(
            [message("m1", "talent@acme.example", NOW - 60 * DAY)],
            &scope(),
            &SyncRequest::incremental(ACCOUNT, NOW, None),
            &collector,
        )
        .expect("pass");
        assert_eq!(report.stored, 1, "the boundary day is inside the window");
    }

    #[test]
    fn a_custom_lookback_is_honoured() {
        let collector = Collector::default();
        let report = run(
            [message("m1", "talent@acme.example", NOW - 10 * DAY)],
            &scope().with_lookback_days(7),
            &SyncRequest::incremental(ACCOUNT, NOW, None),
            &collector,
        )
        .expect("pass");
        assert_eq!(report.too_old, 1);
        assert!(collector.stored().is_empty());
    }

    #[test]
    fn an_incremental_pass_skips_messages_older_than_the_cursor() {
        let collector = Collector::default();
        let report = run(
            [
                message("m1", "talent@acme.example", NOW - 5 * DAY),
                message("m2", "talent@acme.example", NOW - 2 * DAY),
                message("m3", "talent@acme.example", NOW - DAY),
            ],
            &scope(),
            &SyncRequest::incremental(ACCOUNT, NOW, Some(NOW - 2 * DAY)),
            &collector,
        )
        .expect("pass");
        assert_eq!(report.already_seen, 1);
        assert_eq!(report.stored, 2);
        assert_eq!(collector.stored().len(), 2);
    }

    #[test]
    fn a_message_in_the_cursor_second_is_not_lost() {
        // Two messages can share a timestamp; skipping everything at the
        // cursor would drop the second one forever.
        let collector = Collector::default();
        let report = run(
            [message("m1", "talent@acme.example", NOW - DAY)],
            &scope(),
            &SyncRequest::incremental(ACCOUNT, NOW, Some(NOW - DAY)),
            &collector,
        )
        .expect("pass");
        assert_eq!(report.already_seen, 0);
        assert_eq!(report.stored, 1);
    }

    #[test]
    fn a_future_date_never_pushes_the_cursor_past_now() {
        let collector = Collector::default();
        let report = run(
            [message("m1", "talent@acme.example", NOW + 30 * DAY)],
            &scope(),
            &SyncRequest::incremental(ACCOUNT, NOW, None),
            &collector,
        )
        .expect("pass");
        assert_eq!(report.next_after, Some(NOW));
    }

    #[test]
    fn a_future_cursor_from_a_skewed_clock_is_pulled_back_to_now() {
        let collector = Collector::default();
        let report = run(
            [message("m1", "talent@acme.example", NOW - DAY)],
            &scope(),
            &SyncRequest::incremental(ACCOUNT, NOW, Some(NOW + 10 * DAY)),
            &collector,
        )
        .expect("pass");
        assert_eq!(report.next_after, Some(NOW));
    }

    #[test]
    fn a_re_delivered_message_is_stored_once() {
        let collector = Collector::default();
        let report = run(
            [
                message("m1", "talent@acme.example", NOW - DAY),
                message("m1", "talent@acme.example", NOW - DAY),
            ],
            &scope(),
            &SyncRequest::incremental(ACCOUNT, NOW, None),
            &collector,
        )
        .expect("pass");
        assert_eq!(report.stored, 1);
        assert_eq!(report.already_seen, 1);
    }

    #[test]
    fn the_cursor_moves_to_the_newest_scanned_message_even_when_discarded() {
        let collector = Collector::default();
        let report = run(
            [
                message("m1", "friend@personal.example", NOW - DAY),
                message("m2", "talent@acme.example", NOW - 3 * DAY),
            ],
            &scope(),
            &SyncRequest::incremental(ACCOUNT, NOW, Some(NOW - 10 * DAY)),
            &collector,
        )
        .expect("pass");
        assert_eq!(report.next_after, Some(NOW - DAY));
    }

    #[test]
    fn the_cursor_never_moves_backwards() {
        let collector = Collector::default();
        let report = run(
            [message("m1", "talent@acme.example", NOW - 5 * DAY)],
            &scope(),
            &SyncRequest::incremental(ACCOUNT, NOW, Some(NOW - DAY)),
            &collector,
        )
        .expect("pass");
        assert_eq!(report.next_after, Some(NOW - DAY));
    }

    #[test]
    fn a_matched_message_reports_why_it_was_kept_and_where_it_came_from() {
        let collector = Collector::default();
        run(
            [message("m1", "no-reply@greenhouse.io", NOW - DAY)],
            &scope(),
            &SyncRequest::incremental(ACCOUNT, NOW, None),
            &collector,
        )
        .expect("pass");
        assert_eq!(collector.reasons(), vec![MatchReason::AtsSender]);
        assert_eq!(collector.origins(), vec![ImportOrigin::Sync]);
        assert_eq!(MatchReason::AtsSender.as_str(), "ats_sender");
        assert_eq!(ImportOrigin::Sync.as_str(), "sync");
    }

    #[test]
    fn the_report_counts_every_message_once() {
        let collector = Collector::default();
        let report = run(
            [
                message("m1", "friend@personal.example", NOW - DAY),
                message("m2", "talent@acme.example", NOW - 200 * DAY),
                message("m3", "no-reply@greenhouse.io", NOW - DAY),
                message("m4", "talent@acme.example", NOW - 200 * DAY),
            ],
            &scope(),
            &SyncRequest::incremental(ACCOUNT, NOW, None),
            &collector,
        )
        .expect("pass");
        assert_eq!(report.scanned, 4);
        assert_eq!(report.stored, 1);
        assert_eq!(report.discarded, 1);
        assert_eq!(report.too_old, 2);
        assert_eq!(report.already_seen, 0);
    }

    #[test]
    fn a_store_failure_stops_the_pass_and_is_reported() {
        let err = run(
            [message("m1", "talent@acme.example", NOW - DAY)],
            &scope(),
            &SyncRequest::incremental(ACCOUNT, NOW, None),
            &BrokenSink,
        )
        .expect_err("store failure");
        assert_eq!(err, SyncError::Store("disk full".to_string()));
    }

    #[test]
    fn a_manual_import_ignores_the_lookback_window_but_still_matches() {
        let collector = Collector::default();
        let report = run(
            [
                message("m1", "talent@acme.example", NOW - 200 * DAY),
                message("m2", "friend@personal.example", NOW - DAY),
            ],
            &scope(),
            &SyncRequest::import(NOW),
            &collector,
        )
        .expect("pass");
        assert_eq!(report.stored, 1, "an old reply the user picked is kept");
        assert_eq!(report.discarded, 1);
        assert_eq!(report.too_old, 0);
        assert_eq!(collector.origins(), vec![ImportOrigin::Eml]);
        assert_eq!(ImportOrigin::Eml.as_str(), "eml");
    }
}
