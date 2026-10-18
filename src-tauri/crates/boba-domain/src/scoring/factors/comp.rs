//! Q1 compensation vs market (FR-56, `offer-scoring.md` §4): how the posted pay
//! compares with what this role, level and place pays.
//!
//! The comparison is `posted / p50`, clamped to 0.5–1.5 and mapped onto 0–100 —
//! the research document's own formula, with parity at 50 and 1.5× the market at
//! the top. The basis is always named in `matched` ("your own table", BLS OES,
//! Adzuna), because a number the user cannot trace is a number they cannot
//! argue with (FR-56).
//!
//! What the factor refuses to do matters as much:
//!
//! * a benchmark in another currency or period is **not** compared — no exchange
//!   rate is bundled in this slice, and a guessed conversion is a wrong number
//!   dressed as a fact (architecture §8: rates arrive with the importer);
//! * a posting with no pay, or a role with no benchmark, is a missing input, so
//!   the axis renormalises instead of inventing a market (AC2).
//!
//! The Offer keeps a posted range's **lower bound** (FR-48), and that is what is
//! compared: the conservative end of the range, never a midpoint nobody posted.

use crate::posting::{Money, minor_unit_scale};

use super::*;

/// The ratio the scale starts at (half the market) and ends at (one and a half).
const FLOOR_RATIO: f32 = 0.5;
const CEILING_RATIO: f32 = 1.5;

/// Money as the factor names it: major units, ISO code, period.
pub(super) fn money_phrase(money: &Money) -> String {
    format!(
        "{} {}/{}",
        money.amount_minor / minor_unit_scale(&money.currency),
        money.currency,
        money.period.as_str()
    )
}

/// The benchmark for this role and place: the longest role name the Offer's
/// title contains, at a place the Offer names or the user accepts, preferring
/// the user's own table over the public sources (`offer-scoring.md` §4's order).
fn benchmark_for<'a>(inputs: &ScoreInputs<'a>) -> Option<&'a SalaryBenchmark> {
    let title = inputs.posting.title.trim().to_lowercase();
    let places: Vec<String> = inputs
        .posting
        .locations
        .iter()
        .chain(inputs.prefs.locations.iter())
        .map(|place| place.trim().to_lowercase())
        .filter(|place| !place.is_empty())
        .collect();

    let mut candidates: Vec<&SalaryBenchmark> = inputs
        .market
        .salary
        .iter()
        .filter(|benchmark| {
            let role = benchmark.role.trim().to_lowercase();
            !role.is_empty() && title.contains(&role)
        })
        .filter(|benchmark| {
            let location = benchmark.location.trim().to_lowercase();
            !location.is_empty() && places.iter().any(|place| place.contains(&location))
        })
        .collect();
    let basis = BenchmarkBasis::ALL
        .into_iter()
        .find(|basis| candidates.iter().any(|benchmark| benchmark.basis == *basis))?;
    candidates.retain(|benchmark| benchmark.basis == basis);
    candidates
        .into_iter()
        .max_by_key(|benchmark| benchmark.role.trim().len())
}

