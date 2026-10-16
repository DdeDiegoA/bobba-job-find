//! The one document model every side of a port shares (AD-14, AD-15, AD-17).
//!
//! Tailoring produces a [`CvDocument`] or a [`CoverLetterDocument`]; both are
//! plain data with a `schema_version`. Every claim-bearing line carries the
//! `fact_ids` of the profile facts it was built from (AD-15), so a fabricated
//! claim can always be traced back — or rejected.
//!
//! Moved here from `boba-docs::model` in S7.2: [`TemplateId`], [`LayoutReport`]
//! and [`RenderedPdf`] cross `boba_app::ports::RenderPort`, and every type that
//! crosses a port lives in `boba-domain` (AD-14). There is no re-export shim.
//!
//! Renderers are pure functions of `(document, template, locale)`. Strings go
//! into Typst as JSON data and are printed as text, never evaluated as markup
//! (ADR-010).

use serde::{Deserialize, Serialize};

/// Current schema version. Bump on any breaking change to the JSON shape.
pub const SCHEMA_VERSION: u32 = 1;

/// A fresh ULID for a document row, prefixed by type (AD-17: `doc_…`).
pub fn new_document_id() -> String {
    format!("doc_{}", ulid::Ulid::generate())
}

/// Content language of a document (AD-17). Follows the posting's language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    En,
    Es,
}

impl Lang {
    /// The BCP-47 short code stored in the DB and passed to templates.
    pub const fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Es => "es",
        }
    }

    /// Read a BCP-47 tag (`en`, `es`, `es-MX`, `en-US`) down to the two content
    /// languages the MVP drafts in. `None` for anything else (FR-70).
    pub fn parse(tag: &str) -> Option<Self> {
        match tag
            .split(['-', '_'])
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "en" => Some(Lang::En),
            "es" => Some(Lang::Es),
            _ => None,
        }
    }

    /// The language both drafts of a run are written in, and why (FR-70): the
    /// offer's language when it is ES or EN, otherwise EN — with the offer's
    /// language kept so the studio can explain the fallback. The user's
    /// override always wins and needs no notice.
    pub fn for_offer(offer_lang: &str, override_lang: Option<Lang>) -> LanguageChoice {
        if let Some(lang) = override_lang {
            return LanguageChoice {
                lang,
                fallback_from: None,
            };
        }
        match Self::parse(offer_lang) {
            Some(lang) => LanguageChoice {
                lang,
                fallback_from: None,
            },
            None => LanguageChoice {
                lang: Lang::En,
                fallback_from: Some(offer_lang.to_string()),
            },
        }
    }
}

/// What [`Lang::for_offer`] decided (FR-70).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageChoice {
    /// Both drafts are written in this language.
    pub lang: Lang,
    /// The offer's language, set only when it is neither ES nor EN and EN was
    /// used instead. The studio shows a notice explaining the fallback.
    pub fallback_from: Option<String>,
}

/// Which kind of document a row or render holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentKind {
    Cv,
    CoverLetter,
}

impl DocumentKind {
    /// The value stored in `document.kind`.
    pub const fn as_str(self) -> &'static str {
        match self {
            DocumentKind::Cv => "cv",
            DocumentKind::CoverLetter => "cover_letter",
        }
    }

    /// Read the stored value back.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "cv" => Some(DocumentKind::Cv),
            "cover_letter" => Some(DocumentKind::CoverLetter),
            _ => None,
        }
    }
}

/// Stable identifier of a render template (AD-14). Stored in `document.template`
/// and in settings, so renaming one needs a migration. The registry of ids that
/// actually exist is the renderer's (`boba-docs`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TemplateId(String);

impl TemplateId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for TemplateId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A template the user can pick, as the UI lists it (FR-71). The label the user
/// reads is the studio's business; `id` is stable and stored (AD-14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateInfo {
    pub id: TemplateId,
    /// Which document kind this template renders.
    pub kind: DocumentKind,
    /// Whether this is the default for its kind.
    pub default: bool,
}

