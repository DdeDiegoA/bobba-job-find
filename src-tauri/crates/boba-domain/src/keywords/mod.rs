//! Requirement keywords: what the Profile backs, and what it does not (FR-75,
//! SM-8, AD-15).
//!
//! The Offer names its key requirements. Each one is a handful of keywords, and
//! for every keyword there is exactly one question: does a Profile fact back it?
//!
//! * **backed** — the keyword must be readable in the rendered text layer, so
//!   [`backed_keywords`] is the only list a caller may require of a document
//!   (FR-77, and the whole reason this module exists);
//! * **a gap** — no fact backs it. Gaps are shown to the user and are **never
//!   inserted** into a draft (FR-75, AD-15). [`backed_keywords`] never returns
//!   one, so no caller can put one in a document by accident.
//!
//! Matching is deliberately dumb and deterministic (no LLM, no embeddings): fold
//! case and diacritics, compare word by word against the fact's text **in the
//! draft's language**. A fact carries one variant per language and a missing
//! variant is flagged, never invented (AD-15), so a keyword is only ever backed
//! by the variant the draft is written in.

use serde::{Deserialize, Serialize};

use crate::profile::{Lang, ProfileFact};

#[cfg(test)]
mod tests;

/// Words that state no requirement on their own, in both board languages.
const NOISE: &[&str] = &[
    // English
    "a", "an", "and", "are", "as", "at", "be", "been", "but", "by", "etc", "for", "from", "had",
    "has", "have", "in", "into", "is", "it", "its", "of", "on", "or", "our", "plus", "that", "the",
    "their", "them", "they", "this", "to", "us", "was", "we", "were", "will", "with", "would",
    "you", "your", "yours",
    // Spanish
    "al", "como", "con", "de", "del", "el", "en", "es", "esta", "este", "la", "las", "lo", "los",
    "mas", "nuestra", "nuestro", "o", "para", "pero", "por", "que", "se", "son", "su", "sus", "tu",
    "tus", "un", "una", "unas", "unos", "y",
];

/// A keyword a Profile fact backs, or a gap it does not (FR-75).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Coverage {
    /// A Profile fact backs it: the studio shows it, and the ATS check requires
    /// it in the text layer (FR-77).
    Covered,
    /// Nothing in the Profile backs it. Never inserted (FR-75).
    Gap,
}

/// One requirement of the Offer with what the Profile backs of it (FR-75).
///
/// The studio shows `requirement` as covered or a gap, and lists `gaps` as the
/// declared gaps that were **not** added to the draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementCoverage {
    /// The requirement as the Offer states it, so the user recognises it.
    pub requirement: String,
    /// Keywords a Profile fact backs, in the requirement's own order: what the
    /// text layer must show (AC1).
    pub covered: Vec<String>,
    /// Keywords no Profile fact backs (FR-75). Never inserted into a document.
    pub gaps: Vec<String>,
}

impl RequirementCoverage {
    /// Covered when every keyword of the requirement is backed; a requirement
    /// with no keyword at all is vacuously covered and has nothing to show.
    pub fn is_covered(&self) -> bool {
        self.gaps.is_empty()
    }

    /// Every keyword the requirement stated, backed or not.
    pub fn keywords(&self) -> usize {
        self.covered.len() + self.gaps.len()
    }

    /// The keywords of this requirement, each with its verdict: what the panel
    /// lists, in the requirement's own order.
    pub fn verdicts(&self) -> impl Iterator<Item = (&str, Coverage)> {
        self.covered
            .iter()
            .map(|k| (k.as_str(), Coverage::Covered))
            .chain(self.gaps.iter().map(|k| (k.as_str(), Coverage::Gap)))
    }
}

/// Fold a text for matching: lowercase, diacritics stripped (FR-75, FR-77).
///
/// `Ingeniería` and `ingenieria` are the same keyword, and so are `Bogotá` and
/// `Bogota`; a fact and a requirement never miss each other over an accent.
pub fn fold(text: &str) -> String {
    text.chars().map(fold_char).flat_map(char::to_lowercase).collect()
}

