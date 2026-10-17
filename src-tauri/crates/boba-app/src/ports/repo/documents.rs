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

    /// One in-memory row: the document plus what later writes attach to it.
    #[derive(Clone)]
    struct Row {
        document: NewDocument,
        version: i64,
        pdf: Option<Vec<u8>>,
        docx: Option<Vec<u8>>,
        verification_json: Option<String>,
        verified_at: Option<String>,
        review_json: Option<String>,
        updated_at: String,
    }

    impl Row {
        fn record(&self) -> DocumentRecord {
            let document = &self.document;
            DocumentRecord {
                id: document.id.clone(),
                application_id: document.application_id.clone(),
                kind: document.kind,
                lang: document.lang,
                template: document.template.clone(),
                version: self.version,
                model_json: document.model_json.clone(),
                has_pdf: self.pdf.is_some(),
                has_docx: self.docx.is_some(),
                verification_json: self.verification_json.clone(),
                verified_at: self.verified_at.clone(),
                language_fallback_from: document.language_fallback_from.clone(),
                review_json: self.review_json.clone(),
                review_untrusted_origin: self.review_json.is_some(),
                untrusted_origin: document.untrusted_origin,
                created_at: document.created_at.clone(),
                updated_at: self.updated_at.clone(),
            }
        }
    }

    #[derive(Default)]
    pub(crate) struct FakeDocumentsRepo {
        rows: Mutex<Vec<Row>>,
        /// Every critique `set_review` was handed, with the application it judged.
        reviews: Mutex<Vec<(String, String)>>,
        /// When set, every write fails the way a full disk would.
        pub(crate) fail: bool,
    }

    impl FakeDocumentsRepo {
        /// The documents written so far, in order.
        pub(crate) fn rows(&self) -> Vec<NewDocument> {
            self.rows
                .lock()
                .unwrap()
                .iter()
                .map(|row| row.document.clone())
                .collect()
        }

        /// The critiques written so far, as `(application_id, json)`.
        pub(crate) fn reviews(&self) -> Vec<(String, String)> {
            self.reviews.lock().unwrap().clone()
        }

        /// The stored PDF of one document, as a caller would read it back.
        pub(crate) fn stored_pdf(&self, id: &str) -> Option<Vec<u8>> {
            self.rows
                .lock()
                .unwrap()
                .iter()
                .find(|row| row.document.id == id)
                .and_then(|row| row.pdf.clone())
        }

        /// The verification record one document carries, if any.
        pub(crate) fn verification(&self, id: &str) -> Option<String> {
            self.rows
                .lock()
                .unwrap()
                .iter()
                .find(|row| row.document.id == id)
                .and_then(|row| row.verification_json.clone())
        }

        /// A fake whose writes fail, the way a full disk would.
        pub(crate) fn failing() -> Self {
            Self {
                fail: true,
                ..Self::default()
            }
        }

        fn edit<F: FnOnce(&mut Row)>(&self, id: &str, updated_at: &str, apply: F) {
            if let Some(row) = self
                .rows
                .lock()
                .unwrap()
                .iter_mut()
                .find(|row| row.document.id == id)
            {
                apply(row);
                row.updated_at = updated_at.to_string();
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
                .filter(|row| {
                    row.document.application_id == new.application_id
                        && row.document.kind == new.kind
                })
                .count() as i64
                + 1;
            rows.push(Row {
                document: new.clone(),
                version,
                pdf: None,
                docx: None,
                verification_json: None,
                verified_at: None,
                review_json: None,
                updated_at: new.created_at.clone(),
            });
            Ok(version)
        }

        fn get(&self, id: &str) -> Result<Option<DocumentRecord>, DocumentsRepoError> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .iter()
                .find(|row| row.document.id == id)
                .map(Row::record))
        }

        fn list_for_application(
            &self,
            application_id: &str,
        ) -> Result<Vec<DocumentRecord>, DocumentsRepoError> {
            let rows = self.rows.lock().unwrap();
            let mut records: Vec<DocumentRecord> = rows
                .iter()
                .filter(|row| row.document.application_id == application_id)
                .map(Row::record)
                .collect();
            records.sort_by(|a, b| (a.kind.as_str(), a.version).cmp(&(b.kind.as_str(), b.version)));
            Ok(records)
        }

        fn latest(
            &self,
            application_id: &str,
            kind: DocumentKind,
        ) -> Result<Option<DocumentRecord>, DocumentsRepoError> {
            Ok(self
                .list_for_application(application_id)?
                .into_iter()
                .rev()
                .find(|record| record.kind == kind))
        }

        fn set_pdf(
            &self,
            id: &str,
            pdf: &[u8],
            updated_at: &str,
        ) -> Result<(), DocumentsRepoError> {
            if self.fail {
                return Err(DocumentsRepoError::new("disk gone"));
            }
            self.edit(id, updated_at, |row| row.pdf = Some(pdf.to_vec()));
            Ok(())
        }

        fn set_docx(
            &self,
            id: &str,
            docx: &[u8],
            updated_at: &str,
        ) -> Result<(), DocumentsRepoError> {
            if self.fail {
                return Err(DocumentsRepoError::new("disk gone"));
            }
            self.edit(id, updated_at, |row| row.docx = Some(docx.to_vec()));
            Ok(())
        }

        fn mark_verified(
            &self,
            id: &str,
            verification_json: &str,
            verified_at: &str,
        ) -> Result<(), DocumentsRepoError> {
            if self.fail {
                return Err(DocumentsRepoError::new("disk gone"));
            }
            self.edit(id, verified_at, |row| {
                row.verification_json = Some(verification_json.to_string());
                row.verified_at = Some(verified_at.to_string());
            });
            Ok(())
        }

        fn set_review(
            &self,
            application_id: &str,
            review_json: &str,
            updated_at: &str,
        ) -> Result<u32, DocumentsRepoError> {
            if self.fail {
                return Err(DocumentsRepoError::new("disk gone"));
            }
            // Exactly like the real `UPDATE … WHERE application_id = …`: the
            // newest version of each kind the application has is annotated, and
            // an application with no drafts is a `0`, not an error.
            let mut annotated = 0;
            let mut rows = self.rows.lock().unwrap();
            for kind in [DocumentKind::Cv, DocumentKind::CoverLetter] {
                let newest = rows
                    .iter_mut()
                    .filter(|row| {
                        row.document.application_id == application_id && row.document.kind == kind
                    })
                    .max_by_key(|row| row.version);
                if let Some(row) = newest {
                    row.review_json = Some(review_json.to_string());
                    row.updated_at = updated_at.to_string();
                    annotated += 1;
                }
            }
            if annotated > 0 {
                self.reviews
                    .lock()
                    .unwrap()
                    .push((application_id.to_string(), review_json.to_string()));
            }
            Ok(annotated)
        }

        fn pdf(&self, id: &str) -> Result<Option<Vec<u8>>, DocumentsRepoError> {
            Ok(self.stored_pdf(id))
        }

        fn docx(&self, id: &str) -> Result<Option<Vec<u8>>, DocumentsRepoError> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .iter()
                .find(|row| row.document.id == id)
                .and_then(|row| row.docx.clone()))
        }
    }
}
