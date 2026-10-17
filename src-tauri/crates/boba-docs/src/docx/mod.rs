//! DOCX export (S7.7, FR-78, AD-14).
//!
//! A DOCX is the same content as the PDF, in a format an ATS can read back:
//! one column, plain paragraphs, standard headings, contact details as literal
//! text — no tables, no text boxes, no images, no headers, and no Word field
//! codes. What the PDF prints is what this writes, and nothing else: the
//! declared gaps stay in the studio, never in a document (FR-75, AD-15).
//!
//! Every string is written as text through `docx-rs`, which escapes it, so
//! agent-written draft text can never become markup (ADR-010).

use boba_domain::documents::{
    Contact, DocumentKind, DocumentModel, Lang, SCHEMA_VERSION, TemplateId,
};
use docx_rs::{Docx, Paragraph, Run};

use crate::typst::{Template, check as check_template, check_schema};

/// Why a DOCX export failed. Mirrors [`crate::RenderError`] for the checks that
/// are the renderer's, so a caller maps both the same way.
#[derive(Debug, thiserror::Error)]
pub enum DocxError {
    #[error("unknown template: {0}")]
    UnknownTemplate(String),
    #[error("template `{template}` cannot render a {kind:?} document")]
    WrongTemplate {
        template: String,
        kind: DocumentKind,
    },
    #[error("document schema version {version} is not supported (expected {expected})")]
    UnsupportedSchema { version: u32, expected: u32 },
    /// The document carries a different language than the one being exported.
    #[error("document is {document:?} but a {requested:?} export was asked for")]
    LocaleMismatch { document: Lang, requested: Lang },
    /// Writing the package failed.
    #[error("docx write failed: {0}")]
    Write(String),
}

/// A4 for a Spanish draft, US Letter for an English one — the pages the Typst
/// templates set, in twips (1/20 pt), so the two exports are the same document
/// on the same paper.
const A4: (u32, u32) = (11_906, 16_838);
const US_LETTER: (u32, u32) = (12_240, 15_840);

/// Render `document` to DOCX bytes with `template`, in `locale` (FR-78).
///
/// The checks are the PDF renderer's (`crate::typst::Template` resolves the id
/// and owns the kind/schema rules), so a document that exports has already been
/// rendered: the two formats cannot disagree about what is renderable.
pub fn render(
    document: &DocumentModel,
    template: &TemplateId,
    locale: Lang,
) -> Result<Vec<u8>, DocxError> {
    let template = Template::parse(template.as_str())
        .ok_or_else(|| DocxError::UnknownTemplate(template.to_string()))?;
    check_template(template, document.kind()).map_err(|error| match error {
        crate::typst::RenderError::WrongTemplate { template, kind } => DocxError::WrongTemplate {
            template: template.to_string(),
            kind,
        },
        other => DocxError::Write(other.to_string()),
    })?;
    check_schema(document.schema_version()).map_err(|_| DocxError::UnsupportedSchema {
        version: document.schema_version(),
        expected: SCHEMA_VERSION,
    })?;
    if document.lang() != locale {
        return Err(DocxError::LocaleMismatch {
            document: document.lang(),
            requested: locale,
        });
    }

    let (docx, page) = match document {
        DocumentModel::Cv(cv) => (cv_body(cv), page_of(cv.lang)),
        DocumentModel::CoverLetter(cover) => (cover_body(cover), page_of(cover.lang)),
    };
    write(docx.page_size(page.0, page.1))
}

fn page_of(lang: Lang) -> (u32, u32) {
    match lang {
        Lang::Es => A4,
        Lang::En => US_LETTER,
    }
}

