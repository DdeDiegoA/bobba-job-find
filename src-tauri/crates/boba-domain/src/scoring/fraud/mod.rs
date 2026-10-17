//! Recruitment-fraud and ghost-posting signals (S6.8, FR-60, offer-scoring.md §5).
//!
//! Two families of signal, deliberately kept apart:
//!
//! * **Fraud** — pay-to-apply, contact moved off platform, implausible pay.
//!   These are hard red flags: a fraud badge **blocks Auto** for the Offer
//!   (FR-60) and the policy reads [`FraudReport::blocks_auto`].
//! * **Ghost** — signals that the posting may never be filled: stale, vague, no
//!   salary, talent-pool wording. They lower Q4's score and show on the card,
//!   but they never block an application on their own (offer-scoring.md §5: a
//!   posting is *deprioritized, not deleted*).
//!
//! Every signal carries its **source**: the sentence of the posting it was read
//! from, the structured field it read, or the published report whose threshold
//! it uses — AC2's "Ghost signals show with their source". Detection is pure,
//! offline and deterministic (AD-8, FR-61): no LLM, no clock of its own.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::posting::{Money, Offer, PayPeriod};
use crate::scoring::factors::MarketData;
use crate::scoring::gates::QuotedSpan;

/// The report whose 90/180-day thresholds this module reads
/// (offer-scoring.md §5).
pub const GHOST_REPORT: &str = "Unlisted Ghost Jobs Report, September 2026";

/// Posting age from which a posting counts as stale (27.6% of open postings).
pub const STALE_DAYS: i64 = 90;

/// Posting age from which the signal is strong (15.9% of open postings).
pub const VERY_STALE_DAYS: i64 = 180;

/// A description shorter than this, with no requirements, is vague.
pub const VAGUE_WORDS: usize = 60;

/// A year's typical hours, for comparing pay across periods.
const HOURS_PER_YEAR: i64 = 2_080;

/// Pay above this yearly equivalent is implausible (minor units: 1,000,000 a
/// year, or 481 an hour — an outlier no honest remote posting reaches).
const IMPLAUSIBLE_YEARLY_MINOR: i64 = 100_000_000;

/// An offer paying this many times the benchmark's p50 is implausible.
const BENCHMARK_MULTIPLIER: f32 = 3.0;

/// What a signal is (FR-60).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalKind {
    /// The posting asks the applicant for money (fraud).
    PayToApply,
    /// The "recruiter" moves the conversation to WhatsApp, Telegram…
    OffPlatformContact,
    /// Pay far outside what any employer would pay (fraud).
    ImplausiblePay,
    /// Posted, or first seen, longer ago than the report's threshold (ghost).
    GhostStalePosting,
    /// So short and empty it says nothing about the job (ghost).
    GhostVagueDescription,
    /// No salary stated (ghost).
    GhostNoSalary,
    /// "Evergreen", "talent pool", "pending funding"… (ghost).
    GhostTalentPool,
}

impl SignalKind {
    /// The badge id the UI resolves to a label.
    pub fn as_str(self) -> &'static str {
        match self {
            SignalKind::PayToApply => "pay_to_apply",
            SignalKind::OffPlatformContact => "off_platform_contact",
            SignalKind::ImplausiblePay => "implausible_pay",
            SignalKind::GhostStalePosting => "ghost_stale_posting",
            SignalKind::GhostVagueDescription => "ghost_vague_description",
            SignalKind::GhostNoSalary => "ghost_no_salary",
            SignalKind::GhostTalentPool => "ghost_talent_pool",
        }
    }

    /// FR-60: fraud blocks Auto; a ghost signal only warns.
    pub fn is_fraud(self) -> bool {
        matches!(
            self,
            SignalKind::PayToApply | SignalKind::OffPlatformContact | SignalKind::ImplausiblePay
        )
    }
}

/// How loudly a signal speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Medium,
    High,
}

/// Where a signal was read from (AC2: the source travels with the signal).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// The posting's own words, with offsets — untrusted text, shown as text.
    Text(QuotedSpan),
    /// A structured Offer field, by name (`money`, `posted_at`, …).
    Field(String),
    /// The published report a threshold comes from.
    Report(String),
}

/// One signal, with its source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signal {
    pub kind: SignalKind,
    pub severity: Severity,
    pub source: Source,
}

/// What one Offer's text and fields show (FR-60).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FraudReport {
    /// The signals found, in [`SignalKind::ALL`] order.
    pub signals: Vec<Signal>,
}

