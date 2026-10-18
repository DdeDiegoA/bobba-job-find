//! Manual email import (S8.3, FR-104).
//!
//! A reply the user drops in (an `.eml` file) or pastes is parsed here into the
//! same [`RawMessage`] a mailbox adapter produces, and then enters
//! [`crate::sync::run`] like any fetched message: matched against the scope,
//! handed to the same sink, stored with `origin = eml`. There is no second
//! pipeline and no shortcut past the matcher.
//!
//! Parsing is defensive. The file is untrusted input (AD-7): sizes are capped,
//! the body is reduced to plain text (HTML is stripped, never stored), and the
//! parser's output is validated before it is accepted. Nothing here touches the
//! network, the filesystem or a mailbox.

use mail_parser::{Address, HeaderValue, Message, MessageParser};
use sha2::{Digest, Sha256};

use crate::sync::{RawMessage, normalize_message_id};

/// The largest `.eml` that is parsed. A reply with a couple of attachments fits
/// easily; a pathological file does not get to exhaust memory.
pub const MAX_EML_BYTES: usize = 25 * 1024 * 1024;

/// The longest body that is kept, in characters. Classification reads a reply,
/// not an archive; the rest is dropped at import.
pub const MAX_BODY_CHARS: usize = 256 * 1024;

/// Why an `.eml` could not be imported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EmlError {
    /// The file is larger than [`MAX_EML_BYTES`].
    #[error("the email file is too large to import")]
    TooLarge,
    /// The parser rejected the bytes.
    #[error("the email file could not be parsed")]
    Unreadable,
    /// The file parsed but carries none of the headers a message has.
    #[error("the file does not look like an email message")]
    NotAMessage,
}

/// Parse one `.eml` file (or pasted message text) into the pipeline's message.
///
/// `now` (Unix seconds) is the fallback for a message without a usable `Date`
/// header, so a stored row always has a timestamp.
pub fn parse(raw: &[u8], now: i64) -> Result<RawMessage, EmlError> {
    if raw.len() > MAX_EML_BYTES {
        return Err(EmlError::TooLarge);
    }
    let message = MessageParser::new()
        .parse(raw)
        .ok_or(EmlError::Unreadable)?;

    let from = message.from().and_then(Address::first);
    let from_address = from
        .and_then(|address| address.address())
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    let from_name = from
        .and_then(|address| address.name())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string);
    let message_id = message
        .message_id()
        .map(normalize_message_id)
        .filter(|id| !id.is_empty());
    let subject = message.subject().unwrap_or_default().trim().to_string();

    if from_address.is_empty() && message_id.is_none() && subject.is_empty() {
        return Err(EmlError::NotAMessage);
    }

    let received_at = message
        .date()
        .filter(|date| date.is_valid())
        .map(mail_parser::DateTime::to_timestamp)
        .filter(|timestamp| *timestamp > 0)
        .unwrap_or(now);

    let body_text = body(&message);
    let provider_id = message_id
        .clone()
        .unwrap_or_else(|| format!("sha256:{}", sha256_hex(raw)));

    Ok(RawMessage {
        provider_id,
        message_id,
        in_reply_to: header_id(message.in_reply_to()),
        references: header_id(message.references()),
        from_address,
        from_name,
        to_addresses: addresses(message.to()),
        subject,
        received_at,
        body_text,
        has_attachments: message.attachments().next().is_some(),
    })
}

/// Parse pasted message text. The bytes are the same bytes an `.eml` file
/// would carry, so pasting and dropping a file cannot diverge.
pub fn parse_text(text: &str, now: i64) -> Result<RawMessage, EmlError> {
    parse(text.as_bytes(), now)
}

/// The plain-text body, capped. `text/plain` wins; an HTML-only message is
/// reduced to text so the pipeline never sees markup.
fn body(message: &Message<'_>) -> String {
    let text = message
        .text_bodies()
        .filter(|part| !part.is_text_html())
        .find_map(|part| part.text_contents())
        .map(str::to_string)
        .or_else(|| message.body_html(0).map(|html| html_to_text(&html)))
        .unwrap_or_default();
    cap(text)
}

/// A provider body (Graph hands one as `contentType` + `content`) reduced to
/// the pipeline's plain text: HTML is stripped, and the result is capped like a
/// parsed body. Shared with the Graph reader so both edges behave the same.
pub(crate) fn body_from(content_type: &str, content: &str) -> String {
    if content_type.eq_ignore_ascii_case("text") || content_type.eq_ignore_ascii_case("plain") {
        cap(content.to_string())
    } else {
        cap(html_to_text(content))
    }
}

