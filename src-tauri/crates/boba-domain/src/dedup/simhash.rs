//! The description simhash (AD-19): the third dedup signal.
//!
//! The same job re-published on another board is rarely word-for-word the same
//! text — a sentence is cut, the salary line is dropped. A simhash over
//! three-word shingles survives that: at or above [`SIMHASH_SIMILARITY`] the two
//! descriptions are near-duplicates.
//!
//! A description too short to be evidence returns no simhash at all. Two empty
//! descriptions are not the same job, and a global "both are empty" match would
//! merge every offer whose source sent no text.

use std::collections::HashMap;

use super::text::{fnv1a, words};

/// At or above this similarity two descriptions are the same posting (AD-19).
pub const SIMHASH_SIMILARITY: f64 = 0.9;

/// Below this many content words a description says nothing comparable.
const MIN_WORDS: usize = 20;

/// Function words, in both posting languages. They repeat in every posting, so
/// leaving them in would make two unrelated jobs look alike.
const STOPWORDS: &[&str] = &[
    "a", "about", "all", "also", "an", "and", "any", "are", "as", "at", "be", "been", "but", "by",
    "can", "company", "do", "for", "from", "has", "have", "in", "into", "is", "it", "its", "job",
    "looking", "more", "most", "not", "of", "on", "or", "our", "out", "role", "so", "that", "the",
    "their", "this", "to", "up", "us", "we", "what", "which", "who", "will", "with", "you", "your",
    "al", "como", "con", "de", "del", "el", "en", "es", "esta", "la", "las", "lo", "los", "más",
    "no", "o", "para", "por", "que", "se", "su", "sus", "un", "una", "y", "ya",
];

/// The simhash of a posting description, or `None` when it is too short to
/// compare (AD-19).
///
/// Word unigrams weighted by how often they appear, rather than shingles: a
/// board that copies a posting usually cuts or rewrites a sentence, and on a
/// text this short a shingle hash moves far more than the 10% AD-19 allows.
/// Content words only — a posting is mostly "we are looking for", and that is
/// what every other posting says too.
pub fn description_simhash(text: &str) -> Option<u64> {
    let mut frequency: HashMap<String, i32> = HashMap::new();
    for word in words(text) {
        if !STOPWORDS.contains(&word.as_str()) {
            *frequency.entry(word).or_default() += 1;
        }
    }
    if frequency.len() < MIN_WORDS {
        return None;
    }

    let mut bits = [0i32; 64];
    for (word, count) in frequency {
        let hash = fnv1a(word.as_bytes());
        for (bit, weight) in bits.iter_mut().enumerate() {
            *weight += if hash >> bit & 1 == 1 { count } else { -count };
        }
    }

    Some(bits.iter().enumerate().fold(
        0u64,
        |hash, (bit, weight)| {
            if *weight > 0 { hash | 1 << bit } else { hash }
        },
    ))
}

/// How alike two simhashes are, from 0 (nothing in common) to 1 (identical):
/// the share of the 64 bits they agree on.
pub fn similarity(left: u64, right: u64) -> f64 {
    1.0 - f64::from((left ^ right).count_ones()) / 64.0
}
