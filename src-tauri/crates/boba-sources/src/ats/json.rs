//! Field-level JSON helpers shared by the ATS adapters (S4.5).
//!
//! Every adapter reads the payload by field name instead of binding it to a
//! struct: ATS boards add and rename fields without notice (Workable's widget
//! is undocumented), and a missing field must degrade one posting, not the
//! board.

use boba_domain::source::{RateLimits, RawPosting, SearchQuery, SourceError};
use serde_json::Value;

use crate::http::{FetchKind, FetchRequest, HttpClient};

/// GET a JSON document through the shared client and insist on a 200.
pub(crate) async fn get_json(
    http: &HttpClient,
    url: &str,
    limits: RateLimits,
) -> Result<String, SourceError> {
    let response = http
        .get(FetchRequest {
            url,
            kind: FetchKind::Json,
            limits,
            budget_key: None,
        })
        .await?;
    if response.status != 200 {
        return Err(SourceError::Http {
            url: url.to_string(),
            message: format!("status {}", response.status),
        });
    }
    Ok(response.text())
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

/// A number field, as `f64` (ATS salary figures are integral in practice).
pub(crate) fn number(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(Value::as_f64)
}

/// A posting URL a board sent, kept only when it is an absolute `https://` URL.
///
/// Board payloads are untrusted and `RawPosting.url` ends up as a clickable
/// link in the app webview, so `javascript:`, `data:`, `http:` and anything
/// malformed are refused; the adapter then uses the URL it builds itself.
pub(crate) fn https_url(candidate: Option<String>) -> Option<String> {
    let url = candidate?;
    if !url
        .get(..8)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"))
    {
        return None;
    }
    let rest = &url[8..];
    if rest.is_empty() || rest.starts_with('/') {
        return None;
    }
    if url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    Some(url)
}

/// Percent-encode an id before it becomes one path segment, so a source id of
/// `../admin` cannot walk out of the board path.
pub(crate) fn encode_segment(segment: &str) -> String {
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// A boolean field.
pub(crate) fn boolean(value: &Value, key: &str) -> Option<bool> {
    value.get(key).and_then(Value::as_bool)
}

/// The item's own JSON, kept verbatim as provenance (AD-19).
pub(crate) fn provenance(value: &Value) -> String {
    value.to_string()
}

/// Whether a location label reads as remote work.
pub(crate) fn is_remote(location: &str) -> bool {
    let lower = location.to_ascii_lowercase();
    lower.contains("remote") || lower.contains("anywhere")
}

/// Decode the HTML entities the ATS description strings arrive escaped with.
///
/// The payload is still untrusted: `normalise` sanitises the result with
/// ammonia before any UI use (AD-7).
pub(crate) fn html_unescape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(index) = rest.find('&') {
        out.push_str(&rest[..index]);
        let after = &rest[index + 1..];
        let entity_end = after.find(';').filter(|end| *end <= 10);
        let Some(end) = entity_end else {
            out.push('&');
            rest = after;
            continue;
        };
        match &after[..end] {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            "nbsp" => out.push(' '),
            entity => match numeric_entity(entity) {
                Some(character) => out.push(character),
                None => {
                    out.push('&');
                    out.push_str(entity);
                    out.push(';');
                }
            },
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

fn numeric_entity(entity: &str) -> Option<char> {
    let digits = entity.strip_prefix('#')?;
    let code = match digits.strip_prefix(['x', 'X']) {
        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
        None => digits.parse::<u32>().ok()?,
    };
    char::from_u32(code)
}

/// Epoch milliseconds to the UTC RFC 3339 string the store expects (AD-17).
///
/// `normalise` rejects anything else, and `boba-sources` has no date crate, so
/// this is the one place the conversion happens. Lever is the only source that
/// sends epoch milliseconds.
pub(crate) fn rfc3339_from_millis(millis: i64) -> Option<String> {
    if millis < 0 {
        return None;
    }
    let seconds = millis / 1000;
    let days = seconds / 86_400;
    let second_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = second_of_day / 3_600;
    let minute = second_of_day % 3_600 / 60;
    let second = second_of_day % 60;
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{:03}Z",
        millis % 1000
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

/// A `YYYY-MM-DD` date to a UTC RFC 3339 midnight, when the source sends only
/// a day (Workable's `published_on`).
pub(crate) fn date_to_rfc3339(date: &str) -> Option<String> {
    let date = date.trim();
    let mut parts = date.split('-');
    let year = parts.next()?;
    let month = parts.next()?;
    let day = parts.next()?;
    if parts.next().is_some()
        || year.len() != 4
        || month.len() != 2
        || day.len() != 2
        || !year.chars().all(|c| c.is_ascii_digit())
        || !month.chars().all(|c| c.is_ascii_digit())
        || !day.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    // The store wants a real date; 2026-99-99 is not one.
    let year: u32 = year.parse().ok().filter(|year| *year >= 1000)?;
    let month: u32 = month
        .parse()
        .ok()
        .filter(|month| (1..=12).contains(month))?;
    let day: u32 = day.parse().ok().filter(|day| (1..=31).contains(day))?;
    Some(format!("{year:04}-{month:02}-{day:02}T00:00:00Z"))
}

/// The `salary_text` a pay range renders as; `None` when the source gave none.
pub(crate) fn salary_text(
    min: Option<f64>,
    max: Option<f64>,
    currency: Option<&str>,
    interval: Option<&str>,
) -> Option<String> {
    let range = match (min, max) {
        (Some(low), Some(high)) => format!("{low:.0} – {high:.0}"),
        (Some(low), None) => format!("from {low:.0}"),
        (None, Some(high)) => format!("up to {high:.0}"),
        (None, None) => return None,
    };
    let mut text = match currency.map(str::trim).filter(|code| !code.is_empty()) {
        Some(code) => format!("{code} {range}"),
        None => range,
    };
    if let Some(interval) = interval.map(str::trim).filter(|text| !text.is_empty()) {
        text.push_str(" / ");
        text.push_str(interval);
    }
    Some(text)
}

/// Apply the Search Query locally. ATS boards have no server-side search, so
/// the adapter fetches the board and honours `remote` and `roles`/`keywords`
/// here; every other field of the query is ignored.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescapes_the_named_entities_ats_descriptions_use() {
        assert_eq!(
            html_unescape("&lt;p&gt;Rust &amp; TypeScript&lt;/p&gt;"),
            "<p>Rust & TypeScript</p>"
        );
        assert_eq!(
            html_unescape("&quot;quoted&quot; &apos;x&apos;"),
            "\"quoted\" 'x'"
        );
        assert_eq!(html_unescape("a&nbsp;b"), "a b");
        assert_eq!(html_unescape("caf&#233; &#x1F600;"), "café 😀");
        assert_eq!(html_unescape("a & b < c"), "a & b < c");
        assert_eq!(html_unescape("&unknown; stays"), "&unknown; stays");
        assert_eq!(html_unescape("trailing &"), "trailing &");
    }

    #[test]
    fn converts_epoch_millis_to_utc_rfc3339() {
        assert_eq!(
            rfc3339_from_millis(0).as_deref(),
            Some("1970-01-01T00:00:00.000Z")
        );
        assert_eq!(
            rfc3339_from_millis(1_727_740_800_000).as_deref(),
            Some("2024-10-01T00:00:00.000Z")
        );
        assert_eq!(
            rfc3339_from_millis(1_727_740_800_123).as_deref(),
            Some("2024-10-01T00:00:00.123Z")
        );
        assert_eq!(
            rfc3339_from_millis(1_709_164_800_000).as_deref(),
            Some("2024-02-29T00:00:00.000Z")
        );
        assert_eq!(rfc3339_from_millis(-1), None);
    }

    #[test]
    fn accepts_a_day_only_date_and_rejects_anything_else() {
        assert_eq!(
            date_to_rfc3339("2026-09-29").as_deref(),
            Some("2026-09-29T00:00:00Z")
        );
        assert_eq!(date_to_rfc3339("2026-9-29"), None);
        assert_eq!(date_to_rfc3339("29/09/2026"), None);
        assert_eq!(date_to_rfc3339("2026-09-29T07:45:00Z"), None);
    }

    #[test]
    fn renders_a_salary_range() {
        assert_eq!(
            salary_text(
                Some(120_000.0),
                Some(160_000.0),
                Some("EUR"),
                Some("per-year-salary")
            )
            .as_deref(),
            Some("EUR 120000 – 160000 / per-year-salary")
        );
        assert_eq!(
            salary_text(Some(90_000.0), Some(120_000.0), Some("EUR"), Some("yearly")).as_deref(),
            Some("EUR 90000 – 120000 / yearly")
        );
        assert_eq!(
            salary_text(None, Some(80_000.0), None, None).as_deref(),
            Some("up to 80000")
        );
        assert_eq!(salary_text(None, None, Some("EUR"), None), None);
    }

    #[test]
    fn the_query_filters_remote_and_title_terms() {
        let posting = |title: &str, remote: Option<bool>| RawPosting {
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
        };
        let postings = vec![
            posting("Senior Backend Engineer", Some(true)),
            posting("Office Manager", Some(false)),
        ];

        let all = retain_matching(postings.clone(), &SearchQuery::default());
        assert_eq!(all.len(), 2);

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
    fn only_https_urls_are_kept() {
        assert_eq!(
            https_url(Some("https://example.test/j".into())).as_deref(),
            Some("https://example.test/j")
        );
        assert_eq!(
            https_url(Some("HTTPS://example.test/j".into())).as_deref(),
            Some("HTTPS://example.test/j")
        );
        assert_eq!(https_url(Some("http://example.test/j".into())), None);
        assert_eq!(https_url(Some("javascript:alert(1)".into())), None);
        assert_eq!(https_url(Some("data:text/html,x".into())), None);
        assert_eq!(https_url(Some("https:///nohost".into())), None);
        assert_eq!(https_url(Some("https://exa mple.test".into())), None);
        assert_eq!(https_url(None), None);
    }

    #[test]
    fn a_path_segment_is_percent_encoded() {
        assert_eq!(encode_segment("abc-123_x.y~z"), "abc-123_x.y~z");
        assert_eq!(encode_segment("../evil"), "..%2Fevil");
        assert_eq!(encode_segment("a b"), "a%20b");
        assert_eq!(encode_segment("ñ"), "%C3%B1");
    }

    #[test]
    fn a_day_only_date_checks_its_ranges() {
        assert_eq!(date_to_rfc3339("2026-99-99"), None);
        assert_eq!(date_to_rfc3339("2026-13-01"), None);
        assert_eq!(date_to_rfc3339("2026-00-10"), None);
        assert_eq!(date_to_rfc3339("2026-12-32"), None);
        assert_eq!(
            date_to_rfc3339("2026-12-31").as_deref(),
            Some("2026-12-31T00:00:00Z")
        );
    }
}
