//! Score narration (S6.5, FR-59, AD-5, ADR-010).
//!
//! The agent writes the coach's two-to-four sentence explanation of a
//! [`ScoreCard`]; the core decides whether to keep it. Nothing it says may
//! introduce a number or a skill the card does not already contain (FR-59's
//! test), so the words can be warm without ever being invented.
//!
//! Two rules are checked here, both deterministic and offline:
//!
//! * **Sentence count.** FR-59 asks for 2–4 sentences; one is a summary and five
//!   is a report.
//! * **Grounding.** Every number the text names must be one the card holds (a
//!   factor score, a weight, a count, or a number quoted in its evidence), and
//!   every skill-like term it capitalises mid-sentence must appear in the
//!   card's `matched` or `missing` lists.
//!
//! What is deliberately *not* checked: tone, style, and the narration's
//! language. Those are the skill's job (`agent-pack/skills/narrate_scores`), and
//! a local heuristic cannot judge them honestly. The card's numbers are the
//! core's; the narration only puts them in words.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::rank::ScoreCard;

/// FR-59: the shortest narration the UI shows.
pub const MIN_SENTENCES: usize = 2;
/// FR-59: the longest narration the UI shows.
pub const MAX_SENTENCES: usize = 4;

/// One stored Explanation (architecture §12's `score_narration`).
///
/// It is bound to the exact card it explains ([`score_card_id`]): a re-score
/// writes a new card, and the old narration stays with the card it described
/// rather than being re-attached to numbers it never saw (AD-8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoreNarration {
    /// ULID of the row (`nrt_…`).
    pub id: String,
    pub posting_id: String,
    /// The `score_card` row this text explains.
    pub score_card_id: String,
    /// The UI language the text is written in (BCP-47: `en`, `es`).
    pub lang: String,
    /// The narration itself. Untrusted (ADR-010): always rendered as text.
    pub text: String,
    /// ADR-010: the narration is derived from third-party text, so this is `1`
    /// for every row the agent writes. A column rather than a constant because
    /// the schema is the one place the rule cannot be forgotten.
    pub untrusted_origin: bool,
    /// UTC RFC 3339, from the injected clock (AD-17).
    pub created_at: String,
}

/// A fresh ULID for a stored narration (AD-17: `nrt_…`).
pub fn new_narration_id() -> String {
    format!("nrt_{}", ulid::Ulid::generate())
}

/// Why a narration was refused. Every variant is about the text, never about
/// the storage — a run that is refused is asked to write it again.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NarrationViolation {
    /// The text is empty or whitespace.
    #[error("the narration is empty")]
    Empty,
    /// FR-59: 2–4 sentences.
    #[error("the narration has {0} sentences; FR-59 asks for {MIN_SENTENCES}–{MAX_SENTENCES}")]
    Sentences(usize),
    /// FR-59: every number named must exist in the factor data.
    #[error("the narration names the number `{0}`, which is not in the ScoreCard")]
    Number(String),
    /// FR-59: every skill named must exist in the factor data.
    #[error("the narration names `{0}`, which is not in the ScoreCard")]
    Term(String),
}

/// The words a narration may use that are not skills even when capitalised
/// mid-sentence: the axes and the app talk about scores, and every sentence
/// begins with a capitalised word.
const STOPWORDS: &[&str] = &[
    "a", "an", "and", "as", "at", "but", "by", "boba", "fit", "for", "from", "in", "into", "is",
    "it", "its", "of", "on", "or", "our", "quality", "rank", "so", "that", "the", "their", "they",
    "this", "to", "yo", "tu", "tus", "su", "sus", "el", "la", "los", "las", "un", "una", "y", "o",
    "pero", "con", "sin", "por", "para", "en", "es", "son", "de", "del", "que", "se", "tu",
    "encaje", "calidad", "puesto", "oferta",
];

/// Check one narration against the card it explains (FR-59).
///
/// Runs before anything is stored: a run that invents a number or a skill is
/// refused whole, and the refusal names what it invented so the run can fix it.
pub fn check(text: &str, card: &ScoreCard) -> Result<(), NarrationViolation> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(NarrationViolation::Empty);
    }

    let sentences = sentence_count(trimmed);
    if !(MIN_SENTENCES..=MAX_SENTENCES).contains(&sentences) {
        return Err(NarrationViolation::Sentences(sentences));
    }

    let allowed_numbers = allowed_numbers(card);
    for number in numbers_in(trimmed) {
        if !allowed_numbers.contains(&number) {
            return Err(NarrationViolation::Number(number));
        }
    }

    let vocabulary = card_vocabulary(card);
    for term in named_terms(trimmed) {
        if !vocabulary.contains(&term.to_lowercase()) {
            return Err(NarrationViolation::Term(term));
        }
    }

    Ok(())
}

