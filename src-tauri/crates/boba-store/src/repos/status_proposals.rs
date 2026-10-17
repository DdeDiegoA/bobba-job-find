//! Status-proposal repository (S8.4): the single writer of
//! `email_classification` and `status_proposal` (AD-16).
//!
//! The rows are the area's; the messages they read belong to the email area and
//! are only selected here. `StatusProposalRepo` is implemented for [`Store`] so
//! the use case depends on the port, not this module.

use rusqlite::{Connection, OptionalExtension, params};

use boba_app::status_proposals::repo::{
    Classification, StatusProposalRepo, StatusProposalRepoError, StoredMessage,
};
use boba_domain::mail::{ProposalState, ReplyClass, StatusProposal};
use thiserror::Error;

use crate::db::{Store, StoreError};

const MESSAGE_SELECT: &str =
    "SELECT id, from_address, from_name, subject, body_text, received_at FROM email_message";

const PROPOSAL_SELECT: &str = "SELECT id, message_id, application_id, from_status, to_status, \
     class, confidence, quoted_line, untrusted_origin, state, created_at, decided_at \
     FROM status_proposal";

/// A row that cannot be parsed back into its type.
#[derive(Debug, Error)]
enum RowError {
    #[error("unknown {column} '{value}' in {table}")]
    Unknown {
        table: &'static str,
        column: &'static str,
        value: String,
    },
}

impl From<RowError> for StoreError {
    fn from(e: RowError) -> Self {
        StoreError::Sqlite(e.to_string())
    }
}

fn bad(table: &'static str, column: &'static str, value: String) -> rusqlite::Error {
    let e = RowError::Unknown {
        table,
        column,
        value,
    };
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
}

fn message_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<StoredMessage> {
    Ok(StoredMessage {
        id: r.get(0)?,
        from_address: r.get(1)?,
        from_name: r.get(2)?,
        subject: r.get(3)?,
        body_text: r.get(4)?,
        received_at: r.get(5)?,
    })
}

fn proposal_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<StatusProposal> {
    let class: String = r.get(5)?;
    let state: String = r.get(9)?;
    Ok(StatusProposal {
        id: r.get(0)?,
        message_id: r.get(1)?,
        application_id: r.get(2)?,
        from_status: r.get(3)?,
        to_status: r.get(4)?,
        class: ReplyClass::parse(&class).ok_or_else(|| bad("status_proposal", "class", class))?,
        confidence: r.get(6)?,
        quoted_line: r.get(7)?,
        untrusted_origin: r.get::<_, i64>(8)? != 0,
        state: ProposalState::parse(&state)
            .ok_or_else(|| bad("status_proposal", "state", state))?,
        created_at: r.get(10)?,
        decided_at: r.get(11)?,
    })
}

/// Every matched message no classifier has read, oldest first.
pub fn messages_unclassified(conn: &Connection) -> Result<Vec<StoredMessage>, StoreError> {
    let mut stmt = conn
        .prepare(&format!(
            "{MESSAGE_SELECT} m WHERE NOT EXISTS \
             (SELECT 1 FROM email_classification c WHERE c.message_id = m.id) \
             ORDER BY received_at, id"
        ))
        .map_err(StoreError::from)?;
    let rows = stmt.query_map([], message_row).map_err(StoreError::from)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(StoreError::from)
}

/// One stored message by id.
pub fn message_get(conn: &Connection, id: &str) -> Result<Option<StoredMessage>, StoreError> {
    conn.query_row(
        &format!("{MESSAGE_SELECT} WHERE id = ?1"),
        params![id],
        message_row,
    )
    .optional()
    .map_err(StoreError::from)
}

/// Store (or replace) one message's classification.
pub fn classification_put(
    conn: &Connection,
    classification: &Classification,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO email_classification \
         (message_id, class, confidence, quoted_line, method, untrusted_origin, classified_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
         ON CONFLICT(message_id) DO UPDATE SET \
             class = excluded.class, confidence = excluded.confidence, \
             quoted_line = excluded.quoted_line, method = excluded.method, \
             untrusted_origin = excluded.untrusted_origin, \
             classified_at = excluded.classified_at",
        params![
            classification.message_id,
            classification.class.as_str(),
            classification.confidence,
            classification.quoted_line,
            classification.method.as_str(),
            i64::from(classification.untrusted_origin),
            classification.classified_at,
        ],
    )?;
    Ok(())
}

