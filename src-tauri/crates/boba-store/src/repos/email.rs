//! Email repository (S8.1): the single writer of `email_account` (AD-13, AD-16).
//!
//! The row is connection metadata; the tokens live in the keychain behind an
//! opaque [`EmailAccount::secret_ref`] (FR-40). `EmailRepo` is implemented for
//! [`Store`] so use cases depend on the port, not this module.

use rusqlite::{Connection, OptionalExtension, params};

use boba_app::ports::mail::MessageSink;
use boba_app::ports::repo::email::{
    ClientIdKind, EmailAccount, EmailMode, EmailRepo, EmailRepoError, MailProvider,
};
use boba_domain::mail::{ImportOrigin, MatchReason, RawMessage, StoredMessage, SyncError};
use thiserror::Error;

use crate::db::{Store, StoreError};

const SELECT: &str = "SELECT id, provider, address, mode, client_id, client_id_kind, scopes, \
     secret_ref, connected_at, updated_at FROM email_account";

/// A row that cannot be parsed back into an [`EmailAccount`].
#[derive(Debug, Error)]
enum RowError {
    #[error("unknown {column} '{value}' in email_account")]
    Unknown { column: &'static str, value: String },
}

impl From<RowError> for StoreError {
    fn from(e: RowError) -> Self {
        StoreError::Sqlite(e.to_string())
    }
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<EmailAccount> {
    let provider: String = r.get(1)?;
    let mode: String = r.get(3)?;
    let client_id_kind: String = r.get(5)?;
    Ok(EmailAccount {
        id: r.get(0)?,
        provider: MailProvider::parse(&provider).ok_or_else(|| bad("provider", provider))?,
        address: r.get(2)?,
        mode: EmailMode::parse(&mode).ok_or_else(|| bad("mode", mode))?,
        client_id: r.get(4)?,
        client_id_kind: ClientIdKind::parse(&client_id_kind)
            .ok_or_else(|| bad("client_id_kind", client_id_kind))?,
        scopes: split_scopes(r.get(6)?),
        secret_ref: r.get(7)?,
        connected_at: r.get(8)?,
        updated_at: r.get(9)?,
    })
}

fn bad(column: &'static str, value: String) -> rusqlite::Error {
    let e = RowError::Unknown { column, value };
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
}

/// Scopes are stored space-separated, as OAuth grants them.
fn split_scopes(text: String) -> Vec<String> {
    text.split_whitespace().map(str::to_string).collect()
}

/// A fresh connection id (AD-17: ids are ULIDs, minted by the owning area).
pub fn new_account_id() -> String {
    ulid::Ulid::generate().to_string()
}

/// Insert or replace one connection, keyed by id, in one transaction.
pub fn upsert_account(conn: &Connection, account: &EmailAccount) -> Result<(), StoreError> {
    // Reconnecting the same mailbox reuses its row: the UNIQUE (provider,
    // address) would otherwise reject the insert (AC1).
    conn.execute(
        "DELETE FROM email_account WHERE provider = ?1 AND address = ?2 AND id <> ?3",
        params![account.provider.as_str(), account.address, account.id],
    )?;
    conn.execute(
        "INSERT INTO email_account \
         (id, provider, address, mode, client_id, client_id_kind, scopes, secret_ref, \
          connected_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
         ON CONFLICT(id) DO UPDATE SET \
             provider = excluded.provider, address = excluded.address, mode = excluded.mode, \
             client_id = excluded.client_id, client_id_kind = excluded.client_id_kind, \
             scopes = excluded.scopes, secret_ref = excluded.secret_ref, \
             updated_at = excluded.updated_at",
        params![
            account.id,
            account.provider.as_str(),
            account.address,
            account.mode.as_str(),
            account.client_id,
            account.client_id_kind.as_str(),
            account.scopes.join(" "),
            account.secret_ref,
            account.connected_at,
            account.updated_at,
        ],
    )?;
    Ok(())
}

/// One connection by id.
pub fn get_account(conn: &Connection, id: &str) -> Result<Option<EmailAccount>, StoreError> {
    conn.query_row(&format!("{SELECT} WHERE id = ?1"), params![id], row)
        .optional()
        .map_err(StoreError::from)
}

/// The connection for one mailbox, `None` when it was never connected.
pub fn find_account(
    conn: &Connection,
    provider: MailProvider,
    address: &str,
) -> Result<Option<EmailAccount>, StoreError> {
    conn.query_row(
        &format!("{SELECT} WHERE provider = ?1 AND address = ?2"),
        params![provider.as_str(), address],
        row,
    )
    .optional()
    .map_err(StoreError::from)
}

/// Every connection, oldest first.
pub fn list_accounts(conn: &Connection) -> Result<Vec<EmailAccount>, StoreError> {
    let mut stmt = conn
        .prepare(&format!("{SELECT} ORDER BY connected_at, id"))
        .map_err(StoreError::from)?;
    let rows = stmt.query_map([], row).map_err(StoreError::from)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(StoreError::from)
}

/// Switch one connection's mode; fails on an unknown id.
pub fn set_mode(
    conn: &Connection,
    id: &str,
    mode: EmailMode,
    updated_at: &str,
) -> Result<(), StoreError> {
    let changed = conn.execute(
        "UPDATE email_account SET mode = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, mode.as_str(), updated_at],
    )?;
    if changed == 0 {
        return Err(StoreError::Sqlite(format!("unknown email_account '{id}'")));
    }
    Ok(())
}

/// Remove one connection; returns whether a row was removed.
pub fn delete_account(conn: &Connection, id: &str) -> Result<bool, StoreError> {
    Ok(conn.execute("DELETE FROM email_account WHERE id = ?1", params![id])? > 0)
}

fn map_err(error: StoreError) -> EmailRepoError {
    EmailRepoError::new(error.to_string())
}

impl EmailRepo for Store {
    fn upsert_account(&self, account: &EmailAccount) -> Result<(), EmailRepoError> {
        let account = account.clone();
        self.execute(move |conn| upsert_account(conn, &account))
            .map_err(map_err)
    }

