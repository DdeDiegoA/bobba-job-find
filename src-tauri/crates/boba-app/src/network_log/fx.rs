//! FX rates, opt-in (S10.4, FR-115, NFR-1 e).
//!
//! Conversions are a second line under a salary and never replace it. They
//! exist only when the user turned FX on, and they always carry the date of the
//! rate they used, so a stale rate can never masquerade as today's. The rate
//! comes from one free, keyless public source; nothing else is ever asked for a
//! rate, and with the setting off the app makes no FX request at all.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ports::repo::system::{SystemRepo, SystemRepoError};

/// The one `setting` row FX lives in (opt-in, display currency, last rates).
pub const SETTING_KEY: &str = "network.fx";

/// The free, keyless public source (FR-115 assumption): no account, one GET.
pub const SOURCE_HOST: &str = "api.frankfurter.dev";

/// The display currency Boba falls back to before the user picks one.
pub const DEFAULT_TARGET: &str = "USD";

/// Where the latest rates for `target` come from, as the URL to GET.
pub fn source_url(target: &str) -> String {
    format!("https://{SOURCE_HOST}/v1/latest?base={target}")
}

/// The opt-in and the last rates fetched. Stored as one JSON row.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, Type)]
pub struct FxSettings {
    /// The user turned conversions on (AC2: off by default).
    pub enabled: bool,
    /// The currency salaries are converted *to* (ISO-4217, upper case).
    pub target: String,
    /// The date of the rates below, as the source published it.
    pub as_of: Option<String>,
    /// `1 target = rates[C] C` for every currency the source named.
    pub rates: BTreeMap<String, f64>,
}

/// One converted amount, always labelled with the date of its rate (AC2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Converted {
    /// Minor units of the display currency.
    pub amount_minor: i64,
    pub currency: String,
    /// The rate's date, e.g. `2026-10-16`.
    pub as_of: String,
}

/// What one `latest` answer said, before it is stored.
#[derive(Debug, Clone, PartialEq)]
pub struct FxQuote {
    pub as_of: String,
    pub rates: BTreeMap<String, f64>,
}

/// Why FX could not be read, parsed or stored.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FxError {
    #[error("store error: {0}")]
    Store(String),
    #[error("invalid FX answer: {0}")]
    Invalid(String),
}

impl FxError {
    /// The i18n key the UI shows (AD-3: keys cross IPC, never raw text).
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::Store(_) => "errors.network.fx_store",
            Self::Invalid(_) => "errors.network.fx_invalid",
        }
    }
}

impl From<SystemRepoError> for FxError {
    fn from(error: SystemRepoError) -> Self {
        Self::Store(error.to_string())
    }
}

/// The stored setting; off with empty rates when nothing was saved or the row
/// cannot be read (tolerance on read, like every settings document).
pub fn load(system: &dyn SystemRepo) -> Result<FxSettings, FxError> {
    let Some(raw) = system.setting_get(SETTING_KEY)? else {
        return Ok(FxSettings::default());
    };
    Ok(serde_json::from_str(&raw).unwrap_or_default())
}

/// Store the setting (the caller owns the timestamp).
pub fn save(system: &dyn SystemRepo, settings: &FxSettings, now: &str) -> Result<(), FxError> {
    let json =
        serde_json::to_string(settings).map_err(|error| FxError::Invalid(error.to_string()))?;
    system.setting_put(SETTING_KEY, &json, now)?;
    Ok(())
}

/// Parse a `latest` answer. The base must be the currency the rates are meant
/// to be relative to, and at least one rate must come back.
pub fn parse_latest(json: &str, target: &str) -> Result<FxQuote, FxError> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|error| FxError::Invalid(error.to_string()))?;
    let base = value
        .get("base")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| FxError::Invalid("no base".to_string()))?;
    if !base.eq_ignore_ascii_case(target) {
        return Err(FxError::Invalid(format!("base {base} is not {target}")));
    }
    let as_of = value
        .get("date")
        .and_then(serde_json::Value::as_str)
        .filter(|date| !date.is_empty())
        .ok_or_else(|| FxError::Invalid("no date".to_string()))?
        .to_string();
    let rates = value
        .get("rates")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| FxError::Invalid("no rates".to_string()))?;
    let mut parsed = BTreeMap::new();
    for (currency, rate) in rates {
        let Some(rate) = rate.as_f64().filter(|rate| *rate > 0.0) else {
            continue;
        };
        parsed.insert(currency.to_ascii_uppercase(), rate);
    }
    if parsed.is_empty() {
        return Err(FxError::Invalid("no usable rates".to_string()));
    }
    Ok(FxQuote {
        as_of,
        rates: parsed,
    })
}

