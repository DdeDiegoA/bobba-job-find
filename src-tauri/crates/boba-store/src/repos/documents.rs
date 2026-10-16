//! Document repository (S7.1, S7.2): the single writer of the `document` table
//! (AD-16).
//!
//! Rows hold the schema-versioned model JSON plus, once rendered and verified,
//! the PDF/DOCX blobs and the verification report. S7.2 added the
//! [`DocumentsRepo`] trait and conformed this adapter to it, and the
//! `untrusted_origin` column: a draft comes from an agent run, so the store
//! records that its content is untrusted (ADR-010) instead of guessing later.
//! S7.4 adds `review_json`: the fresh-context reviewer's critique of a document,
//! with its own untrusted marker, so the studio can show it after a reload.

use boba_app::ports::repo::documents::{DocumentsRepo, DocumentsRepoError};
use boba_domain::documents::{DocumentKind, DocumentRecord, Lang, NewDocument, TemplateId};
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::db::{Store, StoreError};

/// A stored value that parses to no known variant. The migration's CHECK
/// constraints mean this only happens on a hand-edited database.
#[derive(Debug, thiserror::Error)]
#[error("unknown value {value:?} in document column {column}")]
struct BadValue {
    column: &'static str,
    value: String,
}

fn bad_value(column: &'static str, value: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::new(BadValue { column, value }),
    )
}

const SELECT: &str = "SELECT id, application_id, kind, lang, template, version, model_json, \
     pdf_blob IS NOT NULL, docx_blob IS NOT NULL, verification_json, verified_at, \
     language_fallback_from, review_json, review_untrusted_origin, untrusted_origin, \
     created_at, updated_at \
     FROM document";

fn row(r: &Row<'_>) -> rusqlite::Result<DocumentRecord> {
    let kind: String = r.get(2)?;
    let lang: String = r.get(3)?;
    Ok(DocumentRecord {
        id: r.get(0)?,
        application_id: r.get(1)?,
        kind: DocumentKind::parse(&kind).ok_or_else(|| bad_value("kind", kind))?,
        lang: Lang::parse(&lang).ok_or_else(|| bad_value("lang", lang))?,
        template: TemplateId::new(r.get::<_, String>(4)?),
        version: r.get(5)?,
        model_json: r.get(6)?,
        has_pdf: r.get::<_, i64>(7)? != 0,
        has_docx: r.get::<_, i64>(8)? != 0,
        verification_json: r.get(9)?,
        verified_at: r.get(10)?,
        language_fallback_from: r.get(11)?,
        review_json: r.get(12)?,
        review_untrusted_origin: r.get::<_, i64>(13)? != 0,
        untrusted_origin: r.get::<_, i64>(14)? != 0,
        created_at: r.get(15)?,
        updated_at: r.get(16)?,
    })
}

/// Insert a document, assigning the next `version` for its `(application, kind)`.
pub fn insert(conn: &Connection, new: &NewDocument) -> Result<i64, StoreError> {
    let version = next_version(conn, &new.application_id, new.kind)?;
    conn.execute(
        "INSERT INTO document \
         (id, application_id, kind, lang, template, version, model_json, untrusted_origin, \
          language_fallback_from, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
        params![
            new.id,
            new.application_id,
            new.kind.as_str(),
            new.lang.code(),
            new.template.as_str(),
            version,
            new.model_json,
            i64::from(new.untrusted_origin),
            new.language_fallback_from,
            new.created_at,
        ],
    )?;
    Ok(version)
}

/// Fetch one document row by id.
pub fn get(conn: &Connection, id: &str) -> Result<Option<DocumentRecord>, StoreError> {
    Ok(conn
        .query_row(&format!("{SELECT} WHERE id = ?1"), params![id], row)
        .optional()?)
}

