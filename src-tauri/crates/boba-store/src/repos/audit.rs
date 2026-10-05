//! `audit_event` repository and the `SystemRepo` adapter (AD-12; owner module:
//! core/system, AD-16).
//!
//! The log is append-only at two levels: SQL triggers reject UPDATE/DELETE
//! (`0006_system.sql`), and every row is chained with
//! `hash = blake3(prev_hash ‖ canonical_json(row))` over every column except
//! `hash`. The latest hash is mirrored to the keychain (`audit-head`) so a
//! whole-tail truncation or a re-minted log is visible at startup.
//!
//! Rows carry metadata, IDs and blake3 hashes of payloads — never the payloads.

use std::sync::Arc;

use boba_app::ports::repo::system::{NewIntent, SettingRecord, SystemRepo, SystemRepoError};
use boba_domain::audit::{AuditOutcome, AuditPhase, AuditRow, ChainStatus, GENESIS_HASH};
use keyring_core::Entry;
use rusqlite::{Connection, OptionalExtension as _, params};

use crate::db::{Store, StoreError};
use crate::repos::settings;

/// Keychain entry name for the latest-hash anchor (AD-12).
pub const AUDIT_HEAD_ENTRY: &str = "audit-head";

#[derive(Debug, thiserror::Error)]
pub enum AuditError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("keychain error: {0}")]
    Keychain(String),
    #[error("audit row {id} is not an intent")]
    NotAnIntent { id: String },
}

impl From<AuditError> for SystemRepoError {
    fn from(e: AuditError) -> Self {
        SystemRepoError::new(e.to_string())
    }
}

/// `blake3(prev_hash ‖ canonical_json(row))`, lowercase hex.
fn compute_hash(row: &AuditRow) -> String {
    blake3::hash(&row.chain_payload()).to_hex().to_string()
}

/// Next chain position. The store's single writer actor serialises appends, so
/// this read-then-insert cannot race.
fn next_seq(conn: &Connection) -> Result<i64, AuditError> {
    let seq = conn.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM audit_event",
        [],
        |row| row.get(0),
    )?;
    Ok(seq)
}

/// `hash` of the last row, `None` for an empty log.
fn head_hash(conn: &Connection) -> Result<Option<String>, AuditError> {
    let hash = conn
        .query_row(
            "SELECT hash FROM audit_event ORDER BY seq DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(hash)
}

fn insert(conn: &Connection, row: &AuditRow) -> Result<(), AuditError> {
    conn.execute(
        "INSERT INTO audit_event (
             seq, id, created_at, phase, action, outcome, correlation_id, intent_id,
             actor, target_kind, target_id, detail_json, payload_blake3, prev_hash, hash
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            row.seq,
            row.id,
            row.created_at,
            row.phase.as_str(),
            row.action,
            row.outcome.map(AuditOutcome::as_str),
            row.correlation_id,
            row.intent_id,
            row.actor,
            row.target_kind,
            row.target_id,
            row.detail_json,
            row.payload_blake3,
            row.prev_hash,
            row.hash,
        ],
    )?;
    Ok(())
}

/// Assign `seq`/`prev_hash`, compute `hash`, and append the row (AD-12).
fn append_row(conn: &Connection, mut row: AuditRow) -> Result<AuditRow, AuditError> {
    row.seq = next_seq(conn)?;
    row.prev_hash = head_hash(conn)?.unwrap_or_else(|| GENESIS_HASH.to_string());
    row.hash = compute_hash(&row);
    insert(conn, &row)?;
    Ok(row)
}

/// Append an `intent` row before an outbound action runs.
pub fn append_intent(conn: &Connection, req: &NewIntent) -> Result<AuditRow, AuditError> {
    let row = AuditRow {
        seq: 0,
        id: ulid::Ulid::generate().to_string(),
        created_at: req.created_at.clone(),
        phase: AuditPhase::Intent,
        action: req.action.clone(),
        outcome: None,
        correlation_id: req.correlation_id.clone(),
        intent_id: None,
        actor: req.actor.clone(),
        target_kind: req.target_kind.clone(),
        target_id: req.target_id.clone(),
        detail_json: req.detail_json.clone(),
        payload_blake3: req.payload_blake3.clone(),
        prev_hash: String::new(),
        hash: String::new(),
    };
    append_row(conn, row)
}