/// Convert `amount_minor` from `currency` to the display currency, or `None`
/// when FX is off, the currency is already the display one, or the source did
/// not name it (nothing is imputed, FR-115/AD-8).
pub fn convert(settings: &FxSettings, amount_minor: i64, currency: &str) -> Option<Converted> {
    if !settings.enabled {
        return None;
    }
    let as_of = settings.as_of.clone()?;
    let target = settings.target.trim().to_ascii_uppercase();
    if target.is_empty() || currency.eq_ignore_ascii_case(&target) {
        return None;
    }
    let rate = *settings.rates.get(&currency.to_ascii_uppercase())?;
    if rate <= 0.0 {
        return None;
    }
    // `1 target = rate currency`, so the amount divides by the rate.
    let converted = (amount_minor as f64) / rate;
    if !converted.is_finite() || converted.abs() > i64::MAX as f64 {
        return None;
    }
    Some(Converted {
        amount_minor: converted.round() as i64,
        currency: target,
        as_of,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network_log::host_of;
    use crate::system::testing::FakeSystemRepo;

    const LATEST: &str = r#"{
        "amount": 1.0,
        "base": "EUR",
        "date": "2026-10-16",
        "rates": { "USD": 1.1, "GBP": 0.85, "COP": 4500.5 }
    }"#;

    fn enabled(target: &str, rates: &[(&str, f64)], as_of: &str) -> FxSettings {
        FxSettings {
            enabled: true,
            target: target.to_string(),
            as_of: Some(as_of.to_string()),
            rates: rates
                .iter()
                .map(|(currency, rate)| (currency.to_string(), *rate))
                .collect(),
        }
    }

    #[test]
    fn fx_is_off_by_default_and_converts_nothing() {
        let system = FakeSystemRepo::default();
        let settings = load(&system).expect("load");

        assert!(!settings.enabled);
        assert!(settings.rates.is_empty());
        assert_eq!(convert(&settings, 100_000, "USD"), None);
    }

    #[test]
    fn the_opt_in_round_trips_through_the_settings_row() {
        let mut system = FakeSystemRepo::default();
        let settings = enabled("EUR", &[("USD", 1.1)], "2026-10-16");
        save(&system, &settings, "2026-10-17T09:00:00Z").expect("save");

        assert_eq!(load(&system).expect("load"), settings);
        system.seed_setting(SETTING_KEY, "not json");
        // A row this build cannot read falls back to off, never to a conversion.
        assert_eq!(load(&system).expect("load"), FxSettings::default());
    }

    #[test]
    fn turning_fx_off_keeps_the_rates_but_converts_nothing() {
        let mut settings = enabled("EUR", &[("USD", 1.1)], "2026-10-16");
        assert!(convert(&settings, 100_000, "USD").is_some());

        settings.enabled = false;
        assert_eq!(convert(&settings, 100_000, "USD"), None);
    }

    #[test]
    fn a_salary_in_the_display_currency_is_left_alone() {
        let settings = enabled("EUR", &[("USD", 1.1)], "2026-10-16");
        assert_eq!(convert(&settings, 100_000, "EUR"), None);
    }

    #[test]
    fn a_currency_the_rates_do_not_cover_is_not_converted() {
        let settings = enabled("EUR", &[("USD", 1.1)], "2026-10-16");
        assert_eq!(convert(&settings, 100_000, "JPY"), None);
    }

    #[test]
    fn conversion_rounds_to_minor_units_and_carries_the_rate_date() {
        let settings = enabled("EUR", &[("USD", 1.1)], "2026-10-16");
        let converted = convert(&settings, 100_000, "USD").expect("converted");

        // 1,000.00 USD at 1 EUR = 1.10 USD is 909.09 EUR.
        assert_eq!(converted.amount_minor, 90_909);
        assert_eq!(converted.currency, "EUR");
        assert_eq!(converted.as_of, "2026-10-16");
    }

    #[test]
    fn parsing_the_latest_answer_keeps_the_date_and_the_rates() {
        let quote = parse_latest(LATEST, "EUR").expect("parsed");

        assert_eq!(quote.as_of, "2026-10-16");
        assert_eq!(quote.rates.get("USD"), Some(&1.1));
        assert_eq!(quote.rates.get("COP"), Some(&4500.5));
    }

    #[test]
    fn an_answer_for_another_base_or_without_rates_is_refused() {
        assert!(parse_latest(LATEST, "USD").is_err());
        assert!(parse_latest(r#"{"base":"EUR","rates":{}}"#, "EUR").is_err());
        assert!(parse_latest(r#"{"base":"EUR","date":"2026-10-16"}"#, "EUR").is_err());
        assert!(parse_latest("not json", "EUR").is_err());
    }

    #[test]
    fn the_source_url_names_the_display_currency() {
        assert_eq!(
            source_url("EUR"),
            "https://api.frankfurter.dev/v1/latest?base=EUR"
        );
        assert_eq!(host_of(&source_url("EUR")).expect("host"), SOURCE_HOST);
    }
}