/// Layout facts of a render (AD-14). S7.5 verifies the page budget against them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LayoutReport {
    /// Pages the document laid out to.
    pub pages: u32,
    /// Headings stranded at the end of a page without their first item.
    pub orphans: u32,
}

/// A rendered PDF: the bytes plus what verification needs to know about them.
/// A failed render is never attached to a submission (AD-14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedPdf {
    pub bytes: Vec<u8>,
    pub layout: LayoutReport,
}

/// A named link in the contact block (portfolio, GitHub, LinkedIn…).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub label: String,
    pub url: String,
}

/// The contact block. Rendered literally in the text layer, never in a page
/// header, so ATS parsers read it as body text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contact {
    pub full_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headline: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
}

/// A claim-bearing line: `fact_ids` name the profile facts it is built from
/// (AD-15). An item with no facts is a fabrication and is rejected by the
/// validator in S7.3.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fact_ids: Vec<String>,
}

/// A CV section: a standard heading plus its claim lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    pub heading: String,
    #[serde(default)]
    pub items: Vec<Claim>,
}

/// A tailored CV (schema v1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CvDocument {
    pub schema_version: u32,
    pub lang: Lang,
    pub contact: Contact,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<Claim>,
    #[serde(default)]
    pub sections: Vec<Section>,
    /// Keywords the profile does not back. Never rendered; surfaced in the UI.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub declared_gaps: Vec<String>,
}

impl CvDocument {
    /// The DB kind discriminator for a CV.
    pub const fn kind(&self) -> DocumentKind {
        DocumentKind::Cv
    }

    /// The same draft in another content language (FR-70: the run's choice, not
    /// whatever the agent happened to answer in).
    pub fn with_lang(mut self, lang: Lang) -> Self {
        self.lang = lang;
        self
    }
}

/// A tailored cover letter (schema v1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverLetterDocument {
    pub schema_version: u32,
    pub lang: Lang,
    pub contact: Contact,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub company: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipient: Option<String>,
    /// Pre-formatted for the content locale (AD-17).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub salutation: Option<String>,
    #[serde(default)]
    pub paragraphs: Vec<Claim>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closing: Option<String>,
    /// Keywords the profile does not back. Never rendered; surfaced in the UI.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub declared_gaps: Vec<String>,
}

impl CoverLetterDocument {
    /// The DB kind discriminator for a cover letter.
    pub const fn kind(&self) -> DocumentKind {
        DocumentKind::CoverLetter
    }

    /// The same draft in another content language (FR-70).
    pub fn with_lang(mut self, lang: Lang) -> Self {
        self.lang = lang;
        self
    }
}

/// Either draft, as a render or a stored row holds it (AD-14).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DocumentModel {
    Cv(CvDocument),
    CoverLetter(CoverLetterDocument),
}

impl DocumentModel {
    /// The DB kind discriminator.
    pub const fn kind(&self) -> DocumentKind {
        match self {
            DocumentModel::Cv(_) => DocumentKind::Cv,
            DocumentModel::CoverLetter(_) => DocumentKind::CoverLetter,
        }
    }

    /// The content language this draft is written in (AD-17).
    pub const fn lang(&self) -> Lang {
        match self {
            DocumentModel::Cv(doc) => doc.lang,
            DocumentModel::CoverLetter(doc) => doc.lang,
        }
    }

    /// The schema version stored on the document.
    pub const fn schema_version(&self) -> u32 {
        match self {
            DocumentModel::Cv(doc) => doc.schema_version,
            DocumentModel::CoverLetter(doc) => doc.schema_version,
        }
    }
}

