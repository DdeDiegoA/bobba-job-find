//! F3 — domain/role similarity (FR-55, `offer-scoring.md` §3; S6.9).
//!
//! The encoder itself is an adapter (`boba-embed`, crate `embeddings` feature);
//! what lives here is everything F3 needs that is pure: the model it ships,
//! the two texts an E5 encoder reads (with its asymmetric prefixes), and the
//! cosine → 0–1 mapping [`SemanticScore`] accepts.

use std::path::PathBuf;

use crate::posting::Offer;
use crate::scoring::factors::{Prefs, ProfileSnapshot, SemanticScore};

/// The one model this build ships (tech-stack.md): small, multilingual, 384-dim.
/// fastembed's `EmbeddingModel::MultilingualE5Small` is the same repo.
pub const MODEL_ID: &str = "intfloat/multilingual-e5-small";

/// What the encoder emits per text.
pub const MODEL_DIMENSION: usize = 384;

/// What the opt-in download costs on disk (Story 6.9 AC1).
///
/// The PRD's "about 120 MB" is the quantized build's size; the encoder ships
/// the portable fp32 `model.onnx` (the q8 file is built for AVX512-VNNI CPUs
/// and would not run on every Windows/macOS/Linux machine), so the disclosure
/// states the truth: 470 MB of weights, 17 MB of tokenizer, ~487 MB in all.
/// Under-stating a download is the one thing an opt-in copy must never do.
pub const APPROXIMATE_BYTES: u64 = 487_352_505;

/// Which side of the pair a text is. E5 was trained with asymmetric prefixes:
/// the roles the user wants are the *query*, the Offer is the *passage*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Query,
    Passage,
}

impl Side {
    /// E5's prefix for this side, including its trailing space.
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Query => "query: ",
            Self::Passage => "passage: ",
        }
    }
}

/// The text the encoder reads for one side, with E5's prefix applied exactly
/// once (a caller that already prefixed it is left alone).
pub fn encode_input(side: Side, text: &str) -> String {
    let text = text.trim();
    if text.starts_with(side.prefix()) {
        text.to_string()
    } else {
        format!("{}{text}", side.prefix())
    }
}

/// The Profile's side of the comparison, or `None` when the Profile says
/// nothing — then F3 drops out instead of encoding an empty string, which
/// would be a similarity to nothing.
pub fn profile_text(profile: &ProfileSnapshot, prefs: &Prefs) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if !prefs.roles.is_empty() {
        parts.push(format!("Looking for {}.", prefs.roles.join(", ")));
    }
    if !profile.skills.is_empty() {
        parts.push(format!("Skills: {}.", profile.skills.join(", ")));
    }
    if let Some(seniority) = profile.seniority {
        match profile.years_experience {
            Some(years) => parts.push(format!(
                "{} level with {years} years of experience.",
                seniority.as_str()
            )),
            None => parts.push(format!("{} level.", seniority.as_str())),
        }
    } else if let Some(years) = profile.years_experience {
        parts.push(format!("{years} years of experience."));
    }
    if !profile.education.is_empty() {
        parts.push(format!("Education: {}.", profile.education.join(", ")));
    }
    if !profile.certifications.is_empty() {
        parts.push(format!(
            "Certifications: {}.",
            profile.certifications.join(", ")
        ));
    }
    if !profile.languages.is_empty() {
        let languages: Vec<String> = profile
            .languages
            .iter()
            .map(|language| format!("{} {}", language.language, language.level.as_str()))
            .collect();
        parts.push(format!("Languages: {}.", languages.join(", ")));
    }
    if !profile.locations.is_empty() {
        parts.push(format!("Based in {}.", profile.locations.join(", ")));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// The Offer's side: its title and its plain-text description
/// (`offer-scoring.md` §3: "title + description"). `None` when it has neither.
pub fn offer_text(offer: &Offer) -> Option<String> {
    let title = offer.title.trim();
    let description = offer.description_text.trim();
    if title.is_empty() && description.is_empty() {
        return None;
    }
    Some(match (title.is_empty(), description.is_empty()) {
        (false, false) => format!("{title}. {description}"),
        (false, true) => title.to_string(),
        (true, false) => description.to_string(),
        (true, true) => unreachable!("both empty is handled above"),
    })
}

/// What a completed model download occupies: which model, and the directory
/// the encoder reads it from — the destination the network-activity view lists
/// (Story 6.9 AC1). The adapter owns the files; the core only records this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInstall {
    pub model_id: String,
    pub destination: PathBuf,
    pub dimension: usize,
}

