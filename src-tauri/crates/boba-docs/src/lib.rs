//! `boba-docs` — Typst renderer and document verification (AD-14).
//!
//! S7.1 built the Typst renderer and the embedded template registry; S7.2 puts
//! the adapter behind `boba_app::ports::RenderPort`, so the core renders through
//! a trait over `boba-domain` types and never sees Typst. The document model
//! itself moved to `boba_domain::documents` (AD-14) — there is no `model` module
//! here and no re-export shim.

pub mod docx;
pub mod typst;
pub mod verify;

use boba_app::ports::render::{RenderError, RenderPort};
use boba_domain::documents::{
    DocumentModel, Lang, RenderedPdf, SCHEMA_VERSION, TemplateId, TemplateInfo,
};

use crate::typst::{RenderError as TypstError, Template};

/// The Typst adapter behind [`RenderPort`] (AD-14, ADR-006).
#[derive(Debug, Clone, Copy, Default)]
pub struct TypstRenderer;

impl RenderPort for TypstRenderer {
    fn templates(&self) -> Vec<TemplateInfo> {
        Template::ALL
            .into_iter()
            .map(|template| TemplateInfo {
                id: TemplateId::new(template.name()),
                kind: template.kind(),
                default: template == Template::DEFAULT_CV || template == Template::DEFAULT_COVER,
            })
            .collect()
    }

    fn render_pdf(
        &self,
        document: &DocumentModel,
        template: &TemplateId,
        locale: Lang,
    ) -> Result<RenderedPdf, RenderError> {
        check_locale(document, locale)?;
        let template = resolve(template)?;
        typst::render_document(document, template).map_err(convert)
    }

    fn render_docx(
        &self,
        document: &DocumentModel,
        template: &TemplateId,
        locale: Lang,
    ) -> Result<Vec<u8>, RenderError> {
        // The DOCX writer makes the PDF renderer's checks itself (S7.7), so both
        // formats refuse the same documents.
        docx::render(document, template, locale).map_err(|error| match error {
            docx::DocxError::UnknownTemplate(id) => RenderError::UnknownTemplate(id),
            docx::DocxError::WrongTemplate { template, kind } => {
                RenderError::WrongTemplate { template, kind }
            }
            docx::DocxError::UnsupportedSchema { version, expected } => {
                RenderError::UnsupportedSchema { version, expected }
            }
            docx::DocxError::LocaleMismatch {
                document,
                requested,
            } => RenderError::LocaleMismatch {
                document,
                requested,
            },
            docx::DocxError::Write(message) => RenderError::Render(message),
        })
    }
}

/// A draft is rendered in the language it is written in (AD-17); rendering it as
/// another would label the document with the wrong locale.
fn check_locale(document: &DocumentModel, locale: Lang) -> Result<(), RenderError> {
    if document.lang() == locale {
        Ok(())
    } else {
        Err(RenderError::LocaleMismatch {
            document: document.lang(),
            requested: locale,
        })
    }
}

fn resolve(template: &TemplateId) -> Result<Template, RenderError> {
    Template::parse(template.as_str())
        .ok_or_else(|| RenderError::UnknownTemplate(template.to_string()))
}

