//! `VerifyPort` — what a rendered document measures, and what an ATS reads back
//! out of it (FR-76, FR-77, AD-14).
//! Adapter: `boba-docs` (§4.4).
//!
//! Every type in these signatures is a `boba-domain` type (`DocumentModel`,
//! `TemplateId`, `LayoutReport`, `TextLayerReport`, `VerificationReport`): types
//! that cross a port live in `boba-domain`, and `boba-app` never depends on an
//! adapter (AD-1, AD-14).

use boba_domain::documents::{
    DocumentKind, DocumentModel, LayoutReport, RenderedPdf, TemplateId, TextLayerReport,
    VerificationReport,
};
use thiserror::Error;

/// Why a measurement failed. The port carries its own error so `boba-app` stays
/// free of the adapter's types; the renderer has its own, `RenderError`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum VerifyError {
    /// The template id is not in the adapter's registry.
    #[error("unknown template: {0}")]
    UnknownTemplate(String),
    /// The template belongs to the other document kind (a CV template asked to
    /// measure a cover letter, or the reverse).
    #[error("template `{template}` cannot measure a {kind:?} document")]
    WrongTemplate {
        template: String,
        kind: DocumentKind,
    },
    /// The stored document was written by a newer schema than this build knows.
    #[error("document schema version {version} is not supported (expected {expected})")]
    UnsupportedSchema { version: u32, expected: u32 },
    /// Laying the document out — or reading its text layer back — failed.
    #[error("measurement failed: {0}")]
    Measure(String),
}

/// Measures a document's layout and reads its ATS text layer (FR-76, FR-77).
pub trait VerifyPort: Send + Sync {
    /// The layout facts of `document` rendered with `template`: the pages it laid
    /// out to, and the section headings stranded at the bottom of a page.
    ///
    /// Measurement renders no PDF and writes no file: the core asks only how the
    /// draft *would* lay out, then [`crate::fitting`] decides what to cut.
    fn layout(
        &self,
        document: &DocumentModel,
        template: &TemplateId,
    ) -> Result<LayoutReport, VerifyError>;

    /// What an ATS parser reads back out of `rendered` (FR-77), checked against
    /// `document` itself and against the `required_keywords` a Profile fact backs
    /// (FR-75, AC1: `boba_domain::keywords::backed_keywords`, never a gap).
    ///
    /// A check that cannot be read at all is an error, not a pass: an unreadable
    /// text layer must never look like a verified one (SM-8, AD-14).
    fn text_layer(
        &self,
        document: &DocumentModel,
        rendered: &RenderedPdf,
        required_keywords: &[String],
    ) -> Result<TextLayerReport, VerifyError>;
}