/// Insert one proposal; a message that already has one is left as it is.
pub fn proposal_insert(conn: &Connection, proposal: &StatusProposal) -> Result<bool, StoreError> {
    let changed = conn.execute(
        "INSERT INTO status_proposal \
         (id, message_id, application_id, from_status, to_status, class, confidence, \
          quoted_line, untrusted_origin, state, created_at, decided_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12) \
         ON CONFLICT(message_id) DO NOTHING",
        params![
            proposal.id,
            proposal.message_id,
            proposal.application_id,
            proposal.from_status,
            proposal.to_status,
            proposal.class.as_str(),
            proposal.confidence,
            proposal.quoted_line,
            i64::from(proposal.untrusted_origin),
            proposal.state.as_str(),
            proposal.created_at,
            proposal.decided_at,
        ],
    )?;
    Ok(changed > 0)
}

/// The proposals in one state, newest first.
pub fn proposals(
    conn: &Connection,
    state: ProposalState,
) -> Result<Vec<StatusProposal>, StoreError> {
    let mut stmt = conn
        .prepare(&format!(
            "{PROPOSAL_SELECT} WHERE state = ?1 ORDER BY created_at DESC, id DESC"
        ))
        .map_err(StoreError::from)?;
    let rows = stmt
        .query_map(params![state.as_str()], proposal_row)
        .map_err(StoreError::from)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(StoreError::from)
}

/// One proposal by id.
pub fn proposal_get(conn: &Connection, id: &str) -> Result<Option<StatusProposal>, StoreError> {
    conn.query_row(
        &format!("{PROPOSAL_SELECT} WHERE id = ?1"),
        params![id],
        proposal_row,
    )
    .optional()
    .map_err(StoreError::from)
}

/// Record a decision on one proposal.
pub fn proposal_decide(
    conn: &Connection,
    id: &str,
    application_id: Option<&str>,
    from_status: Option<&str>,
    state: ProposalState,
    decided_at: &str,
) -> Result<bool, StoreError> {
    let changed = conn.execute(
        "UPDATE status_proposal SET \
             application_id = COALESCE(?2, application_id), \
             from_status = COALESCE(?3, from_status), \
             state = ?4, decided_at = ?5 \
         WHERE id = ?1",
        params![id, application_id, from_status, state.as_str(), decided_at],
    )?;
    Ok(changed > 0)
}

fn map_err(error: StoreError) -> StatusProposalRepoError {
    StatusProposalRepoError(error.to_string())
}

impl StatusProposalRepo for Store {
    fn messages_unclassified(&self) -> Result<Vec<StoredMessage>, StatusProposalRepoError> {
        self.read(messages_unclassified).map_err(map_err)
    }

    fn message_get(&self, id: &str) -> Result<Option<StoredMessage>, StatusProposalRepoError> {
        let id = id.to_string();
        self.read(move |conn| message_get(conn, &id))
            .map_err(map_err)
    }

    fn classification_put(
        &self,
        classification: &Classification,
    ) -> Result<(), StatusProposalRepoError> {
        let classification = classification.clone();
        self.execute(move |conn| classification_put(conn, &classification))
            .map_err(map_err)
    }

    fn proposal_insert(&self, proposal: &StatusProposal) -> Result<bool, StatusProposalRepoError> {
        let proposal = proposal.clone();
        self.execute(move |conn| proposal_insert(conn, &proposal))
            .map_err(map_err)
    }

    fn proposals(
        &self,
        state: ProposalState,
    ) -> Result<Vec<StatusProposal>, StatusProposalRepoError> {
        self.read(move |conn| proposals(conn, state))
            .map_err(map_err)
    }

