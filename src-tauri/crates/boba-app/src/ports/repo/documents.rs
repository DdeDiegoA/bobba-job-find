//! `DocumentsRepo` — owns `document` (AD-14, AD-16). Only this area writes the
//! table; other modules read through its queries. The adapter is `boba-store`.
//!
//! S7.1 wrote the SQL before the trait existed; S7.2 adds the trait and conforms
//! the adapter to it. Rows hold the schema-versioned model JSON plus, once
//! rendered and verified, the PDF/DOCX blobs and the verification report.

use boba_domain::documents::{DocumentKind, DocumentRecord, NewDocument};

/// A repo-level failure, stringified so the port stays free of the adapter's
/// error types.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("document store error: {0}")]
pub struct DocumentsRepoError(pub String);

impl DocumentsRepoError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// The `document` owner area (AD-16).
pub trait DocumentsRepo: Send + Sync {
    /// Insert a document, assigning the next `version` for its
    /// `(application, kind)`. Returns the version it wrote.
    fn insert(&self, new: &NewDocument) -> Result<i64, DocumentsRepoError>;

    /// One document row by id, `None` when unknown.
    fn get(&self, id: &str) -> Result<Option<DocumentRecord>, DocumentsRepoError>;

    /// Every document of an application, oldest version first.
    fn list_for_application(
        &self,
        application_id: &str,
    ) -> Result<Vec<DocumentRecord>, DocumentsRepoError>;

    /// The newest version of one kind for an application.
    fn latest(
        &self,
        application_id: &str,
        kind: DocumentKind,
    ) -> Result<Option<DocumentRecord>, DocumentsRepoError>;

    /// Store the rendered PDF blob.
    fn set_pdf(&self, id: &str, pdf: &[u8], updated_at: &str) -> Result<(), DocumentsRepoError>;

    /// Store the rendered DOCX blob.
    fn set_docx(&self, id: &str, docx: &[u8], updated_at: &str) -> Result<(), DocumentsRepoError>;

    /// Record a verification pass (AD-14). Only verified documents may be
    /// submitted.
    fn mark_verified(
        &self,
        id: &str,
        verification_json: &str,
        verified_at: &str,
    ) -> Result<(), DocumentsRepoError>;

    /// Read the rendered PDF blob, `None` until it is rendered.
    fn pdf(&self, id: &str) -> Result<Option<Vec<u8>>, DocumentsRepoError>;

    /// Read the rendered DOCX blob, `None` until it is rendered.
    fn docx(&self, id: &str) -> Result<Option<Vec<u8>>, DocumentsRepoError>;
}
