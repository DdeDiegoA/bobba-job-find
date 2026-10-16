//! `RenderPort` — render a tailored CV/cover document to PDF/DOCX (AD-14).
//! Adapter: `boba-docs`.
//!
//! Every type in these signatures is a `boba-domain` type (`DocumentModel`,
//! `TemplateId`, `RenderedPdf`, `TemplateInfo`): types that cross a port live in
//! `boba-domain`, and `boba-app` never depends on an adapter (AD-1, AD-14).

use boba_domain::documents::{
    DocumentKind, DocumentModel, Lang, RenderedPdf, TemplateId, TemplateInfo,
};
use thiserror::Error;

/// Why a render failed. The port carries its own error so `boba-app` stays free
/// of the adapter's types; a failed render is never attached to a submission.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RenderError {
    /// The template id is not in the adapter's registry.
    #[error("unknown template: {0}")]
    UnknownTemplate(String),
    /// The template belongs to the other document kind (a CV template asked to
    /// render a cover letter, or the reverse).
    #[error("template `{template}` cannot render a {kind:?} document")]
    WrongTemplate {
        template: String,
        kind: DocumentKind,
    },
    /// The stored document was written by a newer schema than this build knows.
    #[error("document schema version {version} is not supported (expected {expected})")]
    UnsupportedSchema { version: u32, expected: u32 },
    /// The document carries a different language than the one being rendered.
    /// Rendering anyway would label an ES document as EN.
    #[error("document is {document:?} but a {requested:?} render was asked for")]
    LocaleMismatch { document: Lang, requested: Lang },
    /// Rendering itself failed (layout, PDF export).
    #[error("render failed: {0}")]
    Render(String),
    /// The format is not implemented in this build. DOCX export lands in S7.7.
    #[error("render error: {0} is not supported yet")]
    Unsupported(&'static str),
}

/// Renders documents. Pure function of `(document, template, locale)` (AD-14).
pub trait RenderPort: Send + Sync {
    /// Every template the user can pick, in a stable order (FR-71).
    fn templates(&self) -> Vec<TemplateInfo>;

    /// Render `document` to PDF with `template`, in `locale`.
    ///
    /// Fails when the template is unknown, belongs to the other kind, the
    /// document's schema is newer than this build, or the document's language is
    /// not `locale`.
    fn render_pdf(
        &self,
        document: &DocumentModel,
        template: &TemplateId,
        locale: Lang,
    ) -> Result<RenderedPdf, RenderError>;

    /// Render `document` to DOCX bytes (S7.7). Until then every call reports
    /// [`RenderError::Unsupported`].
    fn render_docx(
        &self,
        document: &DocumentModel,
        template: &TemplateId,
        locale: Lang,
    ) -> Result<Vec<u8>, RenderError>;
}