/// The CV's paragraphs, in the order the template prints them.
fn cv_body(cv: &boba_domain::documents::CvDocument) -> Docx {
    let mut docx = Docx::new();
    docx = docx.add_paragraph(line(&cv.contact.full_name, Size::Name, true));
    if let Some(headline) = &cv.contact.headline {
        docx = docx.add_paragraph(line(headline, Size::Subtitle, false));
    }
    docx = docx.add_paragraph(line(&contact_line(&cv.contact), Size::Detail, false));
    let links = cv
        .contact
        .links
        .iter()
        .map(|link| format!("{}: {}", link.label, link.url))
        .collect::<Vec<_>>()
        .join("  |  ");
    if !links.is_empty() {
        docx = docx.add_paragraph(line(&links, Size::Detail, false));
    }
    if let Some(summary) = &cv.summary {
        docx = docx.add_paragraph(line("Summary", Size::Heading, true));
        docx = docx.add_paragraph(line(&summary.text, Size::Body, false));
    }
    for section in &cv.sections {
        docx = docx.add_paragraph(line(&section.heading, Size::Heading, true));
        for item in &section.items {
            // A literal bullet, not a Word list style: every ATS reads the
            // character, and a numbering reference it cannot resolve is exactly
            // what this export exists to avoid.
            docx = docx.add_paragraph(line(&format!("• {}", item.text), Size::Body, false));
        }
    }
    docx
}

/// The cover letter's paragraphs, in the order the template prints them.
fn cover_body(cover: &boba_domain::documents::CoverLetterDocument) -> Docx {
    let mut docx = Docx::new();
    docx = docx.add_paragraph(line(&cover.contact.full_name, Size::CoverName, true));
    docx = docx.add_paragraph(line(&contact_line(&cover.contact), Size::Detail, false));
    if let Some(date) = &cover.date {
        docx = docx.add_paragraph(line(date, Size::Body, false));
    }
    if let Some(company) = &cover.company {
        docx = docx.add_paragraph(line(company, Size::Body, true));
    }
    if let Some(recipient) = &cover.recipient {
        docx = docx.add_paragraph(line(recipient, Size::Body, false));
    }
    if let Some(salutation) = &cover.salutation {
        docx = docx.add_paragraph(line(salutation, Size::Body, false));
    }
    for paragraph in &cover.paragraphs {
        docx = docx.add_paragraph(line(&paragraph.text, Size::Body, false));
    }
    if let Some(closing) = &cover.closing {
        docx = docx.add_paragraph(line(closing, Size::Body, false));
    }
    docx
}

/// The contact details the template prints, as one literal line.
fn contact_line(contact: &Contact) -> String {
    [&contact.email, &contact.phone, &contact.location]
        .into_iter()
        .flatten()
        .cloned()
        .collect::<Vec<_>>()
        .join("  |  ")
}

/// The sizes the Typst templates set, in half-points (Word's `w:sz`).
#[derive(Clone, Copy)]
enum Size {
    CoverName,
    Name,
    Heading,
    Subtitle,
    Body,
    Detail,
}

impl Size {
    const fn half_points(self) -> usize {
        match self {
            Size::CoverName => 32,
            Size::Name => 38,
            Size::Heading => 22,
            Size::Subtitle => 21,
            Size::Body => 20,
            Size::Detail => 18,
        }
    }
}

fn line(text: &str, size: Size, bold: bool) -> Paragraph {
    let run = Run::new().add_text(text).size(size.half_points());
    let run = if bold { run.bold() } else { run };
    Paragraph::new().add_run(run)
}

/// Pack the document, returning the bytes.
fn write(docx: Docx) -> Result<Vec<u8>, DocxError> {
    let mut buffer = std::io::Cursor::new(Vec::new());
    docx.build()
        .pack(&mut buffer)
        .map_err(|error| DocxError::Write(error.to_string()))?;
    Ok(buffer.into_inner())
}

#[cfg(test)]
mod tests {
    use boba_domain::documents::{
        Claim, Contact, CoverLetterDocument, CvDocument, DocumentModel, Lang, Link, SCHEMA_VERSION,
        Section, TemplateId,
    };
    use docx_rs::{DocumentChild, ParagraphChild, RunChild};

    use super::*;

    fn contact() -> Contact {
        Contact {
            full_name: "Ada Lovelace".into(),
            headline: Some("Backend developer".into()),
            email: Some("ada@example.com".into()),
            phone: Some("+57 300 000 0000".into()),
            location: Some("Bogota".into()),
            links: vec![Link {
                label: "GitHub".into(),
                url: "github.com/ada".into(),
            }],
        }
    }

