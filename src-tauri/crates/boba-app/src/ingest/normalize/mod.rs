//! Normalisation of a `RawPosting` into an Offer (FR-48, AD-17, AD-19).
//!
//! One place turns every source's payload into the same shape, so ranking and
//! tailoring never see a source-specific record. This is the only place
//! third-party HTML is touched, and it leaves sanitised (AD-7): the Offer holds
//! the ammonia output for the UI to render plus its plain text for the scorer
//! and the simhash. Nothing is invented — a missing field stays empty.
//!
//! `lingua` and `ammonia` live here, in the app layer, so `boba-domain` stays
//! pure (AD-1): the Offer itself is defined in `boba_domain::posting`.

use std::sync::OnceLock;

use boba_domain::posting::{ApplyHints, Money, Offer, Provenance, WorkMode, new_posting_id};
use boba_domain::source::RawPosting;
use html2text::render::TrivialDecorator;
use lingua::{Language, LanguageDetector, LanguageDetectorBuilder};

use crate::ports::repo::ingestion::{IngestionRepo, IngestionRepoError};

#[cfg(test)]
pub mod testing;

/// Wrapping width for the plain-text copy; wide enough to keep lines whole.
const TEXT_WIDTH: usize = 100;

/// Normalise one `RawPosting` into an Offer plus the provenance row it came
/// from (FR-48).
pub fn normalize(raw: &RawPosting, source_id: &str, now: &str) -> (Offer, Provenance) {
    let id = new_posting_id();
    let description_html = raw
        .description_html
        .as_deref()
        .map(ammonia::clean)
        .unwrap_or_default();
    let description_text = to_text(&description_html);

    let money = raw
        .salary_text
        .as_deref()
        .and_then(Money::parse)
        .or_else(|| Money::parse(&description_text));

    let offer = Offer {
        id: id.clone(),
        source_id: source_id.to_string(),
        external_id: raw.external_id.clone(),
        title: raw.title.trim().to_string(),
        company: raw.company.trim().to_string(),
        locations: clean_locations(&raw.locations),
        work_mode: WorkMode::from_remote(raw.remote),
        money,
        posted_at: raw.posted_at.clone(),
        // Only a source that sends one gives a deadline; guessing it from the
        // description would fake the "possibly closed" signal of S4.4.
        deadline: None,
        first_seen: now.to_string(),
        last_seen: now.to_string(),
        lang: detect_lang(&format!("{} {}", raw.title, description_text)),
        description_html,
        description_text: description_text.clone(),
        // Deterministic extraction is S6.2's job; normalisation stores the field
        // so the shape is stable from the first run.
        requirements: Vec::new(),
        apply: ApplyHints {
            url: raw.url.trim().to_string(),
            email: apply_email(&description_text),
        },
    };

    let provenance = Provenance {
        posting_id: id,
        source_id: source_id.to_string(),
        external_id: raw.external_id.clone(),
        url: raw.url.trim().to_string(),
        raw: raw.raw.clone(),
        fetched_at: now.to_string(),
    };
    (offer, provenance)
}

/// Normalise and store an Offer in one step (FR-48).
pub fn ingest(
    repo: &dyn IngestionRepo,
    raw: &RawPosting,
    source_id: &str,
    now: &str,
) -> Result<Offer, IngestionRepoError> {
    let (offer, provenance) = normalize(raw, source_id, now);
    repo.insert_offer(&offer, &provenance)?;
    Ok(offer)
}

fn clean_locations(raw: &[String]) -> Vec<String> {
    let mut locations: Vec<String> = Vec::new();
    for location in raw {
        let location = location.trim();
        if !location.is_empty() && !locations.iter().any(|kept| kept == location) {
            locations.push(location.to_string());
        }
    }
    locations
}

