//! Manual Offer intake (FR-25): a pasted URL or pasted text becomes one Offer.
//!
//! The user finds a job where automation is not allowed (LinkedIn, Indeed, a
//! company page) and pastes it. Both inputs end in the same place: one
//! `RawPosting` normalised by [`crate::ingest::normalize`] and stored under the
//! `manual` source, whose name the UI shows as **Manual** (AC2).
//!
//! Upstream of this module the command layer fetches a pasted URL through the
//! shared `HttpClient` — robots rules, the per-host bucket and the honest
//! User-Agent all apply (AC1, AD-10) — and hands the page here. A fetch that
//! fails never reaches this module: the screen asks for the text instead (AC1).
//!
//! What the page or the text contains is **untrusted** (AD-7): it is read as
//! data, never as instructions, and only ever reaches the UI through
//! `normalize`'s sanitised HTML. The current extraction is deterministic
//! (JSON-LD `JobPosting`, then OpenGraph/`<title>`, then text heuristics); the
//! agent-backed refinement of the same fields lands with `extract_postings` in
//! S6.2, which will hand the agent the text inside the untrusted envelope.

use std::sync::Arc;

use boba_domain::posting::Offer;
use boba_domain::source::{AccessClass, ApplyCapability, LegalTier, RawPosting, SourceRecord};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ingest::normalize;
use crate::ports::clock::ClockPort;
use crate::ports::repo::ingestion::IngestionRepo;
use crate::ports::repo::sources::SourcesRepo;

/// The id every manual Offer carries (`posting.source_id`).
pub const SOURCE_ID: &str = "manual";

/// The name the UI shows for that source (AC2: Source "Manual").
pub const SOURCE_NAME: &str = "Manual";

/// The largest paste stored. A paste is a posting, not a file, and it is kept
/// three times (payload, sanitised HTML, plain text).
pub const MAX_TEXT_BYTES: usize = 1024 * 1024;

/// A manual-intake failure (AD-3: the UI translates `message_key`).
#[derive(Debug, Clone, thiserror::Error)]
pub enum ManualError {
    /// Nothing to read: the pasted text was blank.
    #[error("manual intake got empty input")]
    EmptyInput,
    /// The paste is larger than a posting ever is.
    #[error("manual intake got {0} bytes, over the {MAX_TEXT_BYTES} cap")]
    TooLarge(usize),
    /// The Offer could not be stored.
    #[error(transparent)]
    Repo(#[from] crate::ports::repo::ingestion::IngestionRepoError),
    /// The `manual` source row could not be registered.
    #[error(transparent)]
    Source(#[from] crate::ports::repo::sources::SourcesRepoError),
    /// The process-wide HTTP client is not mounted (no store opened this launch).
    #[error("no HTTP client is mounted")]
    FetchUnavailable,
    /// The page could not be fetched. The reason is diagnostics; what the user
    /// is told is "paste the text instead" (AC1).
    #[error("manual fetch failed: {0}")]
    FetchFailed(String),
}

impl ManualError {
    /// The i18n key the UI shows.
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::EmptyInput => "errors.manual.empty_input",
            Self::TooLarge(_) => "errors.manual.too_large",
            Self::Repo(_) | Self::Source(_) => "errors.manual.store",
            Self::FetchUnavailable => "errors.manual.fetch_unavailable",
            Self::FetchFailed(_) => "errors.manual.fetch_failed",
        }
    }
}

/// The Offer as the manual-add screen shows it right after intake (AC2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct ManualOffer {
    pub id: String,
    pub title: String,
    /// Empty when neither the page nor the text named one (never invented).
    pub company: String,
    pub source_id: String,
    pub source_name: String,
    pub locations: Vec<String>,
    /// `remote` | `hybrid` | `onsite` | `unknown`.
    pub work_mode: String,
    /// Pay in minor units, with its ISO-4217 code and period, so the UI formats
    /// it (S4.2 stores the same three columns, AD-17). A JS number, not an
    /// `i64`: specta refuses BigInt-style types, and a pay amount is nowhere
    /// near 2^53 minor units.
    pub amount_minor: Option<f64>,
    pub currency: Option<String>,
    pub pay_period: Option<String>,
    pub apply_url: String,
    /// BCP-47 content language of the posting.
    pub lang: String,
}

