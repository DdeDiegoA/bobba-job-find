//! Retention sweep adapter (S10.3, FR-113): the SQL behind
//! [`boba_app::retention`].
//!
//! The store owns the open database and runs with `secure_delete=ON`
//! (ADR-004), so the rows this module deletes are zeroed on disk rather than
//! left readable in free pages.
//!
//! Deletes are guarded so the user's record survives:
//!
//! * an Offer with an Application is never deleted — the Application is the
//!   user's pursuit of it (and the FK is `ON DELETE CASCADE`, so deleting the
//!   posting would take the Application with it);
//! * Applications, `document` (sent Packages) and `audit_event` are not named
//!   by any statement here.

use rusqlite::{Connection, params};

use boba_app::retention::{RetentionRepo, RetentionRepoError};

use crate::db::{Store, StoreError};

/// Delete the Offers last seen before `before` that have no Application. Rows
/// that derive from the Offer (`posting_source`, `posting_lifecycle`,
/// `posting_features`, `score_card`, a dismissal) cascade with it.
pub fn purge_unacted_postings(conn: &Connection, before: &str) -> Result<u64, StoreError> {
    let deleted = conn.execute(
        "DELETE FROM posting
         WHERE last_seen < ?1
           AND NOT EXISTS (
               SELECT 1 FROM application a WHERE a.posting_id = posting.id
           )",
        params![before],
    )?;
    Ok(deleted as u64)
}

/// Delete the raw payloads a source sent before `before` (AD-19 keeps them for
/// auditing and re-ranking, not forever). The posting row stays.
pub fn purge_raw_payloads(conn: &Connection, before: &str) -> Result<u64, StoreError> {
    Ok(conn.execute(
        "DELETE FROM posting_source WHERE fetched_at < ?1",
        params![before],
    )? as u64)
}

/// Delete the stored emails received before `before`; their classifications
/// cascade (FR-113: email expires on its own, longer window).
pub fn purge_email(conn: &Connection, before: &str) -> Result<u64, StoreError> {
    Ok(conn.execute(
        "DELETE FROM email_message WHERE received_at < ?1",
        params![before],
    )? as u64)
}

fn map_err(error: StoreError) -> RetentionRepoError {
    RetentionRepoError::new(error.to_string())
}

impl RetentionRepo for Store {
    fn purge_unacted_postings(&self, before: &str) -> Result<u64, RetentionRepoError> {
        let before = before.to_string();
        self.execute(move |conn| purge_unacted_postings(conn, &before))
            .map_err(map_err)
    }

    fn purge_raw_payloads(&self, before: &str) -> Result<u64, RetentionRepoError> {
        let before = before.to_string();
        self.execute(move |conn| purge_raw_payloads(conn, &before))
            .map_err(map_err)
    }

    fn purge_email(&self, before: &str) -> Result<u64, RetentionRepoError> {
        let before = before.to_string();
        self.execute(move |conn| purge_email(conn, &before))
            .map_err(map_err)
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::{Connection, params};

    use super::*;
    use crate::migrate;

    /// The Offer window's cutoff in these tests (90 days before the sweep).
    const BEFORE: &str = "2026-07-19T12:00:00Z";
    /// The email window's cutoff in these tests (180 days before the sweep).
    const EMAIL_BEFORE: &str = "2026-04-20T12:00:00Z";
    const OLD: &str = "2026-06-01T00:00:00Z";
    const NEW: &str = "2026-10-01T00:00:00Z";

    fn connection() -> Connection {
        let mut conn = Connection::open_in_memory().expect("in-memory sqlite");
        conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA secure_delete=ON;")
            .expect("pragmas");
        migrate::embedded()
            .to_latest(&mut conn)
            .expect("apply migrations");
        conn
    }

    fn source(conn: &Connection) {
        conn.execute(
            "INSERT INTO source (id, name, access_class, legal_tier, apply_capability,
                 min_interval_ms, enabled, added_at, updated_at)
             VALUES ('src_1', 'Board', 'official_api', 'safe', 'none', 1000, 1, ?1, ?1)",
            params![OLD],
        )
        .expect("source");
    }