/// Plain text of sanitised HTML, one line, for dedup and scoring. The trivial
/// decorator keeps it plain: no `**emphasis**` or `[link]` markers leak into
/// what the scorer reads.
fn to_text(html: &str) -> String {
    html2text::from_read_with_decorator(html.as_bytes(), TEXT_WIDTH, TrivialDecorator::new())
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The content language (AD-17). Only `en`/`es` are modelled (tech-stack
/// choice, keeps the models small); everything else reads as the default
/// content language, `en`.
fn detect_lang(text: &str) -> String {
    static DETECTOR: OnceLock<LanguageDetector> = OnceLock::new();
    let detector = DETECTOR.get_or_init(|| {
        LanguageDetectorBuilder::from_languages(&[Language::English, Language::Spanish]).build()
    });
    match detector.detect_language_of(text) {
        Some(Language::Spanish) => "es".to_string(),
        _ => "en".to_string(),
    }
}

/// The first apply address in the posting text, if it has one (FR-48 hints).
fn apply_email(text: &str) -> Option<String> {
    text.split_whitespace()
        .map(|token| {
            token
                .trim_matches(|c: char| {
                    !(c.is_ascii_alphanumeric() || matches!(c, '@' | '.' | '-' | '_' | '+'))
                })
                .trim_end_matches('.')
        })
        .find(|token| is_email(token))
        .map(str::to_string)
}

fn is_email(token: &str) -> bool {
    let Some((local, domain)) = token.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && token.matches('@').count() == 1
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
        && local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::repo::ingestion::IngestionRepo;
    use boba_domain::posting::{ApplyHints, Money, PayPeriod, WorkMode};
    use testing::FakeIngestionRepo;

    const NOW: &str = "2026-10-16T00:00:00Z";

    fn raw() -> RawPosting {
        RawPosting {
            external_id: "abc-1".into(),
            title: "  Backend Engineer  ".into(),
            company: "Boba Labs".into(),
            url: "https://jobs.example.com/abc-1".into(),
            locations: vec![
                " Remote ".into(),
                "Bogotá, CO".into(),
                "".into(),
                "Remote".into(),
            ],
            remote: Some(true),
            description_html: Some(
                "<p>We build <b>APIs</b>.</p><script>alert(1)</script>\
                 <p>Apply: hiring@boba.dev</p>"
                    .into(),
            ),
            posted_at: Some("2026-10-01T00:00:00Z".into()),
            salary_text: Some("USD 50,000 - 70,000 per year".into()),
            raw: r#"{"id":"abc-1"}"#.into(),
        }
    }

    #[test]
    fn normalise_maps_every_field_and_writes_one_provenance_row() {
        let (offer, provenance) = normalize(&raw(), "remotive", NOW);

        assert!(offer.id.starts_with("pst_"));
        assert_eq!(offer.source_id, "remotive");
        assert_eq!(offer.external_id, "abc-1");
        assert_eq!(offer.title, "Backend Engineer", "trimmed");
        assert_eq!(offer.company, "Boba Labs");
        assert_eq!(
            offer.locations,
            ["Remote", "Bogotá, CO"],
            "trimmed, deduped"
        );
        assert_eq!(offer.work_mode, WorkMode::Remote);
        assert_eq!(
            offer.money,
            Some(Money {
                amount_minor: 5_000_000,
                currency: "USD".into(),
                period: PayPeriod::Year,
            })
        );
        assert_eq!(offer.posted_at.as_deref(), Some("2026-10-01T00:00:00Z"));
        assert_eq!(offer.deadline, None, "no source gives a deadline yet");
        assert_eq!(offer.first_seen, NOW);
        assert_eq!(offer.last_seen, NOW);
        assert_eq!(offer.lang, "en");
        assert!(offer.requirements.is_empty(), "extracted later (S6.2)");
        assert_eq!(
            offer.apply,
            ApplyHints {
                url: "https://jobs.example.com/abc-1".into(),
                email: Some("hiring@boba.dev".into()),
            }
        );

        assert_eq!(provenance.posting_id, offer.id);
        assert_eq!(provenance.source_id, "remotive");
        assert_eq!(provenance.external_id, "abc-1");
        assert_eq!(provenance.raw, r#"{"id":"abc-1"}"#, "raw payload is kept");
        assert_eq!(provenance.fetched_at, NOW);
    }

    #[test]
    fn description_html_is_sanitised_and_also_kept_as_text() {
        let (offer, _) = normalize(&raw(), "remotive", NOW);

        assert!(
            !offer.description_html.contains("<script"),
            "ammonia strips scripts (AD-7): {}",
            offer.description_html
        );
        assert!(
            offer.description_html.contains("<b>APIs</b>"),
            "safe HTML stays"
        );
        assert!(offer.description_text.contains("We build APIs."));
        assert!(
            !offer.description_text.contains('<'),
            "plain text for scoring: {}",
            offer.description_text
        );
    }

    #[test]
    fn language_is_detected_from_the_title_and_description() {
        let mut spanish = raw();
        spanish.title = "Ingeniero de software".into();
        spanish.description_html =
            Some("<p>Buscamos un ingeniero para desarrollar la plataforma del equipo.</p>".into());

        assert_eq!(normalize(&spanish, "remotive", NOW).0.lang, "es");
        assert_eq!(normalize(&raw(), "remotive", NOW).0.lang, "en");
    }

    #[test]
    fn missing_fields_stay_empty_instead_of_being_invented() {
        let bare = RawPosting {
            external_id: "x".into(),
            title: "Data Analyst".into(),
            company: "Acme".into(),
            url: "https://acme.example/x".into(),
            locations: vec![],
            remote: None,
            description_html: None,
            posted_at: None,
            salary_text: None,
            raw: "{}".into(),
        };
        let (offer, _) = normalize(&bare, "remotive", NOW);

        assert_eq!(offer.work_mode, WorkMode::Unknown);
        assert_eq!(offer.money, None);
        assert_eq!(offer.locations, Vec::<String>::new());
        assert_eq!(offer.description_html, "");
        assert_eq!(offer.description_text, "");
        assert_eq!(offer.posted_at, None);
        assert_eq!(offer.apply.email, None);
        assert!(offer.apply.url.ends_with("/x"));
    }

    #[test]
    fn money_falls_back_to_the_description_text() {
        let mut with_range = raw();
        with_range.salary_text = None;
        with_range.description_html =
            Some("<p>Compensation: €45.000 al mes, fully remote.</p>".into());

        let (offer, _) = normalize(&with_range, "remotive", NOW);
        let money = offer.money.expect("description carries the salary");
        assert_eq!(money.amount_minor, 4_500_000);
        assert_eq!(money.currency, "EUR");
    }

    #[test]
    fn ingest_stores_the_offer_and_its_provenance() {
        let repo = FakeIngestionRepo::default();
        let offer = ingest(&repo, &raw(), "remotive", NOW).expect("stored");

        assert_eq!(repo.get_offer(&offer.id).unwrap().as_ref(), Some(&offer));
        let rows = repo.provenance(&offer.id).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].raw, r#"{"id":"abc-1"}"#);
    }
}