/// A cosine from the adapter as the 0–1 similarity [`SemanticScore`] accepts:
/// below zero is no similarity (never a negative score), and a NaN — a broken
/// adapter — is refused rather than rounded into a number (AC2).
pub fn score_from_cosine(cosine: f32) -> Option<SemanticScore> {
    SemanticScore::new(cosine.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::posting::{ApplyHints, WorkMode};
    use crate::posting_features::Seniority;

    const NOW: &str = "2026-10-17T00:00:00Z";

    fn offer() -> Offer {
        Offer {
            id: "pst_01E5".into(),
            source_id: "remotive".into(),
            external_id: "abc-1".into(),
            title: "Senior Rust Engineer".into(),
            company: "Boba Labs".into(),
            locations: vec!["Remote".into()],
            work_mode: WorkMode::Remote,
            money: None,
            posted_at: None,
            deadline: None,
            first_seen: NOW.into(),
            last_seen: NOW.into(),
            lang: "en".into(),
            description_html: "<p>Build APIs.</p>".into(),
            description_text: "Build Rust services and PostgreSQL schemas.".into(),
            requirements: Vec::new(),
            apply: ApplyHints {
                url: "https://jobs.example.com/abc-1".into(),
                email: None,
            },
        }
    }

    fn profile() -> ProfileSnapshot {
        ProfileSnapshot {
            skills: vec!["Rust".into(), "PostgreSQL".into()],
            seniority: Some(Seniority::Senior),
            years_experience: Some(8),
            education: vec!["BSc Computer Science".into()],
            certifications: Vec::new(),
            languages: Vec::new(),
            locations: vec!["Bogotá".into()],
        }
    }

    fn prefs() -> Prefs {
        Prefs {
            roles: vec!["Backend Engineer".into()],
            ..Prefs::default()
        }
    }

    #[test]
    fn the_offer_side_reads_the_title_and_the_description() {
        let text = offer_text(&offer()).expect("a title and a description");
        assert!(text.contains("Senior Rust Engineer"));
        assert!(text.contains("Build Rust services and PostgreSQL schemas."));
    }

    #[test]
    fn an_offer_with_no_text_has_no_side() {
        let mut empty = offer();
        empty.title = "   ".into();
        empty.description_text = String::new();
        assert_eq!(offer_text(&empty), None);
    }

    #[test]
    fn the_profile_side_reads_the_roles_the_skills_and_the_seniority() {
        let text = profile_text(&profile(), &prefs()).expect("a filled Profile");
        assert!(text.contains("Backend Engineer"));
        assert!(text.contains("Rust"));
        assert!(text.contains("PostgreSQL"));
        assert!(text.contains("senior"));
        assert!(text.contains("Bogotá"));
    }

    #[test]
    fn an_empty_profile_has_no_side() {
        assert_eq!(
            profile_text(&ProfileSnapshot::empty(), &Prefs::default()),
            None
        );
    }

    #[test]
    fn e5_prefixes_each_side_once() {
        assert_eq!(
            encode_input(Side::Query, " backend engineer "),
            "query: backend engineer"
        );
        assert_eq!(encode_input(Side::Passage, "rust"), "passage: rust");
        // Already prefixed by the caller: never twice.
        assert_eq!(encode_input(Side::Query, "query: rust"), "query: rust");
    }

    #[test]
    fn a_cosine_becomes_the_zero_to_one_score_the_factor_reads() {
        assert_eq!(score_from_cosine(1.0).map(SemanticScore::get), Some(1.0));
        assert_eq!(score_from_cosine(0.82).map(SemanticScore::get), Some(0.82));
        assert_eq!(score_from_cosine(0.0).map(SemanticScore::get), Some(0.0));
        // Below zero is no similarity at all, never a negative score (AC2).
        assert_eq!(score_from_cosine(-0.4).map(SemanticScore::get), Some(0.0));
        // A NaN from a broken adapter is refused, not rounded into a number.
        assert_eq!(score_from_cosine(f32::NAN), None);
    }
}
