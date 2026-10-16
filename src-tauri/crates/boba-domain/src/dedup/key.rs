//! The `job_key`: one slug for one job, whoever published it (AD-19).
//!
//! A port of upstream's `job_key.py`: the slug of company + title + location
//! bucket, with an eight-hex **disambiguator** of the same three values so two
//! different jobs whose slugs collide (`C++ Developer` and `C Developer` both
//! slug to `c-developer`) never share a key.
//!
//! The parts are normalised hard on purpose: `Acme Corp` and `Acmé, Inc.` are
//! one company, and `Remote` and `Anywhere` are one location bucket, because
//! two boards never spell a job the same way. Labels with no Latin letters at
//! all still get a key — the slug is empty and the disambiguator carries it.

use super::text::{fnv1a, identity, words};
use crate::posting::Offer;

/// Legal forms a board may or may not append to a company name. Stripped only
/// while something is left, so a company actually called "Co" keeps its name.
const LEGAL_SUFFIXES: &[&str] = &[
    "inc",
    "llc",
    "ltd",
    "limited",
    "gmbh",
    "corp",
    "corporation",
    "co",
    "sa",
    "sas",
    "sl",
    "bv",
    "nv",
    "ag",
    "ab",
    "oy",
    "pty",
    "plc",
    "srl",
    "spa",
];

/// Every word the boards use for "you can work from anywhere".
const REMOTE_WORDS: &[&str] = &[
    "remote",
    "remoto",
    "remota",
    "anywhere",
    "worldwide",
    "global",
    "distributed",
    "telecommute",
    "home",
];

/// The dedup key of an Offer (AD-19). Equal keys mean the same job; the key is
/// stored in `posting.dedup_key` and indexed, so a re-run finds the posting it
/// already made instead of making a second one.
pub fn job_key_of(offer: &Offer) -> String {
    let company = company_words(&offer.company);
    let title = words(&offer.title);
    let bucket = location_bucket(&offer.locations);

    if company.is_empty() && title.is_empty() {
        // Nothing identifies the job. Every anonymous Offer would get the same
        // key, and dedup would merge them all into one.
        return String::new();
    }

    let slug: Vec<String> = [company.join("-"), title.join("-"), bucket.join("-")]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect();
    let identity = [company.join(" "), identity(&offer.title), bucket.join(" ")].join("|");

    format!("{}-{}", slug.join("-"), disambiguator(&identity))
}

/// The company pool the description simhash is compared within: `Acme Corp` →
/// `acme`. Two boards spell the same employer differently far more often than
/// they retitle the job, so the company is the pool, the title is not.
pub(crate) fn company_key(company: &str) -> String {
    company_words(company).join("-")
}

/// The company, without its legal form: `Acme Corp` → `acme`. A dotted form
/// splits into single letters (`S.A.S.` → `s a s`), so a trailing run of them is
/// tried as one too — an ES board writes `Bogotá Labs S.A.S.` where a US one
/// writes `Bogota Labs`.
fn company_words(company: &str) -> Vec<String> {
    let mut words = words(company);
    while words.len() > 1 {
        if LEGAL_SUFFIXES.contains(&words[words.len() - 1].as_str()) {
            words.pop();
            continue;
        }
        let tail = dotted_tail(&words);
        if LEGAL_SUFFIXES.contains(&tail.as_str()) && !tail.is_empty() {
            words.truncate(words.len() - tail.len());
            continue;
        }
        break;
    }
    words
}

/// The trailing run of single letters, as it would have been written dotted.
fn dotted_tail(words: &[String]) -> String {
    words
        .iter()
        .rev()
        .take_while(|word| word.len() == 1)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .cloned()
        .collect()
}

/// The bucket of one Offer, as the string the key is built from (and the string
/// two Offers have to agree on before a similar description means anything).
pub(crate) fn bucket_of(offer: &Offer) -> String {
    location_bucket(&offer.locations).join(" ")
}

/// The location bucket: `remote` when any location says so, else the city
/// before the first comma (`Bogotá, CO` → `bogota`), else nothing — a source
/// that sends no location must not be bucketed with every other unknown.
fn location_bucket(locations: &[String]) -> Vec<String> {
    for location in locations {
        if words(location)
            .iter()
            .any(|word| REMOTE_WORDS.contains(&word.as_str()))
        {
            return vec!["remote".to_string()];
        }
    }
    locations
        .iter()
        .find_map(|location| {
            let city = words(location.split(',').next().unwrap_or_default());
            (!city.is_empty()).then_some(city)
        })
        .unwrap_or_default()
}

/// Eight hex chars of the hash over the identity parts — the disambiguator that
/// keeps two jobs with the same slug apart.
fn disambiguator(identity: &str) -> String {
    format!("{:08x}", fnv1a(identity.as_bytes()) & 0xffff_ffff)
}