pub(super) fn score(inputs: &ScoreInputs<'_>, weight: f32) -> FactorScore {
    let Some(posted) = inputs.posting.money.as_ref() else {
        return FactorScore::missing_input(
            Factor::Compensation,
            weight,
            "the posting shows no pay",
        );
    };
    let Some(benchmark) = benchmark_for(inputs) else {
        return FactorScore::missing_input(
            Factor::Compensation,
            weight,
            "no benchmark for this role, level and place",
        );
    };
    if benchmark.currency != posted.currency || benchmark.period != posted.period {
        return FactorScore::missing_input(
            Factor::Compensation,
            weight,
            &format!(
                "no {}/{} benchmark for this role and place",
                posted.currency,
                posted.period.as_str()
            ),
        );
    }
    if benchmark.p50_minor <= 0 {
        return FactorScore::missing_input(
            Factor::Compensation,
            weight,
            "the benchmark states no median",
        );
    }

    let ratio = posted.amount_minor as f32 / benchmark.p50_minor as f32;
    let score = 100.0 * ((ratio - FLOOR_RATIO) / (CEILING_RATIO - FLOOR_RATIO)).clamp(0.0, 1.0);

    let median = Money {
        amount_minor: benchmark.p50_minor,
        currency: benchmark.currency.clone(),
        period: benchmark.period,
    };
    FactorScore::scored(
        Factor::Compensation,
        score,
        weight,
        Confidence::High,
        vec![
            Evidence::Field("money".to_string()),
            Evidence::Field("market_benchmark".to_string()),
        ],
    )
    .with_matched(vec![format!(
        "{} p50 ({}) against {} posted",
        money_phrase(&median),
        benchmark.basis.as_str(),
        money_phrase(posted)
    )])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::posting::PayPeriod;
    use crate::scoring::factors::testing::*;

    fn q1(fixture: &Fixture) -> FactorScore {
        score(
            &fixture.inputs(),
            fixture.prefs.weights.of(Factor::Compensation),
        )
    }

    /// The offer's pay, in the AD-17 shape the Offer keeps: amount, currency,
    /// period (a posted range keeps its lower bound, FR-48).
    fn paid(fixture: &mut Fixture, amount_minor: i64, currency: &str, period: PayPeriod) {
        fixture.posting.money = Some(Money {
            amount_minor,
            currency: currency.to_string(),
            period,
        });
    }

    // §9: 55 000 EUR against a Madrid p50 of 52 000 — just above the market.
    #[test]
    fn the_worked_example_sits_just_above_its_benchmark() {
        let factor = q1(&Fixture::section_9());
        let score = factor.score.expect("a score");
        assert!(score > 53.0 && score < 59.0, "{score}");
        assert_eq!(factor.confidence, Confidence::High);
        assert!(!factor.matched.is_empty());
        assert_eq!(factor.missing, Vec::<String>::new());
    }

    #[test]
    fn parity_is_the_middle_and_one_and_a_half_times_the_market_is_the_top() {
        let mut fixture = Fixture::section_9();
        paid(&mut fixture, 5_200_000, "EUR", PayPeriod::Year);
        assert_eq!(q1(&fixture).score, Some(50.0));

        paid(&mut fixture, 7_800_000, "EUR", PayPeriod::Year);
        assert_eq!(q1(&fixture).score, Some(100.0));

        paid(&mut fixture, 6_500_000, "EUR", PayPeriod::Year);
        assert_eq!(q1(&fixture).score, Some(75.0));
    }

    #[test]
    fn half_the_market_is_the_floor_of_the_scale() {
        let mut fixture = Fixture::section_9();
        paid(&mut fixture, 2_600_000, "EUR", PayPeriod::Year);
        assert_eq!(q1(&fixture).score, Some(0.0));
    }

    #[test]
    fn a_posting_that_shows_no_pay_leaves_the_factor_missing() {
        let mut fixture = Fixture::section_9();
        fixture.posting.money = None;
        let factor = q1(&fixture);
        assert_eq!(factor.score, None);
        assert_eq!(factor.effective_weight, 0.0);
        assert_eq!(factor.missing, vec!["the posting shows no pay".to_string()]);
    }

    #[test]
    fn a_role_with_no_benchmark_leaves_the_factor_missing() {
        let mut fixture = Fixture::section_9();
        fixture.market.salary = Vec::new();
        let factor = q1(&fixture);
        assert_eq!(factor.score, None);
        assert_eq!(
            factor.missing,
            vec!["no benchmark for this role, level and place".to_string()]
        );
    }

    // Nothing is converted by guesswork: another currency is a missing input,
    // not a made-up ratio (architecture §8: rates arrive with a later slice).
    #[test]
    fn a_benchmark_in_another_currency_is_not_compared() {
        let mut fixture = Fixture::section_9();
        paid(&mut fixture, 5_500_000, "USD", PayPeriod::Year);
        let factor = q1(&fixture);
        assert_eq!(factor.score, None);
        assert!(
            factor.missing[0].contains("no USD/year benchmark"),
            "{:?}",
            factor.missing
        );
    }

    #[test]
    fn a_benchmark_for_another_period_is_not_compared() {
        let mut fixture = Fixture::section_9();
        paid(&mut fixture, 4_500, "EUR", PayPeriod::Hour);
        assert_eq!(q1(&fixture).score, None);
    }

    #[test]
    fn the_benchmark_basis_travels_with_the_factor() {
        let factor = q1(&Fixture::section_9());
        let phrase = factor.matched.join(" ");
        assert!(phrase.contains("user"), "{phrase}");
        assert!(phrase.contains("52000"), "{phrase}");
    }

    #[test]
    fn a_benchmark_for_a_role_the_title_does_not_name_is_not_used() {
        let mut fixture = Fixture::section_9();
        fixture.market.salary[0].role = "product designer".to_string();
        assert_eq!(q1(&fixture).score, None);
    }
}
