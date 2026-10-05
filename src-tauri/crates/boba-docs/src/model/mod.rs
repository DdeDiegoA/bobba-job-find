//! Schema-versioned document model (AD-14, AD-15).
//!
//! Tailoring produces a [`CvDocument`] or a [`CoverLetterDocument`]; both are
//! plain data with a `schema_version`. Every claim-bearing line carries the
//! `fact_ids` of the profile facts it was built from (AD-15), so a fabricated
//! claim can always be traced back — or rejected.
//!
//! Renderers are pure functions of `(document, template, locale)`. Strings go
//! into Typst as JSON data and are printed as text, never evaluated as markup
//! (ADR-010).

use serde::{Deserialize, Serialize};

/// Current schema version. Bump on any breaking change to the JSON shape.
pub const SCHEMA_VERSION: u32 = 1;

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cv_round_trips_as_versioned_json() {
        let doc = CvDocument {
            schema_version: SCHEMA_VERSION,
            lang: Lang::Es,
            contact: Contact {
                full_name: "Ada Lovelace".into(),
                headline: Some("Backend developer".into()),
                email: Some("ada@example.com".into()),
                phone: None,
                location: Some("Bogotá".into()),
                links: vec![],
            },
            summary: Some(Claim {
                text: "Backend developer".into(),
                fact_ids: vec!["fact_1".into()],
            }),
            sections: vec![Section {
                heading: "Experience".into(),
                items: vec![Claim {
                    text: "Shipped an API".into(),
                    fact_ids: vec!["fact_2".into()],
                }],
            }],
            declared_gaps: vec!["Kubernetes".into()],
        };

        let json = serde_json::to_string(&doc).expect("serialize");
        let back: CvDocument = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(doc, back);
        assert_eq!(doc.kind().as_str(), "cv");
        assert_eq!(doc.lang.code(), "es");
    }

    #[test]
    fn optional_fields_are_omitted_from_json() {
        let doc = CoverLetterDocument {
            schema_version: SCHEMA_VERSION,
            lang: Lang::En,
            contact: Contact {
                full_name: "Ada".into(),
                headline: None,
                email: None,
                phone: None,
                location: None,
                links: vec![],
            },
            company: None,
            recipient: None,
            date: None,
            salutation: None,
            paragraphs: vec![],
            closing: None,
            declared_gaps: vec![],
        };
        let json = serde_json::to_string(&doc).expect("serialize");
        assert!(
            !json.contains("email"),
            "None fields must be omitted: {json}"
        );
        assert!(json.contains("\"schema_version\":1"));
    }
}
