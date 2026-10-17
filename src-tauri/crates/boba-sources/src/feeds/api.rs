//! Payload helpers shared by the feed and aggregator adapters (S4.6).
//!
//! The ATS helpers in `ats::json` are board-specific; feeds and aggregators
//! bring their own quirks (epoch dates, legal-notice rows, per-source daily
//! budgets, secret-bearing URLs), so the pieces both share live here.
//!
//! Payloads are untrusted (AD-7): a posting URL is validated before it can
//! become a link, and HTML is left for `normalise` to sanitise with ammonia.

use boba_domain::source::{RateLimits, RawPosting, SearchQuery, SourceError};
use serde_json::Value;

use crate::http::{FetchKind, FetchRequest, HttpClient, PostRequest};

/// GET a JSON document through the shared client and insist on a 200.
///
/// `budget_key` opts the call into the manifest's daily cap (Remotive ≤ 4/day).
/// `public_url`, when given, is what errors carry instead of the real URL: an
/// aggregator key in a query string must never reach a log (Adzuna).
pub(crate) async fn get_json(
    http: &HttpClient,
    url: &str,
    limits: RateLimits,
    budget_key: Option<&str>,
    public_url: Option<&str>,
) -> Result<String, SourceError> {
    let response = http
        .get(FetchRequest {
            url,
            kind: FetchKind::Json,
            limits,
            budget_key,
        })
        .await
        .map_err(|error| redact(error, public_url))?;
    expect_200(response.status, public_url.unwrap_or(url))?;
    Ok(response.text())
}

/// GET an RSS/Atom document (the feed kind never triggers a robots check: the
/// feed URL is the publisher's own distribution channel, ADR-005 §3).
pub(crate) async fn get_feed(
    http: &HttpClient,
    url: &str,
    limits: RateLimits,
    budget_key: Option<&str>,
) -> Result<Vec<u8>, SourceError> {
    let response = http
        .get(FetchRequest {
            url,
            kind: FetchKind::Feed,
            limits,
            budget_key,
        })
        .await?;
    expect_200(response.status, url)?;
    Ok(response.body)
}

/// POST a JSON body through the shared client and insist on a 200.
pub(crate) async fn post_json(
    http: &HttpClient,
    url: &str,
    public_url: &str,
    body: &str,
    limits: RateLimits,
    budget_key: Option<&str>,
) -> Result<String, SourceError> {
    let response = http
        .post(PostRequest {
            url,
            limits,
            budget_key,
            body,
            content_type: "application/json",
        })
        .await
        .map_err(|error| redact(error, Some(public_url)))?;
    expect_200(response.status, public_url)?;
    Ok(response.text())
}

fn expect_200(status: u16, url: &str) -> Result<(), SourceError> {
    if status == 200 {
        Ok(())
    } else {
        Err(SourceError::Http {
            url: url.to_string(),
            message: format!("status {status}"),
        })
    }
}

/// Replace a URL that carries a secret with the one safe to log.
fn redact(error: SourceError, public_url: Option<&str>) -> SourceError {
    let Some(public) = public_url else {
        return error;
    };
    match error {
        SourceError::Http { message, .. } => SourceError::Http {
            url: public.to_string(),
            message,
        },
        SourceError::ResponseTooLarge { limit, .. } => SourceError::ResponseTooLarge {
            url: public.to_string(),
            limit,
        },
        other => other,
    }
}

/// Parse the response text as JSON.
pub(crate) fn parse(body: &str) -> Result<Value, SourceError> {
    serde_json::from_str(body).map_err(|error| SourceError::InvalidPayload {
        message: format!("invalid JSON: {error}"),
    })
}

/// A required array field of an object payload.
pub(crate) fn array<'a>(value: &'a Value, key: &str) -> Result<&'a [Value], SourceError> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| SourceError::InvalidPayload {
            message: format!("missing array '{key}'"),
        })
}

