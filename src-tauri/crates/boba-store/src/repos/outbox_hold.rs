//! Apply Outbox repository (S9.7): the single writer of `apply_outbox` and
//! `apply_rule_dry_run` (AD-16).
//!
//! The table is one queued automatic send; its `state` is the release policy's
//! whole answer, so every move is a single `UPDATE` and an unknown id is an
//! error rather than a silent no-op. `OutboxHoldRepo` is implemented for
//! [`Store`], so the use case depends on the port, not this module.
//!
//! Only what the user changed during the hold travels with the row
//! (`subject_override`, `body_override`); the message itself is composed when the
//! release happens, and the attempt row keeps only its blake3 (AD-12).

use rusqlite::{Connection, OptionalExtension, params};

use boba_app::ports::repo::outbox_hold::{OutboxHoldRepo, OutboxHoldRepoError};
use boba_domain::apply::outbox::{DryRunState, NewOutboxItem, OutboxItem, OutboxState};

use crate::db::{Store, StoreError};

const SELECT: &str = "SELECT id, application_id, rule_id, source_id, channel, company_key, \
     title, company, state, hold_minutes, release_at, subject_override, body_override, note, \
     created_at, updated_at FROM apply_outbox";

fn bad(column: &'static str, value: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::new(StoreError::Sqlite(format!("unknown {column} '{value}'"))),
    )
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<OutboxItem> {
    let state: String = r.get(8)?;
    let hold_minutes: i64 = r.get(9)?;
    Ok(OutboxItem {
        id: r.get(0)?,
        application_id: r.get(1)?,
        rule_id: r.get(2)?,
        source_id: r.get(3)?,
        channel: r.get(4)?,
        company_key: r.get(5)?,
        title: r.get(6)?,
        company: r.get(7)?,
        state: OutboxState::parse(&state).ok_or_else(|| bad("state", state))?,
        hold_minutes: u16::try_from(hold_minutes)
            .map_err(|_| bad("hold_minutes", hold_minutes.to_string()))?,
        release_at: r.get(10)?,
        subject_override: r.get(11)?,
        body_override: r.get(12)?,
        note: r.get(13)?,
        created_at: r.get(14)?,
        updated_at: r.get(15)?,
    })
}

fn repo(error: rusqlite::Error) -> OutboxHoldRepoError {
    OutboxHoldRepoError::new(error.to_string())
}

/// Record one queued item; the live index refuses a second open one for the
/// same application.
pub fn insert(
    conn: &Connection,
    item: &NewOutboxItem,
    now: &str,
) -> Result<OutboxItem, OutboxHoldRepoError> {
    let id = format!("out_{}", ulid::Ulid::generate());
    conn.execute(
        "INSERT INTO apply_outbox (id, application_id, rule_id, source_id, channel, \
         company_key, title, company, state, hold_minutes, release_at, subject_override, \
         body_override, note, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL, NULL, NULL, ?12, ?12)",
        params![
            id,
            item.application_id,
            item.rule_id,
            item.source_id,
            item.channel,
            item.company_key,
            item.title,
            item.company,
            item.state.as_str(),
            i64::from(item.hold_minutes),
            item.release_at,
            now,
        ],
    )
    .map_err(repo)?;
    get(conn, &id)?.ok_or_else(|| OutboxHoldRepoError::new(format!("outbox item {id} vanished")))
}

/// One item by id.
pub fn get(conn: &Connection, id: &str) -> Result<Option<OutboxItem>, OutboxHoldRepoError> {
    conn.query_row(&format!("{SELECT} WHERE id = ?1"), params![id], row)
        .optional()
        .map_err(repo)
}

/// The application's one open item.
pub fn live_for(
    conn: &Connection,
    application_id: &str,
) -> Result<Option<OutboxItem>, OutboxHoldRepoError> {
    conn.query_row(
        &format!(
            "{SELECT} WHERE application_id = ?1 \
             AND state IN ('would_send', 'held', 'releasing')"
        ),
        params![application_id],
        row,
    )
    .optional()
    .map_err(repo)
}