/// Every document of an application, oldest version first.
pub fn list_for_application(
    conn: &Connection,
    application_id: &str,
) -> Result<Vec<DocumentRecord>, StoreError> {
    let mut stmt = conn.prepare(&format!(
        "{SELECT} WHERE application_id = ?1 ORDER BY kind, version"
    ))?;
    let rows = stmt.query_map(params![application_id], row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// The newest version of one kind for an application.
pub fn latest(
    conn: &Connection,
    application_id: &str,
    kind: DocumentKind,
) -> Result<Option<DocumentRecord>, StoreError> {
    Ok(conn
        .query_row(
            &format!(
                "{SELECT} WHERE application_id = ?1 AND kind = ?2 ORDER BY version DESC LIMIT 1"
            ),
            params![application_id, kind.as_str()],
            row,
        )
        .optional()?)
}

/// Store the rendered PDF blob.
pub fn set_pdf(
    conn: &Connection,
    id: &str,
    pdf: &[u8],
    updated_at: &str,
) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE document SET pdf_blob = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, pdf, updated_at],
    )?;
    Ok(())
}

/// Store the rendered DOCX blob.
pub fn set_docx(
    conn: &Connection,
    id: &str,
    docx: &[u8],
    updated_at: &str,
) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE document SET docx_blob = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, docx, updated_at],
    )?;
    Ok(())
}

/// Record a verification pass (AD-14). Only verified documents may be submitted.
pub fn mark_verified(
    conn: &Connection,
    id: &str,
    verification_json: &str,
    verified_at: &str,
) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE document SET verification_json = ?2, verified_at = ?3, updated_at = ?3 WHERE id = ?1",
        params![id, verification_json, verified_at],
    )?;
    Ok(())
}

/// Attach a review run's critique to the drafts it judged: the newest version of
/// each kind for `application_id` (S7.4). Returns how many rows were annotated,
/// so 0 tells the caller there were no drafts to review.
///
/// The critique is agent text and is stored with its own `untrusted_origin`
/// marker (ADR-010); nothing here reads or evaluates the JSON.
pub fn set_review(
    conn: &Connection,
    application_id: &str,
    review_json: &str,
    updated_at: &str,
) -> Result<u32, StoreError> {
    let annotated = conn.execute(
        "UPDATE document SET review_json = ?2, review_untrusted_origin = 1, updated_at = ?3 \
         WHERE application_id = ?1 \
           AND version = (SELECT MAX(version) FROM document AS newest \
                           WHERE newest.application_id = document.application_id \
                             AND newest.kind = document.kind)",
        params![application_id, review_json, updated_at],
    )?;
    Ok(u32::try_from(annotated).unwrap_or(u32::MAX))
}

/// Read the rendered PDF blob.
pub fn pdf(conn: &Connection, id: &str) -> Result<Option<Vec<u8>>, StoreError> {
    blob(conn, id, "pdf_blob")
}

/// Read the rendered DOCX blob.
pub fn docx(conn: &Connection, id: &str) -> Result<Option<Vec<u8>>, StoreError> {
    blob(conn, id, "docx_blob")
}

fn blob(conn: &Connection, id: &str, column: &str) -> Result<Option<Vec<u8>>, StoreError> {
    Ok(conn
        .query_row(
            &format!("SELECT {column} FROM document WHERE id = ?1"),
            params![id],
            |r| r.get::<_, Option<Vec<u8>>>(0),
        )
        .optional()?
        .flatten())
}

/// `MAX()` over no rows is SQL NULL, which maps to `None` here.
fn next_version(
    conn: &Connection,
    application_id: &str,
    kind: DocumentKind,
) -> Result<i64, StoreError> {
    let current: Option<i64> = conn.query_row(
        "SELECT MAX(version) FROM document WHERE application_id = ?1 AND kind = ?2",
        params![application_id, kind.as_str()],
        |r| r.get::<_, Option<i64>>(0),
    )?;
    Ok(current.unwrap_or(0) + 1)
}

/// Map a store failure into the port's stringified error.
fn repo(error: StoreError) -> DocumentsRepoError {
    DocumentsRepoError::new(error.to_string())
}

impl DocumentsRepo for Store {
    fn insert(&self, new: &NewDocument) -> Result<i64, DocumentsRepoError> {
        // Read (`next_version`) and write in one job on the single writer, so two
        // concurrent inserts cannot take the same version (AD-2).
        let new = new.clone();
        self.execute(move |conn| insert(conn, &new)).map_err(repo)
    }

    fn get(&self, id: &str) -> Result<Option<DocumentRecord>, DocumentsRepoError> {
        let id = id.to_owned();
        self.read(move |conn| get(conn, &id)).map_err(repo)
    }

