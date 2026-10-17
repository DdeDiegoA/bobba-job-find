//! Triage repository (S6.6, FR-54, FR-66, FR-67): the single writer of
//! `triage_dismissal`, `triage_override` and `triage_suggestion` (AD-16).
//!
//! One row per decision — one reason per posting, one override per posting, one
//! answer per suggestion — so every write is an upsert and Undo is a delete.
//! A stored value this build does not know is an error, never a silent guess.

use rusqlite::{Connection, OptionalExtension, params};

use boba_app::ports::repo::triage::{TriageRepo, TriageRepoError};
use boba_domain::triage::{
    DismissReason, DismissalRecord, SuggestionDecision, SuggestionDecisionStatus, VetoOverride,
};

use crate::db::{Store, StoreError};

fn repo(error: rusqlite::Error) -> TriageRepoError {
    TriageRepoError::new(error.to_string())
}

fn bad(column: &'static str, value: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::new(StoreError::Sqlite(format!("unknown {column} '{value}'"))),
    )
}

fn dismissal_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<DismissalRecord> {
    let reason: String = r.get(1)?;
    Ok(DismissalRecord {
        posting_id: r.get(0)?,
        reason: DismissReason::parse(&reason).ok_or_else(|| bad("reason", reason))?,
        created_at: r.get(2)?,
    })
}

/// Record (or replace) one Offer's dismiss reason (FR-66).
pub fn dismiss(conn: &Connection, record: &DismissalRecord) -> Result<(), TriageRepoError> {
    conn.execute(
        "INSERT INTO triage_dismissal (posting_id, reason, created_at) VALUES (?1, ?2, ?3) \
         ON CONFLICT (posting_id) DO UPDATE SET \
         reason = excluded.reason, created_at = excluded.created_at",
        params![record.posting_id, record.reason.as_str(), record.created_at],
    )
    .map_err(repo)?;
    Ok(())
}

/// Remove one Offer's reason (Undo); whether a row was there.
pub fn undo_dismiss(conn: &Connection, posting_id: &str) -> Result<bool, TriageRepoError> {
    let removed = conn
        .execute(
            "DELETE FROM triage_dismissal WHERE posting_id = ?1",
            params![posting_id],
        )
        .map_err(repo)?;
    Ok(removed > 0)
}

/// Every reason on record, newest first.
pub fn list_dismissals(conn: &Connection) -> Result<Vec<DismissalRecord>, TriageRepoError> {
    let mut stmt = conn
        .prepare(
            "SELECT posting_id, reason, created_at FROM triage_dismissal \
             ORDER BY created_at DESC, posting_id ASC",
        )
        .map_err(repo)?;
    let rows = stmt.query_map([], dismissal_row).map_err(repo)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(repo)
}

/// Record a VETO override (FR-54). The same Offer overridden twice keeps one row.
pub fn override_veto(conn: &Connection, row: &VetoOverride) -> Result<(), TriageRepoError> {
    conn.execute(
        "INSERT INTO triage_override (posting_id, gate, created_at) VALUES (?1, ?2, ?3) \
         ON CONFLICT (posting_id) DO UPDATE SET \
         gate = excluded.gate, created_at = excluded.created_at",
        params![row.posting_id, row.gate, row.created_at],
    )
    .map_err(repo)?;
    Ok(())
}

/// Every override, oldest first.
pub fn list_overrides(conn: &Connection) -> Result<Vec<VetoOverride>, TriageRepoError> {
    let mut stmt = conn
        .prepare(
            "SELECT posting_id, gate, created_at FROM triage_override \
             ORDER BY created_at ASC, posting_id ASC",
        )
        .map_err(repo)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(VetoOverride {
                posting_id: r.get(0)?,
                gate: r.get(1)?,
                created_at: r.get(2)?,
            })
        })
        .map_err(repo)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(repo)
}

/// One suggestion's answer, `None` while undecided.
pub fn suggestion_decision(
    conn: &Connection,
    id: &str,
) -> Result<Option<SuggestionDecision>, TriageRepoError> {
    conn.query_row(
        "SELECT suggestion_id, status, updated_at FROM triage_suggestion WHERE suggestion_id = ?1",
        params![id],
        |r| {
            let status: String = r.get(1)?;
            Ok(SuggestionDecision {
                id: r.get(0)?,
                status: SuggestionDecisionStatus::parse(&status)
                    .ok_or_else(|| bad("status", status.clone()))?,
                updated_at: r.get(2)?,
            })
        },
    )
    .optional()
    .map_err(repo)
}

