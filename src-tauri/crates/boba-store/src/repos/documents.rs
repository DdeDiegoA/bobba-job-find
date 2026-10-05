//! Document repository (S7.1): the single writer of the `document` table (AD-16).
//!
//! Rows hold the schema-versioned model JSON plus, once rendered and verified,
//! the PDF/DOCX blobs and the verification report. The repo stays typed as
//! plain strings so `boba-store` does not depend on `boba-docs` (AD-1): the
//! caller owns the model types, the store owns the bytes.

use rusqlite::{Connection, OptionalExtension, params};

use crate::db::StoreError;

/// Values needed to insert a new document row. `version` is derived on insert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDocument {
    pub id: String,
    pub application_id: String,
    /// `cv` or `cover_letter` (see `boba_docs::model::DocumentKind`).
    pub kind: String,
    /// `en` or `es`.
    pub lang: String,
    pub template: String,
    pub model_json: String,
    pub created_at: String,
}

/// A document row without its (potentially large) blobs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentRecord {
    pub id: String,
    pub application_id: String,
    pub kind: String,
    pub lang: String,
    pub template: String,
    pub version: i64,
    pub model_json: String,
    pub has_pdf: bool,
    pub has_docx: bool,
    pub verification_json: Option<String>,
    pub verified_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

const SELECT: &str = "SELECT id, application_id, kind, lang, template, version, model_json, \
     pdf_blob IS NOT NULL, docx_blob IS NOT NULL, verification_json, verified_at, created_at, updated_at \
     FROM document";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<DocumentRecord> {
    Ok(DocumentRecord {
        id: r.get(0)?,
        application_id: r.get(1)?,
        kind: r.get(2)?,
        lang: r.get(3)?,
        template: r.get(4)?,
        version: r.get(5)?,
        model_json: r.get(6)?,
        has_pdf: r.get::<_, i64>(7)? != 0,
        has_docx: r.get::<_, i64>(8)? != 0,
        verification_json: r.get(9)?,
        verified_at: r.get(10)?,
        created_at: r.get(11)?,
        updated_at: r.get(12)?,
    })
}

/// Insert a document, assigning the next `version` for its `(application, kind)`.
pub fn insert(conn: &Connection, new: &NewDocument) -> Result<i64, StoreError> {
    let version = next_version(conn, &new.application_id, &new.kind)?;
    conn.execute(
        "INSERT INTO document \
         (id, application_id, kind, lang, template, version, model_json, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
        params![
            new.id,
            new.application_id,
            new.kind,
            new.lang,
            new.template,
            version,
            new.model_json,
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
    kind: &str,
) -> Result<Option<DocumentRecord>, StoreError> {
    Ok(conn
        .query_row(
            &format!(
                "{SELECT} WHERE application_id = ?1 AND kind = ?2 ORDER BY version DESC LIMIT 1"
            ),
            params![application_id, kind],
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

fn next_version(conn: &Connection, application_id: &str, kind: &str) -> Result<i64, StoreError> {
    // `MAX()` over no rows is SQL NULL, which maps to `None` here.
    let current: Option<i64> = conn.query_row(
        "SELECT MAX(version) FROM document WHERE application_id = ?1 AND kind = ?2",
        params![application_id, kind],
        |r| r.get::<_, Option<i64>>(0),
    )?;
    Ok(current.unwrap_or(0) + 1)
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

    fn new(id: &str, kind: &str, template: &str) -> NewDocument {
        NewDocument {
            id: id.into(),
            application_id: "app_1".into(),
            kind: kind.into(),
            lang: "en".into(),
            template: template.into(),
            model_json: "{\"schema_version\":1}".into(),
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
        assert_eq!(insert(&conn, &new("doc_1", "cv", "cv-classic")).unwrap(), 1);
        assert_eq!(insert(&conn, &new("doc_2", "cv", "cv-modern")).unwrap(), 2);
        assert_eq!(
            insert(&conn, &new("doc_3", "cover_letter", "cover-classic")).unwrap(),
            1,
            "versions are per (application, kind)"
        );

        let latest = latest(&conn, "app_1", "cv").unwrap().expect("a cv");
        assert_eq!(latest.id, "doc_2");
        assert_eq!(latest.template, "cv-modern");
        assert_eq!(list_for_application(&conn, "app_1").unwrap().len(), 3);
    }

    #[test]
    fn rendered_blobs_and_verification_round_trip() {
        let conn = connection();
        insert(&conn, &new("doc_1", "cv", "cv-classic")).unwrap();
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
        let mut bad = new("doc_1", "resume", "cv-classic");
        bad.lang = "fr".into();
        assert!(insert(&conn, &bad).is_err(), "kind/lang CHECK must reject");
        assert!(get(&conn, "missing").unwrap().is_none());
        assert!(pdf(&conn, "missing").unwrap().is_none());
    }
}