/// What a tailoring or review run wrote, as the store keeps it before the
/// document is saved (AD-14, AD-16).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDocument {
    pub id: String,
    pub application_id: String,
    pub kind: DocumentKind,
    pub lang: Lang,
    pub template: TemplateId,
    /// The serialized [`CvDocument`] / [`CoverLetterDocument`].
    pub model_json: String,
    /// Drafts come from an agent run (ADR-010), so their origin is untrusted
    /// until the verifier (S7.5/S7.6) has passed them.
    pub untrusted_origin: bool,
    pub created_at: String,
}

/// A document row without its (potentially large) blobs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentRecord {
    pub id: String,
    pub application_id: String,
    pub kind: DocumentKind,
    pub lang: Lang,
    pub template: TemplateId,
    pub version: i64,
    pub model_json: String,
    pub has_pdf: bool,
    pub has_docx: bool,
    pub verification_json: Option<String>,
    pub verified_at: Option<String>,
    pub untrusted_origin: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    // The JSON shape of these documents is read by the Typst templates and by
    // `boba-store`; its tests live in `boba-docs` (this crate has no serde_json,
    // AD-14 keeps it to serde/thiserror/time/ulid).

    #[test]
    fn document_model_carries_kind_lang_and_schema_version() {
        let cv = DocumentModel::Cv(CvDocument {
            schema_version: SCHEMA_VERSION,
            lang: Lang::Es,
            contact: Contact {
                full_name: "Ada".into(),
                headline: None,
                email: None,
                phone: None,
                location: None,
                links: vec![],
            },
            summary: None,
            sections: vec![],
            declared_gaps: vec![],
        });
        assert_eq!(cv.kind(), DocumentKind::Cv);
        assert_eq!(cv.lang(), Lang::Es);
        assert_eq!(cv.schema_version(), SCHEMA_VERSION);
    }

    #[test]
    fn kind_and_lang_parse_the_stored_values() {
        for (stored, kind) in [
            ("cv", DocumentKind::Cv),
            ("cover_letter", DocumentKind::CoverLetter),
        ] {
            assert_eq!(DocumentKind::parse(stored), Some(kind));
            assert_eq!(kind.as_str(), stored);
        }
        assert_eq!(DocumentKind::parse("resume"), None);
        for (tag, lang) in [
            ("en", Lang::En),
            ("en-US", Lang::En),
            ("es", Lang::Es),
            ("es_MX", Lang::Es),
            ("ES", Lang::Es),
        ] {
            assert_eq!(Lang::parse(tag), Some(lang), "{tag}");
        }
        assert_eq!(Lang::parse("fr"), None);
        assert_eq!(Lang::parse(""), None);
    }

    // FR-70: the draft follows the Offer's language; only ES and EN are drafted,
    // anything else falls back to EN with a reason the UI can show.
    #[test]
    fn language_follows_the_offer_and_falls_back_to_english() {
        assert_eq!(
            Lang::for_offer("es-MX", None),
            LanguageChoice {
                lang: Lang::Es,
                fallback_from: None
            }
        );
        assert_eq!(
            Lang::for_offer("en", None),
            LanguageChoice {
                lang: Lang::En,
                fallback_from: None
            }
        );
        assert_eq!(
            Lang::for_offer("fr", None),
            LanguageChoice {
                lang: Lang::En,
                fallback_from: Some("fr".into())
            },
            "an unsupported language drafts in EN and says why"
        );
    }

    #[test]
    fn the_user_override_wins_and_needs_no_notice() {
        assert_eq!(
            Lang::for_offer("es", Some(Lang::En)),
            LanguageChoice {
                lang: Lang::En,
                fallback_from: None
            }
        );
        assert_eq!(
            Lang::for_offer("fr", Some(Lang::Es)),
            LanguageChoice {
                lang: Lang::Es,
                fallback_from: None
            },
            "an override is not a fallback"
        );
    }

    #[test]
    fn new_document_ids_are_prefixed_ulids() {
        let id = new_document_id();
        assert!(id.starts_with("doc_"), "{id}");
        assert_ne!(id, new_document_id());
    }
}