    fn posting(conn: &Connection, id: &str, last_seen: &str) {
        conn.execute(
            "INSERT INTO posting (id, source_id, external_id, title, company, work_mode,
                 first_seen, last_seen, lang, apply_url)
             VALUES (?1, 'src_1', ?1, 'Backend Dev', 'Acme', 'remote', ?2, ?3, 'en',
                 'https://example.com/apply')",
            params![id, OLD, last_seen],
        )
        .expect("posting");
        conn.execute(
            "INSERT INTO posting_source (posting_id, source_id, external_id, url, raw, fetched_at)
             VALUES (?1, 'src_1', ?1, 'https://example.com/job', '{\"raw\":true}', ?2)",
            params![id, last_seen],
        )
        .expect("posting_source");
        conn.execute(
            "INSERT INTO posting_lifecycle (posting_id, updated_at) VALUES (?1, ?2)",
            params![id, last_seen],
        )
        .expect("posting_lifecycle");
    }

    fn application(conn: &Connection, id: &str, posting_id: &str) {
        conn.execute(
            "INSERT INTO application (id, posting_id, status, offer_snapshot_json, created_at, updated_at)
             VALUES (?1, ?2, 'saved', '{}', ?3, ?3)",
            params![id, posting_id, OLD],
        )
        .expect("application");
    }

    fn document(conn: &Connection, id: &str, application_id: &str) {
        conn.execute(
            "INSERT INTO document (id, application_id, kind, lang, template, model_json, created_at, updated_at)
             VALUES (?1, ?2, 'cv', 'en', 'classic', '{}', ?3, ?3)",
            params![id, application_id, OLD],
        )
        .expect("document");
    }

    fn email_account(conn: &Connection) {
        conn.execute(
            "INSERT INTO email_account (id, provider, address, mode, client_id, client_id_kind,
                 scopes, secret_ref, connected_at, updated_at)
             VALUES ('acc_1', 'imap_smtp', 'me@example.com', 'read_send', 'client', 'byo',
                 'Mail.Read Mail.Send', 'mail:acc_1', ?1, ?1)",
            params![OLD],
        )
        .expect("email_account");
    }

    fn email(conn: &Connection, id: &str, received_at: &str) {
        conn.execute(
            "INSERT INTO email_message (id, account_id, provider_message_id, from_address,
                 received_at, body_text, matched_by, origin, imported_at)
             VALUES (?1, 'acc_1', ?1, 'recruiter@acme.com', ?2, 'Hello', 'company_domain', 'sync', ?2)",
            params![id, received_at],
        )
        .expect("email_message");
    }

    fn count(conn: &Connection, table: &str) -> i64 {
        conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .expect("count")
    }

    #[test]
    fn raw_payloads_fetched_before_the_cutoff_are_deleted() {
        let conn = connection();
        source(&conn);
        posting(&conn, "pst_old", OLD);
        posting(&conn, "pst_new", NEW);

        let deleted = purge_raw_payloads(&conn, BEFORE).unwrap();

        assert_eq!(deleted, 1);
        let mut stmt = conn
            .prepare("SELECT posting_id FROM posting_source ORDER BY posting_id")
            .unwrap();
        let left: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(left, vec!["pst_new".to_string()]);
    }

    #[test]
    fn an_old_offer_nobody_acted_on_is_purged_with_its_raw_copy() {
        let conn = connection();
        source(&conn);
        posting(&conn, "pst_old", OLD);
        posting(&conn, "pst_new", NEW);

        let deleted = purge_unacted_postings(&conn, BEFORE).unwrap();

        assert_eq!(deleted, 1);
        assert_eq!(count(&conn, "posting"), 1);
        assert_eq!(count(&conn, "posting_source"), 1, "the raw copy cascades");
        assert_eq!(
            count(&conn, "posting_lifecycle"),
            1,
            "its lifecycle cascades"
        );
    }

    #[test]
    fn an_offer_with_an_application_is_kept() {
        let conn = connection();
        source(&conn);
        posting(&conn, "pst_acted", OLD);
        application(&conn, "app_1", "pst_acted");

        let deleted = purge_unacted_postings(&conn, BEFORE).unwrap();

        assert_eq!(deleted, 0);
        assert_eq!(count(&conn, "posting"), 1, "an Application keeps its Offer");
        assert_eq!(count(&conn, "application"), 1);

        // Its raw copy still expires; the Application's frozen snapshot is what
        // the card shows after that.
        assert_eq!(purge_raw_payloads(&conn, BEFORE).unwrap(), 1);
        assert_eq!(count(&conn, "posting"), 1);
        assert_eq!(count(&conn, "application"), 1);
    }

    #[test]
    fn email_received_before_the_cutoff_is_deleted() {
        let conn = connection();
        email_account(&conn);
        email(&conn, "msg_old", "2026-04-01T00:00:00Z");
        email(&conn, "msg_new", "2026-10-01T00:00:00Z");

        let deleted = purge_email(&conn, EMAIL_BEFORE).unwrap();

        assert_eq!(deleted, 1);
        assert_eq!(count(&conn, "email_message"), 1);
    }

    #[test]
    fn applications_sent_packages_and_the_audit_chain_survive_every_purge() {
        let conn = connection();
        source(&conn);
        posting(&conn, "pst_acted", OLD);
        application(&conn, "app_1", "pst_acted");
        document(&conn, "doc_1", "app_1");
        email_account(&conn);
        email(&conn, "msg_old", "2026-04-01T00:00:00Z");
        conn.execute(
            "INSERT INTO audit_event (id, created_at, phase, action, correlation_id, actor,
                 target_kind, prev_hash, hash)
             VALUES ('aud_1', ?1, 'intent', 'send', 'corr_1', 'core', 'package', 'GENESIS', 'h1')",
            params![OLD],
        )
        .expect("audit_event");

        purge_unacted_postings(&conn, BEFORE).unwrap();
        purge_raw_payloads(&conn, BEFORE).unwrap();
        purge_email(&conn, EMAIL_BEFORE).unwrap();

        assert_eq!(count(&conn, "application"), 1);
        assert_eq!(count(&conn, "document"), 1, "a sent Package is kept");
        assert_eq!(
            count(&conn, "audit_event"),
            1,
            "the audit chain is never swept"
        );
    }

    #[test]
    fn a_second_sweep_deletes_nothing() {
        let conn = connection();
        source(&conn);
        posting(&conn, "pst_old", OLD);
        email_account(&conn);
        email(&conn, "msg_old", "2026-04-01T00:00:00Z");
        purge_unacted_postings(&conn, BEFORE).unwrap();
        purge_raw_payloads(&conn, BEFORE).unwrap();
        purge_email(&conn, EMAIL_BEFORE).unwrap();

        assert_eq!(purge_unacted_postings(&conn, BEFORE).unwrap(), 0);
        assert_eq!(purge_raw_payloads(&conn, BEFORE).unwrap(), 0);
        assert_eq!(purge_email(&conn, EMAIL_BEFORE).unwrap(), 0);
    }
}
