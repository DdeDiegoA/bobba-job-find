//! Apply outbox repository (S9.3): the single writer of `apply_attempt` (AD-16).
//!
//! The row is the evidence a retry reads (AC2). The payload is hashed here and
//! dropped (AD-12): the blake3 goes to the row and to the audit `intent`, the
//! body itself never reaches the database. `OutboxRepo` is implemented for
//! [`Store`], so the engine depends on the port, not this module.

use rusqlite::{Connection, OptionalExtension, params};

use boba_app::apply::engine::outbox::{
    ApplyAttempt, AttemptState, NewAttempt, OutboxRepo, OutboxRepoError,
};

use crate::db::{Store, StoreError};

const SELECT: &str = "SELECT id, application_id, channel, account_id, recipient, subject, \
     payload_blake3, message_id, state, error, created_at, updated_at, sent_at FROM apply_attempt";

fn bad(column: &'static str, value: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::new(StoreError::Sqlite(format!("unknown {column} '{value}'"))),
    )
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ApplyAttempt> {
    let state: String = r.get(8)?;
    Ok(ApplyAttempt {
        id: r.get(0)?,
        application_id: r.get(1)?,
        channel: r.get(2)?,
        account_id: r.get(3)?,
        recipient: r.get(4)?,
        subject: r.get(5)?,
        payload_blake3: r.get(6)?,
        message_id: r.get(7)?,
        state: AttemptState::parse(&state).ok_or_else(|| bad("state", state))?,
        error: r.get(9)?,
        created_at: r.get(10)?,
        updated_at: r.get(11)?,
        sent_at: r.get(12)?,
    })
}

/// Boba's id for one message: unique per attempt and in the shape a mail header
/// uses. Assigned here with the attempt id, so the row, the audit outcome and
/// the receipt all name the same value.
fn message_id_for(attempt_id: &str) -> String {
    format!("<{attempt_id}@boba.local>")
}

/// Record one attempt before its message is sent (AC2).
pub fn insert_attempt(
    conn: &Connection,
    attempt: &NewAttempt,
    now: &str,
) -> Result<ApplyAttempt, OutboxRepoError> {
    let id = format!("att_{}", ulid::Ulid::generate());
    let payload_blake3 = blake3::hash(&attempt.payload).to_hex().to_string();
    let message_id = message_id_for(&id);
    conn.execute(
        "INSERT INTO apply_attempt (id, application_id, channel, account_id, recipient, \
         subject, payload_blake3, message_id, state, error, created_at, updated_at, sent_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', NULL, ?9, ?9, NULL)",
        params![
            id,
            attempt.application_id,
            attempt.channel,
            attempt.account_id,
            attempt.recipient,
            attempt.subject,
            payload_blake3,
            message_id,
            now,
        ],
    )
    .map_err(|e| OutboxRepoError::new(e.to_string()))?;
    Ok(ApplyAttempt {
        id,
        application_id: attempt.application_id.clone(),
        channel: attempt.channel.clone(),
        account_id: attempt.account_id.clone(),
        recipient: attempt.recipient.clone(),
        subject: attempt.subject.clone(),
        payload_blake3,
        message_id: Some(message_id),
        state: AttemptState::Pending,
        error: None,
        created_at: now.to_string(),
        updated_at: now.to_string(),
        sent_at: None,
    })
}

/// Write the result of one attempt (AC2). `sent` also stamps `sent_at`.
pub fn set_state(
    conn: &Connection,
    id: &str,
    state: AttemptState,
    error: Option<&str>,
    now: &str,
) -> Result<(), OutboxRepoError> {
    let sent_at = (state == AttemptState::Sent).then_some(now);
    let changed = conn
        .execute(
            "UPDATE apply_attempt SET state = ?2, error = ?3, updated_at = ?4, \
             sent_at = COALESCE(?5, sent_at) WHERE id = ?1",
            params![id, state.as_str(), error, now, sent_at],
        )
        .map_err(|e| OutboxRepoError::new(e.to_string()))?;
    if changed == 0 {
        return Err(OutboxRepoError::new(format!("unknown attempt {id}")));
    }
    Ok(())
}