/// Keep the first [`MAX_BODY_CHARS`] characters, on a character boundary.
fn cap(text: String) -> String {
    if text.chars().count() <= MAX_BODY_CHARS {
        return text.trim().to_string();
    }
    text.chars()
        .take(MAX_BODY_CHARS)
        .collect::<String>()
        .trim()
        .to_string()
}

/// Normalise an `In-Reply-To`/`References` header into bare message ids.
/// The header may carry a list of ids, which is why its text is split on
/// whitespace.
fn header_id(value: &HeaderValue<'_>) -> Vec<String> {
    // `as_text()` on a list returns only its last element, so the list form
    // (which also covers a single text value) is the one to read.
    let ids: Vec<&str> = value
        .as_text_list()
        .map(|list| list.iter().flat_map(|id| id.split_whitespace()).collect())
        .unwrap_or_default();
    ids.into_iter()
        .map(normalize_message_id)
        .filter(|id| !id.is_empty())
        .collect()
}

/// The `To` addresses, as the header carried them.
fn addresses(address: Option<&Address<'_>>) -> Vec<String> {
    address
        .map(|address| {
            address
                .iter()
                .filter_map(|addr| addr.address())
                .map(|addr| addr.trim().to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// A stable content id for a message without a `Message-ID`, so importing the
/// same file twice is one row.
fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            use std::fmt::Write as _;
            let _ = write!(out, "{byte:02x}");
            out
        })
}

/// Reduce an HTML body to plain text: tags are dropped (a few block elements
/// become a line break), `<script>`/`<style>` content is skipped, common
/// entities are decoded, and whitespace is collapsed.
pub(crate) fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while !rest.is_empty() {
        let Some(start) = rest.find('<') else {
            decode_entities_into(&mut out, rest);
            break;
        };
        decode_entities_into(&mut out, &rest[..start]);
        let after = &rest[start..];
        let Some(end) = after.find('>') else {
            break;
        };
        let tag = after[1..end].trim_start_matches('/').trim();
        let name: String = tag
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or_default()
            .chars()
            .take(16)
            .map(|c| c.to_ascii_lowercase())
            .collect();
        match name.as_str() {
            "br" | "p" | "div" | "li" | "tr" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                out.push('\n');
            }
            "script" | "style" => {
                // Skip the element's contents entirely: markup source is not
                // message text. The search is case-insensitive in place (a
                // lowercased copy of the remainder per tag would make a body
                // full of tags quadratic).
                match find_ascii_ci(after, &format!("</{name}")) {
                    Some(close) => {
                        let tail = &after[close..];
                        match tail.find('>') {
                            Some(gt) => rest = &tail[gt + 1..],
                            None => break,
                        }
                    }
                    // An unterminated script has no message text after it;
                    // stopping also keeps the loop linear.
                    None => break,
                }
                continue;
            }
            _ => {}
        }
        rest = &after[end + 1..];
    }
    collapse_whitespace(&out)
}

/// Find `needle` in `haystack`, ignoring ASCII case, without allocating.
fn find_ascii_ci(haystack: &str, needle: &str) -> Option<usize> {
    let hay = haystack.as_bytes();
    let needle = needle.as_bytes();
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle))
}