    fn proposal_get(&self, id: &str) -> Result<Option<StatusProposal>, StatusProposalRepoError> {
        let id = id.to_string();
        self.read(move |conn| proposal_get(conn, &id))
            .map_err(map_err)
    }

    fn proposal_decide(
        &self,
        id: &str,
        application_id: Option<&str>,
        from_status: Option<&str>,
        state: ProposalState,
        decided_at: &str,
    ) -> Result<bool, StatusProposalRepoError> {
        let id = id.to_string();
        let application_id = application_id.map(str::to_string);
        let from_status = from_status.map(str::to_string);
        let decided_at = decided_at.to_string();
        self.execute(move |conn| {
            proposal_decide(
                conn,
                &id,
                application_id.as_deref(),
                from_status.as_deref(),
                state,
                &decided_at,
            )
        })
        .map_err(map_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate;
    use crate::repos::email::insert_message;
    use boba_domain::mail::{ClassificationMethod, ImportOrigin, MatchReason, RawMessage};

    fn connection() -> Connection {
        let mut conn = Connection::open_in_memory().expect("in-memory sqlite");
        migrate::embedded()
            .to_latest(&mut conn)
            .expect("apply migrations");
        conn
    }

    fn seed_message(conn: &Connection, id: &str, subject: &str, body: &str) {
        let raw = RawMessage {
            provider_id: format!("prov-{id}"),
            from_address: "recruiting@acme.com".to_string(),
            subject: subject.to_string(),
            body_text: body.to_string(),
            received_at: 1_760_000_000,
            ..RawMessage::default()
        };
        insert_message(
            conn,
            id,
            None,
            &raw,
            MatchReason::CompanyDomain,
            ImportOrigin::Eml,
            "2026-10-17T09:00:00Z",
        )
        .unwrap();
    }

    fn application(conn: &Connection, id: &str, posting_id: &str) {
        conn.execute(
            "INSERT INTO source (id, name, access_class, legal_tier, apply_capability, \
             min_interval_ms, added_at, updated_at) \
             VALUES ('src_1', 'Manual', 'external_process', 'safe', 'link_out', 1000, \
             'now', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO posting (id, source_id, external_id, title, company, location_json, \
             work_mode, first_seen, last_seen, lang, apply_url) \
             VALUES (?1, 'src_1', ?1, 'Engineer', 'Acme', '[]', 'remote', 'now', 'now', \
             'en', 'https://acme.example/1')",
            params![posting_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO application (id, posting_id, status, offer_snapshot_json, created_at, \
             updated_at) VALUES (?1, ?2, 'applied', '{}', 'now', 'now')",
            params![id, posting_id],
        )
        .unwrap();
    }

    #[test]
    fn migration_creates_both_tables_with_the_untrusted_default() {
        let conn = connection();
        for table in ["email_classification", "status_proposal"] {
            let count: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE name = ?1",
                    params![table],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "{table}");
        }
        // AC6: a writer that forgets to set the column taints the row.
        let ddl: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name = 'status_proposal'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(ddl.contains("untrusted_origin INTEGER NOT NULL DEFAULT 1"));
        assert!(!ddl.contains("untrusted_origin INTEGER NOT NULL DEFAULT 0"));
    }

    #[test]
    fn a_classification_takes_the_message_out_of_the_unread_set() {
        let conn = connection();
        seed_message(&conn, "msg_1", "Hello", "Thank you for applying.");
        seed_message(&conn, "msg_2", "Hello", "Thanks.");

        assert_eq!(messages_unclassified(&conn).unwrap().len(), 2);

        classification_put(
            &conn,
            &Classification {
                message_id: "msg_1".to_string(),
                class: ReplyClass::Acknowledgement,
                confidence: 0.95,
                quoted_line: "Thank you for applying.".to_string(),
                method: ClassificationMethod::Deterministic,
                untrusted_origin: false,
                classified_at: "2026-10-17T09:00:00Z".to_string(),
            },
        )
        .unwrap();

        let unread = messages_unclassified(&conn).unwrap();
        assert_eq!(unread.len(), 1);
        assert_eq!(unread[0].id, "msg_2");
        assert_eq!(
            message_get(&conn, "msg_1").unwrap().unwrap().body_text,
            "Thank you for applying."
        );
    }

    #[test]
    fn an_llm_classification_cannot_be_stored_untainted() {
        let conn = connection();
        seed_message(&conn, "msg_1", "Hello", "Text.");
        // The CHECK is the invariant, not a caller's habit (AC6, ADR-010).
        let error = classification_put(
            &conn,
            &Classification {
                message_id: "msg_1".to_string(),
                class: ReplyClass::Other,
                confidence: 0.5,
                quoted_line: String::new(),
                method: ClassificationMethod::Llm,
                untrusted_origin: false,
                classified_at: "2026-10-17T09:00:00Z".to_string(),
            },
        );
        assert!(error.is_err());
    }

    #[test]
    fn one_message_produces_one_proposal() {
        let conn = connection();
        seed_message(&conn, "msg_1", "Hello", "Text.");
        application(&conn, "app_1", "pst_1");
        let proposal = StatusProposal {
            id: "sp_1".to_string(),
            message_id: "msg_1".to_string(),
            application_id: Some("app_1".to_string()),
            from_status: Some("applied".to_string()),
            to_status: "interview".to_string(),
            class: ReplyClass::InterviewInvite,
            confidence: 0.97,
            quoted_line: "Interview next week.".to_string(),
            untrusted_origin: false,
            state: ProposalState::Pending,
            created_at: "2026-10-17T09:00:00Z".to_string(),
            decided_at: None,
        };
        assert!(proposal_insert(&conn, &proposal).unwrap());
        assert!(
            !proposal_insert(&conn, &proposal).unwrap(),
            "a second proposal for one message is never written"
        );

        let pending = proposals(&conn, ProposalState::Pending).unwrap();
        assert_eq!(pending.len(), 1);

        assert!(
            proposal_decide(
                &conn,
                "sp_1",
                Some("app_1"),
                Some("applied"),
                ProposalState::Approved,
                "2026-10-17T10:00:00Z",
            )
            .unwrap()
        );
        assert!(proposals(&conn, ProposalState::Pending).unwrap().is_empty());
        let stored = proposal_get(&conn, "sp_1").unwrap().unwrap();
        assert_eq!(stored.state, ProposalState::Approved);
        assert_eq!(stored.decided_at.as_deref(), Some("2026-10-17T10:00:00Z"));
    }

    #[test]
    fn deleting_the_message_deletes_its_verdict_and_proposal() {
        let conn = connection();
        seed_message(&conn, "msg_1", "Hello", "Text.");
        application(&conn, "app_1", "pst_1");
        classification_put(
            &conn,
            &Classification {
                message_id: "msg_1".to_string(),
                class: ReplyClass::Offer,
                confidence: 0.97,
                quoted_line: "Offer letter.".to_string(),
                method: ClassificationMethod::Deterministic,
                untrusted_origin: false,
                classified_at: "2026-10-17T09:00:00Z".to_string(),
            },
        )
        .unwrap();
        proposal_insert(
            &conn,
            &StatusProposal {
                id: "sp_1".to_string(),
                message_id: "msg_1".to_string(),
                application_id: Some("app_1".to_string()),
                from_status: Some("applied".to_string()),
                to_status: "offer".to_string(),
                class: ReplyClass::Offer,
                confidence: 0.97,
                quoted_line: "Offer letter.".to_string(),
                untrusted_origin: false,
                state: ProposalState::Pending,
                created_at: "2026-10-17T09:00:00Z".to_string(),
                decided_at: None,
            },
        )
        .unwrap();

        // Disconnecting a mailbox (or resetting) purges its mail.
        conn.execute("DELETE FROM email_message WHERE id = 'msg_1'", [])
            .unwrap();

        let classifications: i64 = conn
            .query_row("SELECT count(*) FROM email_classification", [], |r| {
                r.get(0)
            })
            .unwrap();
        let proposals: i64 = conn
            .query_row("SELECT count(*) FROM status_proposal", [], |r| r.get(0))
            .unwrap();
        assert_eq!((classifications, proposals), (0, 0));
    }
}