/// Manual intake: the `manual` source, the store and the clock (AC1/AC2).
pub struct ManualIntake {
    repo: Arc<dyn IngestionRepo>,
    sources: Arc<dyn SourcesRepo>,
    clock: Arc<dyn ClockPort>,
}

impl ManualIntake {
    pub fn new(
        repo: Arc<dyn IngestionRepo>,
        sources: Arc<dyn SourcesRepo>,
        clock: Arc<dyn ClockPort>,
    ) -> Self {
        Self {
            repo,
            sources,
            clock,
        }
    }

    /// Store pasted text as an Offer (AC2). Blank input is refused: an Offer
    /// the user never gave us anything for is not worth storing. A paste over
    /// the cap is refused too, rather than stored three times.
    pub fn from_text(&self, text: &str) -> Result<ManualOffer, ManualError> {
        if text.trim().is_empty() {
            return Err(ManualError::EmptyInput);
        }
        if text.len() > MAX_TEXT_BYTES {
            return Err(ManualError::TooLarge(text.len()));
        }
        self.keep(&posting_from_text(text))
    }

    /// Store a fetched page as an Offer (AC1). The URL the user pasted is the
    /// posting's identity and its apply URL (nothing is fetched or guessed here:
    /// the page already arrived).
    pub fn from_page(&self, html: &str, url: &str) -> Result<ManualOffer, ManualError> {
        if html.trim().is_empty() {
            return Err(ManualError::EmptyInput);
        }
        self.keep(&posting_from_page(html, url))
    }

    /// Register the Manual source once, normalise and store the Offer.
    fn keep(&self, raw: &RawPosting) -> Result<ManualOffer, ManualError> {
        let now = self.clock.now_rfc3339();
        if self.sources.get_source(SOURCE_ID)?.is_none() {
            self.sources.upsert_source(&manual_source(&now))?;
        }
        let offer = normalize::ingest(self.repo.as_ref(), raw, SOURCE_ID, &now)?;
        Ok(view(&offer))
    }
}

/// The `manual` source row. It has no board and never runs, so it is ungated;
/// the `manual` access class of the source table has no domain variant yet
/// (S4.1 owns that enum), and `public_json` is the closest thing that says
/// "plain fetch, no opt-in".
fn manual_source(now: &str) -> SourceRecord {
    SourceRecord {
        id: SOURCE_ID.to_string(),
        name: SOURCE_NAME.to_string(),
        access_class: AccessClass::PublicJson,
        legal_tier: LegalTier::Safe,
        apply_capability: ApplyCapability::LinkOut,
        attribution: None,
        min_interval_ms: 2000,
        daily_cap: None,
        enabled: true,
        added_at: now.to_string(),
        updated_at: now.to_string(),
    }
}

fn view(offer: &Offer) -> ManualOffer {
    ManualOffer {
        id: offer.id.clone(),
        title: offer.title.clone(),
        company: offer.company.clone(),
        source_id: offer.source_id.clone(),
        source_name: SOURCE_NAME.to_string(),
        locations: offer.locations.clone(),
        work_mode: offer.work_mode.as_str().to_string(),
        amount_minor: offer.money.as_ref().map(|m| m.amount_minor as f64),
        currency: offer.money.as_ref().map(|m| m.currency.clone()),
        pay_period: offer.money.as_ref().map(|m| m.period.as_str().to_string()),
        apply_url: offer.apply.url.clone(),
        lang: offer.lang.clone(),
    }
}

/// A pasted posting, read as a `RawPosting` (AC2).
pub fn posting_from_text(text: &str) -> RawPosting {
    let text = text.trim();
    RawPosting {
        // A stable id for the same paste, so re-pasting ties to one Offer (S4.3
        // dedups on it); the content itself is untrusted data.
        external_id: format!("pasted-{:016x}", fnv1a(text)),
        title: first_line(text),
        company: String::new(),
        url: String::new(),
        locations: Vec::new(),
        remote: None,
        description_html: Some(paragraphs(text)),
        posted_at: None,
        salary_text: None,
        raw: text.to_string(),
    }
}