/// The words of a text, folded, each once in the order it appears.
///
/// `+`, `#` and `.` stay inside a word so `c++`, `c#`, `node.js` and `.net` are
/// keywords and not punctuation; everything else splits.
pub fn words(text: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    for ch in fold(text).chars() {
        if ch.is_alphanumeric() || ch == '+' || ch == '#' || ch == '.' {
            current.push(ch);
        } else {
            push_word(&mut words, &mut current);
        }
    }
    push_word(&mut words, &mut current);
    words
}

/// The keywords a requirement states: its words, minus the noise (stopwords and
/// bare numbers such as the `5+` of "5+ years"), each once, in order (FR-75).
pub fn requirement_keywords(requirement: &str) -> Vec<String> {
    let mut keywords: Vec<String> = Vec::new();
    for word in words(requirement) {
        let word = word.trim_matches('.');
        if word.is_empty() || NOISE.contains(&word) || !word.chars().any(char::is_alphabetic) {
            continue;
        }
        if !keywords.iter().any(|seen| seen == word) {
            keywords.push(word.to_string());
        }
    }
    keywords
}

/// Every requirement of the Offer with what the Profile backs of it (FR-75).
///
/// `lang` is the language the draft is written in (AD-17): a fact backs a
/// keyword through its variant in that language.
pub fn coverage(
    requirements: &[String],
    profile: &[ProfileFact],
    lang: Lang,
) -> Vec<RequirementCoverage> {
    let backed = profile_words(profile, lang);
    requirements
        .iter()
        .map(|requirement| {
            let mut covered = Vec::new();
            let mut gaps = Vec::new();
            for keyword in requirement_keywords(requirement) {
                if backed.contains(&keyword) {
                    covered.push(keyword);
                } else {
                    gaps.push(keyword);
                }
            }
            RequirementCoverage {
                requirement: requirement.trim().to_string(),
                covered,
                gaps,
            }
        })
        .collect()
}

/// The keywords a Profile fact backs, each once and sorted: exactly what the
/// text layer must show (FR-77, AC1).
///
/// A gap is never in here (FR-75): this is the only list a caller may require of
/// a document, so an unbacked keyword cannot reach a draft through it.
pub fn backed_keywords(requirements: &[String], profile: &[ProfileFact], lang: Lang) -> Vec<String> {
    let mut keywords: Vec<String> = coverage(requirements, profile, lang)
        .into_iter()
        .flat_map(|coverage| coverage.covered)
        .collect();
    keywords.sort();
    keywords.dedup();
    keywords
}

/// The keywords no Profile fact backs, each once and sorted (FR-75, AD-15).
///
/// Shown as declared gaps; never inserted into a document.
pub fn gaps(requirements: &[String], profile: &[ProfileFact], lang: Lang) -> Vec<String> {
    let mut keywords: Vec<String> = coverage(requirements, profile, lang)
        .into_iter()
        .flat_map(|coverage| coverage.gaps)
        .collect();
    keywords.sort();
    keywords.dedup();
    keywords
}

/// Every word the Profile has in `lang`, over all its facts: what can back a
/// keyword (AC1).
fn profile_words(profile: &[ProfileFact], lang: Lang) -> Vec<String> {
    profile
        .iter()
        .filter_map(|fact| fact.text(lang))
        .flat_map(words)
        .collect()
}

/// One folded character, for the accents the two board languages use.
fn fold_char(ch: char) -> char {
    match ch {
        'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ñ' => 'n',
        'ç' => 'c',
        'ý' | 'ÿ' => 'y',
        'ß' => 's',
        _ => ch,
    }
}

/// Close a word and start the next one.
fn push_word(words: &mut Vec<String>, current: &mut String) {
    let word = current.trim_matches('.').to_string();
    current.clear();
    if !word.is_empty() {
        words.push(word);
    }
}
