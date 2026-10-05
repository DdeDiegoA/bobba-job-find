//! `agent_run` / `agent_event` repository (AD-4, AD-16). Owner: agent.
//!
//! Free functions run one statement set against a connection; [`Store`]
//! implements the `AgentRepo` port by routing writes through the single writer
//! actor and reads through the read pool (AD-2). Rows are created by the
//! runtime as a run starts, appended per event, then closed with its outcome.

use boba_app::ports::repo::agent::{
    AgentEventRecord, AgentRepo, AgentRepoError, AgentRunRecord, FinishAgentRun, NewAgentRun,
};
use rusqlite::{Connection, OptionalExtension, params};

use crate::db::{Store, StoreError};

fn sql_err(e: rusqlite::Error) -> StoreError {
    StoreError::Sqlite(e.to_string())
}

/// Open a run with `status = running`.
pub fn insert_run(conn: &Connection, run: &NewAgentRun) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO agent_run
             (id, task_kind, input_trust, cli, model, status, started_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'running', strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        params![run.id, run.task_kind, run.input_trust, run.cli, run.model],
    )?;
    Ok(())
}

/// Append one event at `seq`.
pub fn insert_event(
    conn: &Connection,
    run_id: &str,
    seq: i64,
    kind: &str,
    payload_json: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO agent_event (run_id, seq, kind, payload_json, at)
         VALUES (?1, ?2, ?3, ?4, strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        params![run_id, seq, kind, payload_json],
    )?;
    Ok(())
}

/// Close a run with its terminal status, reason and reported usage.
pub fn update_finish(
    conn: &Connection,
    run_id: &str,
    finish: &FinishAgentRun,
) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE agent_run SET
             status = ?2,
             failure_reason = ?3,
             tokens_in = ?4,
             tokens_out = ?5,
             cost_usd = ?6,
             ended_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE id = ?1",
        params![
            run_id,
            finish.status.id(),
            finish.failure_reason,
            finish.tokens_in,
            finish.tokens_out,
            finish.cost_usd
        ],
    )?;
    Ok(changed > 0)
}

fn run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentRunRecord> {
    Ok(AgentRunRecord {
        id: row.get(0)?,
        task_kind: row.get(1)?,
        input_trust: row.get(2)?,
        cli: row.get(3)?,
        model: row.get(4)?,
        status: row.get(5)?,
        failure_reason: row.get(6)?,
        started_at: row.get(7)?,
        ended_at: row.get(8)?,
        tokens_in: row.get(9)?,
        tokens_out: row.get(10)?,
        cost_usd: row.get(11)?,
    })
}

const RUN_COLUMNS: &str = "id, task_kind, input_trust, cli, model, status, failure_reason,
                           started_at, ended_at, tokens_in, tokens_out, cost_usd";

/// Fetch a run by id.
pub fn get_run(conn: &Connection, run_id: &str) -> rusqlite::Result<Option<AgentRunRecord>> {
    conn.query_row(
        &format!("SELECT {RUN_COLUMNS} FROM agent_run WHERE id = ?1"),
        params![run_id],
        run_from_row,
    )
    .optional()
}

/// All events for a run, ordered by `seq`.
pub fn list_events(conn: &Connection, run_id: &str) -> rusqlite::Result<Vec<AgentEventRecord>> {
    let mut stmt = conn.prepare(
        "SELECT seq, kind, payload_json, at FROM agent_event WHERE run_id = ?1 ORDER BY seq",
    )?;
    let rows = stmt.query_map(params![run_id], |row| {
        Ok(AgentEventRecord {
            seq: row.get(0)?,
            kind: row.get(1)?,
            payload_json: row.get(2)?,
            at: row.get(3)?,
        })
    })?;
    rows.collect()
}

/// Runs left `running` by a crash, oldest first.
pub fn unfinished_runs(conn: &Connection) -> rusqlite::Result<Vec<AgentRunRecord>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {RUN_COLUMNS} FROM agent_run WHERE status = 'running' ORDER BY started_at"
    ))?;
    let rows = stmt.query_map([], run_from_row)?;
    rows.collect()
}

impl AgentRepo for Store {
    fn start_run(&self, run: NewAgentRun) -> Result<(), AgentRepoError> {
        self.execute(move |conn| insert_run(conn, &run).map_err(sql_err))
            .map_err(|e| AgentRepoError(e.to_string()))
    }