/// Every attempt of one application, oldest first — the evidence a retry reads.
pub fn list_attempts(
    conn: &Connection,
    application_id: &str,
) -> Result<Vec<ApplyAttempt>, OutboxRepoError> {
    let store = |error: rusqlite::Error| OutboxRepoError::new(error.to_string());
    let mut statement = conn
        .prepare(&format!(
            "{SELECT} WHERE application_id = ?1 ORDER BY created_at, rowid"
        ))
        .map_err(store)?;
    let attempts = statement
        .query_map(params![application_id], row)
        .map_err(store)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(store)?;
    Ok(attempts)
}

/// Whether one attempt exists (used by the tests and the adapter's guard).
pub fn attempt_get(conn: &Connection, id: &str) -> Option<ApplyAttempt> {
    conn.query_row(&format!("{SELECT} WHERE id = ?1"), params![id], row)
        .optional()
        .ok()
        .flatten()
}

impl OutboxRepo for Store {
    fn attempt_insert(
        &self,
        attempt: &NewAttempt,
        now: &str,
    ) -> Result<ApplyAttempt, OutboxRepoError> {
        let attempt = attempt.clone();
        let now = now.to_string();
        self.execute(move |conn| Ok(insert_attempt(conn, &attempt, &now)))
            .map_err(|e| OutboxRepoError::new(e.to_string()))?
    }

    fn attempt_set_state(
        &self,
        id: &str,
        state: AttemptState,
        error: Option<&str>,
        now: &str,
    ) -> Result<(), OutboxRepoError> {
        let (id, error, now) = (id.to_string(), error.map(str::to_string), now.to_string());
        self.execute(move |conn| Ok(set_state(conn, &id, state, error.as_deref(), &now)))
            .map_err(|e| OutboxRepoError::new(e.to_string()))?
    }

