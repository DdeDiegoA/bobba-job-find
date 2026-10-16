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
        _document: &DocumentModel,
        _template: &TemplateId,
        _locale: Lang,
    ) -> Result<Vec<u8>, RenderError> {
        // S7.7 fills this from `boba-docs/src/docx`.
        Err(RenderError::Unsupported("docx export"))
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
    fn a_claim_without_facts_serialises_as_an_empty_list_not_a_missing_field() {
        let mut doc = cv(Lang::En);
        doc.sections[0].items[0].fact_ids = vec![];
        let value: serde_json::Value =
            serde_json::to_value(&doc).expect("serialize the model the validator reads");
        assert!(
            value["declared_gaps"][0] == "Kubernetes",
            "missing keywords stay on the document: {value}"
        );
        assert!(
            value["sections"][0]["items"][0].get("fact_ids").is_none(),
            "an item with no facts omits the list; S7.3 rejects it"
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

    #[test]
    fn docx_is_reported_unsupported_until_s7_7() {
        let document = DocumentModel::Cv(cv(Lang::En));
        assert_eq!(
            port()
                .render_docx(&document, &TemplateId::new("cv-classic"), Lang::En)
                .expect_err("docx lands in S7.7"),
            RenderError::Unsupported("docx export")
        );
    }
}
