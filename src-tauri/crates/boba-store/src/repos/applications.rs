//! Applications repository (S9.1): the single writer of `application` and
//! `status_event` (AD-16).
//!
//! The row and its event are always written together, in one transaction, and a
//! move only lands when the stored status is still the one the caller saw
//! (AC4). `ApplyRepo` is implemented for [`Store`], so use cases depend on the
//! port, not this module.

use rusqlite::{Connection, OptionalExtension, params};

use boba_app::ports::repo::apply::{ApplyRepo, ApplyRepoError};
use boba_domain::application::{AppStatus, Application, PackageItem, StatusEvent, StatusSource};

use crate::db::{Store, StoreError};

const SELECT: &str = "SELECT id, posting_id, status, offer_snapshot_json, notes, next_action, \
     created_at, updated_at FROM application";

const EVENT_SELECT: &str = "SELECT id, application_id, from_status, to_status, source, reason, \
     created_at FROM status_event";

fn bad(column: &'static str, value: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::new(StoreError::Sqlite(format!("unknown {column} '{value}'"))),
    )
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Application> {
    let status: String = r.get(2)?;
    Ok(Application {
        id: r.get(0)?,
        posting_id: r.get(1)?,
        status: AppStatus::parse(&status).ok_or_else(|| bad("status", status))?,
        offer_snapshot_json: r.get(3)?,
        notes: r.get(4)?,
        next_action: r.get(5)?,
        created_at: r.get(6)?,
        updated_at: r.get(7)?,
    })
}

fn event_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<StatusEvent> {
    let from: Option<String> = r.get(2)?;
    let to: String = r.get(3)?;
    let source: String = r.get(4)?;
    Ok(StatusEvent {
        id: r.get(0)?,
        application_id: r.get(1)?,
        from_status: from
            .map(|v| AppStatus::parse(&v).ok_or_else(|| bad("from_status", v)))
            .transpose()?,
        to_status: AppStatus::parse(&to).ok_or_else(|| bad("to_status", to))?,
        source: StatusSource::parse(&source).ok_or_else(|| bad("source", source))?,
        reason: r.get(5)?,
        created_at: r.get(6)?,
    })
}

fn store(error: impl std::fmt::Display) -> ApplyRepoError {
    ApplyRepoError::Store(error.to_string())
}

/// Insert one application and its birth event, in one transaction (AC4).
pub fn create_application(
    conn: &Connection,
    application: &Application,
    event: &StatusEvent,
) -> Result<(), ApplyRepoError> {
    let tx = conn.unchecked_transaction().map_err(store)?;
    tx.execute(
        "INSERT INTO application (id, posting_id, status, offer_snapshot_json, notes, \
         next_action, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            application.id,
            application.posting_id,
            application.status.as_str(),
            application.offer_snapshot_json,
            application.notes,
            application.next_action,
            application.created_at,
            application.updated_at,
        ],
    )
    .map_err(store)?;
    insert_event(&tx, event)?;
    tx.commit().map_err(store)
}

/// One application by id.
pub fn get_application(conn: &Connection, id: &str) -> Result<Option<Application>, ApplyRepoError> {
    conn.query_row(&format!("{SELECT} WHERE id = ?1"), params![id], row)
        .optional()
        .map_err(store)
}

/// Every application, oldest first (the board groups them by status).
pub fn list_applications(conn: &Connection) -> Result<Vec<Application>, ApplyRepoError> {
    let mut stmt = conn
        .prepare(&format!("{SELECT} ORDER BY created_at, id"))
        .map_err(store)?;
    stmt.query_map([], row)
        .map_err(store)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(store)
}

/// One application's history, oldest first (AC3 timeline).
pub fn list_status_events(
    conn: &Connection,
    application_id: &str,
) -> Result<Vec<StatusEvent>, ApplyRepoError> {
    let mut stmt = conn
        .prepare(&format!(
            "{EVENT_SELECT} WHERE application_id = ?1 ORDER BY created_at, id"
        ))
        .map_err(store)?;
    stmt.query_map(params![application_id], event_row)
        .map_err(store)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(store)
}

