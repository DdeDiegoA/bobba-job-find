//! Offer model (FR-48, AD-17, AD-19).
//!
//! One `RawPosting` from any source becomes one [`Offer`] — the shape ranking,
//! tailoring and the UI all read (FR-48). Every field is normalised here, but
//! nothing is invented: a missing salary or deadline stays `None`, and the raw
//! payload is kept as [`Provenance`], never thrown away (AD-19).
//!
//! Money is one [`Money`] (`amount_minor` + ISO-4217 + period, AD-17); a posted
//! range keeps its **lower bound**, the number the comp-floor gate compares
//! against. Language is a BCP-47 tag; the detector itself lives in `boba-app`
//! (this crate stays free of dependencies).

use serde::{Deserialize, Serialize};

/// A fresh ULID for a posting, prefixed by type (AD-17: `pst_…`).
pub fn new_posting_id() -> String {
    format!("pst_{}", ulid::Ulid::generate())
}

/// What the pay is for (AD-17).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayPeriod {
    Year,
    Month,
    Hour,
}

impl PayPeriod {
    pub const ALL: [PayPeriod; 3] = [Self::Year, Self::Month, Self::Hour];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Year => "year",
            Self::Month => "month",
            Self::Hour => "hour",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.as_str() == value)
    }
}

/// Pay as stored and shown (AD-17).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Money {
    pub amount_minor: i64,
    /// ISO-4217.
    pub currency: String,
    pub period: PayPeriod,
}

const CURRENCY_CODES: &[&str] = &[
    "USD", "EUR", "GBP", "CAD", "AUD", "NZD", "CHF", "SEK", "NOK", "DKK", "PLN", "CZK", "RON",
    "BRL", "MXN", "ARS", "COP", "CLP", "PEN", "INR", "JPY", "SGD", "AED", "ZAR",
];

impl Money {
    /// Read money out of a free-text salary line (FR-48). `None` when the line
    /// has no number at all; the currency defaults to USD and the period to a
    /// year, which is what boards mean when they say neither.
    pub fn parse(text: &str) -> Option<Self> {
        Self::read(text, false)
    }

    /// Read money out of prose, such as a posting description. A bare number
    /// there is usually not pay ("Founded in 2015"), so this asks for an
    /// explicit marker — a currency code or symbol, or a pay period — before it
    /// believes anything.
    pub fn parse_hinted(text: &str) -> Option<Self> {
        Self::read(text, true)
    }

    fn read(text: &str, needs_marker: bool) -> Option<Self> {
        let currency = currency(text);
        let period = period(text);
        if needs_marker && currency.is_none() && period.is_none() {
            return None;
        }
        Some(Self {
            amount_minor: first_amount_minor(text)?,
            // `$` is shared by USD/CAD/AUD/MXN; USD is the default (markets: FR-1).
            currency: currency.unwrap_or_else(|| "USD".to_string()),
            period: period.unwrap_or(PayPeriod::Year),
        })
    }
}

/// The lower bound of the first number in the line, in minor units.
fn first_amount_minor(text: &str) -> Option<i64> {
    let chars: Vec<char> = text.chars().collect();
    for start in 0..chars.len() {
        if !chars[start].is_ascii_digit() {
            continue;
        }
        let mut digits = String::new();
        let mut i = start;
        while i < chars.len() {
            let c = chars[i];
            if c.is_ascii_digit() {
                digits.push(c);
                i += 1;
            } else if is_group_separator(&chars, i) {
                i += 1;
            } else {
                break;
            }
        }
        let mut value: i64 = digits.parse().ok()?;
        // `50k` / `50K`: a thousands shorthand, never a year of `50`.
        if value < 1000 && matches!(chars.get(i), Some('k' | 'K')) {
            value = value.checked_mul(1000)?;
        }
        // Untrusted text: an 18-digit number would overflow minor units.
        return value.checked_mul(100).filter(|minor| *minor > 0);
    }
    None
}

/// `1,234,567` and `45.000` group in threes; `1.5` does not.
fn is_group_separator(chars: &[char], i: usize) -> bool {
    if !matches!(chars[i], ',' | '.' | '_') {
        return false;
    }
    let rest = &chars[i + 1..];
    rest.len() >= 3
        && rest[..3].iter().all(char::is_ascii_digit)
        && !rest.get(3).is_some_and(char::is_ascii_digit)
}