/// The most recent items, newest first.
pub fn list(conn: &Connection, limit: usize) -> Result<Vec<OutboxItem>, OutboxHoldRepoError> {
    let mut stmt = conn
        .prepare(&format!(
            "{SELECT} ORDER BY created_at DESC, rowid DESC LIMIT ?1"
        ))
        .map_err(repo)?;
    let items = stmt
        .query_map(params![i64::try_from(limit).unwrap_or(i64::MAX)], row)
        .map_err(repo)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(repo)?;
    Ok(items)
}

/// The held items whose hold has ended, oldest first (FR-85).
pub fn due(conn: &Connection, now: &str) -> Result<Vec<OutboxItem>, OutboxHoldRepoError> {
    let mut stmt = conn
        .prepare(&format!(
            "{SELECT} WHERE state = 'held' AND release_at <= ?1 ORDER BY release_at, rowid"
        ))
        .map_err(repo)?;
    let items = stmt
        .query_map(params![now], row)
        .map_err(repo)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(repo)?;
    Ok(items)
}

/// The items a crash left mid-release.
pub fn releasing(conn: &Connection) -> Result<Vec<OutboxItem>, OutboxHoldRepoError> {
    let mut stmt = conn
        .prepare(&format!(
            "{SELECT} WHERE state = 'releasing' ORDER BY created_at, rowid"
        ))
        .map_err(repo)?;
    let items = stmt
        .query_map([], row)
        .map_err(repo)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(repo)?;
    Ok(items)
}

/// Claim one held item for one releaser (the concurrency guard of the release
/// sweep). Returns `false` when another caller already moved it.
pub fn claim(conn: &Connection, id: &str, now: &str) -> Result<bool, OutboxHoldRepoError> {
    let changed = conn
        .execute(
            "UPDATE apply_outbox SET state = 'releasing', updated_at = ?2 \
             WHERE id = ?1 AND state = 'held'",
            params![id, now],
        )
        .map_err(repo)?;
    Ok(changed == 1)
}

/// Move one item to `state`.
pub fn set_state(
    conn: &Connection,
    id: &str,
    state: OutboxState,
    note: Option<&str>,
    now: &str,
) -> Result<OutboxItem, OutboxHoldRepoError> {
    let changed = conn
        .execute(
            "UPDATE apply_outbox SET state = ?2, note = COALESCE(?3, note), updated_at = ?4 \
             WHERE id = ?1",
            params![id, state.as_str(), note, now],
        )
        .map_err(repo)?;
    if changed == 0 {
        return Err(OutboxHoldRepoError::new(format!(
            "unknown outbox item {id}"
        )));
    }
    get(conn, id)?.ok_or_else(|| OutboxHoldRepoError::new(format!("unknown outbox item {id}")))
}

/// Replace the user's edits; `None` clears one.
pub fn set_overrides(
    conn: &Connection,
    id: &str,
    subject: Option<&str>,
    body: Option<&str>,
    now: &str,
) -> Result<OutboxItem, OutboxHoldRepoError> {
    let changed = conn
        .execute(
            "UPDATE apply_outbox SET subject_override = ?2, body_override = ?3, updated_at = ?4 \
             WHERE id = ?1",
            params![id, subject, body, now],
        )
        .map_err(repo)?;
    if changed == 0 {
        return Err(OutboxHoldRepoError::new(format!(
            "unknown outbox item {id}"
        )));
    }
    get(conn, id)?.ok_or_else(|| OutboxHoldRepoError::new(format!("unknown outbox item {id}")))
}

/// One rule's dry-run progress.
pub fn dry_run_get(
    conn: &Connection,
    rule_id: &str,
) -> Result<Option<DryRunState>, OutboxHoldRepoError> {
    conn.query_row(
        "SELECT rule_id, required, seen, activated_at FROM apply_rule_dry_run WHERE rule_id = ?1",
        params![rule_id],
        |r| {
            let required: i64 = r.get(1)?;
            let seen: i64 = r.get(2)?;
            Ok(DryRunState {
                rule_id: r.get(0)?,
                required: u8::try_from(required)
                    .map_err(|_| bad("required", required.to_string()))?,
                seen: u8::try_from(seen).map_err(|_| bad("seen", seen.to_string()))?,
                activated_at: r.get(3)?,
            })
        },
    )
    .optional()
    .map_err(repo)
}