/// Run all three checks a render must pass and report what they found (AD-14).
///
/// The layout facts come from [`VerifyPort::layout`] — the pass S7.5 cuts
/// against — and the ATS facts from [`VerifyPort::text_layer`]; the fabrication
/// verdict (FR-74, AD-15) is the caller's, because it is answered from a Profile
/// snapshot this layer does not hold. The report deliberately does **not** decide:
/// the caller knows the document kind and the user's page budget, so
/// [`VerificationReport::is_verified`] asks it there.
pub fn verify_document(
    verify: &dyn VerifyPort,
    document: &DocumentModel,
    template: &TemplateId,
    rendered: &RenderedPdf,
    required_keywords: &[String],
    fabrication_ready: bool,
) -> Result<VerificationReport, VerifyError> {
    Ok(VerificationReport {
        layout: verify.layout(document, template)?,
        text_layer: verify.text_layer(document, rendered, required_keywords)?,
        fabrication_ready,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use boba_domain::documents::{
        Claim, Contact, CvDocument, DocumentKind, Lang, LayoutReport, RenderedPdf, SCHEMA_VERSION,
        Section, TextLayerReport,
    };

    use super::*;

    fn document() -> DocumentModel {
        DocumentModel::Cv(CvDocument {
            schema_version: SCHEMA_VERSION,
            lang: Lang::En,
            contact: Contact {
                full_name: "Ada Lovelace".into(),
                headline: None,
                email: Some("ada@example.com".into()),
                phone: None,
                location: None,
                links: vec![],
            },
            summary: None,
            sections: vec![Section {
                heading: "Experience".into(),
                items: vec![Claim {
                    text: "Shipped the billing pipeline in Rust".into(),
                    fact_ids: vec!["fact_1".into()],
                }],
            }],
            declared_gaps: vec![],
        })
    }

    fn rendered() -> RenderedPdf {
        RenderedPdf {
            bytes: b"%PDF-1.7".to_vec(),
            layout: LayoutReport {
                pages: 1,
                orphans: 0,
            },
        }
    }

    fn passing_layer() -> TextLayerReport {
        TextLayerReport {
            text_layer: "Ada Lovelace ada@example.com Experience".into(),
            ..TextLayerReport::default()
        }
    }

    /// A port that answers with whatever the test handed it, and records what it
    /// was asked (the same shape S7.5's fakes use: no Typst in a unit test).
    struct FakeVerify {
        layout: LayoutReport,
        text_layer: TextLayerReport,
        keywords: Mutex<Vec<String>>,
    }

    impl FakeVerify {
        fn new(layout: LayoutReport, text_layer: TextLayerReport) -> Self {
            Self {
                layout,
                text_layer,
                keywords: Mutex::new(Vec::new()),
            }
        }
    }

    impl VerifyPort for FakeVerify {
        fn layout(
            &self,
            _document: &DocumentModel,
            _template: &TemplateId,
        ) -> Result<LayoutReport, VerifyError> {
            Ok(self.layout)
        }

        fn text_layer(
            &self,
            _document: &DocumentModel,
            _rendered: &RenderedPdf,
            required_keywords: &[String],
        ) -> Result<TextLayerReport, VerifyError> {
            *self.keywords.lock().expect("the fake records keywords") = required_keywords.to_vec();
            Ok(self.text_layer.clone())
        }
    }

    /// A port that cannot read the render at all.
    struct BlindVerify;

    impl VerifyPort for BlindVerify {
        fn layout(
            &self,
            _document: &DocumentModel,
            _template: &TemplateId,
        ) -> Result<LayoutReport, VerifyError> {
            Ok(LayoutReport {
                pages: 1,
                orphans: 0,
            })
        }

        fn text_layer(
            &self,
            _document: &DocumentModel,
            _rendered: &RenderedPdf,
            _required_keywords: &[String],
        ) -> Result<TextLayerReport, VerifyError> {
            Err(VerifyError::Measure("no text layer".into()))
        }
    }

    fn template() -> TemplateId {
        TemplateId::new("cv-classic")
    }

    // AC1/AC3: the verdict comes from the port, and the keywords the Profile
    // backs are what it is asked to look for.
    #[test]
    fn verify_document_reports_both_checks() {
        let verify = FakeVerify::new(
            LayoutReport {
                pages: 2,
                orphans: 0,
            },
            passing_layer(),
        );
        let report = verify_document(
            &verify,
            &document(),
            &template(),
            &rendered(),
            &["rust".into(), "postgres".into()],
            true,
        )
        .expect("both checks answer");

        assert_eq!(report.layout.pages, 2);
        assert!(report.text_layer.passed());
        assert!(report.is_verified(DocumentKind::Cv, 2));
        assert_eq!(
            *verify.keywords.lock().expect("read back"),
            ["rust", "postgres"]
        );
    }

    // AC3: any one of the three failing leaves the document unverified.
    #[test]
    fn a_failed_ats_check_makes_the_document_unverified() {
        let verify = FakeVerify::new(
            LayoutReport {
                pages: 1,
                orphans: 0,
            },
            TextLayerReport {
                missing_keywords: vec!["rust".into()],
                ..passing_layer()
            },
        );
        let report = verify_document(
            &verify,
            &document(),
            &template(),
            &rendered(),
            &["rust".into()],
            true,
        )
        .expect("both checks answer");

        assert!(!report.text_layer.passed());
        assert!(!report.is_verified(DocumentKind::Cv, 2));

        let over = verify_document(&verify, &document(), &template(), &rendered(), &[], false)
            .expect("both checks answer");
        assert!(
            !over.is_verified(DocumentKind::Cv, 2),
            "layout budget and fabrication still gate"
        );
    }

    // SM-8: a render whose text layer cannot be read is an error, never a pass.
    #[test]
    fn an_unreadable_text_layer_is_an_error_not_a_pass() {
        assert_eq!(
            verify_document(
                &BlindVerify,
                &document(),
                &template(),
                &rendered(),
                &[],
                true
            )
            .expect_err("reading the layer failed"),
            VerifyError::Measure("no text layer".into())
        );
    }
}