/// The documents this application holds, through the documents owner (AD-16).
pub fn list_package(
    conn: &Connection,
    application_id: &str,
) -> Result<Vec<PackageItem>, ApplyRepoError> {
    let documents =
        crate::repos::documents::list_for_application(conn, application_id).map_err(store)?;
    Ok(documents
        .into_iter()
        .map(|document| PackageItem {
            document_id: document.id,
            kind: document.kind.as_str().to_string(),
            lang: document.lang.code().to_string(),
            version: document.version,
            verified_at: document.verified_at,
        })
        .collect())
}

/// Move one application and append its event in one transaction (AC2/AC4).
///
/// The `WHERE status = ?from` clause is the race guard: when another actor
/// (a Status Proposal, another window) moved the card first, nothing is written
/// and the caller is told which status it actually found.
pub fn append_status_event(conn: &Connection, event: &StatusEvent) -> Result<(), ApplyRepoError> {
    let tx = conn.unchecked_transaction().map_err(store)?;
    let changed = match event.from_status {
        Some(from) => tx
            .execute(
                "UPDATE application SET status = ?2, updated_at = ?3 WHERE id = ?1 AND status = ?4",
                params![
                    event.application_id,
                    event.to_status.as_str(),
                    event.created_at,
                    from.as_str(),
                ],
            )
            .map_err(store)?,
        // No expectation: the caller only records the event (used with the row
        // that already carries the status).
        None => 0,
    };

    if event.from_status.is_some() && changed == 0 {
        let found: Option<String> = tx
            .query_row(
                "SELECT status FROM application WHERE id = ?1",
                params![event.application_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(store)?;
        // Dropping the transaction rolls the read back with it: nothing was written.
        return match found {
            None => Err(ApplyRepoError::UnknownApplication(
                event.application_id.clone(),
            )),
            Some(found) => Err(ApplyRepoError::Conflict {
                expected: event
                    .from_status
                    .map(AppStatus::as_str)
                    .unwrap_or_default()
                    .to_string(),
                found,
            }),
        };
    }

    insert_event(&tx, event)?;
    tx.commit().map_err(store)
}

fn insert_event(conn: &Connection, event: &StatusEvent) -> Result<(), ApplyRepoError> {
    conn.execute(
        "INSERT INTO status_event (id, application_id, from_status, to_status, source, reason, \
         created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            event.id,
            event.application_id,
            event.from_status.map(AppStatus::as_str),
            event.to_status.as_str(),
            event.source.as_str(),
            event.reason,
            event.created_at,
        ],
    )
    .map_err(store)?;
    Ok(())
}

impl ApplyRepo for Store {
    fn applications(&self) -> Result<Vec<Application>, ApplyRepoError> {
        self.read(|conn| Ok(list_applications(conn)))
            .map_err(|e| ApplyRepoError::Store(e.to_string()))?
    }

    fn application_get(&self, id: &str) -> Result<Option<Application>, ApplyRepoError> {
        let id = id.to_string();
        self.read(move |conn| Ok(get_application(conn, &id)))
            .map_err(|e| ApplyRepoError::Store(e.to_string()))?
    }

    fn status_events(&self, application_id: &str) -> Result<Vec<StatusEvent>, ApplyRepoError> {
        let application_id = application_id.to_string();
        self.read(move |conn| Ok(list_status_events(conn, &application_id)))
            .map_err(|e| ApplyRepoError::Store(e.to_string()))?
    }

    fn package(&self, application_id: &str) -> Result<Vec<PackageItem>, ApplyRepoError> {
        let application_id = application_id.to_string();
        self.read(move |conn| Ok(list_package(conn, &application_id)))
            .map_err(|e| ApplyRepoError::Store(e.to_string()))?
    }

    fn append_status_event(&self, event: &StatusEvent) -> Result<(), ApplyRepoError> {
        let event = event.clone();
        self.execute(move |conn| Ok(append_status_event(conn, &event)))
            .map_err(|e| ApplyRepoError::Store(e.to_string()))?
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use boba_domain::application::{
        AppStatus, Application, PackageItem, StatusEvent, StatusSource,
    };
    use boba_domain::documents::{DocumentKind, Lang, NewDocument, TemplateId};

    use super::*;
    use crate::migrate;

    fn connection() -> Connection {
        let mut conn = Connection::open_in_memory().expect("in-memory sqlite");
        migrate::embedded()
            .to_latest(&mut conn)
            .expect("apply migrations");
        conn.execute(
            "INSERT INTO source (id, name, access_class, legal_tier, apply_capability, \
             min_interval_ms, enabled, added_at, updated_at) \
             VALUES ('remotive', 'Remotive', 'public_json', 'safe', 'link_out', 2000, 1, 't', 't')",
            [],
        )
        .expect("seed source");
        conn.execute(
            "INSERT INTO posting (id, source_id, external_id, title, company, location_json, \
             work_mode, posted_at, deadline, first_seen, last_seen, lang, description_html, \
             description_text, requirements_json, apply_url, apply_email, dedup_key, simhash) \
             VALUES ('pst_1', 'remotive', 'abc-1', 'Backend Engineer', 'Boba Labs', '[]', \
             'remote', NULL, NULL, 't', 't', 'en', '', '', '[]', \
             'https://jobs.example.com/abc-1', NULL, 'k1', NULL)",
            [],
        )
        .expect("seed posting");
        conn
    }

    const SNAPSHOT: &str = r#"{"title":"Backend Engineer","company":"Boba Labs"}"#;

    fn application(status: AppStatus) -> Application {
        Application {
            id: "app_01".into(),
            posting_id: "pst_1".into(),
            status,
            offer_snapshot_json: SNAPSHOT.into(),
            notes: Some("referred by Ana".into()),
            next_action: Some("follow up on Friday".into()),
            created_at: "2026-10-16T09:00:00Z".into(),
            updated_at: "2026-10-16T09:00:00Z".into(),
        }
    }

    fn birth_event() -> StatusEvent {
        StatusEvent {
            id: "evt_01".into(),
            application_id: "app_01".into(),
            from_status: None,
            to_status: AppStatus::Saved,
            source: StatusSource::User,
            reason: None,
            created_at: "2026-10-16T09:00:00Z".into(),
        }
    }

    fn move_event(id: &str, from: AppStatus, to: AppStatus, at: &str) -> StatusEvent {
        StatusEvent {
            id: id.into(),
            application_id: "app_01".into(),
            from_status: Some(from),
            to_status: to,
            source: StatusSource::User,
            reason: None,
            created_at: at.into(),
        }
    }

    #[test]
    fn migration_creates_the_apply_tables() {
        let conn = connection();
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name IN ('application', 'status_event')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn a_stored_event_never_moves_a_card_to_where_it_already_is() {
        let conn = connection();
        let bad = move_event("evt_bad", AppStatus::Applied, AppStatus::Applied, "t");
        let err = conn.execute(
            "INSERT INTO status_event (id, application_id, from_status, to_status, source, \
             reason, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                bad.id,
                bad.application_id,
                AppStatus::Applied.as_str(),
                AppStatus::Applied.as_str(),
                bad.source.as_str(),
                bad.reason,
                bad.created_at,
            ],
        );
        assert!(err.is_err(), "from_status = to_status must be rejected");
    }

    #[test]
    fn create_round_trips_the_row_and_its_birth_event() {
        let conn = connection();
        create_application(&conn, &application(AppStatus::Saved), &birth_event()).expect("create");

        let loaded = get_application(&conn, "app_01").unwrap().expect("row");
        assert_eq!(loaded, application(AppStatus::Saved));
        assert_eq!(loaded.offer_snapshot_json, SNAPSHOT);

        let events = list_status_events(&conn, "app_01").unwrap();
        assert_eq!(events, vec![birth_event()]);
    }

    #[test]
    fn a_move_updates_the_row_and_appends_the_event_in_one_transaction() {
        let conn = connection();
        create_application(&conn, &application(AppStatus::Saved), &birth_event()).expect("create");

        let event = move_event(
            "evt_02",
            AppStatus::Saved,
            AppStatus::Applied,
            "2026-10-17T10:00:00Z",
        );
        append_status_event(&conn, &event).expect("move");

        let loaded = get_application(&conn, "app_01").unwrap().expect("row");
        assert_eq!(loaded.status, AppStatus::Applied);
        assert_eq!(loaded.updated_at, "2026-10-17T10:00:00Z");
        assert_eq!(loaded.notes.as_deref(), Some("referred by Ana"));

        let events = list_status_events(&conn, "app_01").unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1], event);
    }

    #[test]
    fn a_move_that_races_lost_reports_the_status_it_found() {
        let conn = connection();
        create_application(&conn, &application(AppStatus::Saved), &birth_event()).expect("create");
        let first = move_event("evt_02", AppStatus::Saved, AppStatus::Applied, "1");
        append_status_event(&conn, &first).expect("first move");

        // A second window still shows the card as Saved and drops it on Offer.
        let stale = move_event("evt_03", AppStatus::Saved, AppStatus::Offer, "2");
        let err = append_status_event(&conn, &stale).unwrap_err();
        assert_eq!(
            err,
            ApplyRepoError::Conflict {
                expected: "saved".into(),
                found: "applied".into(),
            }
        );
        // The losing move left nothing behind.
        assert_eq!(
            get_application(&conn, "app_01").unwrap().unwrap().status,
            AppStatus::Applied
        );
        assert_eq!(list_status_events(&conn, "app_01").unwrap().len(), 2);
    }

    #[test]
    fn moving_an_application_that_does_not_exist_is_unknown() {
        let conn = connection();
        let event = move_event("evt_02", AppStatus::Saved, AppStatus::Applied, "1");
        let err = append_status_event(&conn, &event).unwrap_err();
        assert_eq!(err, ApplyRepoError::UnknownApplication("app_01".into()));
    }

    #[test]
    fn events_and_applications_come_back_oldest_first() {
        let conn = connection();
        create_application(&conn, &application(AppStatus::Saved), &birth_event()).expect("first");
        let mut second = application(AppStatus::Saved);
        second.id = "app_02".into();
        second.created_at = "2026-10-16T11:00:00Z".into();
        second.updated_at = second.created_at.clone();
        let mut event = birth_event();
        event.id = "evt_02".into();
        event.application_id = "app_02".into();
        event.created_at = second.created_at.clone();
        create_application(&conn, &second, &event).expect("second");

        let ids: Vec<String> = list_applications(&conn)
            .unwrap()
            .into_iter()
            .map(|a| a.id)
            .collect();
        assert_eq!(ids, vec!["app_01".to_string(), "app_02".to_string()]);
    }

    #[test]
    fn the_package_is_read_through_the_documents_owner() {
        let conn = connection();
        create_application(&conn, &application(AppStatus::Saved), &birth_event()).expect("create");
        for (id, kind) in [
            ("doc_1", DocumentKind::Cv),
            ("doc_2", DocumentKind::CoverLetter),
        ] {
            crate::repos::documents::insert(
                &conn,
                &NewDocument {
                    id: id.into(),
                    application_id: "app_01".into(),
                    kind,
                    lang: Lang::En,
                    template: TemplateId::new("classic"),
                    model_json: "{}".into(),
                    untrusted_origin: true,
                    created_at: "2026-10-16T09:00:00Z".into(),
                },
            )
            .expect("document");
        }

        let package = list_package(&conn, "app_01").unwrap();
        assert_eq!(
            package,
            vec![
                PackageItem {
                    document_id: "doc_2".into(),
                    kind: "cover_letter".into(),
                    lang: "en".into(),
                    version: 1,
                    verified_at: None,
                },
                PackageItem {
                    document_id: "doc_1".into(),
                    kind: "cv".into(),
                    lang: "en".into(),
                    version: 1,
                    verified_at: None,
                },
            ]
        );
    }
}