fn currency(text: &str) -> Option<String> {
    let upper = text.to_uppercase();
    let chars: Vec<char> = upper.chars().collect();
    for code in CURRENCY_CODES {
        let bytes = code.as_bytes();
        for (i, _) in upper.match_indices(code) {
            let before = i.checked_sub(1).and_then(|b| chars.get(b));
            let after = chars.get(i + bytes.len());
            let bounded = |c: &char| !c.is_ascii_alphanumeric();
            if before.is_none_or(bounded) && after.is_none_or(bounded) {
                return Some((*code).to_string());
            }
        }
    }
    if text.contains('€') {
        Some("EUR".to_string())
    } else if text.contains('£') {
        Some("GBP".to_string())
    } else if text.contains("R$") {
        Some("BRL".to_string())
    } else if text.contains('$') {
        Some("USD".to_string())
    } else {
        None
    }
}

fn period(text: &str) -> Option<PayPeriod> {
    let lower = text.to_lowercase();
    // Spanish and English share these boards (FR-1); the keywords are ordered
    // hour → month → year so `per hour` never reads as a year. Month must stay
    // before year: `mensual` contains `anual`.
    for keyword in ["hour", "hr", "por hora", "hora"] {
        if lower.contains(keyword) {
            return Some(PayPeriod::Hour);
        }
    }
    for keyword in ["month", "/mo", "mensual", "al mes", "por mes"] {
        if lower.contains(keyword) {
            return Some(PayPeriod::Month);
        }
    }
    for keyword in ["year", "annual", "annum", "año", "anual", "yr"] {
        if lower.contains(keyword) {
            return Some(PayPeriod::Year);
        }
    }
    None
}

/// The UTC RFC 3339 form of a timestamp a source sent (AD-17), or `None` when
/// it is not one. Sources are untrusted input: an offset is converted rather
/// than stored, and a string no consumer could parse is dropped instead of
/// poisoning the age and repost signals of S4.4.
pub fn utc_rfc3339(value: &str) -> Option<String> {
    let instant =
        time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339).ok()?;
    instant
        .to_offset(time::UtcOffset::UTC)
        .format(&time::format_description::well_known::Rfc3339)
        .ok()
}

/// How the job is worked (FR-48). `Hybrid` is a later refinement: a source that
/// only tells us `remote: false` cannot be read as anything but onsite today.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkMode {
    Remote,
    Hybrid,
    Onsite,
    Unknown,
}

impl WorkMode {
    pub const ALL: [WorkMode; 4] = [Self::Remote, Self::Hybrid, Self::Onsite, Self::Unknown];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Remote => "remote",
            Self::Hybrid => "hybrid",
            Self::Onsite => "onsite",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|w| w.as_str() == value)
    }

    /// A source's remote flag, as far as it goes.
    pub fn from_remote(remote: Option<bool>) -> Self {
        match remote {
            Some(true) => Self::Remote,
            Some(false) => Self::Onsite,
            None => Self::Unknown,
        }
    }
}

/// Where and how to apply (FR-48: apply-channel hints).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyHints {
    /// The posting's canonical URL.
    pub url: String,
    /// An apply address found in the posting text, when there is one.
    pub email: Option<String>,
}

/// A normalised Offer (FR-48, AD-17).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Offer {
    pub id: String,
    /// The source this Offer was first seen on.
    pub source_id: String,
    pub external_id: String,
    pub title: String,
    pub company: String,
    pub locations: Vec<String>,
    pub work_mode: WorkMode,
    pub money: Option<Money>,
    pub posted_at: Option<String>,
    pub deadline: Option<String>,
    /// UTC RFC 3339 (AD-17).
    pub first_seen: String,
    pub last_seen: String,
    /// BCP-47 content language of the posting.
    pub lang: String,
    /// Sanitised HTML (AD-7); the only form the UI may render.
    pub description_html: String,
    /// The same description as plain text, for scoring and dedup.
    pub description_text: String,
    pub requirements: Vec<String>,
    pub apply: ApplyHints,
}

