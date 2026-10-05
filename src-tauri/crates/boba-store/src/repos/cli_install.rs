//! `cli_install` repository (AD-4 detection results; owner module: agent, AD-16).
//!
//! One row per probed CLI installation, keyed by `(kind, path)` so a re-probe
//! updates in place. `id` is a ULID (AD-17); `probed_at` is stamped by the DB as
//! UTC RFC 3339 (AD-17).

use rusqlite::{Connection, params};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CliInstallError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
}

/// A stored CLI installation row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliInstallRecord {
    pub id: String,
    pub kind: String,
    pub path: String,
    pub version: String,
    pub capabilities_json: String,
    pub probed_at: String,
}

/// Insert or refresh an installation, returning its id.
pub fn upsert(
    conn: &Connection,
    kind: &str,
    path: &str,
    version: &str,
    capabilities_json: &str,
) -> Result<String, CliInstallError> {
    let id = ulid::Ulid::generate().to_string();
    conn.execute(
        "INSERT INTO cli_install (id, kind, path, version, capabilities_json, probed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, strftime('%Y-%m-%dT%H:%M:%fZ','now'))
         ON CONFLICT(kind, path) DO UPDATE SET
             version = excluded.version,
             capabilities_json = excluded.capabilities_json,
             probed_at = excluded.probed_at",
        params![id, kind, path, version, capabilities_json],
    )?;
    let stored: String = conn.query_row(
        "SELECT id FROM cli_install WHERE kind = ?1 AND path = ?2",
        params![kind, path],
        |row| row.get(0),
    )?;
    Ok(stored)
}

/// All stored installations, ordered by kind then path.
pub fn list(conn: &Connection) -> Result<Vec<CliInstallRecord>, CliInstallError> {
    let mut stmt = conn.prepare(
        "SELECT id, kind, path, version, capabilities_json, probed_at
         FROM cli_install ORDER BY kind, path",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(CliInstallRecord {
            id: row.get(0)?,
            kind: row.get(1)?,
            path: row.get(2)?,
            version: row.get(3)?,
            capabilities_json: row.get(4)?,
            probed_at: row.get(5)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Remove one installation by id. Returns whether a row was deleted.
pub fn delete(conn: &Connection, id: &str) -> Result<bool, CliInstallError> {
    Ok(conn.execute("DELETE FROM cli_install WHERE id = ?1", params![id])? > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open");
        conn.execute_batch(include_str!("../../migrations/0002_cli_install.sql"))
            .expect("apply 0002");
        conn
    }

    #[test]
    fn upsert_then_list_round_trips_a_row() {
        let conn = conn();
        let id = upsert(
            &conn,
            "claude_code",
            "/usr/local/bin/claude",
            "2.1.289",
            "{\"mcp\":true}",
        )
        .expect("upsert");
        assert!(id.starts_with("01"), "ULID, got {id}");

        let rows = list(&conn).expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, "claude_code");
        assert_eq!(rows[0].path, "/usr/local/bin/claude");
        assert_eq!(rows[0].version, "2.1.289");
        assert_eq!(rows[0].capabilities_json, "{\"mcp\":true}");
        assert!(rows[0].probed_at.contains('T'));
        assert!(rows[0].probed_at.ends_with('Z'));
    }

    #[test]
    fn upsert_same_kind_and_path_refreshes_in_place() {
        let conn = conn();
        let first = upsert(&conn, "codex", "/opt/codex", "0.9.0", "{}").expect("first");
        let second =
            upsert(&conn, "codex", "/opt/codex", "0.9.1", "{\"mcp\":true}").expect("second");

        assert_eq!(first, second, "id is stable across re-probes");
        let rows = list(&conn).expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].version, "0.9.1");
    }

    #[test]
    fn a_second_path_for_the_same_kind_is_a_distinct_row() {
        let conn = conn();
        upsert(&conn, "aider", "/a/aider", "0.86.1", "{}").expect("a");
        upsert(&conn, "aider", "/b/aider", "0.86.1", "{}").expect("b");
        assert_eq!(list(&conn).expect("list").len(), 2);
    }

    #[test]
    fn delete_removes_only_the_named_row() {
        let conn = conn();
        let keep = upsert(&conn, "aider", "/a/aider", "0.86.1", "{}").expect("keep");
        let drop = upsert(&conn, "aider", "/b/aider", "0.86.1", "{}").expect("drop");

        assert!(delete(&conn, &drop).expect("delete"));
        assert!(!delete(&conn, &drop).expect("delete again"));
        let rows = list(&conn).expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, keep);
    }
}