/// Every stored dry-run row.
pub fn dry_run_list(conn: &Connection) -> Result<Vec<DryRunState>, OutboxHoldRepoError> {
    let mut stmt = conn
        .prepare(
            "SELECT rule_id, required, seen, activated_at FROM apply_rule_dry_run \
             ORDER BY rule_id",
        )
        .map_err(repo)?;
    let states = stmt
        .query_map([], |r| {
            let required: i64 = r.get(1)?;
            let seen: i64 = r.get(2)?;
            Ok(DryRunState {
                rule_id: r.get(0)?,
                required: u8::try_from(required)
                    .map_err(|_| bad("required", required.to_string()))?,
                seen: u8::try_from(seen).map_err(|_| bad("seen", seen.to_string()))?,
                activated_at: r.get(3)?,
            })
        })
        .map_err(repo)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(repo)?;
    Ok(states)
}

/// Insert or replace one rule's dry-run progress.
pub fn dry_run_upsert(
    conn: &Connection,
    state: &DryRunState,
    now: &str,
) -> Result<(), OutboxHoldRepoError> {
    conn.execute(
        "INSERT INTO apply_rule_dry_run (rule_id, required, seen, activated_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5) \
         ON CONFLICT(rule_id) DO UPDATE SET required = excluded.required, seen = excluded.seen, \
         activated_at = excluded.activated_at, updated_at = excluded.updated_at",
        params![
            state.rule_id,
            i64::from(state.required),
            i64::from(state.seen),
            state.activated_at,
            now,
        ],
    )
    .map_err(repo)?;
    Ok(())
}

impl OutboxHoldRepo for Store {
    fn outbox_insert(
        &self,
        item: &NewOutboxItem,
        now: &str,
    ) -> Result<OutboxItem, OutboxHoldRepoError> {
        let (item, now) = (item.clone(), now.to_string());
        self.execute(move |conn| Ok(insert(conn, &item, &now)))
            .map_err(|e| OutboxHoldRepoError::new(e.to_string()))?
    }

    fn outbox_get(&self, id: &str) -> Result<Option<OutboxItem>, OutboxHoldRepoError> {
        let id = id.to_string();
        self.read(move |conn| Ok(get(conn, &id)))
            .map_err(|e| OutboxHoldRepoError::new(e.to_string()))?
    }

    fn outbox_live_for(
        &self,
        application_id: &str,
    ) -> Result<Option<OutboxItem>, OutboxHoldRepoError> {
        let application_id = application_id.to_string();
        self.read(move |conn| Ok(live_for(conn, &application_id)))
            .map_err(|e| OutboxHoldRepoError::new(e.to_string()))?
    }

    fn outbox_list(&self, limit: usize) -> Result<Vec<OutboxItem>, OutboxHoldRepoError> {
        self.read(move |conn| Ok(list(conn, limit)))
            .map_err(|e| OutboxHoldRepoError::new(e.to_string()))?
    }

    fn outbox_due(&self, now: &str) -> Result<Vec<OutboxItem>, OutboxHoldRepoError> {
        let now = now.to_string();
        self.read(move |conn| Ok(due(conn, &now)))
            .map_err(|e| OutboxHoldRepoError::new(e.to_string()))?
    }

    fn outbox_releasing(&self) -> Result<Vec<OutboxItem>, OutboxHoldRepoError> {
        self.read(|conn| Ok(releasing(conn)))
            .map_err(|e| OutboxHoldRepoError::new(e.to_string()))?
    }

    fn outbox_claim(&self, id: &str, now: &str) -> Result<bool, OutboxHoldRepoError> {
        let (id, now) = (id.to_string(), now.to_string());
        self.execute(move |conn| Ok(claim(conn, &id, &now)))
            .map_err(|e| OutboxHoldRepoError::new(e.to_string()))?
    }

    fn outbox_set_state(
        &self,
        id: &str,
        state: OutboxState,
        note: Option<&str>,
        now: &str,
    ) -> Result<OutboxItem, OutboxHoldRepoError> {
        let (id, note, now) = (id.to_string(), note.map(str::to_string), now.to_string());
        self.execute(move |conn| Ok(set_state(conn, &id, state, note.as_deref(), &now)))
            .map_err(|e| OutboxHoldRepoError::new(e.to_string()))?
    }