    fn attempts_for(&self, application_id: &str) -> Result<Vec<ApplyAttempt>, OutboxRepoError> {
        let application_id = application_id.to_string();
        self.read(move |conn| Ok(list_attempts(conn, &application_id)))
            .map_err(|e| OutboxRepoError::new(e.to_string()))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate;

    const NOW: &str = "2026-10-17T00:00:00Z";

    fn connection() -> Connection {
        let mut conn = Connection::open_in_memory().expect("in-memory sqlite");
        migrate::embedded()
            .to_latest(&mut conn)
            .expect("apply migrations");
        // The row carries `application_id`, so the parent rows exist.
        conn.execute(
            "INSERT INTO source (id, name, access_class, legal_tier, apply_capability, \
             min_interval_ms, enabled, added_at, updated_at) \
             VALUES ('remotive', 'Remotive', 'public_json', 'safe', 'email', 2000, 1, 't', 't')",
            [],
        )
        .expect("seed source");
        conn.execute(
            "INSERT INTO posting (id, source_id, external_id, title, company, location_json, \
             work_mode, posted_at, deadline, first_seen, last_seen, lang, description_html, \
             description_text, requirements_json, apply_url, apply_email, dedup_key, simhash) \
             VALUES ('pst_1', 'remotive', 'abc-1', 'Backend Engineer', 'Boba Labs', '[]', \
             'remote', NULL, NULL, 't', 't', 'en', '', '', '[]', \
             'https://jobs.example.com/abc-1', 'jobs@nimbus.example', 'k1', NULL)",
            [],
        )
        .expect("seed posting");
        for id in ["app_01", "app_02"] {
            conn.execute(
                "INSERT INTO application (id, posting_id, status, offer_snapshot_json, notes, \
                 next_action, created_at, updated_at) \
                 VALUES (?1, 'pst_1', 'drafted', '{}', NULL, NULL, 't', 't')",
                params![id],
            )
            .expect("seed application");
        }
        conn
    }

    fn attempt(application_id: &str, recipient: &str) -> NewAttempt {
        NewAttempt {
            application_id: application_id.to_string(),
            channel: "email".to_string(),
            account_id: "eml_1".to_string(),
            recipient: recipient.to_string(),
            subject: "Application: Backend Engineer".to_string(),
            payload: b"to:jobs@nimbus.example\nsubject:Application: Backend Engineer\n".to_vec(),
        }
    }

    #[test]
    fn an_attempt_is_recorded_pending_with_a_hash_not_the_payload() {
        let conn = connection();
        let stored = insert_attempt(&conn, &attempt("app_01", "jobs@nimbus.example"), NOW).unwrap();

        assert!(stored.id.starts_with("att_"), "a prefixed ULID (AD-17)");
        assert_eq!(stored.state, AttemptState::Pending);
        assert_eq!(stored.recipient, "jobs@nimbus.example");
        assert_eq!(
            stored.payload_blake3,
            blake3::hash(&attempt("app_01", "jobs@nimbus.example").payload)
                .to_hex()
                .to_string(),
            "the row keeps the payload's blake3 (AD-12)"
        );
        assert_eq!(
            stored.message_id.as_deref(),
            Some(format!("<{}@boba.local>", stored.id).as_str())
        );
        assert_eq!(stored.sent_at, None);
    }

    #[test]
    fn a_sent_attempt_is_stamped_and_a_failure_keeps_its_class() {
        let conn = connection();
        let sent = insert_attempt(&conn, &attempt("app_01", "jobs@nimbus.example"), NOW).unwrap();
        set_state(
            &conn,
            &sent.id,
            AttemptState::Sent,
            None,
            "2026-10-17T00:05:00Z",
        )
        .unwrap();
        let stored = attempt_get(&conn, &sent.id).unwrap();
        assert_eq!(stored.state, AttemptState::Sent);
        assert_eq!(stored.sent_at.as_deref(), Some("2026-10-17T00:05:00Z"));
        assert_eq!(stored.error, None);

        // A live (sent) attempt keeps the next one out; another application is
        // its own evidence.
        insert_attempt(&conn, &attempt("app_01", "jobs@nimbus.example"), NOW)
            .expect_err("AC2: one live attempt per application");

        let failed = insert_attempt(&conn, &attempt("app_02", "hire@other.example"), NOW).unwrap();
        set_state(
            &conn,
            &failed.id,
            AttemptState::Uncertain,
            Some("unreachable"),
            NOW,
        )
        .unwrap();
        let stored = attempt_get(&conn, &failed.id).unwrap();
        assert_eq!(stored.state, AttemptState::Uncertain);
        assert_eq!(stored.error.as_deref(), Some("unreachable"));
    }

    #[test]
    fn attempts_are_read_oldest_first_and_only_for_their_application() {
        let conn = connection();
        let first = insert_attempt(&conn, &attempt("app_01", "jobs@nimbus.example"), NOW).unwrap();
        // A refusal frees the application for the next attempt.
        set_state(
            &conn,
            &first.id,
            AttemptState::Failed,
            Some("rejected"),
            NOW,
        )
        .unwrap();
        let second = insert_attempt(
            &conn,
            &attempt("app_01", "jobs@nimbus.example"),
            "2026-10-17T00:01:00Z",
        )
        .unwrap();
        insert_attempt(&conn, &attempt("app_02", "hire@other.example"), NOW).unwrap();

        let attempts = list_attempts(&conn, "app_01").unwrap();
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0].id, first.id);
        assert_eq!(attempts[1].id, second.id);
    }

    #[test]
    fn an_uncertain_attempt_still_allows_the_users_confirmed_resend() {
        let conn = connection();
        let first = insert_attempt(&conn, &attempt("app_01", "jobs@nimbus.example"), NOW).unwrap();
        set_state(
            &conn,
            &first.id,
            AttemptState::Uncertain,
            Some("unreachable"),
            NOW,
        )
        .unwrap();

        // `uncertain` is not proof of a send, so the user's word can add the
        // attempt that proves it (AC2).
        let second = insert_attempt(&conn, &attempt("app_01", "jobs@nimbus.example"), NOW).unwrap();

        assert_ne!(first.id, second.id);
        assert_eq!(list_attempts(&conn, "app_01").unwrap().len(), 2);
    }

    #[test]
    fn an_unknown_attempt_is_an_error_not_a_silent_write() {
        let conn = connection();
        let error = set_state(&conn, "att_missing", AttemptState::Sent, None, NOW).unwrap_err();
        assert!(error.to_string().contains("att_missing"));
    }

    #[test]
    fn the_state_check_refuses_a_value_this_build_does_not_know() {
        let conn = connection();
        let stored = insert_attempt(&conn, &attempt("app_01", "jobs@nimbus.example"), NOW).unwrap();
        conn.execute(
            "UPDATE apply_attempt SET state = 'sending' WHERE id = ?1",
            params![stored.id],
        )
        .expect_err("the CHECK refuses it");
    }
}