/// A fetched page, read as a `RawPosting` (AC1).
pub fn posting_from_page(html: &str, url: &str) -> RawPosting {
    let json_ld = job_posting(html);
    let og = |property: &str| meta_content(html, property);
    let title = json_ld
        .as_ref()
        .and_then(|posting| text_field(posting, "title"))
        .or_else(|| og("og:title"))
        .or_else(|| element_text(html, "title"))
        .unwrap_or_default();
    let description = json_ld
        .as_ref()
        .and_then(|posting| text_field(posting, "description"))
        .unwrap_or_else(|| html.to_string());
    RawPosting {
        external_id: url.trim().to_string(),
        title,
        company: json_ld
            .as_ref()
            .and_then(company_of)
            .or_else(|| og("og:site_name"))
            .unwrap_or_default(),
        url: url.trim().to_string(),
        locations: json_ld.as_ref().map(locations_of).unwrap_or_default(),
        // Cached pages carry `JobPosting.jobLocationType`; anything else stays
        // unknown rather than being guessed from the text.
        remote: json_ld.as_ref().and_then(|posting| {
            text_field(posting, "jobLocationType")
                .map(|kind| kind.eq_ignore_ascii_case("TELECOMMUTE"))
        }),
        description_html: Some(description),
        // Only a date the page really carries (AD-17 drops what it cannot read).
        posted_at: json_ld
            .as_ref()
            .and_then(|posting| text_field(posting, "datePosted")),
        salary_text: json_ld.as_ref().and_then(salary_text),
        raw: html.to_string(),
    }
}

/// The first line of a paste, capped: a posting pasted as text starts with its
/// title far more often than not, and a runaway line must not become one.
fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .chars()
        .take(200)
        .collect()
}