    fn list_for_application(
        &self,
        application_id: &str,
    ) -> Result<Vec<DocumentRecord>, DocumentsRepoError> {
        let application_id = application_id.to_owned();
        self.read(move |conn| list_for_application(conn, &application_id))
            .map_err(repo)
    }

    fn latest(
        &self,
        application_id: &str,
        kind: DocumentKind,
    ) -> Result<Option<DocumentRecord>, DocumentsRepoError> {
        let application_id = application_id.to_owned();
        self.read(move |conn| latest(conn, &application_id, kind))
            .map_err(repo)
    }

    fn set_pdf(&self, id: &str, pdf: &[u8], updated_at: &str) -> Result<(), DocumentsRepoError> {
        let (id, pdf, updated_at) = (id.to_owned(), pdf.to_vec(), updated_at.to_owned());
        self.execute(move |conn| set_pdf(conn, &id, &pdf, &updated_at))
            .map_err(repo)
    }

    fn set_docx(&self, id: &str, docx: &[u8], updated_at: &str) -> Result<(), DocumentsRepoError> {
        let (id, docx, updated_at) = (id.to_owned(), docx.to_vec(), updated_at.to_owned());
        self.execute(move |conn| set_docx(conn, &id, &docx, &updated_at))
            .map_err(repo)
    }

    fn mark_verified(
        &self,
        id: &str,
        verification_json: &str,
        verified_at: &str,
    ) -> Result<(), DocumentsRepoError> {
        let (id, verification_json, verified_at) = (
            id.to_owned(),
            verification_json.to_owned(),
            verified_at.to_owned(),
        );
        self.execute(move |conn| mark_verified(conn, &id, &verification_json, &verified_at))
            .map_err(repo)
    }

    fn set_review(
        &self,
        application_id: &str,
        review_json: &str,
        updated_at: &str,
    ) -> Result<u32, DocumentsRepoError> {
        let (application_id, review_json, updated_at) = (
            application_id.to_owned(),
            review_json.to_owned(),
            updated_at.to_owned(),
        );
        self.execute(move |conn| set_review(conn, &application_id, &review_json, &updated_at))
            .map_err(repo)
    }

    fn pdf(&self, id: &str) -> Result<Option<Vec<u8>>, DocumentsRepoError> {
        let id = id.to_owned();
        self.read(move |conn| pdf(conn, &id)).map_err(repo)
    }