    fn cv() -> CvDocument {
        CvDocument {
            schema_version: SCHEMA_VERSION,
            lang: Lang::En,
            contact: contact(),
            summary: Some(Claim {
                text: "Backend developer with eight years of APIs.".into(),
                fact_ids: vec!["fact_1".into()],
            }),
            sections: vec![
                Section {
                    heading: "Experience".into(),
                    items: vec![
                        Claim {
                            text: "Shipped a billing API for 2M users.".into(),
                            fact_ids: vec!["fact_2".into()],
                        },
                        Claim {
                            text: "Cut p99 latency by half.".into(),
                            fact_ids: vec!["fact_3".into()],
                        },
                    ],
                },
                Section {
                    heading: "Skills".into(),
                    items: vec![Claim {
                        text: "Rust and PostgreSQL.".into(),
                        fact_ids: vec!["fact_4".into()],
                    }],
                },
            ],
            declared_gaps: vec!["Kubernetes".into()],
        }
    }

    fn cover() -> CoverLetterDocument {
        CoverLetterDocument {
            schema_version: SCHEMA_VERSION,
            lang: Lang::En,
            contact: contact(),
            company: Some("Acme".into()),
            recipient: Some("Hiring team".into()),
            date: Some("2026-10-17".into()),
            salutation: Some("Dear hiring team,".into()),
            paragraphs: vec![
                Claim {
                    text: "I would like to apply for the backend role.".into(),
                    fact_ids: vec!["fact_1".into()],
                },
                Claim {
                    text: "I built the billing API you describe.".into(),
                    fact_ids: vec!["fact_2".into()],
                },
            ],
            closing: Some("Sincerely, Ada".into()),
            declared_gaps: vec![],
        }
    }

    /// The paragraphs of a DOCX, in document order — what a reader (or an ATS)
    /// gets out of it.
    fn paragraphs(bytes: &[u8]) -> Vec<String> {
        let docx = docx_rs::read_docx(bytes).expect("the bytes we wrote are a DOCX");
        let mut out = Vec::new();
        for child in &docx.document.children {
            let DocumentChild::Paragraph(paragraph) = child else {
                continue;
            };
            let text: String = paragraph
                .children
                .iter()
                .filter_map(|child| match child {
                    ParagraphChild::Run(run) => Some(
                        run.children
                            .iter()
                            .filter_map(|child| match child {
                                RunChild::Text(text) => Some(text.text.as_str()),
                                _ => None,
                            })
                            .collect::<String>(),
                    ),
                    _ => None,
                })
                .collect();
            out.push(text);
        }
        out
    }

    /// Every non-empty line of the PDF's text layer, folded for whitespace.
    fn pdf_lines(bytes: &[u8]) -> Vec<String> {
        pdf_extract::extract_text_from_mem(bytes)
            .expect("the PDF we just rendered has a text layer")
            .lines()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .map(|line| line.to_string())
            .collect()
    }

    /// A text with every whitespace run removed and the bullets dropped: the
    /// formatting both formats agree is invisible to the content an ATS reads.
    fn content(text: &str) -> String {
        text.chars()
            .filter(|c| !c.is_whitespace() && *c != '•' && *c != '-')
            .collect()
    }

    fn render_pdf(document: &DocumentModel, template: &str) -> Vec<u8> {
        crate::typst::render_document(document, Template::parse(template).expect("template"))
            .expect("render")
            .bytes
    }

    // AC2: the DOCX says exactly what the PDF says. The text layer groups blocks
    // into lines by layout while the DOCX keeps them as paragraphs, so the two
    // are compared as content: same text, same order, whitespace and the bullet
    // character aside.
    #[test]
    fn cv_docx_carries_the_content_of_the_pdf() {
        let document = DocumentModel::Cv(cv());
        let bytes = render(&document, &TemplateId::new("cv-classic"), Lang::En).expect("export");

        assert!(bytes.starts_with(b"PK"), "a DOCX is a zip package");
        let written = paragraphs(&bytes);
        assert_eq!(
            content(&written.join(" ")),
            content(&pdf_lines(&render_pdf(&document, "cv-classic")).join(" ")),
            "the DOCX and the PDF carry the same content"
        );
        // And nothing the PDF does not print: the declared gaps stay in the
        // studio (FR-75, AD-15).
        assert!(
            !written.iter().any(|p| p.contains("Kubernetes")),
            "a declared gap must never reach a document"
        );
        assert_eq!(written[0], "Ada Lovelace");
        assert_eq!(written[1], "Backend developer");
        assert_eq!(
            written[4], "Summary",
            "the summary follows the contact block"
        );
        assert!(written.iter().any(|p| p == "Experience"));
        assert!(
            written
                .iter()
                .any(|p| p.ends_with("Shipped a billing API for 2M users."))
        );
    }