/// How many sentences a text holds: segments that end in `.`, `!` or `?` and
/// carry at least one letter or digit. A trailing fragment without punctuation
/// counts too, so "One. Two" is two sentences.
fn sentence_count(text: &str) -> usize {
    let mut count = 0;
    let mut has_word = false;
    for character in text.chars() {
        if character.is_alphanumeric() {
            has_word = true;
        } else if matches!(character, '.' | '!' | '?') && has_word {
            count += 1;
            has_word = false;
        }
    }
    if has_word {
        count += 1;
    }
    count
}

/// Every number the card can honestly be described with: the axes and factor
/// scores (rounded, as the UI shows them), the weights (as percentages), the
/// matched/missing counts, and any number quoted in the card's own evidence.
fn allowed_numbers(card: &ScoreCard) -> HashSet<String> {
    let mut allowed = HashSet::new();

    if let Some(fit) = card.fit {
        push_rounded(&mut allowed, fit);
    }
    if let Some(quality) = card.quality {
        push_rounded(&mut allowed, quality);
    }
    push_rounded(&mut allowed, card.rank);

    let mut total_matched = 0;
    let mut total_missing = 0;
    for factor in &card.factors {
        if let Some(score) = factor.score {
            push_rounded(&mut allowed, score);
        }
        push_rounded(&mut allowed, factor.weight * 100.0);
        push_rounded(&mut allowed, factor.effective_weight * 100.0);
        allowed.insert(factor.matched.len().to_string());
        allowed.insert(factor.missing.len().to_string());
        allowed.insert((factor.matched.len() + factor.missing.len()).to_string());
        total_matched += factor.matched.len();
        total_missing += factor.missing.len();
        for line in factor.matched.iter().chain(factor.missing.iter()) {
            for number in numbers_in(line) {
                allowed.insert(number);
            }
        }
        for evidence in &factor.evidence {
            let words = evidence_text(evidence);
            for number in numbers_in(&words) {
                allowed.insert(number);
            }
        }
    }
    allowed.insert(total_matched.to_string());
    allowed.insert(total_missing.to_string());
    allowed.insert((total_matched + total_missing).to_string());

    allowed
}

/// One rounded number into the allowed set, as the UI would show it.
fn push_rounded(allowed: &mut HashSet<String>, value: f32) {
    allowed.insert(format!("{}", value.round() as i64));
}

/// The card's own skill vocabulary: every word of every `matched` and `missing`
/// entry, lowercased, so a narration may name them back without inventing one.
fn card_vocabulary(card: &ScoreCard) -> HashSet<String> {
    let mut vocabulary = HashSet::new();
    for factor in &card.factors {
        for term in factor.matched.iter().chain(factor.missing.iter()) {
            for word in words(term) {
                vocabulary.insert(word.to_lowercase());
            }
        }
    }
    vocabulary
}

/// The evidence of one factor as plain words, for the number check.
fn evidence_text(evidence: &super::gates::Evidence) -> String {
    match evidence {
        super::gates::Evidence::Quoted(span) => span.text.clone(),
        super::gates::Evidence::Field(name) => name.clone(),
    }
}

/// The digit runs of a text, normalised: `65,000` and `65000` are the same
/// number. A decimal point between digits is kept, so `82.5` stays `82.5`.
fn numbers_in(text: &str) -> Vec<String> {
    let mut numbers = Vec::new();
    let mut current = String::new();
    let mut last_was_digit = false;
    for character in text.chars() {
        if character.is_ascii_digit() {
            current.push(character);
            last_was_digit = true;
        } else if matches!(character, ',' | '.') && last_was_digit && next_is_digit(text, character)
        {
            // A separator inside a number; keep a decimal point only.
            if character == '.' {
                current.push('.');
            }
        } else {
            if !current.is_empty() {
                numbers.push(current.trim_end_matches('.').to_string());
                current.clear();
            }
            last_was_digit = false;
        }
    }
    if !current.is_empty() {
        numbers.push(current.trim_end_matches('.').to_string());
    }
    numbers
}

/// Whether the separator at this point of the string is followed by a digit.
/// The scanning loop cannot look ahead cheaply, so this re-reads the tail; the
/// strings are one sentence long, so the cost is irrelevant.
fn next_is_digit(text: &str, separator: char) -> bool {
    let mut seen = false;
    for character in text.chars() {
        if !seen {
            if character == separator {
                seen = true;
            }
            continue;
        }
        return character.is_ascii_digit();
    }
    false
}