/// Append an `outcome` row after `intent` runs (or, with
/// [`AuditOutcome::UnknownAfterCrash`], after a crash is discovered).
pub fn append_outcome(
    conn: &Connection,
    intent: &AuditRow,
    outcome: AuditOutcome,
    detail_json: &str,
    created_at: &str,
) -> Result<AuditRow, AuditError> {
    if intent.phase != AuditPhase::Intent {
        return Err(AuditError::NotAnIntent {
            id: intent.id.clone(),
        });
    }
    let row = AuditRow {
        seq: 0,
        id: ulid::Ulid::generate().to_string(),
        created_at: created_at.to_string(),
        phase: AuditPhase::Outcome,
        action: intent.action.clone(),
        outcome: Some(outcome),
        correlation_id: intent.correlation_id.clone(),
        intent_id: Some(intent.id.clone()),
        actor: intent.actor.clone(),
        target_kind: intent.target_kind.clone(),
        target_id: intent.target_id.clone(),
        detail_json: detail_json.to_string(),
        payload_blake3: None,
        prev_hash: String::new(),
        hash: String::new(),
    };
    append_row(conn, row)
}

/// Every row, in chain order.
pub fn all_rows(conn: &Connection) -> Result<Vec<AuditRow>, AuditError> {
    let mut stmt = conn.prepare(
        "SELECT seq, id, created_at, phase, action, outcome, correlation_id, intent_id,
                actor, target_kind, target_id, detail_json, payload_blake3, prev_hash, hash
         FROM audit_event ORDER BY seq",
    )?;
    let rows = stmt.query_map([], |row| {
        let phase: String = row.get(3)?;
        let outcome: Option<String> = row.get(5)?;
        Ok(AuditRow {
            seq: row.get(0)?,
            id: row.get(1)?,
            created_at: row.get(2)?,
            phase: AuditPhase::parse(&phase).unwrap_or(AuditPhase::Intent),
            action: row.get(4)?,
            outcome: outcome.as_deref().and_then(AuditOutcome::parse),
            correlation_id: row.get(6)?,
            intent_id: row.get(7)?,
            actor: row.get(8)?,
            target_kind: row.get(9)?,
            target_id: row.get(10)?,
            detail_json: row.get(11)?,
            payload_blake3: row.get(12)?,
            prev_hash: row.get(13)?,
            hash: row.get(14)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Walk the chain, re-hashing every row (AD-12 tamper check).
pub fn verify_chain(conn: &Connection) -> Result<ChainStatus, AuditError> {
    let rows = all_rows(conn)?;
    let mut prev = GENESIS_HASH.to_string();
    let mut broken_at = None;
    for row in &rows {
        if row.prev_hash != prev || row.hash != compute_hash(row) {
            broken_at = Some(row.seq);
            break;
        }
        prev.clone_from(&row.hash);
    }
    Ok(ChainStatus {
        rows: rows.len(),
        head: rows.last().map(|r| r.hash.clone()),
        broken_at,
    })
}

/// `intent` rows with no `outcome` row completing them (crash, FR-89).
pub fn unresolved_intents(conn: &Connection) -> Result<Vec<AuditRow>, AuditError> {
    let mut stmt = conn.prepare(
        "SELECT i.seq, i.id, i.created_at, i.phase, i.action, i.outcome, i.correlation_id,
                i.intent_id, i.actor, i.target_kind, i.target_id, i.detail_json,
                i.payload_blake3, i.prev_hash, i.hash
         FROM audit_event i
         WHERE i.phase = 'intent'
           AND NOT EXISTS (
               SELECT 1 FROM audit_event o
               WHERE o.phase = 'outcome' AND o.intent_id = i.id
           )
         ORDER BY i.seq",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(AuditRow {
            seq: row.get(0)?,
            id: row.get(1)?,
            created_at: row.get(2)?,
            phase: AuditPhase::Intent,
            action: row.get(4)?,
            outcome: None,
            correlation_id: row.get(6)?,
            intent_id: row.get(7)?,
            actor: row.get(8)?,
            target_kind: row.get(9)?,
            target_id: row.get(10)?,
            detail_json: row.get(11)?,
            payload_blake3: row.get(12)?,
            prev_hash: row.get(13)?,
            hash: row.get(14)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Read the keychain `audit-head` anchor, `None` when never written.
pub fn anchor_get(service: &str) -> Result<Option<String>, AuditError> {
    let entry = anchor_entry(service)?;
    match entry.get_password() {
        Ok(hash) => Ok(Some(hash)),
        Err(keyring_core::Error::NoEntry) => Ok(None),
        Err(e) => Err(AuditError::Keychain(e.to_string())),
    }
}

/// Mirror the latest hash to the keychain `audit-head` anchor.
pub fn anchor_set(service: &str, hash: &str) -> Result<(), AuditError> {
    anchor_entry(service)?
        .set_password(hash)
        .map_err(|e| AuditError::Keychain(e.to_string()))
}

fn anchor_entry(service: &str) -> Result<Entry, AuditError> {
    crate::keys::install_platform_store().map_err(|e| AuditError::Keychain(e.to_string()))?;
    Entry::new(service, AUDIT_HEAD_ENTRY).map_err(|e| AuditError::Keychain(e.to_string()))
}

/// The `SystemRepo` adapter over the one encrypted store (AD-2).
pub struct SqliteSystemRepo {
    store: Arc<Store>,
    service: String,
}

impl SqliteSystemRepo {
    pub fn new(store: Arc<Store>, service: impl Into<String>) -> Self {
        Self {
            store,
            service: service.into(),
        }
    }
}

fn store_err(e: StoreError) -> SystemRepoError {
    SystemRepoError::new(e.to_string())
}

impl SystemRepo for SqliteSystemRepo {
    fn setting_get(&self, key: &str) -> Result<Option<String>, SystemRepoError> {
        let key = key.to_string();
        self.store
            .read(move |conn| {
                settings::get(conn, &key).map_err(|e| StoreError::Sqlite(e.to_string()))
            })
            .map_err(store_err)
    }

    fn setting_put(&self, key: &str, value: &str, updated_at: &str) -> Result<(), SystemRepoError> {
        let (key, value, updated_at) = (key.to_string(), value.to_string(), updated_at.to_string());
        self.store
            .execute(move |conn| {
                settings::put(conn, &key, &value, &updated_at)
                    .map_err(|e| StoreError::Sqlite(e.to_string()))
            })
            .map_err(store_err)
    }

    fn setting_list(&self) -> Result<Vec<SettingRecord>, SystemRepoError> {
        self.store
            .read(|conn| settings::list(conn).map_err(|e| StoreError::Sqlite(e.to_string())))
            .map_err(store_err)
    }

    fn setting_delete(&self, key: &str) -> Result<bool, SystemRepoError> {
        let key = key.to_string();
        self.store
            .execute(move |conn| {
                settings::delete(conn, &key).map_err(|e| StoreError::Sqlite(e.to_string()))
            })
            .map_err(store_err)
    }

    fn audit_append_intent(&self, intent: &NewIntent) -> Result<AuditRow, SystemRepoError> {
        let intent = intent.clone();
        self.store
            .execute(move |conn| {
                append_intent(conn, &intent).map_err(|e| StoreError::Sqlite(e.to_string()))
            })
            .map_err(store_err)
    }

    fn audit_append_outcome(
        &self,
        intent: &AuditRow,
        outcome: AuditOutcome,
        detail_json: &str,
        created_at: &str,
    ) -> Result<AuditRow, SystemRepoError> {
        let (intent, detail_json, created_at) = (
            intent.clone(),
            detail_json.to_string(),
            created_at.to_string(),
        );
        self.store
            .execute(move |conn| {
                append_outcome(conn, &intent, outcome, &detail_json, &created_at)
                    .map_err(|e| StoreError::Sqlite(e.to_string()))
            })
            .map_err(store_err)
    }

    fn audit_head_hash(&self) -> Result<Option<String>, SystemRepoError> {
        self.store
            .read(|conn| head_hash(conn).map_err(|e| StoreError::Sqlite(e.to_string())))
            .map_err(store_err)
    }

    fn audit_verify_chain(&self) -> Result<ChainStatus, SystemRepoError> {
        self.store
            .read(|conn| verify_chain(conn).map_err(|e| StoreError::Sqlite(e.to_string())))
            .map_err(store_err)
    }

    fn audit_unresolved_intents(&self) -> Result<Vec<AuditRow>, SystemRepoError> {
        self.store
            .read(|conn| unresolved_intents(conn).map_err(|e| StoreError::Sqlite(e.to_string())))
            .map_err(store_err)
    }

    fn audit_anchor_get(&self) -> Result<Option<String>, SystemRepoError> {
        anchor_get(&self.service).map_err(Into::into)
    }

    fn audit_anchor_set(&self, hash: &str) -> Result<(), SystemRepoError> {
        anchor_set(&self.service, hash).map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, OnceLock};

    use super::*;
    use crate::db::StoreConfig;
    use tempfile::TempDir;

    const KEY_HEX: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open");
        conn.execute_batch(include_str!("../../migrations/0006_system.sql"))
            .expect("apply 0006");
        conn
    }

    fn mock_keychain() {
        static ONCE: OnceLock<()> = OnceLock::new();
        ONCE.get_or_init(|| {
            let store: Arc<keyring_core::CredentialStore> =
                keyring_core::mock::Store::new().expect("mock keychain");
            keyring_core::set_default_store(store);
        });
    }

    fn intent(action: &str, correlation: &str) -> NewIntent {
        NewIntent {
            action: action.to_string(),
            correlation_id: correlation.to_string(),
            actor: "core".to_string(),
            target_kind: "source".to_string(),
            target_id: Some("src_1".to_string()),
            detail_json: "{}".to_string(),
            payload_blake3: None,
            created_at: "2026-10-05T01:00:00.000Z".to_string(),
        }
    }

    // AD-12: intent then outcome link into one chain and verify cleanly.
    #[test]
    fn intent_and_outcome_chain_and_verify() {
        let conn = conn();
        let i = append_intent(&conn, &intent("apply.submitted", "att_1")).unwrap();
        assert_eq!(i.prev_hash, GENESIS_HASH);
        assert_eq!(i.hash, compute_hash(&i));

        let o = append_outcome(
            &conn,
            &i,
            AuditOutcome::Ok,
            "{\"ref\":\"pkg_1\"}",
            "2026-10-05T01:00:01.000Z",
        )
        .unwrap();
        assert_eq!(o.prev_hash, i.hash);
        assert_eq!(o.intent_id.as_deref(), Some(i.id.as_str()));

        let status = verify_chain(&conn).unwrap();
        assert!(status.is_intact(), "{status:?}");
        assert_eq!(status.rows, 2);
        assert_eq!(status.head.as_deref(), Some(o.hash.as_str()));
        assert_eq!(head_hash(&conn).unwrap().as_deref(), Some(o.hash.as_str()));
    }

    // AD-12: the stored hash is exactly blake3(prev_hash ‖ JCS(row)).
    #[test]
    fn stored_hash_is_blake3_of_prev_hash_and_canonical_json() {
        let conn = conn();
        let row = append_intent(&conn, &intent("email.sent", "att_2")).unwrap();
        let expected =
            blake3::hash(&[row.prev_hash.as_bytes(), row.canonical_json().as_bytes()].concat())
                .to_hex()
                .to_string();
        assert_eq!(row.hash, expected);
    }

    // AC-4: the SQL trigger rejects UPDATE.
    #[test]
    fn update_is_rejected_by_the_trigger() {
        let conn = conn();
        append_intent(&conn, &intent("apply.submitted", "att_3")).unwrap();
        let err = conn
            .execute("UPDATE audit_event SET action = 'tampered'", [])
            .expect_err("UPDATE must be rejected");
        assert!(err.to_string().contains("append-only"), "{err}");
    }

    // AC-4: the SQL trigger rejects DELETE.
    #[test]
    fn delete_is_rejected_by_the_trigger() {
        let conn = conn();
        append_intent(&conn, &intent("apply.submitted", "att_4")).unwrap();
        let err = conn
            .execute("DELETE FROM audit_event", [])
            .expect_err("DELETE must be rejected");
        assert!(err.to_string().contains("append-only"), "{err}");
    }

    // AC (adversarial): a rewritten row is caught by re-hashing, even if the
    // trigger is bypassed (defence in depth).
    #[test]
    fn tampered_row_fails_verification() {
        let conn = conn();
        append_intent(&conn, &intent("apply.submitted", "att_5")).unwrap();
        append_outcome(
            &conn,
            &all_rows(&conn).unwrap()[0],
            AuditOutcome::Ok,
            "{}",
            "2026-10-05T01:00:02.000Z",
        )
        .unwrap();

        conn.execute_batch("DROP TRIGGER audit_event_no_update;")
            .unwrap();
        conn.execute("UPDATE audit_event SET actor = 'evil' WHERE seq = 1", [])
            .unwrap();

        let status = verify_chain(&conn).unwrap();
        assert_eq!(status.broken_at, Some(1), "{status:?}");
        assert!(!status.is_intact());
    }

    // AC (adversarial): a reordered row breaks the prev_hash link.
    #[test]
    fn reordered_rows_fail_verification() {
        let conn = conn();
        let i = append_intent(&conn, &intent("apply.submitted", "att_6")).unwrap();
        append_outcome(
            &conn,
            &i,
            AuditOutcome::Ok,
            "{}",
            "2026-10-05T01:00:03.000Z",
        )
        .unwrap();

        conn.execute_batch("DROP TRIGGER audit_event_no_update;")
            .unwrap();
        // Swap the two seq values (offset first so the PRIMARY KEY never
        // clashes): the rows now link in the wrong order.
        conn.execute("UPDATE audit_event SET seq = seq + 100", [])
            .unwrap();
        conn.execute(
            "UPDATE audit_event SET seq = CASE seq WHEN 101 THEN 2 WHEN 102 THEN 1 ELSE seq END",
            [],
        )
        .unwrap();

        let status = verify_chain(&conn).unwrap();
        assert!(
            !status.is_intact(),
            "reorder must break the chain: {status:?}"
        );
    }

    // AC (adversarial): dropping the tail keeps the prefix valid, so only the
    // keychain anchor reveals the truncation.
    #[test]
    fn truncated_tail_is_detected_by_the_anchor_not_the_chain() {
        mock_keychain();
        let conn = conn();
        let a = append_intent(&conn, &intent("apply.submitted", "att_7")).unwrap();
        let b = append_outcome(
            &conn,
            &a,
            AuditOutcome::Ok,
            "{}",
            "2026-10-05T01:00:04.000Z",
        )
        .unwrap();
        anchor_set("test.audit.truncate", &b.hash).unwrap();

        conn.execute_batch("DROP TRIGGER audit_event_no_delete;")
            .unwrap();
        conn.execute("DELETE FROM audit_event WHERE seq = 2", [])
            .unwrap();

        let status = verify_chain(&conn).unwrap();
        assert!(status.is_intact(), "prefix stays internally valid");
        let anchor = anchor_get("test.audit.truncate").unwrap();
        assert_ne!(status.head, anchor, "head must diverge from the anchor");
    }

    // AC-2: an intent with no outcome is unresolved, then recovered.
    #[test]
    fn crashed_intent_is_unresolved_then_recovered() {
        let conn = conn();
        let i = append_intent(&conn, &intent("apply.submitted", "att_8")).unwrap();
        assert_eq!(unresolved_intents(&conn).unwrap().len(), 1);

        append_outcome(
            &conn,
            &i,
            AuditOutcome::UnknownAfterCrash,
            "{\"reason\":\"no_outcome_at_startup\"}",
            "2026-10-05T01:00:05.000Z",
        )
        .unwrap();

        assert!(unresolved_intents(&conn).unwrap().is_empty());
        assert!(verify_chain(&conn).unwrap().is_intact());
    }

    // The repository adapter wires settings + the chain through the real store.
    #[test]
    fn sqlite_system_repo_wires_settings_and_chain_through_the_store() {
        mock_keychain();
        let dir = TempDir::new().unwrap();
        let key = crate::keys::DbKey::from_hex(KEY_HEX).unwrap();
        let store = Arc::new(
            Store::open_with_key(
                StoreConfig::new(dir.path().join("boba.db"), dir.path().join("snapshots")),
                key,
            )
            .expect("store opens"),
        );
        let repo = SqliteSystemRepo::new(store, "test.audit.repo");

        repo.setting_put("theme", "dark", "2026-10-05T01:00:00Z")
            .unwrap();
        assert_eq!(repo.setting_get("theme").unwrap().as_deref(), Some("dark"));

        let i = repo
            .audit_append_intent(&intent("email.sent", "att_9"))
            .unwrap();
        let o = repo
            .audit_append_outcome(&i, AuditOutcome::Ok, "{}", "2026-10-05T01:00:06.000Z")
            .unwrap();
        assert_eq!(
            repo.audit_head_hash().unwrap().as_deref(),
            Some(o.hash.as_str())
        );
        assert!(repo.audit_verify_chain().unwrap().is_intact());

        repo.audit_anchor_set(&o.hash).unwrap();
        assert_eq!(
            repo.audit_anchor_get().unwrap().as_deref(),
            Some(o.hash.as_str())
        );
    }
}