impl FraudReport {
    /// Whether this Offer carries a fraud badge: if it does, Auto is blocked
    /// (FR-60) and no rule may override it.
    pub fn blocks_auto(&self) -> bool {
        self.signals.iter().any(|signal| signal.kind.is_fraud())
    }

    /// One signal of this kind, when the Offer has it.
    pub fn find(&self, kind: SignalKind) -> Option<&Signal> {
        self.signals.iter().find(|signal| signal.kind == kind)
    }
}

/// Detect the fraud and ghost signals in one Offer (AC2).
///
/// `now` is the RFC 3339 instant "stale" is measured against; an unreadable
/// posting date yields no stale signal (nothing is imputed).
pub fn detect(offer: &Offer, market: &MarketData, now: &str) -> FraudReport {
    let mut signals = Vec::new();

    // Fraud first (FR-60): each is a hard red flag.
    if let Some(span) = first_phrase(&offer.description_text, PAY_TO_APPLY) {
        signals.push(Signal {
            kind: SignalKind::PayToApply,
            severity: Severity::High,
            source: Source::Text(span),
        });
    }
    if let Some(span) = first_phrase(&offer.description_text, OFF_PLATFORM) {
        signals.push(Signal {
            kind: SignalKind::OffPlatformContact,
            severity: Severity::High,
            source: Source::Text(span),
        });
    }
    if implausible_pay(offer, market) {
        signals.push(Signal {
            kind: SignalKind::ImplausiblePay,
            severity: Severity::High,
            source: Source::Field("money".to_string()),
        });
    }

    // Ghost signals (offer-scoring.md §5): they deprioritise, never block.
    if let Some(severity) = stale(offer, now) {
        signals.push(Signal {
            kind: SignalKind::GhostStalePosting,
            severity,
            source: Source::Report(GHOST_REPORT.to_string()),
        });
    }
    if vague(offer) {
        signals.push(Signal {
            kind: SignalKind::GhostVagueDescription,
            severity: Severity::Medium,
            source: Source::Field("description_text".to_string()),
        });
    }
    if offer.money.is_none() {
        signals.push(Signal {
            kind: SignalKind::GhostNoSalary,
            severity: Severity::Medium,
            source: Source::Field("money".to_string()),
        });
    }
    if let Some(span) = first_phrase(&offer.description_text, TALENT_POOL) {
        signals.push(Signal {
            kind: SignalKind::GhostTalentPool,
            severity: Severity::Medium,
            source: Source::Text(span),
        });
    }

    FraudReport { signals }
}

/// The posting wording that asks the applicant for money (EN and ES).
const PAY_TO_APPLY: &[&str] = &[
    "application fee",
    "processing fee",
    "registration fee",
    "training fee",
    "security deposit",
    "upfront payment",
    "pay to apply",
    "pay a fee",
    "fee is required",
    "purchase your own equipment",
    "buy your own equipment",
    "tarifa de inscripción",
    "cuota de inscripción",
    "depósito de seguridad",
    "pago por aplicar",
    "pago inicial",
];

/// The wording that moves the conversation off the platform (EN and ES).
const OFF_PLATFORM: &[&str] = &[
    "whatsapp",
    "telegram",
    "signal app",
    "dm me",
    "text me at",
    "message me on",
    "personal email",
    "gmail.com",
    "hotmail.com",
    "outlook.com",
    "correo personal",
    "escríbeme por whatsapp",
];

/// The wording of a posting that exists to fill a pipeline, not a role.
const TALENT_POOL: &[&str] = &[
    "talent pool",
    "evergreen",
    "pending funding",
    "subject to award",
    "future opportunities",
    "general application",
];

/// The first occurrence of any phrase, as the sentence it sits in.
fn first_phrase(text: &str, phrases: &[&str]) -> Option<QuotedSpan> {
    let folded = crate::scoring::gates::scan::fold(text);
    let mut best: Option<(usize, usize)> = None;
    for phrase in phrases {
        for start in crate::scoring::gates::scan::matches(&folded, phrase) {
            let candidate = (start, start + phrase.len());
            if best.is_none_or(|found| candidate.0 < found.0) {
                best = Some(candidate);
            }
        }
    }
    best.map(|(start, end)| crate::scoring::gates::scan::sentence(text, start, end))
}

