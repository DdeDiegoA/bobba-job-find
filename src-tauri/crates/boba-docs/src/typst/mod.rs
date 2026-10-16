//! Embedded Typst PDF renderer (ADR-006, AD-14).
//!
//! Renderers are pure functions of `(document, template, locale)`. The document
//! is handed to the template as JSON in `sys.inputs.data`; the template prints
//! every string as text and never evaluates it as markup (ADR-010). Fonts are
//! bundled (OFL-1.1) and loaded from memory, so output is identical on every OS.

use typst::foundations::{Bytes, Dict, Value};
use typst_as_lib::TypstEngine;
use typst_layout::PagedDocument;
use typst_pdf::PdfOptions;

use boba_domain::documents::{
    CoverLetterDocument, CvDocument, DocumentKind, DocumentModel, LayoutReport, RenderedPdf,
    SCHEMA_VERSION,
};

/// Bundled OFL fonts. Lato covers Latin Extended (ES diacritics).
const FONTS: [&[u8]; 3] = [
    include_bytes!("../../../../../templates/fonts/Lato-Regular.ttf"),
    include_bytes!("../../../../../templates/fonts/Lato-Bold.ttf"),
    include_bytes!("../../../../../templates/fonts/Lato-Italic.ttf"),
];

const CV_CLASSIC: &str = include_str!("../../../../../templates/cv/classic.typ");
const CV_MODERN: &str = include_str!("../../../../../templates/cv/modern.typ");
const COVER_CLASSIC: &str = include_str!("../../../../../templates/cover/classic.typ");
const COVER_MODERN: &str = include_str!("../../../../../templates/cover/modern.typ");

/// A renderable template. The user picks one; names are stable (stored in the DB
/// and in settings), so renaming needs a migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Template {
    CvClassic,
    CvModern,
    CoverClassic,
    CoverModern,
}

impl Template {
    /// Templates available for a CV.
    pub const CV: [Template; 2] = [Template::CvClassic, Template::CvModern];
    /// Templates available for a cover letter.
    pub const COVER: [Template; 2] = [Template::CoverClassic, Template::CoverModern];
    /// Every template, in a stable order.
    pub const ALL: [Template; 4] = [
        Template::CvClassic,
        Template::CvModern,
        Template::CoverClassic,
        Template::CoverModern,
    ];
    /// The default CV template a user gets before changing it.
    pub const DEFAULT_CV: Template = Template::CvClassic;
    /// The default cover-letter template.
    pub const DEFAULT_COVER: Template = Template::CoverClassic;

    /// Stable identifier stored in settings and in `document.template`.
    pub const fn name(self) -> &'static str {
        match self {
            Template::CvClassic => "cv-classic",
            Template::CvModern => "cv-modern",
            Template::CoverClassic => "cover-classic",
            Template::CoverModern => "cover-modern",
        }
    }

    /// Which document kind this template renders.
    pub const fn kind(self) -> DocumentKind {
        match self {
            Template::CvClassic | Template::CvModern => DocumentKind::Cv,
            Template::CoverClassic | Template::CoverModern => DocumentKind::CoverLetter,
        }
    }

    /// The embedded Typst source.
    const fn source(self) -> &'static str {
        match self {
            Template::CvClassic => CV_CLASSIC,
            Template::CvModern => CV_MODERN,
            Template::CoverClassic => COVER_CLASSIC,
            Template::CoverModern => COVER_MODERN,
        }
    }

    /// Resolve a stored template name.
    pub fn parse(name: &str) -> Option<Template> {
        Template::ALL.into_iter().find(|t| t.name() == name)
    }
}

/// Why a render failed. A failed render is never attached to a submission (AD-14).
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("document schema version {0} is not supported (expected {SCHEMA_VERSION})")]
    UnsupportedSchema(u32),
    #[error("template `{template}` cannot render a {kind:?} document")]
    WrongTemplate {
        template: &'static str,
        kind: DocumentKind,
    },
    #[error("could not serialize the document: {0}")]
    Json(#[from] serde_json::Error),
    #[error("typst compile failed: {0}")]
    Compile(String),
    #[error("typst pdf export failed: {0}")]
    Pdf(String),
}