    fn record_event(
        &self,
        run_id: &str,
        seq: i64,
        kind: &str,
        payload_json: &str,
    ) -> Result<(), AgentRepoError> {
        let (run_id, kind, payload) = (
            run_id.to_string(),
            kind.to_string(),
            payload_json.to_string(),
        );
        self.execute(move |conn| insert_event(conn, &run_id, seq, &kind, &payload).map_err(sql_err))
            .map_err(|e| AgentRepoError(e.to_string()))
    }

    fn finish_run(&self, run_id: &str, finish: FinishAgentRun) -> Result<(), AgentRepoError> {
        let run_id = run_id.to_string();
        self.execute(move |conn| {
            update_finish(conn, &run_id, &finish)
                .map(|_| ())
                .map_err(sql_err)
        })
        .map_err(|e| AgentRepoError(e.to_string()))
    }

    fn get_run(&self, run_id: &str) -> Result<Option<AgentRunRecord>, AgentRepoError> {
        let run_id = run_id.to_string();
        self.read(move |conn| get_run(conn, &run_id).map_err(sql_err))
            .map_err(|e| AgentRepoError(e.to_string()))
    }

    fn list_events(&self, run_id: &str) -> Result<Vec<AgentEventRecord>, AgentRepoError> {
        let run_id = run_id.to_string();
        self.read(move |conn| list_events(conn, &run_id).map_err(sql_err))
            .map_err(|e| AgentRepoError(e.to_string()))
    }

    fn unfinished_runs(&self) -> Result<Vec<AgentRunRecord>, AgentRepoError> {
        self.read(|conn| unfinished_runs(conn).map_err(sql_err))
            .map_err(|e| AgentRepoError(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use boba_app::ports::repo::agent::RunStatus;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open");
        conn.execute_batch(include_str!("../../migrations/0004_agent_runs.sql"))
            .expect("apply 0004");
        conn
    }

    fn new_run(id: &str) -> NewAgentRun {
        NewAgentRun {
            id: id.to_string(),
            task_kind: "extract_postings".to_string(),
            input_trust: "untrusted".to_string(),
            cli: "claude_code".to_string(),
            model: Some("sonnet".to_string()),
        }
    }

    #[test]
    fn run_and_events_round_trip_in_order() {
        let conn = conn();
        insert_run(&conn, &new_run("01H")).unwrap();
        insert_event(&conn, "01H", 0, "started", "{\"type\":\"started\"}").unwrap();
        insert_event(&conn, "01H", 1, "text", "{\"type\":\"text\"}").unwrap();

        let run = get_run(&conn, "01H").unwrap().expect("run");
        assert_eq!(run.status, "running");
        assert_eq!(run.cli, "claude_code");
        assert_eq!(run.model.as_deref(), Some("sonnet"));
        assert!(run.ended_at.is_none());
        assert!(run.started_at.contains('T'));

        let events = list_events(&conn, "01H").unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].seq, 1);
        assert_eq!(events[1].kind, "text");
    }

    #[test]
    fn finish_sets_status_reason_and_usage() {
        let conn = conn();
        insert_run(&conn, &new_run("01H")).unwrap();
        let finish = FinishAgentRun {
            status: RunStatus::Failed,
            failure_reason: Some("auth_expired".to_string()),
            tokens_in: Some(10),
            tokens_out: None,
            cost_usd: None,
        };
        assert!(update_finish(&conn, "01H", &finish).unwrap());

        let run = get_run(&conn, "01H").unwrap().expect("run");
        assert_eq!(run.status, "failed");
        assert_eq!(run.failure_reason.as_deref(), Some("auth_expired"));
        assert_eq!(run.tokens_in, Some(10));
        assert_eq!(run.tokens_out, None, "unreported usage stays null");
        assert!(run.ended_at.is_some());
    }

    #[test]
    fn unfinished_runs_are_only_running_ones() {
        let conn = conn();
        insert_run(&conn, &new_run("A")).unwrap();
        insert_run(&conn, &new_run("B")).unwrap();
        update_finish(
            &conn,
            "B",
            &FinishAgentRun {
                status: RunStatus::Finished,
                failure_reason: None,
                tokens_in: None,
                tokens_out: None,
                cost_usd: None,
            },
        )
        .unwrap();

        let unfinished = unfinished_runs(&conn).unwrap();
        assert_eq!(unfinished.len(), 1);
        assert_eq!(unfinished[0].id, "A");
    }

    #[test]
    fn events_cascade_when_a_run_is_deleted() {
        let conn = conn();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        insert_run(&conn, &new_run("01H")).unwrap();
        insert_event(&conn, "01H", 0, "started", "{}").unwrap();
        conn.execute("DELETE FROM agent_run WHERE id = '01H'", [])
            .unwrap();
        assert!(list_events(&conn, "01H").unwrap().is_empty());
    }
}
