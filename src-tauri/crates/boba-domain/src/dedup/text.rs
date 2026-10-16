//! The text helpers the dedup key and the simhash share: folding, splitting
//! into words, and one stable hash. Untrusted text (AD-7) goes through here, so
//! nothing here may panic on any input.

/// A label as its lowercased words, with punctuation and accents spent:
/// `Acmé, Inc.` → `["acme", "inc"]`.
pub(crate) fn words(text: &str) -> Vec<String> {
    folded(text)
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// A label as one identity string: lowercase, accents folded, runs of
/// whitespace collapsed, inner punctuation **kept** so `C++ Developer` is not
/// `C Developer`.
pub(crate) fn identity(text: &str) -> String {
    folded(text)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_string()
}

/// FNV-1a 64. Inlined because the values here only have to be stable for the
/// life of one local database, not cryptographic — and this crate keeps its
/// dependency list short (AD-1).
pub(crate) fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn folded(text: &str) -> impl Iterator<Item = char> + '_ {
    text.chars().flat_map(|c| fold(c).to_lowercase())
}

/// Fold the accents ES/EN/FR/DE boards actually send, so one company has one
/// name. Anything else keeps its own spelling.
fn fold(c: char) -> char {
    match c {
        'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ñ' => 'n',
        'ç' => 'c',
        other => other,
    }
}