/// Decode the handful of HTML entities a mail body actually uses. An unknown
/// entity is kept verbatim rather than silently deleted.
fn decode_entities_into(out: &mut String, text: &str) {
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let Some(end) = tail.find(';').filter(|end| *end <= 12) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        match &tail[1..end] {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "#39" | "#039" | "apos" => out.push('\''),
            "nbsp" => out.push(' '),
            _ => out.push_str(&tail[..=end]),
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
}

/// Collapse runs of whitespace into one space and keep at most one newline per
/// line, so a stripped HTML body reads like the plain-text one.
fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pending_space = false;
    let mut pending_newline = false;
    for ch in text.chars() {
        if ch == '\n' || ch == '\r' {
            pending_newline = true;
            pending_space = false;
        } else if ch.is_whitespace() {
            if !pending_newline {
                pending_space = true;
            }
        } else {
            if pending_newline {
                if !out.is_empty() {
                    out.push('\n');
                }
                pending_newline = false;
            } else if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_760_000_000;

    fn plain() -> String {
        concat!(
            "From: Ana Recruiter <ana.recruiter@acme.example>\r\n",
            "To: me@example.com\r\n",
            "Subject: Your application at Acme\r\n",
            "Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n",
            "Message-ID: <ABC-123@acme.example>\r\n",
            "In-Reply-To: <SENT-1@boba.local>\r\n",
            "References: <older@elsewhere.example>\r\n",
            "        <SENT-1@boba.local>\r\n",
            "Content-Type: text/plain; charset=utf-8\r\n",
            "\r\n",
            "Hola, thanks for applying. We would like to talk.\r\n",
        )
        .to_string()
    }

    #[test]
    fn a_plain_message_is_parsed_into_the_pipeline_shape() {
        let message = parse(plain().as_bytes(), NOW).expect("parse");
        assert_eq!(message.from_address, "ana.recruiter@acme.example");
        assert_eq!(message.from_name.as_deref(), Some("Ana Recruiter"));
        assert_eq!(message.subject, "Your application at Acme");
        assert_eq!(message.message_id.as_deref(), Some("abc-123@acme.example"));
        assert_eq!(message.provider_id, "abc-123@acme.example");
        assert!(message.body_text.contains("thanks for applying"));
        assert!(!message.has_attachments);
        // 2024-01-01T10:00:00Z
        assert_eq!(message.received_at, 1_704_103_200);
        assert_eq!(
            message.in_reply_to,
            vec!["sent-1@boba.local".to_string()],
            "the header id is normalised"
        );
        assert_eq!(
            message.references,
            vec![
                "older@elsewhere.example".to_string(),
                "sent-1@boba.local".to_string()
            ],
            "several ids in one header value are split"
        );
        assert_eq!(message.to_addresses, vec!["me@example.com".to_string()]);
    }

    #[test]
    fn an_encoded_word_subject_is_decoded() {
        let raw = concat!(
            "From: ana@acme.example\r\n",
            "Subject: =?UTF-8?B?RW50cmV2aXN0YQ==?=\r\n",
            "Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n",
            "\r\n",
            "body\r\n",
        );
        let message = parse(raw.as_bytes(), NOW).expect("parse");
        assert_eq!(message.subject, "Entrevista");
    }

    #[test]
    fn multipart_prefers_the_plain_text_part() {
        let raw = concat!(
            "From: ana@acme.example\r\n",
            "Subject: Re: your application\r\n",
            "Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n",
            "MIME-Version: 1.0\r\n",
            "Content-Type: multipart/alternative; boundary=\"b\"\r\n",
            "\r\n",
            "--b\r\n",
            "Content-Type: text/html; charset=utf-8\r\n",
            "\r\n",
            "<p>HTML body</p>\r\n",
            "--b\r\n",
            "Content-Type: text/plain; charset=utf-8\r\n",
            "\r\n",
            "Plain body\r\n",
            "--b--\r\n",
        );
        let message = parse(raw.as_bytes(), NOW).expect("parse");
        assert_eq!(message.body_text, "Plain body");
    }

    #[test]
    fn an_html_only_body_is_stripped_to_text() {
        let raw = concat!(
            "From: ana@acme.example\r\n",
            "Subject: Re: your application\r\n",
            "Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n",
            "MIME-Version: 1.0\r\n",
            "Content-Type: text/html; charset=utf-8\r\n",
            "\r\n",
            "<html><head><style>p { color: red }</style></head><body>",
            "<p>Hi &amp; welcome</p><script>alert('x')</script>",
            "<p>See you <b>soon</b>&nbsp;!</p></body></html>\r\n",
        );
        let message = parse(raw.as_bytes(), NOW).expect("parse");
        assert!(!message.body_text.contains('<'), "{}", message.body_text);
        assert!(
            !message.body_text.contains("alert"),
            "{}",
            message.body_text
        );
        assert!(
            !message.body_text.contains("color"),
            "{}",
            message.body_text
        );
        assert!(
            message.body_text.contains("Hi & welcome"),
            "{}",
            message.body_text
        );
        assert!(
            message.body_text.contains("See you soon"),
            "{}",
            message.body_text
        );
    }

    /// The fastest of three runs: one scheduler stall must not decide a test.
    fn fastest_of_three(html: &str) -> std::time::Duration {
        (0..3)
            .map(|_| {
                let started = std::time::Instant::now();
                let text = html_to_text(std::hint::black_box(html));
                std::hint::black_box(text);
                started.elapsed()
            })
            .min()
            .expect("three runs")
    }

    /// Guards against quadratic work without a wall-clock deadline (FX19): four
    /// times the input takes about four times as long when the work is linear
    /// and about sixteen when it is quadratic. Under load both runs slow down
    /// together, so the ratio holds; the 50ms slack covers a base run fast
    /// enough to be pure timer noise.
    fn assert_linear_time(named: &str, once: &str, four_times: &str) {
        let small = fastest_of_three(once).as_secs_f64();
        let big = fastest_of_three(four_times).as_secs_f64();
        assert!(
            big <= small * 10.0 + 0.050,
            "{named}: 4x the input took {big:.4}s against {small:.4}s; that is not linear"
        );
    }

    #[test]
    fn many_script_tags_are_stripped_in_linear_time() {
        let tagged = |count: usize| {
            let mut html = String::from("<html><body>");
            for _ in 0..count {
                html.push_str("<script>alert('x')</script><p>hello</p>");
            }
            html
        };
        let html = tagged(50_000);
        let text = html_to_text(&html);
        assert!(text.contains("hello"));
        assert!(!text.contains("alert"));

        assert_linear_time("50k tags", &html, &tagged(200_000));
    }

    #[test]
    fn fifty_thousand_unclosed_script_tags_stay_fast() {
        // The exact shape that made the first version quadratic: no closing
        // tag anywhere, so every later open tag would rescan the whole rest.
        let html = "<script>".repeat(50_000);
        assert!(html_to_text(&html).is_empty());

        assert_linear_time("unclosed tags", &html, &"<script>".repeat(200_000));
    }

    #[test]
    fn an_unterminated_script_stops_instead_of_rescanning_the_remainder() {
        let body = |count: usize| format!("<p>before</p><script>{}", "<p>after</p>".repeat(count));
        let html = body(50_000);
        assert_eq!(html_to_text(&html), "before");

        assert_linear_time("unterminated script", &html, &body(200_000));
    }

    #[test]
    fn a_message_without_a_date_is_stamped_with_now() {
        let raw = concat!(
            "From: ana@acme.example\r\n",
            "Subject: no date\r\n",
            "\r\n",
            "body\r\n",
        );
        let message = parse(raw.as_bytes(), NOW).expect("parse");
        assert_eq!(message.received_at, NOW);
    }

    #[test]
    fn a_message_without_a_message_id_gets_a_stable_content_id() {
        let raw = concat!(
            "From: ana@acme.example\r\n",
            "Subject: no id\r\n",
            "Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n",
            "\r\n",
            "body\r\n",
        );
        let first = parse(raw.as_bytes(), NOW).expect("parse");
        let second = parse(raw.as_bytes(), NOW).expect("parse");
        assert!(
            first.provider_id.starts_with("sha256:"),
            "{}",
            first.provider_id
        );
        assert_eq!(first.provider_id, second.provider_id);
        assert_ne!(
            first.provider_id,
            parse(format!("{raw}x").as_bytes(), NOW)
                .unwrap()
                .provider_id
        );
    }

    #[test]
    fn an_attachment_is_reported_but_not_parsed() {
        let raw = concat!(
            "From: ana@acme.example\r\n",
            "Subject: with attachment\r\n",
            "Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n",
            "MIME-Version: 1.0\r\n",
            "Content-Type: multipart/mixed; boundary=\"b\"\r\n",
            "\r\n",
            "--b\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "Please find attached.\r\n",
            "--b\r\n",
            "Content-Type: application/pdf; name=\"cv.pdf\"\r\n",
            "Content-Disposition: attachment; filename=\"cv.pdf\"\r\n",
            "Content-Transfer-Encoding: base64\r\n",
            "\r\n",
            "JVBERi0xLjQK\r\n",
            "--b--\r\n",
        );
        let message = parse(raw.as_bytes(), NOW).expect("parse");
        assert!(message.has_attachments);
        assert!(message.body_text.contains("Please find attached"));
    }

    #[test]
    fn an_oversized_file_is_refused_before_parsing() {
        let too_large = vec![b'x'; MAX_EML_BYTES + 1];
        assert_eq!(parse(&too_large, NOW), Err(EmlError::TooLarge));
    }

    #[test]
    fn a_file_that_is_not_a_message_is_refused() {
        assert_eq!(
            parse(b"just some text, no headers at all", NOW),
            Err(EmlError::NotAMessage)
        );
    }

    #[test]
    fn a_giant_body_is_capped() {
        let body = "a".repeat(MAX_BODY_CHARS + 1024);
        let raw = format!(
            "From: ana@acme.example\r\nSubject: long\r\nDate: Mon, 1 Jan 2024 10:00:00 +0000\r\n\r\n{body}"
        );
        let message = parse(raw.as_bytes(), NOW).expect("parse");
        assert_eq!(message.body_text.chars().count(), MAX_BODY_CHARS);
    }

    #[test]
    fn pasted_text_takes_the_same_path_as_a_file() {
        let message = parse_text(&plain(), NOW).expect("parse");
        assert_eq!(message.subject, "Your application at Acme");
    }
}
