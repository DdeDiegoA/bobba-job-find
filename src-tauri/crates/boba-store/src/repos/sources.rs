//! Sources repository (S4.1): the single writer of `source`, `source_run` and
//! `source_optin` (AD-9, AD-16).
//!
//! The store keeps the typed `boba_domain::source` records; the enum columns are
//! stored as their `as_str` text and parsed back on read. `SourcesRepo` (whose
//! supertraits are `Send + Sync`) is implemented for [`Store`], which is itself
//! `Send + Sync`, so use cases depend on the port, not this module.

use rusqlite::{Connection, OptionalExtension, params};

use boba_app::ports::repo::sources::{SourcesRepo, SourcesRepoError};
use boba_domain::apply::session_policy::SessionAutoOptIn;
use boba_domain::source::{OptInRecord, RunStatus, SourceRecord, SourceRunRecord};

use crate::db::{Store, StoreError};

const SELECT: &str = "SELECT id, name, access_class, legal_tier, apply_capability, attribution, \
     min_interval_ms, daily_cap, enabled, added_at, updated_at FROM source";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<SourceRecord> {
    Ok(SourceRecord {
        id: r.get(0)?,
        name: r.get(1)?,
        access_class: parse(r.get(2)?)?,
        legal_tier: parse(r.get(3)?)?,
        apply_capability: parse(r.get(4)?)?,
        attribution: r.get(5)?,
        min_interval_ms: r.get::<_, i64>(6)? as u64,
        daily_cap: r.get::<_, Option<i64>>(7)?.map(|v| v as u32),
        enabled: r.get::<_, i64>(8)? != 0,
        added_at: r.get(9)?,
        updated_at: r.get(10)?,
    })
}

fn parse<T: std::str::FromStr<Err = boba_domain::source::SourceError>>(
    text: String,
) -> rusqlite::Result<T> {
    text.parse().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

/// Insert or refresh one configured source.
pub fn upsert_source(conn: &Connection, record: &SourceRecord) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO source \
         (id, name, access_class, legal_tier, apply_capability, attribution, min_interval_ms, \
          daily_cap, enabled, added_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
         ON CONFLICT(id) DO UPDATE SET \
             name = excluded.name, access_class = excluded.access_class, \
             legal_tier = excluded.legal_tier, apply_capability = excluded.apply_capability, \
             attribution = excluded.attribution, min_interval_ms = excluded.min_interval_ms, \
             daily_cap = excluded.daily_cap, enabled = excluded.enabled, \
             updated_at = excluded.updated_at",
        params![
            record.id,
            record.name,
            record.access_class.as_str(),
            record.legal_tier.as_str(),
            record.apply_capability.as_str(),
            record.attribution,
            record.min_interval_ms as i64,
            record.daily_cap.map(i64::from),
            i64::from(record.enabled),
            record.added_at,
            record.updated_at,
        ],
    )?;
    Ok(())
}

pub fn get_source(conn: &Connection, id: &str) -> Result<Option<SourceRecord>, StoreError> {
    Ok(conn
        .query_row(&format!("{SELECT} WHERE id = ?1"), params![id], row)
        .optional()?)
}

pub fn list_sources(conn: &Connection) -> Result<Vec<SourceRecord>, StoreError> {
    let mut stmt = conn.prepare(&format!("{SELECT} ORDER BY name, id"))?;
    let rows = stmt.query_map([], row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Enable or disable a source without touching its manifest.
pub fn set_source_enabled(
    conn: &Connection,
    id: &str,
    enabled: bool,
    updated_at: &str,
) -> Result<(), StoreError> {
    let changed = conn.execute(
        "UPDATE source SET enabled = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, i64::from(enabled), updated_at],
    )?;
    if changed == 0 {
        return Err(StoreError::Sqlite(format!("unknown source '{id}'")));
    }
    Ok(())
}

/// Record (or refresh) a per-source opt-in.
pub fn record_optin(conn: &Connection, optin: &OptInRecord) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO source_optin (source_id, warning_version, accepted_at) VALUES (?1, ?2, ?3) \
         ON CONFLICT(source_id) DO UPDATE SET \
             warning_version = excluded.warning_version, accepted_at = excluded.accepted_at",
        params![optin.source_id, optin.warning_version, optin.accepted_at],
    )?;
    Ok(())
}