/// Record the user's answer to a suggestion; a later answer replaces it.
pub fn save_suggestion_decision(
    conn: &Connection,
    row: &SuggestionDecision,
) -> Result<(), TriageRepoError> {
    conn.execute(
        "INSERT INTO triage_suggestion (suggestion_id, status, updated_at) VALUES (?1, ?2, ?3) \
         ON CONFLICT (suggestion_id) DO UPDATE SET \
         status = excluded.status, updated_at = excluded.updated_at",
        params![row.id, row.status.as_str(), row.updated_at],
    )
    .map_err(repo)?;
    Ok(())
}

impl TriageRepo for Store {
    fn dismiss(&self, record: &DismissalRecord) -> Result<(), TriageRepoError> {
        let record = record.clone();
        self.execute(move |conn| Ok(dismiss(conn, &record)))
            .map_err(|e| TriageRepoError::new(e.to_string()))?
    }

    fn undo_dismiss(&self, posting_id: &str) -> Result<bool, TriageRepoError> {
        let posting_id = posting_id.to_string();
        self.execute(move |conn| Ok(undo_dismiss(conn, &posting_id)))
            .map_err(|e| TriageRepoError::new(e.to_string()))?
    }

    fn dismissals(&self) -> Result<Vec<DismissalRecord>, TriageRepoError> {
        self.read(|conn| Ok(list_dismissals(conn)))
            .map_err(|e| TriageRepoError::new(e.to_string()))?
    }

    fn override_veto(&self, row: &VetoOverride) -> Result<(), TriageRepoError> {
        let row = row.clone();
        self.execute(move |conn| Ok(override_veto(conn, &row)))
            .map_err(|e| TriageRepoError::new(e.to_string()))?
    }

    fn overrides(&self) -> Result<Vec<VetoOverride>, TriageRepoError> {
        self.read(|conn| Ok(list_overrides(conn)))
            .map_err(|e| TriageRepoError::new(e.to_string()))?
    }

    fn suggestion_decision(&self, id: &str) -> Result<Option<SuggestionDecision>, TriageRepoError> {
        let id = id.to_string();
        self.read(move |conn| Ok(suggestion_decision(conn, &id)))
            .map_err(|e| TriageRepoError::new(e.to_string()))?
    }

    fn save_suggestion_decision(&self, row: &SuggestionDecision) -> Result<(), TriageRepoError> {
        let row = row.clone();
        self.execute(move |conn| Ok(save_suggestion_decision(conn, &row)))
            .map_err(|e| TriageRepoError::new(e.to_string()))?
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;
    use crate::migrate;

    const NOW: &str = "2026-10-17T09:00:00Z";

    fn connection() -> Connection {
        let mut conn = Connection::open_in_memory().expect("in-memory sqlite");
        // The store opens its connections with foreign keys on; the cascade
        // test below is only meaningful with the same setting.
        conn.execute("PRAGMA foreign_keys = ON", [])
            .expect("foreign keys on");
        migrate::embedded()
            .to_latest(&mut conn)
            .expect("apply migrations");
        conn.execute(
            "INSERT INTO source (id, name, access_class, legal_tier, apply_capability, \
             min_interval_ms, enabled, added_at, updated_at) \
             VALUES ('greenhouse', 'Greenhouse', 'public_json', 'safe', 'link_out', 2000, 1, 't', 't')",
            [],
        )
        .expect("seed source");
        for id in ["pst_1", "pst_2"] {
            conn.execute(
                "INSERT INTO posting (id, source_id, external_id, title, company, location_json, \
                 work_mode, posted_at, deadline, first_seen, last_seen, lang, description_html, \
                 description_text, requirements_json, apply_url, apply_email, dedup_key, simhash) \
                 VALUES (?1, 'greenhouse', ?1, 'Backend Engineer', 'Boba Labs', '[]', \
                 'remote', NULL, NULL, 't', 't', 'en', '', '', '[]', \
                 'https://jobs.example.com/1', NULL, ?1, NULL)",
                params![id],
            )
            .expect("seed posting");
        }
        conn
    }

    fn record(posting_id: &str, reason: DismissReason) -> DismissalRecord {
        DismissalRecord {
            posting_id: posting_id.to_string(),
            reason,
            created_at: NOW.to_string(),
        }
    }

    /// AC1: the migration creates the three tables.
    #[test]
    fn the_migration_creates_the_triage_tables() {
        let conn = connection();
        for table in ["triage_dismissal", "triage_override", "triage_suggestion"] {
            let found: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    params![table],
                    |r| r.get(0),
                )
                .expect("count");
            assert_eq!(found, 1, "{table} missing");
        }
    }