/// FR-60's implausible pay: an absolute outlier, or far above the benchmark
/// when one matches (same currency and period, same role).
fn implausible_pay(offer: &Offer, market: &MarketData) -> bool {
    let Some(money) = &offer.money else {
        return false;
    };
    if yearly_minor(money) > IMPLAUSIBLE_YEARLY_MINOR {
        return true;
    }
    let title = offer.title.to_lowercase();
    market.salary.iter().any(|benchmark| {
        benchmark.currency == money.currency
            && benchmark.period == money.period
            && benchmark.p50_minor > 0
            && (title.contains(&benchmark.role.to_lowercase())
                || benchmark.role.to_lowercase().contains(&title))
            && money.amount_minor as f32 > benchmark.p50_minor as f32 * BENCHMARK_MULTIPLIER
    })
}

/// A pay amount's yearly equivalent in minor units.
fn yearly_minor(money: &Money) -> i64 {
    match money.period {
        PayPeriod::Year => money.amount_minor,
        PayPeriod::Month => money.amount_minor.checked_mul(12).unwrap_or(i64::MAX),
        PayPeriod::Hour => money
            .amount_minor
            .checked_mul(HOURS_PER_YEAR)
            .unwrap_or(i64::MAX),
    }
}

/// How stale the posting is, when its date is readable (nothing is imputed).
fn stale(offer: &Offer, now: &str) -> Option<Severity> {
    let now = OffsetDateTime::parse(now, &Rfc3339).ok()?;
    let posted = offer
        .posted_at
        .as_deref()
        .or(Some(offer.first_seen.as_str()))
        .and_then(|value| OffsetDateTime::parse(value, &Rfc3339).ok())?;
    let days = (now - posted).whole_days();
    if days >= VERY_STALE_DAYS {
        Some(Severity::High)
    } else if days >= STALE_DAYS {
        Some(Severity::Medium)
    } else {
        None
    }
}