/// Render a CV to PDF bytes.
pub fn render_cv(document: &CvDocument, template: Template) -> Result<Vec<u8>, RenderError> {
    check(template, DocumentKind::Cv)?;
    check_schema(document.schema_version)?;
    render(template, &serde_json::to_string(document)?)
}

/// Render a cover letter to PDF bytes.
pub fn render_cover(
    document: &CoverLetterDocument,
    template: Template,
) -> Result<Vec<u8>, RenderError> {
    check(template, DocumentKind::CoverLetter)?;
    check_schema(document.schema_version)?;
    render(template, &serde_json::to_string(document)?)
}

fn check(template: Template, kind: DocumentKind) -> Result<(), RenderError> {
    if template.kind() == kind {
        Ok(())
    } else {
        Err(RenderError::WrongTemplate {
            template: template.name(),
            kind,
        })
    }
}

fn check_schema(version: u32) -> Result<(), RenderError> {
    if version == SCHEMA_VERSION {
        Ok(())
    } else {
        Err(RenderError::UnsupportedSchema(version))
    }
}

/// Render either draft to PDF, with the layout facts verification reads (S7.2).
pub fn render_document(
    document: &DocumentModel,
    template: Template,
) -> Result<RenderedPdf, RenderError> {
    // The templates branch on `lang`, so the JSON carries the draft's own
    // language; it is the same shape S7.1 handed the templates.
    let (kind, version, json) = match document {
        DocumentModel::Cv(doc) => (
            DocumentKind::Cv,
            doc.schema_version,
            serde_json::to_string(doc)?,
        ),
        DocumentModel::CoverLetter(doc) => (
            DocumentKind::CoverLetter,
            doc.schema_version,
            serde_json::to_string(doc)?,
        ),
    };
    check(template, kind)?;
    check_schema(version)?;
    let (bytes, pages) = render_paged(template, &json)?;
    Ok(RenderedPdf {
        bytes,
        layout: LayoutReport {
            pages,
            // shortcut: orphan headings are counted in S7.5's layout pass; this
            // renderer only knows the page count.
            orphans: 0,
        },
    })
}

/// Compile `template` against `document_json` and export a PDF.
fn render(template: Template, document_json: &str) -> Result<Vec<u8>, RenderError> {
    Ok(render_paged(template, document_json)?.0)
}

/// Compile and export, also reporting how many pages the document laid out to.
fn render_paged(template: Template, document_json: &str) -> Result<(Vec<u8>, u32), RenderError> {
    let engine = TypstEngine::builder()
        .main_file(template.source())
        .fonts(FONTS)
        .build();

    let mut inputs = Dict::new();
    inputs.insert(
        "data".into(),
        Value::Bytes(Bytes::from_string(document_json.to_string())),
    );

    let compiled = engine.compile_with_input::<_, PagedDocument>(inputs);
    let document = compiled
        .output
        .map_err(|e| RenderError::Compile(e.to_string()))?;
    let pages = u32::try_from(document.pages().len()).unwrap_or(u32::MAX);
    let pdf = typst_pdf::pdf(&document, &PdfOptions::default())
        .map_err(|e| RenderError::Pdf(format!("{e:?}")))?;
    Ok((pdf, pages))
}

#[cfg(test)]
mod tests {
    use super::*;
    use boba_domain::documents::{Claim, Contact, Lang, Link, Section};

    fn cv(lang: Lang, name: &str, item: &str) -> CvDocument {
        CvDocument {
            schema_version: SCHEMA_VERSION,
            lang,
            contact: Contact {
                full_name: name.into(),
                headline: Some("Backend developer".into()),
                email: Some("ada@example.com".into()),
                phone: Some("+57 300 000 0000".into()),
                location: Some("Bogotá, Colombia".into()),
                links: vec![Link {
                    label: "GitHub".into(),
                    url: "github.com/ada".into(),
                }],
            },
            summary: Some(Claim {
                text: "Backend developer with 6 years of experience.".into(),
                fact_ids: vec!["fact_summary".into()],
            }),
            sections: vec![Section {
                heading: "Experience".into(),
                items: vec![Claim {
                    text: item.into(),
                    fact_ids: vec!["fact_1".into()],
                }],
            }],
            declared_gaps: vec!["Kubernetes".into()],
        }
    }