    // AC2: the cover letter exports the same way, salutation and closing
    // included.
    #[test]
    fn cover_docx_carries_the_content_of_the_pdf() {
        let document = DocumentModel::CoverLetter(cover());
        let bytes = render(&document, &TemplateId::new("cover-classic"), Lang::En).expect("export");

        let written = paragraphs(&bytes);
        assert_eq!(
            content(&written.join(" ")),
            content(&pdf_lines(&render_pdf(&document, "cover-classic")).join(" ")),
            "the DOCX and the PDF carry the same content"
        );
        assert!(written.iter().any(|p| p == "Dear hiring team,"));
        assert!(written.iter().any(|p| p == "Sincerely, Ada"));
    }

    // AC2: ATS-safe formatting — one column of paragraphs, no table, no text
    // box, no drawing, no image, and no field code an ATS cannot resolve.
    #[test]
    fn the_docx_is_one_column_of_plain_paragraphs() {
        let document = DocumentModel::Cv(cv());
        let bytes = render(&document, &TemplateId::new("cv-classic"), Lang::En).expect("export");
        let docx = docx_rs::read_docx(&bytes).expect("read back");

        assert!(
            !docx.document.children.iter().any(|child| matches!(
                child,
                DocumentChild::Table(_) | DocumentChild::TableOfContents(_)
            )),
            "an ATS reads a table's cells in an order nobody wrote: no tables"
        );
        for child in &docx.document.children {
            let DocumentChild::Paragraph(paragraph) = child else {
                continue;
            };
            for child in &paragraph.children {
                let ParagraphChild::Run(run) = child else {
                    continue;
                };
                for child in &run.children {
                    assert!(
                        !matches!(
                            child,
                            RunChild::FieldChar(_)
                                | RunChild::InstrText(_)
                                | RunChild::InstrTextString(_)
                                | RunChild::Drawing(_)
                                | RunChild::Shape(_)
                        ),
                        "no field, text box or image may reach a document: {child:?}"
                    );
                }
            }
        }
        assert!(
            docx.media.is_empty(),
            "an exported document carries no media"
        );
    }

    // The same refusals the PDF renderer makes: an unknown template, a template
    // of the other kind, a newer schema and a locale that disagrees with the
    // draft.
    #[test]
    fn unknown_template_wrong_kind_schema_and_locale_are_rejected() {
        let document = DocumentModel::Cv(cv());

        assert!(matches!(
            render(&document, &TemplateId::new("nope"), Lang::En),
            Err(DocxError::UnknownTemplate(_))
        ));
        assert!(matches!(
            render(&document, &TemplateId::new("cover-classic"), Lang::En),
            Err(DocxError::WrongTemplate { .. })
        ));
        assert!(matches!(
            render(&document, &TemplateId::new("cv-classic"), Lang::Es),
            Err(DocxError::LocaleMismatch { .. })
        ));

        let mut newer = cv();
        newer.schema_version = SCHEMA_VERSION + 1;
        assert!(matches!(
            render(
                &DocumentModel::Cv(newer),
                &TemplateId::new("cv-classic"),
                Lang::En
            ),
            Err(DocxError::UnsupportedSchema { .. })
        ));
    }

    // ADR-010: text that came from an agent or a posting is written as text,
    // never as markup.
    #[test]
    fn draft_text_is_escaped_not_evaluated() {
        let mut doc = cv();
        doc.sections[0].items[0].text = "Led <A&B> integration".into();
        let bytes = render(
            &DocumentModel::Cv(doc),
            &TemplateId::new("cv-classic"),
            Lang::En,
        )
        .expect("export");
        let written = paragraphs(&bytes);
        assert!(
            written.iter().any(|p| p.contains("Led <A&B> integration")),
            "the text reads back as itself: {written:#?}"
        );
    }
}