/// A description too short to describe anything, with no requirements either.
fn vague(offer: &Offer) -> bool {
    let words = offer
        .description_text
        .split_whitespace()
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .count();
    words < VAGUE_WORDS && offer.requirements.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::posting::{ApplyHints, WorkMode};
    use crate::scoring::factors::{BenchmarkBasis, SalaryBenchmark};

    const NOW: &str = "2026-10-17T00:00:00Z";

    fn money(amount_minor: i64, currency: &str, period: PayPeriod) -> Money {
        Money {
            amount_minor,
            currency: currency.to_string(),
            period,
        }
    }

    /// A posting with nothing wrong with it: fresh, paid, detailed.
    fn clean() -> Offer {
        Offer {
            id: "pst_01".to_string(),
            source_id: "greenhouse".to_string(),
            external_id: "gh-42".to_string(),
            title: "Senior Data Engineer".to_string(),
            company: "Boba Labs".to_string(),
            locations: vec!["Remote, Spain".to_string()],
            work_mode: WorkMode::Remote,
            money: Some(money(6_000_000, "EUR", PayPeriod::Year)),
            posted_at: Some("2026-10-10T00:00:00Z".to_string()),
            deadline: None,
            first_seen: "2026-10-10T00:00:00Z".to_string(),
            last_seen: NOW.to_string(),
            lang: "en".to_string(),
            description_html: String::new(),
            description_text: "We are looking for a senior data engineer to own our streaming \
                platform. You will design pipelines in dbt and Airflow, partner with analytics, \
                and mentor two engineers. We offer a permanent contract, a learning budget and \
                a fully remote team across Europe. Requirements: five years with Python and SQL, \
                production experience with dbt and Airflow, and strong written English."
                .to_string(),
            requirements: vec!["Python".to_string(), "SQL".to_string()],
            apply: ApplyHints {
                url: "https://boards.example.com/r/gh-42".to_string(),
                email: None,
            },
        }
    }

    fn market() -> MarketData {
        MarketData::default()
    }

    #[test]
    fn a_clean_posting_has_no_signals() {
        let report = detect(&clean(), &market(), NOW);
        assert_eq!(report.signals, Vec::new());
        assert!(!report.blocks_auto());
    }

    // AC2: pay-to-apply is a fraud badge, quoted from the posting.
    #[test]
    fn a_pay_to_apply_phrase_is_a_fraud_badge() {
        let mut offer = clean();
        offer.description_text = "Remote data entry role. A 200 USD application fee is \
            required before the interview."
            .to_string();
        let report = detect(&offer, &market(), NOW);

        let signal = report.find(SignalKind::PayToApply).expect("a badge");
        assert_eq!(signal.severity, Severity::High);
        match &signal.source {
            Source::Text(span) => {
                assert!(span.text.contains("application fee"), "{}", span.text);
                assert_eq!(
                    &offer.description_text[span.start..span.end],
                    span.text,
                    "the span points into the posting"
                );
            }
            other => panic!("expected a quote, got {other:?}"),
        }
        assert!(report.blocks_auto(), "FR-60: a fraud badge blocks Auto");
    }

    #[test]
    fn spanish_pay_to_apply_wording_is_detected_too() {
        let mut offer = clean();
        offer.lang = "es".to_string();
        offer.description_text = "Puesto remoto. Se requiere una tarifa de inscripción para \
            continuar con el proceso."
            .to_string();
        let report = detect(&offer, &market(), NOW);
        assert!(report.find(SignalKind::PayToApply).is_some());
    }

    // AC2: contact moved off platform.
    #[test]
    fn off_platform_contact_is_a_fraud_badge() {
        let mut offer = clean();
        offer.description_text =
            "Send your CV normally. Contact me on WhatsApp to arrange the interview.".to_string();
        let report = detect(&offer, &market(), NOW);

        let signal = report
            .find(SignalKind::OffPlatformContact)
            .expect("a badge");
        assert_eq!(signal.severity, Severity::High);
        assert!(matches!(signal.source, Source::Text(_)));
        assert!(report.blocks_auto());
    }

    // AC2: implausible pay, from the structured field.
    #[test]
    fn implausible_hourly_pay_is_a_fraud_badge() {
        let mut offer = clean();
        offer.money = Some(money(500_000, "USD", PayPeriod::Hour));
        let report = detect(&offer, &market(), NOW);

        let signal = report.find(SignalKind::ImplausiblePay).expect("a badge");
        assert_eq!(signal.source, Source::Field("money".to_string()));
        assert!(report.blocks_auto());
    }

    #[test]
    fn implausible_pay_against_a_benchmark_is_a_fraud_badge() {
        let mut offer = clean();
        offer.money = Some(money(9_000_000, "EUR", PayPeriod::Year));
        let market = MarketData {
            salary: vec![SalaryBenchmark {
                role: "data engineer".to_string(),
                location: "madrid".to_string(),
                currency: "EUR".to_string(),
                period: PayPeriod::Year,
                p50_minor: 1_000_000,
                basis: BenchmarkBasis::User,
                as_of: "2026-09-30T00:00:00Z".to_string(),
            }],
            ..MarketData::default()
        };
        assert!(
            detect(&offer, &market, NOW)
                .find(SignalKind::ImplausiblePay)
                .is_some()
        );
    }

    #[test]
    fn plausible_pay_against_a_benchmark_is_silent() {
        let mut offer = clean();
        offer.money = Some(money(1_500_000, "EUR", PayPeriod::Year));
        let market = MarketData {
            salary: vec![SalaryBenchmark {
                role: "data engineer".to_string(),
                location: "madrid".to_string(),
                currency: "EUR".to_string(),
                period: PayPeriod::Year,
                p50_minor: 1_000_000,
                basis: BenchmarkBasis::User,
                as_of: "2026-09-30T00:00:00Z".to_string(),
            }],
            ..MarketData::default()
        };
        assert!(
            detect(&offer, &market, NOW)
                .find(SignalKind::ImplausiblePay)
                .is_none()
        );
    }

    // AC2: a ghost signal names its source; it does not block Auto.
    #[test]
    fn a_stale_posting_is_a_ghost_signal_with_its_source() {
        let mut offer = clean();
        offer.posted_at = Some("2026-06-01T00:00:00Z".to_string()); // 138 days
        let report = detect(&offer, &market(), NOW);

        let signal = report.find(SignalKind::GhostStalePosting).expect("a badge");
        assert_eq!(signal.severity, Severity::Medium);
        assert_eq!(signal.source, Source::Report(GHOST_REPORT.to_string()));
        assert!(
            !report.blocks_auto(),
            "a ghost signal warns; only fraud blocks Auto"
        );
    }

    #[test]
    fn a_very_stale_posting_is_high_severity() {
        let mut offer = clean();
        offer.posted_at = Some("2026-03-01T00:00:00Z".to_string()); // 230 days
        let report = detect(&offer, &market(), NOW);
        let signal = report.find(SignalKind::GhostStalePosting).expect("a badge");
        assert_eq!(signal.severity, Severity::High);
    }

    #[test]
    fn a_posting_with_no_date_at_all_is_not_called_stale() {
        let mut offer = clean();
        offer.posted_at = None;
        offer.first_seen = "2026-10-10T00:00:00Z".to_string();
        assert!(
            detect(&offer, &market(), NOW)
                .find(SignalKind::GhostStalePosting)
                .is_none()
        );
    }

    #[test]
    fn a_vague_description_is_a_ghost_signal() {
        let mut offer = clean();
        offer.description_text = "We are hiring. Great team and great pay.".to_string();
        offer.requirements.clear();
        let report = detect(&offer, &market(), NOW);

        let signal = report
            .find(SignalKind::GhostVagueDescription)
            .expect("a badge");
        assert_eq!(signal.source, Source::Field("description_text".to_string()));
    }

    #[test]
    fn a_detailed_description_is_not_vague() {
        assert!(
            detect(&clean(), &market(), NOW)
                .find(SignalKind::GhostVagueDescription)
                .is_none()
        );
    }

    #[test]
    fn no_salary_is_a_ghost_signal() {
        let mut offer = clean();
        offer.money = None;
        let report = detect(&offer, &market(), NOW);
        let signal = report.find(SignalKind::GhostNoSalary).expect("a badge");
        assert_eq!(signal.source, Source::Field("money".to_string()));
        assert!(!report.blocks_auto());
    }

    #[test]
    fn talent_pool_wording_is_quoted() {
        let mut offer = clean();
        offer.description_text = "This is an evergreen posting for our talent pool. We keep \
            applications on file for future roles."
            .to_string();
        let report = detect(&offer, &market(), NOW);
        let signal = report.find(SignalKind::GhostTalentPool).expect("a badge");
        match &signal.source {
            Source::Text(span) => assert!(
                span.text.to_lowercase().contains("evergreen"),
                "{}",
                span.text
            ),
            other => panic!("expected a quote, got {other:?}"),
        }
    }

    #[test]
    fn detection_is_deterministic() {
        let mut offer = clean();
        offer.posted_at = Some("2026-06-01T00:00:00Z".to_string());
        offer.description_text =
            "Apply now. A registration fee applies. Contact me on Telegram.".to_string();
        let first = detect(&offer, &market(), NOW);
        assert_eq!(first, detect(&offer, &market(), NOW));
        assert!(!first.signals.is_empty());
    }

    // The report lists fraud first, then ghost signals, in one stable order.
    #[test]
    fn signals_are_in_a_stable_order() {
        let mut offer = clean();
        offer.posted_at = Some("2026-06-01T00:00:00Z".to_string());
        offer.description_text =
            "Apply now. A registration fee applies. Contact me on Telegram.".to_string();
        let report = detect(&offer, &market(), NOW);
        let kinds: Vec<SignalKind> = report.signals.iter().map(|signal| signal.kind).collect();
        let mut sorted = kinds.clone();
        sorted.sort();
        assert_eq!(kinds, sorted, "fraud before ghost, document order");
    }

    #[test]
    fn every_badge_id_is_stable() {
        assert_eq!(SignalKind::PayToApply.as_str(), "pay_to_apply");
        assert_eq!(
            SignalKind::OffPlatformContact.as_str(),
            "off_platform_contact"
        );
        assert_eq!(SignalKind::ImplausiblePay.as_str(), "implausible_pay");
        assert_eq!(
            SignalKind::GhostStalePosting.as_str(),
            "ghost_stale_posting"
        );
        assert_eq!(
            SignalKind::GhostVagueDescription.as_str(),
            "ghost_vague_description"
        );
        assert_eq!(SignalKind::GhostNoSalary.as_str(), "ghost_no_salary");
        assert_eq!(SignalKind::GhostTalentPool.as_str(), "ghost_talent_pool");
        assert!(SignalKind::PayToApply.is_fraud());
        assert!(!SignalKind::GhostNoSalary.is_fraud());
    }

    #[test]
    fn a_year_of_pay_is_the_unit_the_thresholds_read() {
        // 100k/hour is exactly the implausible line (minor units).
        assert_eq!(
            yearly_minor(&money(100_000, "USD", PayPeriod::Hour)),
            208_000_000
        );
        assert_eq!(
            yearly_minor(&money(8_333_333, "USD", PayPeriod::Month)),
            99_999_996
        );
        assert_eq!(
            yearly_minor(&money(2_000_000, "USD", PayPeriod::Year)),
            2_000_000
        );
    }
}