/// A trimmed, non-empty string field.
pub(crate) fn text(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

/// A trimmed, non-empty string at a nested path.
pub(crate) fn nested_text(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

/// A number field as `f64`.
pub(crate) fn number(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(Value::as_f64)
}

/// A whole-number field as `i64`.
pub(crate) fn integer(value: &Value, key: &str) -> Option<i64> {
    value.get(key).and_then(Value::as_i64)
}

/// A boolean field.
pub(crate) fn boolean(value: &Value, key: &str) -> Option<bool> {
    value.get(key).and_then(Value::as_bool)
}

/// The item's own JSON, kept verbatim as provenance (AD-19).
pub(crate) fn provenance(value: &Value) -> String {
    value.to_string()
}

/// The one apply-URL validator, re-exported from the ATS module (S4.6 review
/// F2): feed payloads are untrusted and `RawPosting.url` becomes a clickable
/// link in the app webview, so `javascript:`, `data:`, `http:` and anything
/// malformed are refused by the same check S4.5 uses.
pub(crate) use crate::ats::json::https_url;

/// Whether a location or title label reads as remote work.
pub(crate) fn is_remote(location: &str) -> bool {
    let lower = location.to_ascii_lowercase();
    lower.contains("remote") || lower.contains("anywhere") || lower.contains("worldwide")
}

/// Percent-encode one query-string value (Adzuna `what`/`where`, Jooble never
/// uses it: its input is a JSON body).
pub(crate) fn encode_query(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// Apply the Search Query locally for feeds with no server-side search.
///
/// `remote` and the `roles`/`keywords` terms are honoured against the title;
/// every other field is ignored (the same rule the ATS adapters follow).
pub(crate) fn retain_matching(postings: Vec<RawPosting>, query: &SearchQuery) -> Vec<RawPosting> {
    let terms: Vec<String> = query
        .roles
        .iter()
        .chain(query.keywords.iter())
        .map(|term| term.trim().to_ascii_lowercase())
        .filter(|term| !term.is_empty())
        .collect();
    postings
        .into_iter()
        .filter(|posting| {
            if let Some(remote) = query.remote
                && posting.remote != Some(remote)
            {
                return false;
            }
            if terms.is_empty() {
                return true;
            }
            let title = posting.title.to_ascii_lowercase();
            terms.iter().any(|term| title.contains(term.as_str()))
        })
        .collect()
}

/// Epoch seconds to the UTC RFC 3339 string the store expects (AD-17).
///
/// Same civil-calendar arithmetic as `ats::json::rfc3339_from_millis`, kept
/// local so the feeds module does not depend on the board module.
pub(crate) fn rfc3339_from_secs(secs: i64) -> Option<String> {
    if secs < 0 {
        return None;
    }
    let days = secs / 86_400;
    let second_of_day = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = second_of_day / 3_600;
    let minute = second_of_day % 3_600 / 60;
    let second = second_of_day % 60;
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"
    ))
}

/// Days since 1970-01-01 to a civil (year, month, day), after Howard Hinnant's
/// `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// A source timestamp to UTC RFC 3339, or `None` when it is not a timestamp.
///
/// Remotive sends a naive `2026-09-12T10:00:00`, which is UTC for the feed;
/// anything already zoned is kept as sent. The store rejects whatever still
/// does not parse, so nothing broken is written.
pub(crate) fn utc_timestamp(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if text.ends_with('Z') || has_utc_offset(text) {
        return Some(text.to_string());
    }
    let (date, time) = text.split_once('T')?;
    if !valid_date(date) || !valid_time(time) {
        return None;
    }
    Some(format!("{text}Z"))
}

fn has_utc_offset(text: &str) -> bool {
    let Some(index) = text.rfind(['+', '-']) else {
        return false;
    };
    let offset = &text[index..];
    offset.len() == 6
        && offset.as_bytes()[3] == b':'
        && offset[1..3].bytes().all(|b| b.is_ascii_digit())
        && offset[4..6].bytes().all(|b| b.is_ascii_digit())
}

fn valid_date(date: &str) -> bool {
    let mut parts = date.split('-');
    let (Some(year), Some(month), Some(day), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    let digits = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
    year.len() == 4
        && digits(year)
        && year.parse::<u32>().is_ok_and(|year| year >= 1000)
        && month.len() == 2
        && digits(month)
        && month
            .parse::<u32>()
            .is_ok_and(|month| (1..=12).contains(&month))
        && day.len() == 2
        && digits(day)
        && day.parse::<u32>().is_ok_and(|day| (1..=31).contains(&day))
}

fn valid_time(time: &str) -> bool {
    let time = time.split('.').next().unwrap_or(time);
    let mut parts = time.split(':');
    let (Some(hour), Some(minute), Some(second), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    let digits = |part: &str| part.len() == 2 && part.bytes().all(|b| b.is_ascii_digit());
    digits(hour)
        && digits(minute)
        && digits(second)
        && hour.parse::<u32>().is_ok_and(|hour| hour < 24)
        && minute.parse::<u32>().is_ok_and(|minute| minute < 60)
        && second.parse::<u32>().is_ok_and(|second| second <= 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn posting(title: &str, remote: Option<bool>) -> RawPosting {
        RawPosting {
            external_id: title.into(),
            title: title.into(),
            company: "Boba".into(),
            url: "https://example.test/1".into(),
            locations: vec![],
            remote,
            description_html: None,
            posted_at: None,
            salary_text: None,
            raw: "{}".into(),
        }
    }

    #[test]
    fn only_https_urls_are_kept() {
        assert_eq!(
            https_url(Some("https://example.test/j".into())).as_deref(),
            Some("https://example.test/j")
        );
        assert_eq!(https_url(Some("http://example.test/j".into())), None);
        assert_eq!(https_url(Some("javascript:alert(1)".into())), None);
        assert_eq!(https_url(Some("data:text/html,x".into())), None);
        assert_eq!(https_url(Some("https:///nohost".into())), None);
        assert_eq!(https_url(Some("https://exa mple.test".into())), None);
        assert_eq!(https_url(None), None);
    }

    #[test]
    fn remote_labels_cover_the_feed_wording() {
        assert!(is_remote("Remote"));
        assert!(is_remote("Anywhere in the World"));
        assert!(is_remote("Worldwide"));
        assert!(!is_remote("Berlin, Germany"));
    }

    #[test]
    fn the_query_filters_remote_and_title_terms() {
        let postings = vec![
            posting("Senior Backend Engineer", Some(true)),
            posting("Office Manager", Some(false)),
        ];
        assert_eq!(
            retain_matching(postings.clone(), &SearchQuery::default()).len(),
            2
        );
        let remote = SearchQuery {
            remote: Some(true),
            ..SearchQuery::default()
        };
        assert_eq!(retain_matching(postings.clone(), &remote).len(), 1);
        let backend = SearchQuery {
            roles: vec!["backend".into()],
            ..SearchQuery::default()
        };
        let matched = retain_matching(postings, &backend);
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].title, "Senior Backend Engineer");
    }

    #[test]
    fn converts_epoch_seconds_to_utc_rfc3339() {
        assert_eq!(
            rfc3339_from_secs(0).as_deref(),
            Some("1970-01-01T00:00:00Z")
        );
        assert_eq!(
            rfc3339_from_secs(1_791_763_200).as_deref(),
            Some("2026-10-12T00:00:00Z")
        );
        assert_eq!(
            rfc3339_from_secs(1_709_164_800).as_deref(),
            Some("2024-02-29T00:00:00Z")
        );
        assert_eq!(rfc3339_from_secs(-1), None);
    }

    #[test]
    fn normalises_a_naive_timestamp_and_keeps_a_zoned_one() {
        assert_eq!(
            utc_timestamp("2026-09-12T10:00:00").as_deref(),
            Some("2026-09-12T10:00:00Z")
        );
        assert_eq!(
            utc_timestamp("2026-09-12T10:00:00.123").as_deref(),
            Some("2026-09-12T10:00:00.123Z")
        );
        assert_eq!(
            utc_timestamp("2026-09-12T10:00:00+02:00").as_deref(),
            Some("2026-09-12T10:00:00+02:00")
        );
        assert_eq!(
            utc_timestamp("2026-09-12T10:00:00Z").as_deref(),
            Some("2026-09-12T10:00:00Z")
        );
        assert_eq!(utc_timestamp("last Tuesday"), None);
        assert_eq!(utc_timestamp("2026-99-12T10:00:00"), None);
        assert_eq!(utc_timestamp("2026-09-12T25:00:00"), None);
        assert_eq!(utc_timestamp(""), None);
    }

    #[test]
    fn query_values_are_percent_encoded() {
        assert_eq!(encode_query("rust backend"), "rust%20backend");
        assert_eq!(encode_query("Berlin, Germany"), "Berlin%2C%20Germany");
        assert_eq!(encode_query("a&b=c"), "a%26b%3Dc");
    }
}
