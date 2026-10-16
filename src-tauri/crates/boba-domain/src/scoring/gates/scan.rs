//! Deterministic scanning of posting text for the Gates (FR-53, AD-8): the
//! quoted evidence spans and the two posting-side lexicons — work-eligibility
//! wording and required working languages. Offline, no LLM, no IO.
//!
//! Matching runs on a byte-length-preserving lowercase fold, so a span's offsets
//! always point back into `Offer::description_text` and the UI can quote the
//! exact words (FR-53, AD-7: the quote is plain data, never markup).

use serde::{Deserialize, Serialize};

use super::{Cefr, PermitKind};

/// An exact span of a posting's text: byte offsets into the original string,
/// with the quoted text, trimmed. `text[start..end] == text`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuotedSpan {
    pub text: String,
    pub start: usize,
    pub end: usize,
}

/// Eligibility wording → the authorization it demands. `region` is a lowercase
/// ISO 3166-1 alpha-2 code or `eu`; an empty region with [`PermitKind::Clearance`]
/// is region-free.
const ELIGIBILITY: &[(&str, &str, PermitKind)] = &[
    (
        "must be authorized to work in the us",
        "us",
        PermitKind::WorkPermit,
    ),
    ("authorized to work in the us", "us", PermitKind::WorkPermit),
    (
        "must have the right to work in the us",
        "us",
        PermitKind::WorkPermit,
    ),
    ("must be a us citizen", "us", PermitKind::Citizenship),
    ("us citizen", "us", PermitKind::Citizenship),
    ("u.s. citizen", "us", PermitKind::Citizenship),
    ("us citizenship", "us", PermitKind::Citizenship),
    ("us only", "us", PermitKind::WorkPermit),
    ("u.s. only", "us", PermitKind::WorkPermit),
    ("must be an eu citizen", "eu", PermitKind::Citizenship),
    ("eu citizenship", "eu", PermitKind::Citizenship),
    (
        "must be authorized to work in the eu",
        "eu",
        PermitKind::WorkPermit,
    ),
    ("authorized to work in the eu", "eu", PermitKind::WorkPermit),
    ("right to work in the eu", "eu", PermitKind::WorkPermit),
    ("eu work permit", "eu", PermitKind::WorkPermit),
    ("eu only", "eu", PermitKind::WorkPermit),
    ("security clearance", "", PermitKind::Clearance),
    ("clearance required", "", PermitKind::Clearance),
];

/// Language names as postings write them (EN and ES) → BCP-47 tag.
const LANGUAGES: &[(&str, &str)] = &[
    ("english", "en"),
    ("inglés", "en"),
    ("spanish", "es"),
    ("español", "es"),
    ("castellano", "es"),
    ("german", "de"),
    ("alemán", "de"),
    ("deutsch", "de"),
    ("french", "fr"),
    ("francés", "fr"),
    ("portuguese", "pt"),
    ("portugués", "pt"),
    ("italian", "it"),
    ("italiano", "it"),
    ("dutch", "nl"),
    ("neerlandés", "nl"),
];

/// Words that turn a named language into a requirement (FR-53). English and
/// Spanish, because both boards feed the same run (FR-1).
const REQUIREMENTS: &[&str] = &[
    "required",
    "requires",
    "requirement",
    "mandatory",
    "requerido",
    "requiere",
    "fluent",
    "fluency",
    "fluido",
    "fluidez",
    "native",
    "nativo",
    "bilingual",
    "bilingüe",
    "proficient",
    "proficiency",
    "advanced",
    "must speak",
    "working knowledge",
    "excellent command",
    "mother tongue",
    "lengua materna",
    "nivel",
];

/// Level words postings write instead of a CEFR level, mapped to the bar they
/// stand for. A posting that states no level at all asks for [`Cefr::A1`].
const LEVEL_WORDS: &[(&str, Cefr)] = &[
    ("mother tongue", Cefr::C2),
    ("lengua materna", Cefr::C2),
    ("native", Cefr::C2),
    ("nativo", Cefr::C2),
    ("bilingual", Cefr::C2),
    ("bilingüe", Cefr::C2),
    ("fluent", Cefr::C1),
    ("fluency", Cefr::C1),
    ("fluido", Cefr::C1),
    ("fluidez", Cefr::C1),
    ("proficient", Cefr::C1),
    ("proficiency", Cefr::C1),
    ("advanced", Cefr::B2),
    ("professional", Cefr::B2),
    ("working knowledge", Cefr::B2),
    ("conversational", Cefr::B1),
    ("intermediate", Cefr::B1),
    ("basic", Cefr::A2),
    ("beginner", Cefr::A2),
];