fn convert(error: TypstError) -> RenderError {
    match error {
        TypstError::UnsupportedSchema(version) => RenderError::UnsupportedSchema {
            version,
            expected: SCHEMA_VERSION,
        },
        TypstError::WrongTemplate { template, kind } => RenderError::WrongTemplate {
            template: template.to_string(),
            kind,
        },
        TypstError::Json(e) => RenderError::Render(e.to_string()),
        TypstError::Compile(e) => RenderError::Render(e),
        TypstError::Pdf(e) => RenderError::Render(e),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use boba_domain::documents::{
        Claim, Contact, CvDocument, DocumentKind, Lang, SCHEMA_VERSION, Section,
    };

    use super::*;

    fn cv(lang: Lang) -> CvDocument {
        CvDocument {
            schema_version: SCHEMA_VERSION,
            lang,
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
        }
    }

    fn port() -> Arc<dyn RenderPort> {
        Arc::new(TypstRenderer)
    }

    // S7.1's model tests: the JSON the templates read is the JSON the model
    // writes. They live here now because `boba-domain` has no serde_json (AD-14).
    #[test]
    fn cv_round_trips_as_versioned_json() {
        let doc = cv(Lang::Es);
        let json = serde_json::to_string(&doc).expect("serialize");
        let back: CvDocument = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(doc, back);
        assert_eq!(doc.kind().as_str(), "cv");
        assert_eq!(doc.lang.code(), "es");
    }

    #[test]
    fn every_claim_carries_fact_ids_and_a_missing_list_fails_to_parse() {
        let mut doc = cv(Lang::En);
        doc.sections[0].items[0].fact_ids = vec![];
        let value: serde_json::Value =
            serde_json::to_value(&doc).expect("serialize the model the validator reads");
        assert_eq!(
            value["sections"][0]["items"][0]["fact_ids"],
            serde_json::json!([]),
            "the list is always written, even when it is empty"
        );
        assert_eq!(value["declared_gaps"][0], "Kubernetes");

        // AC2: a claim-bearing item without its facts is not a draft we accept.
        let missing = r#"{"schema_version":1,"lang":"en",
            "contact":{"full_name":"Ada"},
            "sections":[{"heading":"Experience","items":[{"text":"Led the billing team"}]}]}"#;
        assert!(
            serde_json::from_str::<CvDocument>(missing).is_err(),
            "an item with no fact_ids must not deserialize"
        );
    }

    #[test]
    fn templates_lists_both_kinds_with_their_defaults() {
        let templates = port().templates();
        assert_eq!(templates.len(), 4);
        assert_eq!(templates[0].id.as_str(), "cv-classic");
        assert_eq!(templates[0].kind, DocumentKind::Cv);
        assert!(templates[0].default, "cv-classic is the default CV");
        assert_eq!(
            templates.iter().filter(|t| t.default).count(),
            2,
            "one default per kind"
        );
    }

    // AC4: the port takes a DocumentModel, a TemplateId and a locale, and every
    // type in the signature is a boba-domain type.
    #[test]
    fn render_pdf_returns_bytes_and_a_layout_report() {
        let document = DocumentModel::Cv(cv(Lang::Es));
        let rendered = port()
            .render_pdf(&document, &TemplateId::new("cv-classic"), Lang::Es)
            .expect("render");
        assert!(rendered.bytes.starts_with(b"%PDF-"));
        assert_eq!(rendered.layout.pages, 1, "a one-section CV fits one page");
    }

    #[test]
    fn unknown_template_wrong_kind_and_schema_are_rejected() {
        let port = port();
        let document = DocumentModel::Cv(cv(Lang::En));

        assert_eq!(
            port.render_pdf(&document, &TemplateId::new("nope"), Lang::En)
                .expect_err("unknown template"),
            RenderError::UnknownTemplate("nope".into())
        );

        let mut bad = cv(Lang::En);
        bad.schema_version = 99;
        assert_eq!(
            port.render_pdf(
                &DocumentModel::Cv(bad),
                &TemplateId::new("cv-classic"),
                Lang::En
            )
            .expect_err("newer schema"),
            RenderError::UnsupportedSchema {
                version: 99,
                expected: SCHEMA_VERSION
            }
        );

        assert_eq!(
            port.render_pdf(&document, &TemplateId::new("cover-classic"), Lang::En)
                .expect_err("a cover template cannot render a CV"),
            RenderError::WrongTemplate {
                template: "cover-classic".into(),
                kind: DocumentKind::Cv
            }
        );
    }

    #[test]
    fn a_locale_that_disagrees_with_the_draft_is_rejected() {
        let document = DocumentModel::Cv(cv(Lang::Es));
        assert_eq!(
            port()
                .render_pdf(&document, &TemplateId::new("cv-classic"), Lang::En)
                .expect_err("mismatched locale"),
            RenderError::LocaleMismatch {
                document: Lang::Es,
                requested: Lang::En
            }
        );
    }

    // AC2 (S7.7, FR-78): the same document exports as a DOCX, and the refusals
    // are the ones the PDF render makes — the port reports them the same way, so
    // a caller maps one error type for both formats.
    #[test]
    fn docx_is_exported_and_refuses_what_the_pdf_refuses() {
        let document = DocumentModel::Cv(cv(Lang::En));
        let bytes = port()
            .render_docx(&document, &TemplateId::new("cv-classic"), Lang::En)
            .expect("export");
        assert!(bytes.starts_with(b"PK"), "a DOCX is a zip package");

        assert_eq!(
            port()
                .render_docx(&document, &TemplateId::new("cover-classic"), Lang::En)
                .expect_err("a cover template cannot export a CV"),
            RenderError::WrongTemplate {
                template: "cover-classic".into(),
                kind: DocumentKind::Cv
            }
        );
        assert_eq!(
            port()
                .render_docx(&document, &TemplateId::new("cv-classic"), Lang::Es)
                .expect_err("mismatched locale"),
            RenderError::LocaleMismatch {
                document: Lang::En,
                requested: Lang::Es
            }
        );
        assert_eq!(
            port()
                .render_docx(&document, &TemplateId::new("nope"), Lang::En)
                .expect_err("unknown template"),
            RenderError::UnknownTemplate("nope".into())
        );
    }
}