    fn cover(lang: Lang) -> CoverLetterDocument {
        CoverLetterDocument {
            schema_version: SCHEMA_VERSION,
            lang,
            contact: Contact {
                full_name: "Ada Lovelace".into(),
                headline: None,
                email: Some("ada@example.com".into()),
                phone: None,
                location: None,
                links: vec![],
            },
            company: Some("Acme".into()),
            recipient: Some("Hiring team".into()),
            date: Some("5 October 2026".into()),
            salutation: Some("Dear Hiring team,".into()),
            paragraphs: vec![Claim {
                text: "I would like to apply for the role.".into(),
                fact_ids: vec!["fact_letter".into()],
            }],
            closing: Some("Sincerely, Ada".into()),
            declared_gaps: vec![],
        }
    }

    fn text_of(pdf: &[u8]) -> String {
        pdf_extract::extract_text_from_mem(pdf).expect("pdf text layer")
    }

    /// Cheap substring scan over the raw PDF bytes.
    fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn every_template_renders_a_real_text_pdf_in_both_languages() {
        for template in Template::CV {
            for lang in [Lang::En, Lang::Es] {
                let pdf = render_cv(&cv(lang, "Ada Lovelace", "Shipped an API"), template)
                    .unwrap_or_else(|e| panic!("{} {lang:?}: {e}", template.name()));
                assert!(
                    pdf.starts_with(b"%PDF-"),
                    "{} must be a PDF",
                    template.name()
                );
                assert!(
                    contains_bytes(&pdf, b"Lato"),
                    "{} must embed the bundled Lato font, not fall back",
                    template.name()
                );
                let text = text_of(&pdf);
                assert!(
                    text.contains("Ada Lovelace"),
                    "{} {lang:?}: missing contact name, got: {text}",
                    template.name()
                );
                assert!(
                    text.contains("Experience"),
                    "{} {lang:?}: missing standard heading",
                    template.name()
                );
                assert!(
                    text.contains("Shipped an API"),
                    "{} {lang:?}: missing body text",
                    template.name()
                );
            }
        }

        for template in Template::COVER {
            for lang in [Lang::En, Lang::Es] {
                let pdf = render_cover(&cover(lang), template)
                    .unwrap_or_else(|e| panic!("{} {lang:?}: {e}", template.name()));
                assert!(pdf.starts_with(b"%PDF-"));
                let text = text_of(&pdf);
                assert!(text.contains("Ada Lovelace"), "cover contact");
                assert!(
                    text.contains("I would like to apply for the role."),
                    "cover body: {text}"
                );
            }
        }
    }

    // ADR-010: agent output is untrusted. Typst markup in a string must render
    // literally, never be evaluated.
    #[test]
    fn markup_in_strings_is_rendered_as_text_not_evaluated() {
        let payload = "#(1+1) #eval(\"1+1\") = heading";
        let doc = cv(Lang::En, "Ada Lovelace", payload);
        let pdf = render_cv(&doc, Template::CvClassic).expect("render");
        let text = text_of(&pdf);
        assert!(text.contains(payload), "payload must be literal: {text}");
        assert!(
            !text.contains("2"),
            "payload must not be evaluated, got: {text}"
        );
    }

    #[test]
    fn wrong_template_and_schema_are_rejected() {
        let cv = cv(Lang::En, "Ada", "x");
        assert!(matches!(
            render_cv(&cv, Template::CoverClassic),
            Err(RenderError::WrongTemplate { .. })
        ));

        let mut bad = cv;
        bad.schema_version = 99;
        assert!(matches!(
            render_cv(&bad, Template::CvClassic),
            Err(RenderError::UnsupportedSchema(99))
        ));
    }

    #[test]
    fn template_names_round_trip() {
        for template in Template::ALL {
            assert_eq!(Template::parse(template.name()), Some(template));
        }
        assert_eq!(Template::parse("nope"), None);
    }
}