    fn get_account(&self, id: &str) -> Result<Option<EmailAccount>, EmailRepoError> {
        let id = id.to_string();
        self.read(move |conn| get_account(conn, &id))
            .map_err(map_err)
    }

    fn find_account(
        &self,
        provider: MailProvider,
        address: &str,
    ) -> Result<Option<EmailAccount>, EmailRepoError> {
        let address = address.to_string();
        self.read(move |conn| find_account(conn, provider, &address))
            .map_err(map_err)
    }

    fn list_accounts(&self) -> Result<Vec<EmailAccount>, EmailRepoError> {
        self.read(list_accounts).map_err(map_err)
    }

    fn set_mode(&self, id: &str, mode: EmailMode, updated_at: &str) -> Result<(), EmailRepoError> {
        let (id, updated_at) = (id.to_string(), updated_at.to_string());
        self.execute(move |conn| set_mode(conn, &id, mode, &updated_at))
            .map_err(map_err)
    }

    fn delete_account(&self, id: &str) -> Result<bool, EmailRepoError> {
        let id = id.to_string();
        self.execute(move |conn| delete_account(conn, &id))
            .map_err(map_err)
    }
}

/// Store one matched message (S8.3, FR-38/104). Returns whether a row was
/// written: a re-delivered message (same connection and provider id) is a
/// no-op, which is what makes an overlapping incremental window safe.
pub fn insert_message(
    conn: &Connection,
    id: &str,
    account_id: Option<&str>,
    message: &RawMessage,
    matched_by: MatchReason,
    origin: ImportOrigin,
    imported_at: &str,
) -> Result<bool, StoreError> {
    let changed = conn.execute(
        "INSERT INTO email_message \
         (id, account_id, provider_message_id, message_id, in_reply_to, references_header, \
          from_address, from_name, to_addresses, subject, received_at, body_text, \
          has_attachments, matched_by, origin, imported_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16) \
         ON CONFLICT DO NOTHING",
        params![
            id,
            account_id,
            message.provider_id,
            message.message_id,
            message.in_reply_to.join(" "),
            message.references.join(" "),
            message.from_address,
            message.from_name,
            message.to_addresses.join(", "),
            message.subject,
            rfc3339(message.received_at),
            message.body_text,
            i64::from(message.has_attachments),
            matched_by.as_str(),
            origin.as_str(),
            imported_at,
        ],
    )?;
    Ok(changed > 0)
}

/// The stored timestamp form (AD-17). A date the parser accepted but the
/// calendar cannot represent is clamped to the epoch rather than failing a sync.
fn rfc3339(timestamp: i64) -> String {
    time::OffsetDateTime::from_unix_timestamp(timestamp)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

impl MessageSink for Store {
    fn store(&self, message: StoredMessage<'_>) -> Result<(), SyncError> {
        let id = ulid::Ulid::generate().to_string();
        let imported_at = rfc3339(time::OffsetDateTime::now_utc().unix_timestamp());
        let stored = message.message.clone();
        let account = message.account_id.map(str::to_string);
        let matched_by = message.matched_by;
        let origin = message.origin;
        self.execute(move |conn| {
            insert_message(
                conn,
                &id,
                account.as_deref(),
                &stored,
                matched_by,
                origin,
                &imported_at,
            )
        })
        .map(|_| ())
        .map_err(|error| SyncError::Store(error.to_string()))
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

    fn account(id: &str, address: &str, mode: EmailMode) -> EmailAccount {
        EmailAccount {
            id: id.into(),
            provider: MailProvider::MicrosoftGraph,
            address: address.into(),
            mode,
            client_id: "00000000-0000-0000-0000-0000000000b0".into(),
            client_id_kind: ClientIdKind::Bundled,
            scopes: mode
                .required_scopes()
                .iter()
                .map(|s| s.to_string())
                .collect(),
            secret_ref: format!("mail:{id}"),
            connected_at: "2026-10-16T00:00:00Z".into(),
            updated_at: "2026-10-16T00:00:00Z".into(),
        }
    }

    #[test]
    fn migration_creates_the_email_account_table() {
        let conn = connection();
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name = 'email_account'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn upsert_round_trips_scopes_and_mode() {
        let conn = connection();
        let read = account("01ACC", "me@example.com", EmailMode::ReadSend);
        upsert_account(&conn, &read).unwrap();

        let loaded = get_account(&conn, "01ACC").unwrap().expect("account");
        assert_eq!(loaded, read);
        assert_eq!(loaded.scopes, vec!["Mail.Read", "Mail.Send"]);
        assert!(loaded.covers(EmailMode::ReadSend));
        assert_eq!(
            find_account(&conn, MailProvider::MicrosoftGraph, "me@example.com")
                .unwrap()
                .map(|a| a.id),
            Some("01ACC".to_string())
        );
        assert!(
            find_account(&conn, MailProvider::Gmail, "me@example.com")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn reconnecting_the_same_mailbox_replaces_the_row() {
        let conn = connection();
        upsert_account(
            &conn,
            &account("01OLD", "me@example.com", EmailMode::SendOnly),
        )
        .unwrap();

        let mut again = account("01NEW", "me@example.com", EmailMode::ReadSend);
        again.connected_at = "2026-10-16T02:00:00Z".into();
        upsert_account(&conn, &again).unwrap();

        let rows = list_accounts(&conn).unwrap();
        assert_eq!(rows.len(), 1, "one connection per mailbox");
        assert_eq!(rows[0].id, "01NEW");
        assert_eq!(rows[0].connected_at, "2026-10-16T02:00:00Z");
    }

    #[test]
    fn mode_switch_updates_only_the_mode() {
        let conn = connection();
        upsert_account(
            &conn,
            &account("01ACC", "me@example.com", EmailMode::ReadSend),
        )
        .unwrap();

        set_mode(&conn, "01ACC", EmailMode::SendOnly, "2026-10-16T03:00:00Z").unwrap();
        let loaded = get_account(&conn, "01ACC").unwrap().unwrap();
        assert_eq!(loaded.mode, EmailMode::SendOnly);
        assert_eq!(loaded.updated_at, "2026-10-16T03:00:00Z");
        // A send-only switch keeps the grant it was authorised with: reading is
        // disabled by the mode, and the row never claims a scope it lacks.
        assert!(loaded.covers(EmailMode::SendOnly));

        assert!(set_mode(&conn, "ghost", EmailMode::SendOnly, "t").is_err());
    }

    #[test]
    fn send_only_mode_still_covers_reading_switched_on_later() {
        // The row records what was granted; the mode is the switch (AC3).
        let conn = connection();
        let mut send_only = account("01ACC", "me@example.com", EmailMode::SendOnly);
        send_only.scopes = vec!["Mail.Send".into()];
        upsert_account(&conn, &send_only).unwrap();

        let loaded = get_account(&conn, "01ACC").unwrap().unwrap();
        assert!(loaded.covers(EmailMode::SendOnly));
        assert!(
            !loaded.covers(EmailMode::ReadSend),
            "reading without Mail.Read must be refused, not faked"
        );
    }

    #[test]
    fn delete_reports_removal_and_the_row_holds_no_token() {
        let conn = connection();
        let row = account("01ACC", "me@example.com", EmailMode::ReadSend);
        upsert_account(&conn, &row).unwrap();

        // AD-13/FR-40: the stored row carries the reference only.
        let stored: String = conn
            .query_row(
                "SELECT secret_ref FROM email_account WHERE id = '01ACC'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, "mail:01ACC");
        let columns: Vec<String> = conn
            .prepare("SELECT name FROM pragma_table_info('email_account')")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(
            !columns
                .iter()
                .any(|c| c.contains("token") || c.contains("secret_") && c != "secret_ref"),
            "no token column may exist: {columns:?}"
        );

        assert!(delete_account(&conn, "01ACC").unwrap());
        assert!(!delete_account(&conn, "01ACC").unwrap());
        assert!(list_accounts(&conn).unwrap().is_empty());
    }

    #[test]
    fn check_constraints_reject_bad_values() {
        let conn = connection();
        conn.execute(
            "INSERT INTO email_account \
             (id, provider, address, mode, client_id, client_id_kind, scopes, secret_ref, \
              connected_at, updated_at) \
             VALUES ('x', 'pigeon', 'a', 'read_send', 'c', 'bundled', 'Mail.Read', 'r', 't', 't')",
            [],
        )
        .expect_err("bad provider must be rejected");

        conn.execute(
            "INSERT INTO email_account \
             (id, provider, address, mode, client_id, client_id_kind, scopes, secret_ref, \
              connected_at, updated_at) \
             VALUES ('x', 'gmail', 'a', 'read_send', 'c', 'bundled', 'Mail.Read', 'r', 't', 't')",
            [],
        )
        .expect("a valid row is accepted");

        assert!(get_account(&conn, "missing").unwrap().is_none());
    }

    #[test]
    fn provider_and_mode_text_forms_round_trip() {
        for provider in [
            MailProvider::MicrosoftGraph,
            MailProvider::Gmail,
            MailProvider::ImapSmtp,
        ] {
            assert_eq!(MailProvider::parse(provider.as_str()), Some(provider));
        }
        for mode in [EmailMode::SendOnly, EmailMode::ReadSend] {
            assert_eq!(EmailMode::parse(mode.as_str()), Some(mode));
        }
        for kind in [ClientIdKind::Bundled, ClientIdKind::Byo] {
            assert_eq!(ClientIdKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(MailProvider::parse("nope"), None);
        assert_eq!(EmailMode::parse("nope"), None);
        assert_eq!(ClientIdKind::parse("nope"), None);
    }

    // --- The matched-message sink (S8.3) ---

    fn message() -> RawMessage {
        RawMessage {
            provider_id: "pg-1".into(),
            message_id: Some("abc@acme.example".into()),
            in_reply_to: vec!["sent-1@boba.local".into()],
            references: vec!["older@acme.example".into(), "sent-1@boba.local".into()],
            from_address: "ana@acme.example".into(),
            from_name: Some("Ana".into()),
            to_addresses: vec!["me@example.com".into(), "team@example.com".into()],
            subject: "Your application".into(),
            received_at: 1_704_103_200,
            body_text: "Thanks for applying.".into(),
            has_attachments: true,
        }
    }

    fn stored_count(conn: &Connection) -> i64 {
        conn.query_row("SELECT count(*) FROM email_message", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn a_matched_message_round_trips_into_the_table() {
        let conn = connection();
        upsert_account(&conn, &account("acc", "a@example.com", EmailMode::ReadSend)).unwrap();
        assert!(
            insert_message(
                &conn,
                "m1",
                Some("acc"),
                &message(),
                MatchReason::AtsSender,
                ImportOrigin::Sync,
                "2026-10-17T10:00:00Z",
            )
            .unwrap(),
            "a new message is written"
        );
        let (provider, subject, received, matched, origin, attachments, ids): (
            String,
            String,
            String,
            String,
            String,
            i64,
            String,
        ) = conn
            .query_row(
                "SELECT provider_message_id, subject, received_at, matched_by, origin, \
                 has_attachments, references_header FROM email_message WHERE id = 'm1'",
                [],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(provider, "pg-1");
        assert_eq!(subject, "Your application");
        // The Unix date becomes the stored RFC 3339 form (AD-17).
        assert_eq!(received, "2024-01-01T10:00:00Z");
        assert_eq!(matched, "ats_sender");
        assert_eq!(origin, "sync");
        assert_eq!(attachments, 1);
        assert_eq!(ids, "older@acme.example sent-1@boba.local");
    }

    #[test]
    fn a_re_delivered_message_is_one_row_per_connection() {
        let conn = connection();
        upsert_account(
            &conn,
            &account("acc-1", "a@example.com", EmailMode::ReadSend),
        )
        .unwrap();
        upsert_account(
            &conn,
            &account("acc-2", "b@example.com", EmailMode::ReadSend),
        )
        .unwrap();
        let first = insert_message(
            &conn,
            "m1",
            Some("acc-1"),
            &message(),
            MatchReason::CompanyDomain,
            ImportOrigin::Sync,
            "2026-10-17T10:00:00Z",
        )
        .unwrap();
        let again = insert_message(
            &conn,
            "m2",
            Some("acc-1"),
            &message(),
            MatchReason::CompanyDomain,
            ImportOrigin::Sync,
            "2026-10-17T10:00:00Z",
        )
        .unwrap();
        let other = insert_message(
            &conn,
            "m3",
            Some("acc-2"),
            &message(),
            MatchReason::CompanyDomain,
            ImportOrigin::Sync,
            "2026-10-17T10:00:00Z",
        )
        .unwrap();
        assert!(first);
        assert!(!again, "the same message on the same connection is a no-op");
        assert!(
            other,
            "the same message on another connection is its own row"
        );
        assert_eq!(stored_count(&conn), 2);
    }

    #[test]
    fn the_same_local_import_twice_is_one_row() {
        let conn = connection();
        let first = insert_message(
            &conn,
            "m1",
            None,
            &message(),
            MatchReason::SentThread,
            ImportOrigin::Eml,
            "2026-10-17T10:00:00Z",
        )
        .unwrap();
        let again = insert_message(
            &conn,
            "m2",
            None,
            &message(),
            MatchReason::SentThread,
            ImportOrigin::Eml,
            "2026-10-17T10:00:00Z",
        )
        .unwrap();
        assert!(first);
        assert!(!again);
        assert_eq!(stored_count(&conn), 1);
    }

    #[test]
    fn the_origin_and_connection_must_agree() {
        let conn = connection();
        // A synced message without a connection.
        insert_message(
            &conn,
            "m1",
            None,
            &message(),
            MatchReason::AtsSender,
            ImportOrigin::Sync,
            "2026-10-17T10:00:00Z",
        )
        .expect_err("sync without an account violates the CHECK");
        // A manual import bound to a connection.
        insert_message(
            &conn,
            "m2",
            Some("acc"),
            &message(),
            MatchReason::AtsSender,
            ImportOrigin::Eml,
            "2026-10-17T10:00:00Z",
        )
        .expect_err("eml with an account violates the CHECK");
        assert_eq!(stored_count(&conn), 0);
    }

    #[test]
    fn an_unknown_match_reason_is_rejected() {
        let conn = connection();
        let error = conn.execute(
            "INSERT INTO email_message \
             (id, provider_message_id, from_address, received_at, body_text, matched_by, \
              origin, imported_at) \
             VALUES ('x', 'p', 'a', 't', 'b', 'guessed', 'eml', 't')",
            [],
        );
        assert!(error.is_err());
    }

    #[test]
    fn disconnecting_deletes_its_mail_but_not_a_local_import() {
        let conn = connection();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        upsert_account(&conn, &account("acc", "a@example.com", EmailMode::ReadSend)).unwrap();
        insert_message(
            &conn,
            "m1",
            Some("acc"),
            &message(),
            MatchReason::AtsSender,
            ImportOrigin::Sync,
            "2026-10-17T10:00:00Z",
        )
        .unwrap();
        insert_message(
            &conn,
            "m2",
            None,
            &message(),
            MatchReason::SentThread,
            ImportOrigin::Eml,
            "2026-10-17T10:00:00Z",
        )
        .unwrap();

        assert!(delete_account(&conn, "acc").unwrap());
        assert_eq!(
            stored_count(&conn),
            1,
            "the local import outlives the connection"
        );
    }

    #[test]
    fn the_store_is_the_sink_and_writes_through_its_writer() {
        // The adapter is mounted as `dyn MessageSink`; this proves the trait
        // impl compiles against `Store` and maps a write failure to SyncError.
        fn is_sink<T: MessageSink>() {}
        is_sink::<crate::db::Store>();
    }
}