/// Lowercase without changing the byte length, so match offsets stay valid on
/// the original text (`İ` lowercases to two bytes: such a character is left as
/// it stands).
fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        let mut lower = ch.to_lowercase();
        let first = lower.next().unwrap_or(ch);
        match (lower.next(), first.len_utf8() == ch.len_utf8()) {
            (None, true) => out.push(first),
            _ => out.push(ch),
        }
    }
    out
}

/// Every eligibility demand the posting states, with the sentence that states
/// it, in document order. Two phrasings of one demand are one demand.
pub(super) fn demands(text: &str) -> Vec<(QuotedSpan, &'static str, PermitKind)> {
    let folded = fold(text);
    let mut found: Vec<(QuotedSpan, &'static str, PermitKind)> = Vec::new();
    for (phrase, region, permit) in ELIGIBILITY {
        let mut from = 0;
        while let Some(offset) = folded[from..].find(phrase) {
            let start = from + offset;
            from = start + phrase.len();
            if found.iter().any(|(_, found_region, found_permit)| {
                found_region == region && found_permit == permit
            }) {
                continue;
            }
            found.push((sentence(text, start, from), region, *permit));
        }
    }
    found.sort_by_key(|(span, _, _)| span.start);
    found
}

/// Required working languages: the sentence that requires each one, its tag and
/// the level bar it states (if any). A language the posting merely names is not
/// a requirement.
pub(super) fn required_languages(text: &str) -> Vec<(QuotedSpan, &'static str, Option<Cefr>)> {
    let folded = fold(text);
    let mut found: Vec<(QuotedSpan, &'static str, Option<Cefr>)> = Vec::new();
    for (name, tag) in LANGUAGES {
        let mut from = 0;
        while let Some(offset) = folded[from..].find(name) {
            let start = from + offset;
            let end = start + name.len();
            from = end;
            if found.iter().any(|(_, found_tag, _)| found_tag == tag) {
                continue;
            }
            let span = sentence(text, start, end);
            let near = near(text, &span, start, end);
            if !requires(fold(near).as_str()) {
                continue;
            }
            found.push((span, *tag, stated_bar(&fold(near))));
        }
    }
    found.sort_by_key(|(span, _, _)| span.start);
    found
}

/// Whether a window of text states a requirement.
fn requires(window: &str) -> bool {
    REQUIREMENTS.iter().any(|word| window.contains(word))
}

/// The bar a posting states: an explicit CEFR level wins over a level word.
/// `None` when it states none, which any declared level clears.
fn stated_bar(window: &str) -> Option<Cefr> {
    let levels: Vec<Cefr> = window
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter_map(Cefr::parse)
        .collect();
    if let Some(explicit) = levels.into_iter().max() {
        return Some(explicit);
    }
    LEVEL_WORDS
        .iter()
        .filter(|(word, _)| window.contains(word))
        .map(|(_, level)| *level)
        .max()
}

/// The sentence (or bullet line) around `[start, end)`, trimmed — the words the
/// UI quotes. Boundaries are `.` `!` `?` `;` and a newline; a dot inside a word
/// ("U.S.") does not end a sentence.
fn sentence(text: &str, start: usize, end: usize) -> QuotedSpan {
    let left = text[..start]
        .char_indices()
        .filter(|(i, ch)| ends_sentence(text, *i, *ch))
        .map(|(i, ch)| i + ch.len_utf8())
        .next_back()
        .unwrap_or(0);
    let right = text[end..]
        .char_indices()
        .find(|(i, ch)| ends_sentence(text, end + *i, *ch))
        .map_or(text.len(), |(i, _)| end + i);
    let raw = &text[left..right];
    let lead = raw.len() - raw.trim_start().len();
    let text = raw.trim();
    QuotedSpan {
        text: text.to_string(),
        start: left + lead,
        end: left + lead + text.len(),
    }
}

/// Whether the character at byte `i` ends a sentence.
fn ends_sentence(text: &str, i: usize, ch: char) -> bool {
    match ch {
        '!' | '?' | ';' | '\n' => true,
        '.' => !text[i + 1..]
            .chars()
            .next()
            .is_some_and(char::is_alphanumeric),
        _ => false,
    }
}

/// The text within 40 characters of `[start, end)`, never reaching outside the
/// sentence: a requirement word in another sentence is not about this language.
///
/// shortcut: a fixed 40-character window with sentence clamping, no NLP; widen
/// it only if real postings miss requirements.
fn near<'a>(text: &'a str, sentence: &QuotedSpan, start: usize, end: usize) -> &'a str {
    let left = text[..start]
        .char_indices()
        .rev()
        .nth(40)
        .map_or(0, |(i, _)| i)
        .max(sentence.start);
    let right = text[end..]
        .char_indices()
        .nth(40)
        .map_or(text.len(), |(i, _)| end + i)
        .min(sentence.end);
    &text[left..right]
}