/// Pasted text as HTML: escaped (it is untrusted, AD-7) and split into
/// paragraphs so the sanitised copy the UI renders keeps its shape.
fn paragraphs(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| format!("<p>{}</p>", escape(line)))
        .collect::<Vec<_>>()
        .join("")
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// FNV-1a 64-bit: small and dependency-free, and only needs to be stable for
/// the same paste (the real dedup key is S4.3's job).
fn fnv1a(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The page's `JobPosting` JSON-LD block, when it has one: publishers put the
/// same fields there that a board API returns, which beats reading og tags.
fn job_posting(html: &str) -> Option<serde_json::Value> {
    json_ld_blocks(html)
        .into_iter()
        .filter_map(|block| serde_json::from_str::<serde_json::Value>(block).ok())
        .find_map(|value| find_job_posting(&value).cloned())
}

/// Depth-first search of a JSON-LD value for the first `@type: JobPosting`
/// (the node may sit in an array or an `@graph`).
fn find_job_posting(value: &serde_json::Value) -> Option<&serde_json::Value> {
    match value {
        serde_json::Value::Array(items) => items.iter().find_map(find_job_posting),
        serde_json::Value::Object(map) => {
            if has_type(map.get("@type"), "JobPosting") {
                return Some(value);
            }
            map.get("@graph").and_then(find_job_posting)
        }
        _ => None,
    }
}

fn has_type(value: Option<&serde_json::Value>, wanted: &str) -> bool {
    match value {
        Some(serde_json::Value::String(text)) => text.eq_ignore_ascii_case(wanted),
        Some(serde_json::Value::Array(items)) => {
            items.iter().any(|item| has_type(Some(item), wanted))
        }
        _ => false,
    }
}

/// `type="application/ld+json"` script bodies, in document order.
fn json_ld_blocks(html: &str) -> Vec<&str> {
    let mut blocks = Vec::new();
    let mut rest = html;
    while let Some(at) = find_ci(rest, "<script") {
        let after = &rest[at + "<script".len()..];
        let Some(tag_end) = after.find('>') else {
            break;
        };
        let (tag, body) = after.split_at(tag_end + 1);
        rest = body;
        if !find_ci(tag, "application/ld+json").is_some() {
            continue;
        }
        let Some(close) = find_ci(rest, "</script") else {
            break;
        };
        blocks.push(&rest[..close]);
        rest = &rest[close..];
    }
    blocks
}

fn find_ci(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .to_ascii_lowercase()
        .find(&needle.to_ascii_lowercase())
}

/// A `<meta property="…" content="…">` (or `name=`) value.
fn meta_content(html: &str, property: &str) -> Option<String> {
    let mut rest = html;
    while let Some(at) = find_ci(rest, "<meta") {
        let after = &rest[at + "<meta".len()..];
        let tag_end = after.find('>')?;
        let (tag, tail) = after.split_at(tag_end + 1);
        rest = tail;
        let attributes = attributes(tag);
        let key = attributes
            .iter()
            .find(|(name, _)| name == "property" || name == "name")
            .map(|(_, value)| value.as_str());
        if key.is_some_and(|key| key.eq_ignore_ascii_case(property)) {
            let content = attributes
                .iter()
                .find(|(name, _)| name == "content")
                .map(|(_, value)| value.clone())?;
            let content = plain(&content);
            if !content.is_empty() {
                return Some(content);
            }
        }
    }
    None
}

/// A tag's text content, e.g. `<title>…</title>`.
fn element_text(html: &str, name: &str) -> Option<String> {
    let open = find_ci(html, &format!("<{name}"))?;
    let after = &html[open..];
    let content = after.split_once('>')?.1;
    let close = find_ci(content, &format!("</{name}"))?;
    let text = plain(&content[..close]);
    (!text.is_empty()).then_some(text)
}

/// `name="value"` pairs of one start tag. Unquoted values are read as-is.
fn attributes(tag: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let bytes: Vec<char> = tag.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_alphabetic() {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len()
            && (bytes[i].is_ascii_alphanumeric() || bytes[i] == '-' || bytes[i] == ':')
        {
            i += 1;
        }
        let name: String = bytes[start..i].iter().collect();
        while i < bytes.len() && bytes[i].is_whitespace() {
            i += 1;
        }
        if bytes.get(i) != Some(&'=') {
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_whitespace() {
            i += 1;
        }
        let quote = bytes.get(i).copied().filter(|c| *c == '"' || *c == '\'');
        if quote.is_some() {
            i += 1;
        }
        let value_start = i;
        while i < bytes.len() {
            if Some(bytes[i]) == quote
                || (quote.is_none() && (bytes[i].is_whitespace() || matches!(bytes[i], '>' | '/')))
            {
                break;
            }
            i += 1;
        }
        pairs.push((
            name.to_ascii_lowercase(),
            bytes[value_start..i].iter().collect(),
        ));
        if quote.is_some() {
            i += 1;
        }
    }
    pairs
}

/// A string field of a JSON-LD node.
fn text_field(value: &serde_json::Value, key: &str) -> Option<String> {
    let text = value.get(key)?.as_str()?.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// The hiring organisation's name (`hiringOrganization` is an object, and some
/// publishers send a plain string).
fn company_of(value: &serde_json::Value) -> Option<String> {
    let organisation = value.get("hiringOrganization")?;
    if let Some(name) = organisation.as_str() {
        let name = name.trim();
        return (!name.is_empty()).then(|| name.to_string());
    }
    text_field(organisation, "name")
}

/// `JobPosting.jobLocation`, as `Place` or `PostalAddress`, one or many.
fn locations_of(value: &serde_json::Value) -> Vec<String> {
    let mut locations: Vec<String> = Vec::new();
    let places = match value.get("jobLocation") {
        Some(serde_json::Value::Array(items)) => items.as_slice(),
        Some(single) => std::slice::from_ref(single),
        None => &[],
    };
    for place in places {
        let address = place.get("address").unwrap_or(place);
        let parts: Vec<String> = ["addressLocality", "addressRegion", "addressCountry"]
            .into_iter()
            .filter_map(|key| text_field(address, key))
            .collect();
        let joined = parts.join(", ");
        if !joined.is_empty() && !locations.contains(&joined) {
            locations.push(joined);
        }
    }
    locations
}

/// A string-or-number field of a JSON-LD node: `minValue` and friends are
/// JSON numbers, `title` and friends are strings.
fn scalar_field(value: &serde_json::Value, key: &str) -> Option<String> {
    match value.get(key)? {
        serde_json::Value::String(text) => {
            let text = text.trim();
            (!text.is_empty()).then(|| text.to_string())
        }
        serde_json::Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// `JobPosting.baseSalary`, flattened into the free text `Money::parse` reads.
fn salary_text(value: &serde_json::Value) -> Option<String> {
    let salary = value.get("baseSalary")?;
    let currency = text_field(salary, "currency")?;
    let amount = salary.get("value").unwrap_or(salary);
    let number = scalar_field(amount, "minValue")
        .or_else(|| scalar_field(amount, "value"))
        .or_else(|| scalar_field(amount, "maxValue"))?;
    // `unitText` is MONTH/YEAR/HOUR; `Money::parse` reads the period from prose,
    // so it travels as prose ("per month") rather than being dropped.
    let period = match text_field(amount, "unitText") {
        Some(unit) => format!(" per {}", unit.to_lowercase()),
        None => String::new(),
    };
    Some(format!("{currency} {number}{period}"))
}

/// HTML a fragment carries, as plain text: entities decoded, tags dropped (the
/// same reading `normalize` gives the description, AD-7).
fn plain(fragment: &str) -> String {
    html2text::from_read(fragment.as_bytes(), 1000)
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::normalize::testing::FakeIngestionRepo;
    use crate::system::FixedClock;
    use boba_domain::posting::WorkMode;

    use testing::FakeSourcesRepo;

    const NOW: &str = "2026-10-16T12:00:00Z";
    const PAGE_URL: &str = "https://boards.example.com/acme/1";

    fn intake() -> (ManualIntake, Arc<FakeIngestionRepo>, FakeSourcesRepo) {
        let repo = Arc::new(FakeIngestionRepo::default());
        let ingestion: Arc<dyn IngestionRepo> = repo.clone();
        let sources = FakeSourcesRepo::default();
        let intake = ManualIntake::new(
            ingestion,
            Arc::new(sources.clone()),
            Arc::new(FixedClock::new(NOW)),
        );
        (intake, repo, sources)
    }

    /// AC2: pasted text becomes a scored Offer under the Manual source.
    #[test]
    fn pasted_text_becomes_an_offer_from_the_manual_source() {
        let (intake, repo, _sources) = intake();
        let text = "Senior Rust Engineer\nAcme Corp builds boba.\n\
                    Salary: USD 120,000 per year\nApply: hiring@acme.example";

        let offer = intake.from_text(text).expect("stored");

        assert_eq!(offer.source_id, "manual");
        assert_eq!(offer.source_name, "Manual", "AC2: Source 'Manual'");
        assert_eq!(offer.title, "Senior Rust Engineer");
        assert_eq!(offer.company, "", "never invented (AD-15)");
        assert_eq!(offer.amount_minor, Some(12_000_000.0));
        assert_eq!(offer.currency.as_deref(), Some("USD"));
        assert_eq!(offer.pay_period.as_deref(), Some("year"));
        assert_eq!(offer.work_mode, "unknown");
        assert_eq!(offer.apply_url, "");

        // The row the UI will read back is the same Offer.
        let stored = repo.get_offer(&offer.id).unwrap().expect("stored row");
        assert_eq!(stored.source_id, "manual");
        assert_eq!(stored.apply.email.as_deref(), Some("hiring@acme.example"));
        assert!(stored.description_text.contains("Acme Corp builds boba."));
        assert_eq!(stored.first_seen, NOW);
        assert_eq!(stored.last_seen, NOW);
    }

    #[test]
    fn the_provenance_keeps_the_paste_verbatim() {
        let (intake, repo, _sources) = intake();
        let text = "Backend Engineer at Acme";
        let offer = intake.from_text(text).expect("stored");

        let rows = repo.provenance(&offer.id).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].raw, text);
        assert_eq!(rows[0].source_id, "manual");
        assert_eq!(rows[0].fetched_at, NOW);
    }

    #[test]
    fn a_paste_that_is_only_whitespace_is_refused() {
        let (intake, repo, sources) = intake();

        let error = intake.from_text("  \n\t ").expect_err("refused");
        assert_eq!(error.message_key(), "errors.manual.empty_input");
        assert_eq!(repo.provenance("").unwrap().len(), 0);
        assert!(
            sources.is_empty(),
            "nothing is registered for an empty paste"
        );
    }

    #[test]
    fn a_paste_over_the_cap_is_refused() {
        let (intake, repo, _sources) = intake();

        let error = intake
            .from_text(&"x".repeat(MAX_TEXT_BYTES + 1))
            .expect_err("refused");
        assert_eq!(error.message_key(), "errors.manual.too_large");
        assert_eq!(repo.provenance("").unwrap().len(), 0);
    }

    /// Untrusted text stays data (AD-7): the sanitised copy the UI renders may
    /// not carry markup, and the plain copy keeps it as characters.
    #[test]
    fn markup_in_a_paste_is_escaped_and_never_rendered() {
        let (intake, repo, _) = intake();
        let offer = intake
            .from_text("Job Title\n<script>alert(1)</script> & <b>bold</b>")
            .expect("stored");
        let stored = repo.get_offer(&offer.id).unwrap().unwrap();

        assert!(
            !stored.description_html.contains("<script"),
            "untrusted markup reached the renderable copy: {}",
            stored.description_html
        );
        assert!(!stored.description_html.contains("<b>bold</b>"));
        assert!(
            stored
                .description_text
                .contains("<script>alert(1)</script>")
        );
    }

    #[test]
    fn the_same_paste_keeps_one_identity() {
        let first = posting_from_text("Data Analyst\nAcme");
        let again = posting_from_text("Data Analyst\nAcme");
        let other = posting_from_text("Data Analyst\nGlobex");

        assert_eq!(first.external_id, again.external_id);
        assert_eq!(
            first.external_id,
            format!("pasted-{:016x}", fnv1a("Data Analyst\nAcme"))
        );
        assert_ne!(first.external_id, other.external_id);
    }

    /// One row for the Manual source, however many Offers are added.
    #[test]
    fn the_manual_source_is_registered_once() {
        let (intake, _, sources) = intake();

        intake.from_text("First posting").expect("stored");
        intake.from_text("Second posting").expect("stored");

        assert_eq!(sources.len(), 1);
        let row = sources.get("manual").expect("registered");
        assert_eq!(row.name, "Manual");
        assert!(row.enabled);
        assert_eq!(row.added_at, NOW);
        assert!(
            !row.access_class.requires_opt_in(),
            "the manual source is ungated (AC1: paste is always allowed)"
        );
    }

    /// AC1: a page carrying schema.org `JobPosting` JSON-LD fills the Offer.
    #[test]
    fn a_page_with_json_ld_is_read_from_it() {
        let (intake, repo, _) = intake();
        let page = r#"<html><head>
            <meta property="og:title" content="Ignore me">
            <script type="application/ld+json">
            {"@context":"https://schema.org","@type":"JobPosting",
             "title":"Staff Engineer",
             "description":"<p>Build <b>systems</b>.</p>",
             "datePosted":"2026-10-01T00:00:00+02:00",
             "jobLocationType":"TELECOMMUTE",
             "hiringOrganization":{"@type":"Organization","name":"Acme"},
             "jobLocation":{"@type":"Place","address":{"@type":"PostalAddress",
               "addressLocality":"Bogotá","addressCountry":"CO"}},
             "baseSalary":{"@type":"MonetaryAmount","currency":"COP",
               "value":{"@type":"QuantitativeValue","minValue":8000000,"unitText":"MONTH"}}}
            </script></head><body>Ignore me too</body></html>"#;

        let offer = intake.from_page(page, PAGE_URL).expect("stored");
        let stored = repo.get_offer(&offer.id).unwrap().unwrap();

        assert_eq!(offer.title, "Staff Engineer", "not the og tag");
        assert_eq!(offer.company, "Acme");
        assert_eq!(offer.locations, ["Bogotá, CO"]);
        assert_eq!(offer.work_mode, WorkMode::Remote.as_str());
        assert_eq!(offer.apply_url, PAGE_URL);
        assert_eq!(offer.lang, "en");
        assert_eq!(offer.amount_minor, Some(800_000_000.0));
        assert_eq!(offer.currency.as_deref(), Some("COP"));
        assert_eq!(offer.pay_period.as_deref(), Some("month"));
        assert!(stored.description_text.contains("Build systems."));
        assert!(
            stored.description_html.contains("<b>systems</b>"),
            "safe HTML from the publisher stays: {}",
            stored.description_html
        );
        assert_eq!(stored.posted_at.as_deref(), Some("2026-09-30T22:00:00Z"));
        assert_eq!(stored.external_id, PAGE_URL);
    }

    /// The JSON-LD node may arrive wrapped in an array or an `@graph`.
    #[test]
    fn json_ld_is_found_in_arrays_and_graphs() {
        let page = r#"<script type="application/ld+json">
            {"@graph":[{"@type":"WebSite"},{"@type":"JobPosting","title":"Graph Job"}]}
            </script>"#;
        assert_eq!(posting_from_page(page, PAGE_URL).title, "Graph Job");

        let list = r#"<script type="application/ld+json">
            [{"@type":"BreadcrumbList"},{"@type":["JobPosting"],"title":"List Job"}]
            </script>"#;
        assert_eq!(posting_from_page(list, PAGE_URL).title, "List Job");
    }

    #[test]
    fn a_page_without_json_ld_falls_back_to_open_graph_then_the_title_tag() {
        let og = r#"<html><head>
            <title>Acme Careers</title>
            <meta property="og:title" content="Backend Engineer &amp; friends">
            <meta property="og:site_name" content="Acme">
            </head><body><p>Job description body</p></body></html>"#;
        let posting = posting_from_page(og, PAGE_URL);
        assert_eq!(
            posting.title, "Backend Engineer & friends",
            "entities decoded"
        );
        assert_eq!(posting.company, "Acme");
        assert_eq!(
            posting.description_html.as_deref(),
            Some(og),
            "the whole page is the raw text"
        );

        let plain_title = "<html><head><title>Data Analyst</title></head><body>x</body></html>";
        assert_eq!(
            posting_from_page(plain_title, PAGE_URL).title,
            "Data Analyst"
        );
        assert_eq!(posting_from_page(plain_title, PAGE_URL).company, "");
    }

    #[test]
    fn a_page_that_names_nothing_still_produces_a_raw_posting() {
        let posting = posting_from_page("<html><body>unstructured</body></html>", PAGE_URL);

        assert_eq!(posting.title, "");
        assert_eq!(posting.company, "");
        assert_eq!(posting.remote, None);
        assert_eq!(posting.external_id, PAGE_URL);
    }

    /// A broken JSON-LD block is skipped, never fatal.
    #[test]
    fn a_malformed_json_ld_block_is_ignored() {
        let page = r#"<script type="application/ld+json">{not json</script>
            <title>Fallback Title</title>"#;
        assert_eq!(posting_from_page(page, PAGE_URL).title, "Fallback Title");
    }

    /// A pasted page whose body is markup never reaches the UI as markup (AD-7).
    #[test]
    fn a_fetched_page_is_sanitised_before_it_is_stored() {
        let (intake, repo, _) = intake();
        let page = r#"<html><head><title>Engineer</title></head>
            <body><p>Real text</p><script>steal()</script>
            <img src="x" onerror="steal()"></body></html>"#;

        let offer = intake.from_page(page, PAGE_URL).expect("stored");
        let stored = repo.get_offer(&offer.id).unwrap().unwrap();

        assert!(stored.description_html.contains("Real text"));
        assert!(!stored.description_html.contains("<script"));
        assert!(!stored.description_html.contains("onerror"));
        assert_eq!(
            repo.provenance(&offer.id).unwrap()[0].raw,
            page,
            "the payload is kept verbatim (AD-19)"
        );
    }

    #[test]
    fn an_empty_page_is_refused() {
        let (intake, _, sources) = intake();
        assert_eq!(
            intake.from_page("   ", PAGE_URL).unwrap_err().message_key(),
            "errors.manual.empty_input"
        );
        assert!(sources.is_empty());
    }

    #[test]
    fn attributes_are_read_with_either_quote_style() {
        let pairs = attributes(" property='og:title' content=Hello/>");
        assert_eq!(
            pairs,
            vec![
                ("property".to_string(), "og:title".to_string()),
                ("content".to_string(), "Hello".to_string()),
            ]
        );
    }
}

// Declared last on purpose: the shared i18n gate (AD-21) only reads a file up to
// its first `#[cfg(test)]`, and the `message_key`s above are core-emitted keys it
// must see.
#[cfg(test)]
pub mod testing;