pub fn get_optin(conn: &Connection, source_id: &str) -> Result<Option<OptInRecord>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT source_id, warning_version, accepted_at FROM source_optin WHERE source_id = ?1",
            params![source_id],
            |r| {
                Ok(OptInRecord {
                    source_id: r.get(0)?,
                    warning_version: r.get(1)?,
                    accepted_at: r.get(2)?,
                })
            },
        )
        .optional()?)
}

/// Remove a per-source opt-in (S11.1a: disabling a Source closes the gate).
pub fn remove_optin(conn: &Connection, source_id: &str) -> Result<(), StoreError> {
    conn.execute(
        "DELETE FROM source_optin WHERE source_id = ?1",
        params![source_id],
    )?;
    Ok(())
}

pub fn opted_in_ids(conn: &Connection) -> Result<Vec<String>, StoreError> {
    let mut stmt = conn.prepare("SELECT source_id FROM source_optin ORDER BY source_id")?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// The separate Auto opt-in for one Source (S11.3, FR-91).
pub fn auto_opt_in(
    conn: &Connection,
    source_id: &str,
) -> Result<Option<SessionAutoOptIn>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT source_id, confirmed_name, warning_version, accepted_at \
             FROM source_auto_optin WHERE source_id = ?1",
            params![source_id],
            |r| {
                Ok(SessionAutoOptIn {
                    source_id: r.get(0)?,
                    confirmed_name: r.get(1)?,
                    warning_version: r.get(2)?,
                    accepted_at: r.get(3)?,
                })
            },
        )
        .optional()?)
}

/// Record (or re-confirm) the Auto opt-in; the row replaces by Source.
pub fn record_auto_opt_in(conn: &Connection, optin: &SessionAutoOptIn) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO source_auto_optin (source_id, confirmed_name, warning_version, accepted_at) \
         VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(source_id) DO UPDATE SET \
             confirmed_name = excluded.confirmed_name, \
             warning_version = excluded.warning_version, \
             accepted_at = excluded.accepted_at",
        params![
            optin.source_id,
            optin.confirmed_name,
            optin.warning_version,
            optin.accepted_at
        ],
    )?;
    Ok(())
}

/// Revoke the Auto opt-in (idempotent); the reading opt-in is not touched.
pub fn remove_auto_opt_in(conn: &Connection, source_id: &str) -> Result<(), StoreError> {
    conn.execute(
        "DELETE FROM source_auto_optin WHERE source_id = ?1",
        params![source_id],
    )?;
    Ok(())
}

/// The deadline a challenge set for Auto (`source.paused_until`).
pub fn auto_paused_until(conn: &Connection, source_id: &str) -> Result<Option<String>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT paused_until FROM source WHERE id = ?1",
            params![source_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten())
}

/// Set (or clear) the Auto pause deadline on the Source row.
pub fn set_auto_paused_until(
    conn: &Connection,
    source_id: &str,
    until: Option<&str>,
    updated_at: &str,
) -> Result<(), StoreError> {
    let changed = conn.execute(
        "UPDATE source SET paused_until = ?2, updated_at = ?3 WHERE id = ?1",
        params![source_id, until, updated_at],
    )?;
    if changed == 0 {
        return Err(StoreError::Sqlite(format!("unknown source '{source_id}'")));
    }
    Ok(())
}

pub fn start_run(conn: &Connection, run: &SourceRunRecord) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO source_run (id, source_id, status, started_at, finished_at, postings_found, error) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            run.id,
            run.source_id,
            run.status.as_str(),
            run.started_at,
            run.finished_at,
            run.postings_found,
            run.error,
        ],
    )?;
    Ok(())
}

