//! `VerifyPort` — measure what a document lays out to (FR-76, AD-14).
//! Adapter: `boba-docs` (§4.4). S7.6 extends this port with the ATS text-layer
//! check.
//!
//! Every type in these signatures is a `boba-domain` type (`DocumentModel`,
//! `TemplateId`, `LayoutReport`): types that cross a port live in `boba-domain`,
//! and `boba-app` never depends on an adapter (AD-1, AD-14).

use boba_domain::documents::{DocumentKind, DocumentModel, LayoutReport, TemplateId};
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
    /// Laying the document out failed.
    #[error("layout failed: {0}")]
    Measure(String),
}

/// Measures a document's layout (FR-76).
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
}
