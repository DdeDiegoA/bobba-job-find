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

    /// Attach a review run's critique (`save_review`, S7.4) to the drafts it
    /// judged: the newest version of each kind for `application_id`. Returns how
    /// many rows were annotated, so a caller can tell an application with no
    /// drafts — 0 — from one that was annotated. The critique is agent text
    /// (ADR-010), so it only ever lands on a row the store already marks
    /// untrusted.
    fn set_review(
        &self,
        application_id: &str,
        review_json: &str,
        updated_at: &str,
    ) -> Result<u32, DocumentsRepoError>;

    /// Read the rendered PDF blob, `None` until it is rendered.
    fn pdf(&self, id: &str) -> Result<Option<Vec<u8>>, DocumentsRepoError>;

    /// Read the rendered DOCX blob, `None` until it is rendered.
    fn docx(&self, id: &str) -> Result<Option<Vec<u8>>, DocumentsRepoError>;
}

/// An in-memory [`DocumentsRepo`] for the use cases under test. It lives here,
/// next to the trait, so every `boba-app` module that needs a document store in
/// a test shares one; the real SQL lives behind the adapter, `boba-store`.
#[cfg(test)]
pub(crate) mod fake {
    use std::sync::Mutex;

    use boba_domain::documents::{DocumentKind, DocumentRecord, NewDocument};

    use super::{DocumentsRepo, DocumentsRepoError};

    #[derive(Default)]
    pub(crate) struct FakeDocumentsRepo {
        rows: Mutex<Vec<NewDocument>>,
        /// Every critique `set_review` was handed, with the application it judged.
        reviews: Mutex<Vec<(String, String)>>,
        /// When set, every write fails the way a full disk would.
        pub(crate) fail: bool,
    }

    impl FakeDocumentsRepo {
        /// The documents written so far, in order.
        pub(crate) fn rows(&self) -> Vec<NewDocument> {
            self.rows.lock().unwrap().clone()
        }

        /// The critiques written so far, as `(application_id, json)`.
        pub(crate) fn reviews(&self) -> Vec<(String, String)> {
            self.reviews.lock().unwrap().clone()
        }

        /// A fake whose writes fail, the way a full disk would.
        pub(crate) fn failing() -> Self {
            Self {
                fail: true,
                ..Self::default()
            }
        }
    }

    impl DocumentsRepo for FakeDocumentsRepo {
        fn insert(&self, new: &NewDocument) -> Result<i64, DocumentsRepoError> {
            if self.fail {
                return Err(DocumentsRepoError::new("disk gone"));
            }
            let mut rows = self.rows.lock().unwrap();
            // Versions are per (application, kind), like the real store's.
            let version = rows
                .iter()
                .filter(|row| row.application_id == new.application_id && row.kind == new.kind)
                .count() as i64
                + 1;
            rows.push(new.clone());
            Ok(version)
        }

        fn get(&self, _id: &str) -> Result<Option<DocumentRecord>, DocumentsRepoError> {
            Ok(None)
        }

        fn list_for_application(
            &self,
            _application_id: &str,
        ) -> Result<Vec<DocumentRecord>, DocumentsRepoError> {
            Ok(vec![])
        }

        fn latest(
            &self,
            _application_id: &str,
            _kind: DocumentKind,
        ) -> Result<Option<DocumentRecord>, DocumentsRepoError> {
            Ok(None)
        }

        fn set_pdf(&self, _: &str, _: &[u8], _: &str) -> Result<(), DocumentsRepoError> {
            Ok(())
        }

        fn set_docx(&self, _: &str, _: &[u8], _: &str) -> Result<(), DocumentsRepoError> {
            Ok(())
        }

        fn mark_verified(&self, _: &str, _: &str, _: &str) -> Result<(), DocumentsRepoError> {
            Ok(())
        }

        fn set_review(
            &self,
            application_id: &str,
            review_json: &str,
            _updated_at: &str,
        ) -> Result<u32, DocumentsRepoError> {
            if self.fail {
                return Err(DocumentsRepoError::new("disk gone"));
            }
            let annotated = {
                let rows = self.rows.lock().unwrap();
                [DocumentKind::Cv, DocumentKind::CoverLetter]
                    .iter()
                    .filter(|kind| {
                        rows.iter()
                            .any(|row| row.application_id == application_id && row.kind == **kind)
                    })
                    .count()
            };
            // No row matched: there is nothing to annotate and nothing is
            // written, exactly like the real `UPDATE` (S7.4 refuses the critique).
            if annotated > 0 {
                self.reviews
                    .lock()
                    .unwrap()
                    .push((application_id.to_string(), review_json.to_string()));
            }
            Ok(annotated as u32)
        }

        fn pdf(&self, _: &str) -> Result<Option<Vec<u8>>, DocumentsRepoError> {
            Ok(None)
        }

        fn docx(&self, _: &str) -> Result<Option<Vec<u8>>, DocumentsRepoError> {
            Ok(None)
        }
    }
}