    fn outbox_set_overrides(
        &self,
        id: &str,
        subject: Option<&str>,
        body: Option<&str>,
        now: &str,
    ) -> Result<OutboxItem, OutboxHoldRepoError> {
        let (id, now) = (id.to_string(), now.to_string());
        let (subject, body) = (subject.map(str::to_string), body.map(str::to_string));
        self.execute(move |conn| {
            Ok(set_overrides(
                conn,
                &id,
                subject.as_deref(),
                body.as_deref(),
                &now,
            ))
        })
        .map_err(|e| OutboxHoldRepoError::new(e.to_string()))?
    }

    fn dry_run_get(&self, rule_id: &str) -> Result<Option<DryRunState>, OutboxHoldRepoError> {
        let rule_id = rule_id.to_string();
        self.read(move |conn| Ok(dry_run_get(conn, &rule_id)))
            .map_err(|e| OutboxHoldRepoError::new(e.to_string()))?
    }

    fn dry_run_list(&self) -> Result<Vec<DryRunState>, OutboxHoldRepoError> {
        self.read(|conn| Ok(dry_run_list(conn)))
            .map_err(|e| OutboxHoldRepoError::new(e.to_string()))?
    }

    fn dry_run_upsert(&self, state: &DryRunState, now: &str) -> Result<(), OutboxHoldRepoError> {
        let (state, now) = (state.clone(), now.to_string());
        self.execute(move |conn| Ok(dry_run_upsert(conn, &state, &now)))
            .map_err(|e| OutboxHoldRepoError::new(e.to_string()))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate;

    const NOW: &str = "2026-10-17T09:00:00Z";
    const LATER: &str = "2026-10-17T09:30:00Z";

    fn connection() -> Connection {
        let mut conn = Connection::open_in_memory().expect("in-memory sqlite");
        migrate::embedded()
            .to_latest(&mut conn)
            .expect("apply migrations");
        conn.execute(
            "INSERT INTO auto_rule (id, name, enabled, source_ids, channels, min_rank, min_fit, \
             min_confidence, allow_flags, verdict, daily_cap, per_company_cap, per_company_days, \
             active_start, active_end, created_at, updated_at) \
             VALUES ('rule_1', 'Email × remote', 1, '[\"remotive\"]', '[\"email\"]', 80, 80, \
             'medium', 0, 'good', 5, 1, 30, 540, 1080, 't', 't')",
            [],
        )
        .expect("seed rule");
        conn
    }

    fn item(application_id: &str, state: OutboxState, release_at: &str) -> NewOutboxItem {
        NewOutboxItem {
            application_id: application_id.to_string(),
            rule_id: "rule_1".to_string(),
            source_id: "remotive".to_string(),
            channel: "email".to_string(),
            company_key: "acme".to_string(),
            title: "Backend Engineer".to_string(),
            company: "Acme".to_string(),
            state,
            hold_minutes: 30,
            release_at: release_at.to_string(),
        }
    }

    #[test]
    fn a_queued_item_is_stored_with_its_prefixed_id_and_reads_back() {
        let conn = connection();
        let stored = insert(&conn, &item("app_01", OutboxState::Held, LATER), NOW).unwrap();

        assert!(stored.id.starts_with("out_"), "a prefixed ULID (AD-17)");
        assert_eq!(stored.state, OutboxState::Held);
        assert_eq!(stored.title, "Backend Engineer");
        assert_eq!(stored.company, "Acme");
        assert_eq!(stored.hold_minutes, 30);
        assert_eq!(stored.release_at, LATER);
        assert_eq!(stored.subject_override, None);
        assert_eq!(stored.created_at, NOW);
        assert_eq!(get(&conn, &stored.id).unwrap(), Some(stored));
    }

    #[test]
    fn an_application_holds_at_most_one_open_item() {
        let conn = connection();
        insert(&conn, &item("app_01", OutboxState::Held, LATER), NOW).unwrap();
        insert(&conn, &item("app_01", OutboxState::Held, LATER), NOW)
            .expect_err("the live index refuses a second open item");

        // Once the first left the path, the application may be queued again.
        let first = live_for(&conn, "app_01").unwrap().unwrap();
        set_state(&conn, &first.id, OutboxState::Cancelled, Some("user"), NOW).unwrap();
        assert!(live_for(&conn, "app_01").unwrap().is_none());
        insert(&conn, &item("app_01", OutboxState::Held, LATER), NOW).unwrap();
    }

    #[test]
    fn only_a_held_item_whose_time_has_come_is_due() {
        let conn = connection();
        let early = insert(&conn, &item("app_01", OutboxState::Held, LATER), NOW).unwrap();
        insert(
            &conn,
            &item("app_02", OutboxState::Held, "2026-10-17T10:00:00Z"),
            NOW,
        )
        .unwrap();
        // A dry run is never due, however old it is.
        insert(&conn, &item("app_03", OutboxState::WouldSend, NOW), NOW).unwrap();

        assert!(due(&conn, NOW).unwrap().is_empty(), "the hold still runs");
        let due_now = due(&conn, LATER).unwrap();
        assert_eq!(due_now.len(), 1);
        assert_eq!(due_now[0].id, early.id);
    }

    #[test]
    fn a_state_move_is_an_error_for_an_unknown_id_not_a_silent_write() {
        let conn = connection();
        let error = set_state(&conn, "out_missing", OutboxState::Sent, None, NOW).unwrap_err();
        assert!(error.to_string().contains("out_missing"));
    }

    #[test]
    fn only_one_releaser_can_claim_an_item() {
        let conn = connection();
        let item = insert(&conn, &item("app_01", OutboxState::Held, NOW), NOW).unwrap();

        assert!(claim(&conn, &item.id, NOW).unwrap());
        assert!(
            !claim(&conn, &item.id, NOW).unwrap(),
            "the second sweeper loses the item"
        );
        assert_eq!(
            get(&conn, &item.id).unwrap().unwrap().state,
            OutboxState::Releasing
        );
    }

    #[test]
    fn a_releasing_item_is_found_for_the_startup_reconcile() {
        let conn = connection();
        let item = insert(&conn, &item("app_01", OutboxState::Held, NOW), NOW).unwrap();
        set_state(&conn, &item.id, OutboxState::Releasing, None, NOW).unwrap();

        let releasing = releasing(&conn).unwrap();
        assert_eq!(releasing.len(), 1);
        assert_eq!(releasing[0].id, item.id);
    }

    #[test]
    fn the_users_edits_survive_and_none_clears_them() {
        let conn = connection();
        let item = insert(&conn, &item("app_01", OutboxState::Held, LATER), NOW).unwrap();

        let edited = set_overrides(&conn, &item.id, Some("Hi"), Some("Body"), NOW).unwrap();
        assert_eq!(edited.subject_override.as_deref(), Some("Hi"));
        assert_eq!(edited.body_override.as_deref(), Some("Body"));

        let cleared = set_overrides(&conn, &item.id, None, None, NOW).unwrap();
        assert_eq!(cleared.subject_override, None);
        assert_eq!(cleared.body_override, None);
    }

    #[test]
    fn a_rule_with_no_dry_run_row_has_none_and_one_is_written_after_its_first_match() {
        let conn = connection();
        assert_eq!(dry_run_get(&conn, "rule_1").unwrap(), None);

        let mut state = DryRunState::new("rule_1");
        state.seen = 1;
        dry_run_upsert(&conn, &state, NOW).unwrap();

        assert_eq!(dry_run_get(&conn, "rule_1").unwrap(), Some(state.clone()));
        assert_eq!(dry_run_list(&conn).unwrap(), vec![state.clone()]);

        state.activated_at = Some(NOW.to_string());
        dry_run_upsert(&conn, &state, LATER).unwrap();
        assert_eq!(dry_run_get(&conn, "rule_1").unwrap(), Some(state));
    }

    #[test]
    fn the_dry_run_bounds_are_checked_by_the_table_too() {
        let conn = connection();
        let mut state = DryRunState::new("rule_1");
        state.required = 11;
        dry_run_upsert(&conn, &state, NOW).expect_err("the CHECK refuses 11");
        state.required = 0;
        dry_run_upsert(&conn, &state, NOW).expect_err("the CHECK refuses 0");
    }
}