/// The words of a phrase, keeping `c++`, `c#` and `node.js` whole enough to be
/// matched back: letters, digits and the `+#.` that technical names use.
fn words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    for character in text.chars() {
        if character.is_alphanumeric()
            || matches!(character, '+' | '#')
            || (character == '.' && !current.is_empty())
        {
            current.push(character);
        } else {
            if !current.is_empty() {
                words.push(current.trim_end_matches('.').to_string());
                current.clear();
            }
        }
    }
    if !current.is_empty() {
        words.push(current.trim_end_matches('.').to_string());
    }
    words
}

/// The skill-like terms a narration names: a capitalised word that is not the
/// first word of a sentence and not one of [`STOPWORDS`]. Lowercase technical
/// names that are not in the card are a limitation of a local heuristic; the
/// skill tells the run to copy the card's spelling.
fn named_terms(text: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let mut at_sentence_start = true;
    for raw in text.split_whitespace() {
        let word = raw.trim_matches(|c: char| !c.is_alphanumeric() && c != '+' && c != '#');
        if word.is_empty() {
            continue;
        }
        let starts_upper = word.chars().next().is_some_and(char::is_uppercase);
        let is_acronym = word.len() >= 2 && word.chars().all(|c| c.is_ascii_uppercase());
        let known = STOPWORDS.contains(&word.to_lowercase().as_str());
        if !at_sentence_start
            && (starts_upper || is_acronym)
            && !known
            && !word.chars().all(char::is_numeric)
        {
            terms.push(word.to_string());
        }
        at_sentence_start = raw.ends_with(['.', '!', '?']);
    }
    terms
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scoring::factors::{Factor, FactorScore};
    use crate::scoring::gates::Confidence;
    use crate::scoring::rank::Verdict;

    /// A card with two named skills and one missing one, at known numbers.
    fn card() -> ScoreCard {
        let mut factors = Vec::new();
        for factor in Factor::ALL {
            let mut score = FactorScore::scored(factor, 80.0, 0.1, Confidence::High, Vec::new());
            if factor == Factor::Skills {
                score.score = Some(80.0);
                score.matched = vec!["dbt".to_string(), "Airflow".to_string()];
                score.missing = vec!["Kubernetes".to_string()];
            }
            factors.push(score);
        }
        ScoreCard {
            model_version: 1,
            factors,
            fit: Some(80.0),
            quality: Some(80.0),
            rank: 80.0,
            verdict: Verdict::Strong,
            confidence: Confidence::High,
        }
    }

    #[test]
    fn a_grounded_narration_is_accepted() {
        let text = "You match 80 of 80 points here, with dbt and Airflow standing out. \
                    Kubernetes is the one skill missing, so that is the gap to close.";
        assert_eq!(check(text, &card()), Ok(()));
    }

    #[test]
    fn an_invented_number_is_refused() {
        let text = "You match 97 of 99 points here. dbt and Airflow stand out.";
        assert_eq!(
            check(text, &card()),
            Err(NarrationViolation::Number("97".to_string()))
        );
    }

    #[test]
    fn an_invented_skill_is_refused() {
        let text = "You match the skills here, with dbt and Airflow. The gap is Terraform.";
        assert_eq!(
            check(text, &card()),
            Err(NarrationViolation::Term("Terraform".to_string()))
        );
    }

    #[test]
    fn one_sentence_is_too_short() {
        let text = "You match dbt and Airflow.";
        assert_eq!(check(text, &card()), Err(NarrationViolation::Sentences(1)));
    }

    #[test]
    fn five_sentences_are_too_long() {
        let text = "One. Two. Three. Four. Five.";
        assert_eq!(check(text, &card()), Err(NarrationViolation::Sentences(5)));
    }

    #[test]
    fn an_empty_narration_is_refused() {
        assert_eq!(check("   ", &card()), Err(NarrationViolation::Empty));
    }

    #[test]
    fn a_number_quoted_by_the_card_counts_as_grounded() {
        let mut card = card();
        card.factors[0].missing = vec!["5 years of Kubernetes".to_string()];
        let text = "You match most of the profile. The posting asks for 5 years of Kubernetes.";
        assert_eq!(check(text, &card), Ok(()));
    }

    #[test]
    fn narration_ids_are_addressable() {
        assert!(new_narration_id().starts_with("nrt_"));
    }
}