/// The raw payload one source contributed to an Offer (AD-19: dedup merges
/// Offers, it never deletes provenance).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub posting_id: String,
    pub source_id: String,
    pub external_id: String,
    pub url: String,
    /// The source's payload verbatim. Untrusted (AD-7).
    pub raw: String,
    pub fetched_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_parses_a_range_with_code_and_period() {
        assert_eq!(
            Money::parse("USD 50,000 - 70,000 per year"),
            Some(Money {
                amount_minor: 5_000_000,
                currency: "USD".into(),
                period: PayPeriod::Year,
            })
        );
    }

    #[test]
    fn money_parses_a_symbol_and_an_hourly_period() {
        assert_eq!(
            Money::parse("$45/hour"),
            Some(Money {
                amount_minor: 4_500,
                currency: "USD".into(),
                period: PayPeriod::Hour,
            })
        );
    }

    #[test]
    fn money_parses_european_separators_and_a_spanish_month() {
        assert_eq!(
            Money::parse("45.000 € al mes"),
            Some(Money {
                amount_minor: 4_500_000,
                currency: "EUR".into(),
                period: PayPeriod::Month,
            })
        );
    }

    #[test]
    fn money_parses_a_k_suffix_and_an_english_year() {
        assert_eq!(
            Money::parse("50k EUR annual"),
            Some(Money {
                amount_minor: 5_000_000,
                currency: "EUR".into(),
                period: PayPeriod::Year,
            })
        );
    }

    #[test]
    fn money_defaults_to_a_year_when_no_period_is_given() {
        assert_eq!(
            Money::parse("£60,000"),
            Some(Money {
                amount_minor: 6_000_000,
                currency: "GBP".into(),
                period: PayPeriod::Year,
            })
        );
    }

    #[test]
    fn money_is_none_without_a_number() {
        assert_eq!(Money::parse("Competitive salary"), None);
        assert_eq!(Money::parse(""), None);
    }

    #[test]
    fn money_in_prose_needs_an_explicit_marker() {
        // A description is full of numbers that are not pay (review finding 1).
        assert_eq!(Money::parse_hinted("Founded in 2015, 40 engineers."), None);
        assert_eq!(Money::parse_hinted("Team of 20 in Madrid."), None);
        assert_eq!(Money::parse_hinted("Competitive salary"), None);

        let euros = Money::parse_hinted("Compensation: 45.000 € al mes").unwrap();
        assert_eq!(euros.currency, "EUR");
        assert_eq!(euros.period, PayPeriod::Month);

        let year = Money::parse_hinted("50,000 a year plus equity").unwrap();
        assert_eq!(year.amount_minor, 5_000_000, "the year marker is enough");
        assert_eq!(year.period, PayPeriod::Year);
    }

    #[test]
    fn money_does_not_overflow_on_absurd_numbers() {
        assert_eq!(Money::parse("USD 9999999999999999999 per year"), None);
        assert_eq!(Money::parse("123456789012345678"), None, "18 digits * 100");
        assert_eq!(
            Money::parse_hinted("salary: $123456789012345678"),
            None,
            "untrusted text must not panic"
        );
    }

    #[test]
    fn posted_dates_are_normalised_to_utc_rfc3339() {
        assert_eq!(
            utc_rfc3339("2026-10-01T00:00:00Z").as_deref(),
            Some("2026-10-01T00:00:00Z")
        );
        assert_eq!(
            utc_rfc3339("2026-10-01T02:00:00+02:00").as_deref(),
            Some("2026-10-01T00:00:00Z"),
            "an offset is converted, not kept (AD-17)"
        );
        assert_eq!(utc_rfc3339("2026-10-01"), None, "a date is not a timestamp");
        assert_eq!(utc_rfc3339("last Tuesday"), None);
    }

    #[test]
    fn work_mode_maps_the_remote_flag() {
        assert_eq!(WorkMode::from_remote(Some(true)), WorkMode::Remote);
        assert_eq!(WorkMode::from_remote(Some(false)), WorkMode::Onsite);
        assert_eq!(WorkMode::from_remote(None), WorkMode::Unknown);
    }

    #[test]
    fn a_posting_id_is_a_prefixed_ulid() {
        let id = new_posting_id();
        assert!(id.starts_with("pst_"), "{id}");
        assert_eq!(id.len(), 4 + 26, "{id}");
        assert_ne!(id, new_posting_id());
    }
}
