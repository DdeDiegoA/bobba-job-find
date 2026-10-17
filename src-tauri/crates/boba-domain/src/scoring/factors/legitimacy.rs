//! Q4 posting legitimacy / anti-ghost (FR-56, `offer-scoring.md` §5): is this a
//! real, live opening, or one nobody is filling?
//!
//! The research document's rule shapes the whole factor: **start at 100 and
//! subtract**, exposing every signal that fired, and never label a single
//! posting a "ghost job". A posting with two or three signals is deprioritised,
//! not deleted (FR-54's Excluded list stays for Gates, not for this).
//!
//! The signals are the ones the app already holds (architecture §7: "ghost-job
//! signals come from data we already hold"): the posting's **age** against the
//! injected clock (the strongest signal — 27.6% of open postings are dated more
//! than 90 days), whether it **shows its pay**, and the **red-flag phrases**
//! S6.2 read out of its own words, each with the sentence it came from.
//!
//! Repost history (S4.4's `posting_source` rows) and the employer-posted bonus
//! an ATS board earns are not here: neither has reached this slice's inputs.
//! When S4.4's signals arrive, they join the same list.
//!
//! Nothing is guessed: an unreadable date or a clock that cannot be parsed costs
//! **confidence**, never score (AC2).

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::posting_features::FeatureKind;

use super::*;

/// Days a posting may sit before the penalties start, and where they steepen
/// (`offer-scoring.md` §5's own thresholds).
const AGING_DAYS: i64 = 60;
const STALE_DAYS: i64 = 90;
const AGING_PENALTY: f32 = 10.0;
const STALE_PENALTY: f32 = 25.0;

/// A posting that hides its pay is a weaker signal than age, but a signal.
const NO_PAY_PENALTY: f32 = 10.0;

/// Each red-flag phrase, and the cap on all of them together (§5: two or three
/// strong signals deprioritise a posting; they do not delete it).
const RED_FLAG_PENALTY: f32 = 15.0;
const RED_FLAG_CAP: f32 = 30.0;

/// Whole days from `then` to `now`, both UTC RFC 3339. `None` when either is
/// not a timestamp this build can read.
fn days_between(now: &str, then: &str) -> Option<i64> {
    let now = OffsetDateTime::parse(now, &Rfc3339).ok()?;
    let then = OffsetDateTime::parse(then, &Rfc3339).ok()?;
    Some((now - then).whole_days())
}