    fn docx(&self, id: &str) -> Result<Option<Vec<u8>>, DocumentsRepoError> {
        let id = id.to_owned();
        self.read(move |conn| docx(conn, &id)).map_err(repo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate;

    fn connection() -> Connection {
        let mut conn = Connection::open_in_memory().expect("in-memory sqlite");
        migrate::embedded()
            .to_latest(&mut conn)
            .expect("apply migrations");
        conn
    }

    fn new(id: &str, kind: DocumentKind, template: &str) -> NewDocument {
        NewDocument {
            id: id.into(),
            application_id: "app_1".into(),
            kind,
            lang: Lang::En,
            template: TemplateId::new(template),
            model_json: "{\"schema_version\":1}".into(),
            language_fallback_from: None,
            untrusted_origin: true,
            created_at: "2026-10-05T00:00:00Z".into(),
        }
    }

    #[test]
    fn migration_creates_the_document_table() {
        let conn = connection();
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name = 'document'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn insert_bumps_version_and_latest_returns_the_newest() {
        let conn = connection();
        assert_eq!(
            insert(&conn, &new("doc_1", DocumentKind::Cv, "cv-classic")).unwrap(),
            1
        );
        assert_eq!(
            insert(&conn, &new("doc_2", DocumentKind::Cv, "cv-modern")).unwrap(),
            2
        );
        assert_eq!(
            insert(
                &conn,
                &new("doc_3", DocumentKind::CoverLetter, "cover-classic")
            )
            .unwrap(),
            1,
            "versions are per (application, kind)"
        );

        let latest = latest(&conn, "app_1", DocumentKind::Cv)
            .unwrap()
            .expect("a cv");
        assert_eq!(latest.id, "doc_2");
        assert_eq!(latest.template.as_str(), "cv-modern");
        assert_eq!(list_for_application(&conn, "app_1").unwrap().len(), 3);
    }

    #[test]
    fn rendered_blobs_and_verification_round_trip() {
        let conn = connection();
        insert(&conn, &new("doc_1", DocumentKind::Cv, "cv-classic")).unwrap();
        assert!(!get(&conn, "doc_1").unwrap().unwrap().has_pdf);

        set_pdf(&conn, "doc_1", b"%PDF-1.7 test", "t1").unwrap();
        set_docx(&conn, "doc_1", b"PK\x03\x04", "t1").unwrap();
        let rec = get(&conn, "doc_1").unwrap().unwrap();
        assert!(rec.has_pdf && rec.has_docx);
        assert_eq!(rec.updated_at, "t1");
        assert_eq!(pdf(&conn, "doc_1").unwrap().unwrap(), b"%PDF-1.7 test");
        assert_eq!(docx(&conn, "doc_1").unwrap().unwrap(), b"PK\x03\x04");

        assert!(rec.verified_at.is_none());
        mark_verified(&conn, "doc_1", "{\"ok\":true}", "t2").unwrap();
        let rec = get(&conn, "doc_1").unwrap().unwrap();
        assert_eq!(rec.verified_at.as_deref(), Some("t2"));
        assert_eq!(rec.verification_json.as_deref(), Some("{\"ok\":true}"));
    }

    #[test]
    fn kind_and_lang_are_constrained() {
        let conn = connection();
        assert!(get(&conn, "missing").unwrap().is_none());
        assert!(pdf(&conn, "missing").unwrap().is_none());

        // The model cannot express an unknown kind or language, so the CHECK is
        // what keeps a hand-edited row out; prove it still rejects one.
        let inserted = conn.execute(
            "INSERT INTO document (id, application_id, kind, lang, template, version, \
             model_json, created_at, updated_at) \
             VALUES ('doc_x', 'app_1', 'resume', 'fr', 'cv-classic', 1, '{}', 't', 't')",
            [],
        );
        assert!(inserted.is_err(), "kind/lang CHECK must reject");
    }

    // AC7 (S7.2): a draft comes from an agent run, so it is stored untrusted, and
    // a row written by a database migrated from 0003 reads back untrusted too.
    #[test]
    fn untrusted_origin_defaults_to_one_and_travels_with_the_row() {
        let conn = connection();

        let trusted = NewDocument {
            untrusted_origin: false,
            ..new("doc_1", DocumentKind::Cv, "cv-classic")
        };
        insert(&conn, &trusted).unwrap();
        assert!(!get(&conn, "doc_1").unwrap().unwrap().untrusted_origin);

        insert(
            &conn,
            &new("doc_2", DocumentKind::CoverLetter, "cover-classic"),
        )
        .unwrap();
        assert!(get(&conn, "doc_2").unwrap().unwrap().untrusted_origin);
        assert_eq!(
            get(&conn, "doc_2").unwrap().unwrap().language_fallback_from,
            None,
            "a draft in the Offer's own language has nothing to explain"
        );

        let default: String = conn
            .query_row(
                "SELECT dflt_value FROM pragma_table_info('document') WHERE name = 'untrusted_origin'",
                [],
                |r| r.get(0),
            )
            .expect("the column exists");
        assert_eq!(default, "1", "the migration's default is untrusted");
    }

    // S7.4 (FR-22/FR-72): the reviewer's critique is stored on the drafts it
    // judged — the newest version of each kind — and read back with them.
    #[test]
    fn a_critique_is_stored_on_the_newest_version_of_each_kind() {
        let conn = connection();
        insert(&conn, &new("doc_1", DocumentKind::Cv, "cv-classic")).unwrap();
        insert(
            &conn,
            &new("doc_2", DocumentKind::CoverLetter, "cover-classic"),
        )
        .unwrap();
        insert(&conn, &new("doc_3", DocumentKind::Cv, "cv-modern")).unwrap();

        assert_eq!(
            set_review(&conn, "app_1", "[{\"severity\":\"high\"}]", "t1").unwrap(),
            2,
            "the newest CV and the cover letter"
        );
        let cv = get(&conn, "doc_3").unwrap().expect("the newest cv");
        assert_eq!(cv.review_json.as_deref(), Some("[{\"severity\":\"high\"}]"));
        assert!(cv.review_untrusted_origin, "the critique is agent text");
        assert_eq!(cv.updated_at, "t1");
        assert_eq!(
            get(&conn, "doc_1")
                .unwrap()
                .expect("the old cv")
                .review_json,
            None,
            "the superseded version keeps no critique"
        );
        assert_eq!(
            get(&conn, "doc_2")
                .unwrap()
                .expect("the letter")
                .review_json
                .as_deref(),
            Some("[{\"severity\":\"high\"}]")
        );
        assert_eq!(
            latest(&conn, "app_1", DocumentKind::Cv)
                .unwrap()
                .expect("the newest cv")
                .review_json
                .as_deref(),
            Some("[{\"severity\":\"high\"}]"),
            "the studio reads it through the same queries as the draft"
        );
    }

    /// An application with no drafts has nothing to annotate: the critique is
    /// refused by the caller, and no row is touched.
    #[test]
    fn a_critique_of_an_application_with_no_drafts_touches_nothing() {
        let conn = connection();
        insert(&conn, &new("doc_1", DocumentKind::Cv, "cv-classic")).unwrap();
        assert_eq!(set_review(&conn, "app_none", "[]", "t1").unwrap(), 0);
        assert_eq!(
            get(&conn, "doc_1").unwrap().expect("the row").review_json,
            None
        );
        assert_eq!(
            get(&conn, "doc_1").unwrap().expect("the row").updated_at,
            "2026-10-05T00:00:00Z",
            "an untouched row keeps its timestamp"
        );
    }

    /// ADR-010: a critique carries its own untrusted marker from the start, so
    /// the studio renders it as data whether or not the row it sits on is
    /// flagged.
    #[test]
    fn the_critique_column_is_untrusted_by_default() {
        let conn = connection();
        let (default, notnull): (Option<String>, i64) = conn
            .query_row(
                "SELECT dflt_value, \"notnull\" FROM pragma_table_info('document') \
                 WHERE name = 'review_untrusted_origin'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("the column exists");
        assert_eq!(
            default.as_deref(),
            Some("1"),
            "the migration's default is untrusted"
        );
        assert_eq!(notnull, 1);

        // A critique written without a marker takes that default.
        insert(&conn, &new("doc_1", DocumentKind::Cv, "cv-classic")).unwrap();
        set_review(&conn, "app_1", "[]", "t1").unwrap();
        assert!(
            get(&conn, "doc_1")
                .unwrap()
                .expect("the row")
                .review_untrusted_origin
        );

        // The CHECK keeps it a flag, not a value.
        let out_of_range = conn.execute(
            "UPDATE document SET review_untrusted_origin = 2 WHERE id = 'doc_1'",
            [],
        );
        assert!(out_of_range.is_err(), "CHECK rejects anything but 0 or 1");

        // And a user's own words are the only thing that writes 0.
        conn.execute(
            "UPDATE document SET review_untrusted_origin = 0 WHERE id = 'doc_1'",
            [],
        )
        .expect("0 is a writable value");
        assert!(
            !get(&conn, "doc_1")
                .unwrap()
                .expect("the row")
                .review_untrusted_origin
        );
    }

    // FR-70: the reason a draft is not in the Offer's language is stored with the
    // document, so the studio can explain itself after a reload.
    #[test]
    fn a_language_fallback_is_persisted_with_its_document() {
        let conn = connection();
        let fallback = NewDocument {
            language_fallback_from: Some("de".into()),
            ..new("doc_1", DocumentKind::Cv, "cv-classic")
        };
        insert(&conn, &fallback).unwrap();

        let record = get(&conn, "doc_1").unwrap().expect("the row");
        assert_eq!(record.language_fallback_from.as_deref(), Some("de"));
        assert_eq!(record.lang, Lang::En, "the fallback language is EN");
        assert_eq!(
            latest(&conn, "app_1", DocumentKind::Cv)
                .unwrap()
                .expect("the row")
                .language_fallback_from
                .as_deref(),
            Some("de")
        );
    }
}