pub fn finish_run(
    conn: &Connection,
    id: &str,
    finished_at: &str,
    status: RunStatus,
    postings_found: i64,
    error: Option<&str>,
) -> Result<(), StoreError> {
    let changed = conn.execute(
        "UPDATE source_run SET status = ?2, finished_at = ?3, postings_found = ?4, error = ?5 \
         WHERE id = ?1",
        params![id, status.as_str(), finished_at, postings_found, error],
    )?;
    if changed == 0 {
        return Err(StoreError::Sqlite(format!("unknown source_run '{id}'")));
    }
    Ok(())
}

/// The newest runs of one source, newest first, capped at `limit` (S4.8).
pub fn recent_runs(
    conn: &Connection,
    source_id: &str,
    limit: usize,
) -> Result<Vec<SourceRunRecord>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT id, source_id, status, started_at, finished_at, postings_found, error \
         FROM source_run WHERE source_id = ?1 \
         ORDER BY started_at DESC, id DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![source_id, limit as i64], |r| {
        Ok(SourceRunRecord {
            id: r.get(0)?,
            source_id: r.get(1)?,
            status: parse(r.get(2)?)?,
            started_at: r.get(3)?,
            finished_at: r.get(4)?,
            postings_found: r.get(5)?,
            error: r.get(6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn map_err(error: StoreError) -> SourcesRepoError {
    SourcesRepoError::Store(error.to_string())
}

impl SourcesRepo for Store {
    fn upsert_source(&self, record: &SourceRecord) -> Result<(), SourcesRepoError> {
        let record = record.clone();
        self.execute(move |conn| upsert_source(conn, &record))
            .map_err(map_err)
    }

    fn get_source(&self, id: &str) -> Result<Option<SourceRecord>, SourcesRepoError> {
        let id = id.to_string();
        self.read(move |conn| get_source(conn, &id))
            .map_err(map_err)
    }

    fn list_sources(&self) -> Result<Vec<SourceRecord>, SourcesRepoError> {
        self.read(list_sources).map_err(map_err)
    }

    fn set_source_enabled(
        &self,
        id: &str,
        enabled: bool,
        updated_at: &str,
    ) -> Result<(), SourcesRepoError> {
        let id = id.to_string();
        let updated_at = updated_at.to_string();
        self.execute(move |conn| set_source_enabled(conn, &id, enabled, &updated_at))
            .map_err(map_err)
    }

    fn record_optin(&self, optin: &OptInRecord) -> Result<(), SourcesRepoError> {
        let optin = optin.clone();
        self.execute(move |conn| record_optin(conn, &optin))
            .map_err(map_err)
    }

    fn get_optin(&self, source_id: &str) -> Result<Option<OptInRecord>, SourcesRepoError> {
        let source_id = source_id.to_string();
        self.read(move |conn| get_optin(conn, &source_id))
            .map_err(map_err)
    }

    fn remove_optin(&self, source_id: &str) -> Result<(), SourcesRepoError> {
        let source_id = source_id.to_string();
        self.execute(move |conn| remove_optin(conn, &source_id))
            .map_err(map_err)
    }

    fn opted_in_ids(&self) -> Result<Vec<String>, SourcesRepoError> {
        self.read(opted_in_ids).map_err(map_err)
    }

    fn auto_opt_in(&self, source_id: &str) -> Result<Option<SessionAutoOptIn>, SourcesRepoError> {
        let source_id = source_id.to_string();
        self.read(move |conn| auto_opt_in(conn, &source_id))
            .map_err(map_err)
    }

    fn record_auto_opt_in(&self, optin: &SessionAutoOptIn) -> Result<(), SourcesRepoError> {
        let optin = optin.clone();
        self.execute(move |conn| record_auto_opt_in(conn, &optin))
            .map_err(map_err)
    }

    fn remove_auto_opt_in(&self, source_id: &str) -> Result<(), SourcesRepoError> {
        let source_id = source_id.to_string();
        self.execute(move |conn| remove_auto_opt_in(conn, &source_id))
            .map_err(map_err)
    }

    fn auto_paused_until(&self, source_id: &str) -> Result<Option<String>, SourcesRepoError> {
        let source_id = source_id.to_string();
        self.read(move |conn| auto_paused_until(conn, &source_id))
            .map_err(map_err)
    }

    fn set_auto_paused_until(
        &self,
        source_id: &str,
        until: Option<&str>,
        updated_at: &str,
    ) -> Result<(), SourcesRepoError> {
        let source_id = source_id.to_string();
        let until = until.map(str::to_string);
        let updated_at = updated_at.to_string();
        self.execute(move |conn| {
            set_auto_paused_until(conn, &source_id, until.as_deref(), &updated_at)
        })
        .map_err(map_err)
    }

    fn start_run(&self, run: &SourceRunRecord) -> Result<(), SourcesRepoError> {
        let run = run.clone();
        self.execute(move |conn| start_run(conn, &run))
            .map_err(map_err)
    }

    fn recent_runs(
        &self,
        source_id: &str,
        limit: usize,
    ) -> Result<Vec<SourceRunRecord>, SourcesRepoError> {
        let source_id = source_id.to_string();
        self.read(move |conn| recent_runs(conn, &source_id, limit))
            .map_err(map_err)
    }

    fn finish_run(
        &self,
        id: &str,
        finished_at: &str,
        status: RunStatus,
        postings_found: i64,
        error: Option<&str>,
    ) -> Result<(), SourcesRepoError> {
        let id = id.to_string();
        let finished_at = finished_at.to_string();
        let error = error.map(str::to_string);
        self.execute(move |conn| {
            finish_run(
                conn,
                &id,
                &finished_at,
                status,
                postings_found,
                error.as_deref(),
            )
        })
        .map_err(map_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate;
    use boba_domain::source::{AccessClass, ApplyCapability, LegalTier};

    fn connection() -> Connection {
        let mut conn = Connection::open_in_memory().expect("in-memory sqlite");
        migrate::embedded()
            .to_latest(&mut conn)
            .expect("apply migrations");
        conn
    }

    fn source(id: &str, class: AccessClass) -> SourceRecord {
        SourceRecord {
            id: id.into(),
            name: id.into(),
            access_class: class,
            legal_tier: LegalTier::Safe,
            apply_capability: ApplyCapability::LinkOut,
            attribution: Some(format!("{id} jobs")),
            min_interval_ms: 2000,
            daily_cap: None,
            enabled: true,
            added_at: "2026-10-05T00:00:00Z".into(),
            updated_at: "2026-10-05T00:00:00Z".into(),
        }
    }

    #[test]
    fn migration_creates_the_source_tables() {
        let conn = connection();
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name IN ('source', 'source_run', 'source_optin')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 3);
    }

    #[test]
    fn upsert_round_trips_and_updates_in_place() {
        let conn = connection();
        let mut record = source("remotive", AccessClass::PublicJson);
        record.daily_cap = Some(4);
        upsert_source(&conn, &record).unwrap();

        let loaded = get_source(&conn, "remotive").unwrap().expect("source");
        assert_eq!(loaded, record);
        assert_eq!(loaded.rate_limits().daily_cap, Some(4));

        let mut updated = record.clone();
        updated.name = "Remotive (renamed)".into();
        updated.enabled = false;
        updated.updated_at = "2026-10-06T00:00:00Z".into();
        upsert_source(&conn, &updated).unwrap();

        assert_eq!(list_sources(&conn).unwrap().len(), 1, "upsert, not insert");
        let loaded = get_source(&conn, "remotive").unwrap().unwrap();
        assert_eq!(loaded.name, "Remotive (renamed)");
        assert!(!loaded.enabled);
        assert_eq!(loaded.added_at, "2026-10-05T00:00:00Z", "added_at is kept");
    }

    #[test]
    fn opt_in_is_recorded_and_listed() {
        let conn = connection();
        upsert_source(&conn, &source("linkedin", AccessClass::SessionBrowser)).unwrap();

        assert!(get_optin(&conn, "linkedin").unwrap().is_none());
        assert!(opted_in_ids(&conn).unwrap().is_empty());

        let optin = OptInRecord {
            source_id: "linkedin".into(),
            warning_version: "v1".into(),
            accepted_at: "2026-10-05T00:00:00Z".into(),
        };
        record_optin(&conn, &optin).unwrap();
        assert_eq!(get_optin(&conn, "linkedin").unwrap().unwrap(), optin);
        assert_eq!(opted_in_ids(&conn).unwrap(), vec!["linkedin".to_string()]);
    }

    #[test]
    fn opt_in_requires_a_known_source() {
        let conn = connection();
        let optin = OptInRecord {
            source_id: "ghost".into(),
            warning_version: "v1".into(),
            accepted_at: "2026-10-05T00:00:00Z".into(),
        };
        assert!(record_optin(&conn, &optin).is_err(), "FK must reject");
    }

    #[test]
    fn auto_opt_in_is_separate_and_round_trips() {
        let conn = connection();
        upsert_source(&conn, &source("linkedin", AccessClass::SessionBrowser)).unwrap();

        assert!(auto_opt_in(&conn, "linkedin").unwrap().is_none());
        assert!(
            get_optin(&conn, "linkedin").unwrap().is_none(),
            "the Auto opt-in is not the reading one"
        );

        let optin = SessionAutoOptIn {
            source_id: "linkedin".into(),
            confirmed_name: "linkedin".into(),
            warning_version: "v1".into(),
            accepted_at: "2026-10-17T09:00:00Z".into(),
        };
        record_auto_opt_in(&conn, &optin).unwrap();
        assert_eq!(auto_opt_in(&conn, "linkedin").unwrap().unwrap(), optin);
        assert!(
            get_optin(&conn, "linkedin").unwrap().is_none(),
            "recording Auto does not record reading"
        );

        let re_confirmed = SessionAutoOptIn {
            accepted_at: "2026-10-18T09:00:00Z".into(),
            ..optin.clone()
        };
        record_auto_opt_in(&conn, &re_confirmed).unwrap();
        assert_eq!(
            auto_opt_in(&conn, "linkedin").unwrap().unwrap(),
            re_confirmed,
            "a re-confirmation replaces the row"
        );

        remove_auto_opt_in(&conn, "linkedin").unwrap();
        assert!(auto_opt_in(&conn, "linkedin").unwrap().is_none());
        remove_auto_opt_in(&conn, "linkedin").unwrap();
    }

    #[test]
    fn auto_opt_in_requires_a_known_source() {
        let conn = connection();
        let optin = SessionAutoOptIn {
            source_id: "ghost".into(),
            confirmed_name: "ghost".into(),
            warning_version: "v1".into(),
            accepted_at: "2026-10-17T09:00:00Z".into(),
        };
        assert!(record_auto_opt_in(&conn, &optin).is_err(), "FK must reject");
    }

    #[test]
    fn the_auto_pause_deadline_lives_on_the_source_row() {
        let conn = connection();
        upsert_source(&conn, &source("linkedin", AccessClass::SessionBrowser)).unwrap();
        assert_eq!(auto_paused_until(&conn, "linkedin").unwrap(), None);

        set_auto_paused_until(
            &conn,
            "linkedin",
            Some("2026-10-24T09:00:00Z"),
            "2026-10-17T09:00:00Z",
        )
        .unwrap();
        assert_eq!(
            auto_paused_until(&conn, "linkedin").unwrap().as_deref(),
            Some("2026-10-24T09:00:00Z")
        );

        set_auto_paused_until(&conn, "linkedin", None, "2026-10-25T09:00:00Z").unwrap();
        assert_eq!(auto_paused_until(&conn, "linkedin").unwrap(), None);

        assert!(
            set_auto_paused_until(&conn, "ghost", None, "2026-10-25T09:00:00Z").is_err(),
            "an unknown source is refused, not created"
        );
    }

    #[test]
    fn run_lifecycle_is_recorded() {
        let conn = connection();
        upsert_source(&conn, &source("remotive", AccessClass::PublicJson)).unwrap();
        let run = SourceRunRecord {
            id: "run_1".into(),
            source_id: "remotive".into(),
            status: RunStatus::Running,
            started_at: "2026-10-05T00:00:00Z".into(),
            finished_at: None,
            postings_found: 0,
            error: None,
        };
        start_run(&conn, &run).unwrap();
        finish_run(
            &conn,
            "run_1",
            "2026-10-05T00:01:00Z",
            RunStatus::Ok,
            12,
            None,
        )
        .unwrap();

        let (status, found): (String, i64) = conn
            .query_row(
                "SELECT status, postings_found FROM source_run WHERE id = 'run_1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "ok");
        assert_eq!(found, 12);

        assert!(finish_run(&conn, "missing", "t", RunStatus::Failed, 0, Some("boom")).is_err());
    }

    #[test]
    fn recent_runs_returns_the_newest_first_and_caps_the_limit() {
        let conn = connection();
        upsert_source(&conn, &source("remotive", AccessClass::PublicJson)).unwrap();
        upsert_source(&conn, &source("remoteok", AccessClass::PublicJson)).unwrap();
        for (id, day, status) in [
            ("run_1", "01", RunStatus::Ok),
            ("run_2", "02", RunStatus::Failed),
            ("run_3", "03", RunStatus::Ok),
        ] {
            start_run(
                &conn,
                &SourceRunRecord {
                    id: id.into(),
                    source_id: "remotive".into(),
                    status: RunStatus::Running,
                    started_at: format!("2026-10-{day}T00:00:00Z"),
                    finished_at: None,
                    postings_found: 0,
                    error: None,
                },
            )
            .unwrap();
            finish_run(
                &conn,
                id,
                &format!("2026-10-{day}T00:01:00Z"),
                status,
                5,
                None,
            )
            .unwrap();
        }
        start_run(
            &conn,
            &SourceRunRecord {
                id: "other".into(),
                source_id: "remoteok".into(),
                status: RunStatus::Ok,
                started_at: "2026-10-04T00:00:00Z".into(),
                finished_at: Some("2026-10-04T00:01:00Z".into()),
                postings_found: 9,
                error: None,
            },
        )
        .unwrap();

        let runs = recent_runs(&conn, "remotive", 2).unwrap();
        assert_eq!(runs.len(), 2, "the limit caps the rows");
        assert_eq!(runs[0].id, "run_3", "newest first");
        assert_eq!(runs[1].id, "run_2");
        assert_eq!(runs[0].postings_found, 5);
        assert_eq!(runs[0].status, RunStatus::Ok);

        let all = recent_runs(&conn, "remotive", 10).unwrap();
        assert_eq!(all.len(), 3, "only this source's runs");
        assert_eq!(recent_runs(&conn, "ghost", 10).unwrap(), Vec::new());
    }

    #[test]
    fn check_constraints_reject_bad_values() {
        let conn = connection();
        conn.execute(
            "INSERT INTO source (id, name, access_class, legal_tier, apply_capability, \
             min_interval_ms, enabled, added_at, updated_at) \
             VALUES ('x', 'x', 'telepathy', 'safe', 'none', 2000, 1, 't', 't')",
            [],
        )
        .expect_err("bad access_class must be rejected");

        assert!(get_source(&conn, "missing").unwrap().is_none());
        assert!(set_source_enabled(&conn, "missing", false, "t").is_err());
    }
}