pub(super) fn score(inputs: &ScoreInputs<'_>, weight: f32) -> FactorScore {
    // A row with no date, no pay and no text at all gives the factor nothing to
    // judge: every signal it reads is an absence, and subtracting from a
    // starting 100 would score it a clean 90 (AC2: no answer is no answer).
    if inputs.posting.posted_at.is_none()
        && inputs.posting.money.is_none()
        && inputs.posting.description_text.trim().is_empty()
    {
        return FactorScore::missing_input(
            Factor::Legitimacy,
            weight,
            "the posting has no date, no pay and no text to read",
        );
    }

    let mut penalty = 0.0;
    let mut triggered: Vec<String> = Vec::new();
    let mut evidence: Vec<Evidence> = Vec::new();

    let age = inputs
        .posting
        .posted_at
        .as_deref()
        .and_then(|posted_at| days_between(inputs.now, posted_at));
    match age {
        Some(days) if days > STALE_DAYS => {
            penalty += STALE_PENALTY;
            triggered.push(format!("{days} days old"));
            evidence.push(Evidence::Field("posted_at".to_string()));
        }
        Some(days) if days >= AGING_DAYS => {
            penalty += AGING_PENALTY;
            triggered.push(format!("{days} days old"));
            evidence.push(Evidence::Field("posted_at".to_string()));
        }
        _ => {}
    }

    if inputs.posting.money.is_none() {
        penalty += NO_PAY_PENALTY;
        triggered.push("no pay shown".to_string());
        evidence.push(Evidence::Field("money".to_string()));
    }

    let red_flags: Vec<&crate::posting_features::Claim> =
        inputs.features.of_kind(FeatureKind::RedFlag).collect();
    let red_flag_penalty = (RED_FLAG_PENALTY * red_flags.len() as f32).min(RED_FLAG_CAP);
    penalty += red_flag_penalty;
    for claim in &red_flags {
        triggered.push(claim.value.trim().to_string());
        evidence.push(claim.evidence.clone());
    }

    // The age is the strongest signal §5 has: without it the factor still
    // answers, but on less.
    let confidence = if age.is_some() {
        Confidence::High
    } else {
        Confidence::Medium
    };

    FactorScore::scored(
        Factor::Legitimacy,
        100.0 - penalty,
        weight,
        confidence,
        evidence,
    )
    .with_missing(triggered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::posting_features::PostingFeatures;
    use crate::scoring::factors::testing::*;

    fn q4(fixture: &Fixture) -> FactorScore {
        score(
            &fixture.inputs(),
            fixture.prefs.weights.of(Factor::Legitimacy),
        )
    }

    /// The posting as if it had been published `days` before the injected clock.
    fn posted(fixture: &mut Fixture, days: i64) {
        let at = time::OffsetDateTime::parse(NOW, &time::format_description::well_known::Rfc3339)
            .expect("NOW parses")
            - time::Duration::days(days);
        fixture.posting.posted_at = Some(
            at.format(&time::format_description::well_known::Rfc3339)
                .expect("formats"),
        );
    }

    // §9: posted twelve days ago, with the pay shown and no red-flag wording —
    // the example's "fresh, employer-posted, salary shown".
    #[test]
    fn the_worked_example_is_a_clean_posting() {
        let factor = q4(&Fixture::section_9());
        assert_eq!(factor.score, Some(100.0));
        assert_eq!(factor.confidence, Confidence::High);
        assert_eq!(factor.matched, Vec::<String>::new());
        assert_eq!(factor.missing, Vec::<String>::new());
    }

    #[test]
    fn a_posting_over_ninety_days_old_is_the_strong_signal() {
        let mut fixture = Fixture::section_9();
        posted(&mut fixture, 100);
        let factor = q4(&fixture);
        assert_eq!(factor.score, Some(75.0));
        assert_eq!(factor.missing, vec!["100 days old".to_string()]);
    }

    #[test]
    fn a_posting_in_its_second_month_is_the_weaker_signal() {
        let mut fixture = Fixture::section_9();
        posted(&mut fixture, 70);
        let factor = q4(&fixture);
        assert_eq!(factor.score, Some(90.0));
        assert_eq!(factor.missing, vec!["70 days old".to_string()]);
    }

    #[test]
    fn pay_withheld_costs_the_legitimacy_score_too() {
        let mut fixture = Fixture::section_9();
        fixture.posting.money = None;
        let factor = q4(&fixture);
        assert_eq!(factor.score, Some(90.0));
        assert_eq!(factor.missing, vec!["no pay shown".to_string()]);
    }

    #[test]
    fn enough_red_flag_phrases_top_out_the_penalty() {
        let mut fixture = Fixture::section_9();
        fixture.features = PostingFeatures::new("pst_01SECTION9", 3);
        fixture.features.extend([
            red_flag("evergreen"),
            red_flag("talent pool"),
            red_flag("pending funding"),
            red_flag("subject to award"),
        ]);
        let factor = q4(&fixture);
        assert_eq!(factor.score, Some(70.0));
        assert_eq!(factor.missing.len(), 4);
        assert_eq!(factor.evidence.len(), 4);
    }

    #[test]
    fn two_red_flag_phrases_are_two_penalties() {
        let mut fixture = Fixture::section_9();
        fixture.features = PostingFeatures::new("pst_01SECTION9", 3);
        fixture
            .features
            .extend([red_flag("evergreen"), red_flag("talent pool")]);
        assert_eq!(q4(&fixture).score, Some(70.0));
    }

    // A posting with no date is not a suspicious posting: the strongest signal
    // is simply not readable, which costs confidence, never score (AC2).
    #[test]
    fn an_unknown_age_costs_confidence_rather_than_score() {
        let mut fixture = Fixture::section_9();
        fixture.posting.posted_at = None;
        let factor = q4(&fixture);
        assert_eq!(factor.score, Some(100.0));
        assert_eq!(factor.confidence, Confidence::Medium);
    }

    #[test]
    fn a_clock_that_cannot_be_read_leaves_the_age_unknown() {
        let mut fixture = Fixture::section_9();
        fixture.now = "not a timestamp".to_string();
        let factor = q4(&fixture);
        assert_eq!(factor.score, Some(100.0));
        assert_eq!(factor.confidence, Confidence::Medium);
    }

    #[test]
    fn a_future_posting_date_is_not_a_penalty() {
        let mut fixture = Fixture::section_9();
        posted(&mut fixture, -3);
        assert_eq!(q4(&fixture).score, Some(100.0));
    }

    // AC2: nothing to read is nothing to judge — never a clean 90.
    #[test]
    fn a_posting_with_nothing_to_read_leaves_the_factor_missing() {
        let mut fixture = Fixture::section_9();
        fixture.posting.posted_at = None;
        fixture.posting.money = None;
        fixture.posting.description_text = "  ".to_string();
        let factor = q4(&fixture);
        assert_eq!(factor.score, None);
        assert_eq!(
            factor.missing,
            vec!["the posting has no date, no pay and no text to read".to_string()]
        );
    }
}