    /// AC1: one reason per posting; dismissing again replaces it.
    #[test]
    fn a_reason_is_stored_once_and_replaced_on_a_second_dismissal() {
        let conn = connection();
        dismiss(&conn, &record("pst_1", DismissReason::Salary)).expect("first");
        dismiss(&conn, &record("pst_1", DismissReason::Company)).expect("second");

        let rows = list_dismissals(&conn).expect("read");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].reason, DismissReason::Company);
    }

    /// AC1: Undo removes the reason and says whether there was one.
    #[test]
    fn undo_removes_the_reason_and_reports_it() {
        let conn = connection();
        dismiss(&conn, &record("pst_1", DismissReason::Location)).expect("store");
        assert!(undo_dismiss(&conn, "pst_1").expect("undo"));
        assert!(!undo_dismiss(&conn, "pst_1").expect("undo again"));
        assert!(list_dismissals(&conn).expect("read").is_empty());
    }

    /// FR-67: the newest dismissal reads first.
    #[test]
    fn dismissals_read_newest_first() {
        let conn = connection();
        dismiss(
            &conn,
            &DismissalRecord {
                posting_id: "pst_1".to_string(),
                reason: DismissReason::Salary,
                created_at: "2026-10-16T09:00:00Z".to_string(),
            },
        )
        .expect("old");
        dismiss(&conn, &record("pst_2", DismissReason::Salary)).expect("new");

        let rows = list_dismissals(&conn).expect("read");
        assert_eq!(rows[0].posting_id, "pst_2");
        assert_eq!(rows[1].posting_id, "pst_1");
    }

    /// AC2: an override round trips, and overriding again keeps one row.
    #[test]
    fn an_override_round_trips_and_is_recorded_once() {
        let conn = connection();
        let row = VetoOverride {
            posting_id: "pst_1".to_string(),
            gate: "eligibility".to_string(),
            created_at: NOW.to_string(),
        };
        override_veto(&conn, &row).expect("store");
        override_veto(&conn, &row).expect("again");

        let rows = list_overrides(&conn).expect("read");
        assert_eq!(rows, vec![row]);
    }

    /// AC3: a suggestion's answer round trips and is replaced, never duplicated.
    #[test]
    fn a_suggestion_answer_round_trips() {
        let conn = connection();
        assert_eq!(
            suggestion_decision(&conn, "exclude_onsite").expect("read"),
            None
        );

        let accepted = SuggestionDecision {
            id: "exclude_onsite".to_string(),
            status: SuggestionDecisionStatus::Accepted,
            updated_at: NOW.to_string(),
        };
        save_suggestion_decision(&conn, &accepted).expect("accept");
        assert_eq!(
            suggestion_decision(&conn, "exclude_onsite").expect("read"),
            Some(accepted.clone())
        );

        let ignored = SuggestionDecision {
            status: SuggestionDecisionStatus::Ignored,
            ..accepted
        };
        save_suggestion_decision(&conn, &ignored).expect("ignore");
        assert_eq!(
            suggestion_decision(&conn, "exclude_onsite").expect("read"),
            Some(ignored)
        );
    }

    /// Deleting a posting takes its decisions with it (no orphan rows).
    #[test]
    fn deleting_a_posting_cascades_to_its_triage_rows() {
        let conn = connection();
        dismiss(&conn, &record("pst_1", DismissReason::Other)).expect("dismiss");
        override_veto(
            &conn,
            &VetoOverride {
                posting_id: "pst_1".to_string(),
                gate: "language".to_string(),
                created_at: NOW.to_string(),
            },
        )
        .expect("override");

        conn.execute("DELETE FROM posting WHERE id = 'pst_1'", [])
            .expect("delete");

        assert!(list_dismissals(&conn).expect("read").is_empty());
        assert!(list_overrides(&conn).expect("read").is_empty());
    }
}
