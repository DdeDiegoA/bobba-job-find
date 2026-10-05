//! `setting` repository (typed config in the DB; owner module: core/system,
//! AD-16). Values are stored as TEXT and parsed by the typed layer in
//! `boba-app::system::settings`; the app-level defaults are compiled in, so an
//! absent row always has a defined value (Consistency Conventions: config).

use boba_app::ports::repo::system::SettingRecord;
use rusqlite::{Connection, OptionalExtension as _, params};

/// Read one setting value, `None` when the row is absent.
pub fn get(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT value FROM setting WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
}

/// Insert or replace one setting value.
pub fn put(conn: &Connection, key: &str, value: &str, updated_at: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO setting (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![key, value, updated_at],
    )?;
    Ok(())
}

/// All settings, ordered by key.
pub fn list(conn: &Connection) -> rusqlite::Result<Vec<SettingRecord>> {
    let mut stmt = conn.prepare("SELECT key, value, updated_at FROM setting ORDER BY key")?;
    let rows = stmt.query_map([], |row| {
        Ok(SettingRecord {
            key: row.get(0)?,
            value: row.get(1)?,
            updated_at: row.get(2)?,
        })
    })?;
    rows.collect()
}

/// Remove one setting (it falls back to its compiled default). Returns whether
/// a row was deleted.
pub fn delete(conn: &Connection, key: &str) -> rusqlite::Result<bool> {
    Ok(conn.execute("DELETE FROM setting WHERE key = ?1", params![key])? > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open");
        conn.execute_batch(include_str!("../../migrations/0006_system.sql"))
            .expect("apply 0006");
        conn
    }

    #[test]
    fn absent_setting_reads_as_none() {
        let conn = conn();
        assert_eq!(get(&conn, "theme").unwrap(), None);
    }

    #[test]
    fn put_then_get_round_trips_and_refreshes() {
        let conn = conn();
        put(&conn, "theme", "system", "2026-10-05T01:00:00Z").unwrap();
        assert_eq!(get(&conn, "theme").unwrap().as_deref(), Some("system"));

        put(&conn, "theme", "dark", "2026-10-05T02:00:00Z").unwrap();
        let rows = list(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].value, "dark");
        assert_eq!(rows[0].updated_at, "2026-10-05T02:00:00Z");
    }

    #[test]
    fn list_is_sorted_and_delete_reports_removal() {
        let conn = conn();
        put(&conn, "ui_locale", "es", "2026-10-05T01:00:00Z").unwrap();
        put(&conn, "theme", "dark", "2026-10-05T01:00:00Z").unwrap();

        let keys: Vec<_> = list(&conn).unwrap().into_iter().map(|r| r.key).collect();
        assert_eq!(keys, vec!["theme".to_string(), "ui_locale".to_string()]);

        assert!(delete(&conn, "theme").unwrap());
        assert!(!delete(&conn, "theme").unwrap());
        assert_eq!(list(&conn).unwrap().len(), 1);
    }
}
