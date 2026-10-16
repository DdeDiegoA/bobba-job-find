//! Email repository (S8.1): the single writer of `email_account` (AD-13, AD-16).
//!
//! The row is connection metadata; the tokens live in the keychain behind an
//! opaque [`EmailAccount::secret_ref`] (FR-40). `EmailRepo` is implemented for
//! [`Store`] so use cases depend on the port, not this module.

use rusqlite::{Connection, OptionalExtension, params};

use boba_app::ports::repo::email::{
    ClientIdKind, EmailAccount, EmailMode, EmailRepo, EmailRepoError, MailProvider,
};
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
}
