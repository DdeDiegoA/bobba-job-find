//! Auto Rules repository (S9.6): the single writer of `auto_rule` and
//! `auto_rule_attempt` (AD-16).
//!
//! `AutoRulesRepo` is implemented for [`Store`], so the use cases depend on the
//! port, not this module. The channels and the source scope are JSON arrays in
//! one TEXT column: both are short lists read as a whole, and the rule itself is
//! the unit of work, never one channel.

use rusqlite::{Connection, params};

use boba_app::ports::repo::auto_rules::{AutoRulesRepo, AutoRulesRepoError};
use boba_domain::apply::policy::{AutoAttemptEntry, AutoRule};
use boba_domain::scoring::gates::Confidence;
use boba_domain::scoring::rank::Verdict;

use crate::db::{Store, StoreError};

const SELECT: &str = "SELECT id, name, enabled, source_ids, channels, min_rank, min_fit, \
     min_confidence, allow_flags, verdict, daily_cap, per_company_cap, per_company_days, \
     active_start, active_end, created_at, updated_at FROM auto_rule";

fn bad(column: &'static str, value: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::new(StoreError::Sqlite(format!("unknown {column} '{value}'"))),
    )
}

fn confidence(value: &str) -> Option<Confidence> {
    match value {
        "low" => Some(Confidence::Low),
        "medium" => Some(Confidence::Medium),
        "high" => Some(Confidence::High),
        _ => None,
    }
}

fn confidence_str(value: Confidence) -> &'static str {
    match value {
        Confidence::Low => "low",
        Confidence::Medium => "medium",
        Confidence::High => "high",
    }
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<AutoRule> {
    let source_ids: String = r.get(3)?;
    let channels: String = r.get(4)?;
    let confidence_text: String = r.get(7)?;
    let verdict: String = r.get(9)?;
    Ok(AutoRule {
        id: r.get(0)?,
        name: r.get(1)?,
        enabled: r.get::<_, i64>(2)? != 0,
        source_ids: serde_json::from_str(&source_ids).map_err(|_| bad("source_ids", source_ids))?,
        channels: serde_json::from_str(&channels).map_err(|_| bad("channels", channels))?,
        min_rank: r.get(5)?,
        min_fit: r.get(6)?,
        min_confidence: confidence(&confidence_text)
            .ok_or_else(|| bad("min_confidence", confidence_text))?,
        allow_flags: r.get::<_, i64>(8)? != 0,
        min_verdict: Verdict::parse(&verdict).ok_or_else(|| bad("verdict", verdict))?,
        daily_cap: r.get(10)?,
        per_company_cap: r.get(11)?,
        per_company_days: r.get(12)?,
        active_hours: boba_domain::apply::policy::ActiveHours {
            start_minute: r.get(13)?,
            end_minute: r.get(14)?,
        },
        created_at: r.get(15)?,
        updated_at: r.get(16)?,
    })
}

/// The newest automatic send to one Source, `None` when there is none.
pub fn last_attempt_at_for_source(
    conn: &Connection,
    source_id: &str,
) -> Result<Option<String>, AutoRulesRepoError> {
    conn.query_row(
        "SELECT created_at FROM auto_rule_attempt WHERE source_id = ?1 \
         ORDER BY created_at DESC LIMIT 1",
        params![source_id],
        |r| r.get(0),
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(repo(other)),
    })
}

/// Every rule, oldest first.
pub fn list_rules(conn: &Connection) -> Result<Vec<AutoRule>, AutoRulesRepoError> {
    let mut stmt = conn
        .prepare(&format!("{SELECT} ORDER BY created_at, id"))
        .map_err(repo)?;
    let rows = stmt.query_map([], row).map_err(repo)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(repo)
}

/// Insert or replace one rule.
pub fn upsert_rule(conn: &Connection, rule: &AutoRule) -> Result<(), AutoRulesRepoError> {
    let source_ids = serde_json::to_string(&rule.source_ids)
        .map_err(|e| AutoRulesRepoError::new(e.to_string()))?;
    let channels = serde_json::to_string(&rule.channels)
        .map_err(|e| AutoRulesRepoError::new(e.to_string()))?;
    conn.execute(
        "INSERT INTO auto_rule (id, name, enabled, source_ids, channels, min_rank, min_fit, \
         min_confidence, allow_flags, verdict, daily_cap, per_company_cap, per_company_days, \
         active_start, active_end, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17) \
         ON CONFLICT(id) DO UPDATE SET \
           name = excluded.name, enabled = excluded.enabled, source_ids = excluded.source_ids, \
           channels = excluded.channels, min_rank = excluded.min_rank, min_fit = excluded.min_fit, \
           min_confidence = excluded.min_confidence, allow_flags = excluded.allow_flags, \
           verdict = excluded.verdict, daily_cap = excluded.daily_cap, \
           per_company_cap = excluded.per_company_cap, \
           per_company_days = excluded.per_company_days, \
           active_start = excluded.active_start, active_end = excluded.active_end, \
           updated_at = excluded.updated_at",
        params![
            rule.id,
            rule.name,
            rule.enabled as i64,
            source_ids,
            channels,
            rule.min_rank,
            rule.min_fit,
            confidence_str(rule.min_confidence),
            rule.allow_flags as i64,
            rule.min_verdict.as_str(),
            rule.daily_cap,
            rule.per_company_cap,
            rule.per_company_days,
            rule.active_hours.start_minute,
            rule.active_hours.end_minute,
            rule.created_at,
            rule.updated_at,
        ],
    )
    .map_err(repo)?;
    Ok(())
}

/// Remove one rule. Returns whether a row was deleted.
pub fn delete_rule(conn: &Connection, id: &str) -> Result<bool, AutoRulesRepoError> {
    conn.execute("DELETE FROM auto_rule WHERE id = ?1", params![id])
        .map(|deleted| deleted > 0)
        .map_err(repo)
}

/// Record one automatic submission.
pub fn insert_attempt(
    conn: &Connection,
    entry: &AutoAttemptEntry,
) -> Result<(), AutoRulesRepoError> {
    conn.execute(
        "INSERT INTO auto_rule_attempt \
         (id, rule_id, application_id, source_id, channel, company_key, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            entry.id,
            entry.rule_id,
            entry.application_id,
            entry.source_id,
            entry.channel.as_str(),
            entry.company_key,
            entry.created_at,
        ],
    )
    .map_err(repo)?;
    Ok(())
}

fn repo(error: rusqlite::Error) -> AutoRulesRepoError {
    AutoRulesRepoError::new(error.to_string())
}

fn count(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<u32, AutoRulesRepoError> {
    conn.query_row(sql, params, |r| r.get::<_, i64>(0))
        .map(|count| count.max(0) as u32)
        .map_err(repo)
}

/// Automatic submissions at or after `since`.
pub fn count_global_since(conn: &Connection, since: &str) -> Result<u32, AutoRulesRepoError> {
    count(
        conn,
        "SELECT COUNT(*) FROM auto_rule_attempt WHERE created_at >= ?1",
        params![since],
    )
}

/// One rule's automatic submissions at or after `since`.
pub fn count_rule_since(
    conn: &Connection,
    rule_id: &str,
    since: &str,
) -> Result<u32, AutoRulesRepoError> {
    count(
        conn,
        "SELECT COUNT(*) FROM auto_rule_attempt WHERE rule_id = ?1 AND created_at >= ?2",
        params![rule_id, since],
    )
}

/// Automatic submissions to one company at or after `since`.
pub fn count_company_since(
    conn: &Connection,
    company_key: &str,
    since: &str,
) -> Result<u32, AutoRulesRepoError> {
    count(
        conn,
        "SELECT COUNT(*) FROM auto_rule_attempt WHERE company_key = ?1 AND created_at >= ?2",
        params![company_key, since],
    )
}

/// One Source's automatic submissions on one channel at or after `since`
/// (S11.3: the Session Board cap is per Source, stricter than any rule).
pub fn count_source_since(
    conn: &Connection,
    source_id: &str,
    channel: &str,
    since: &str,
) -> Result<u32, AutoRulesRepoError> {
    count(
        conn,
        "SELECT COUNT(*) FROM auto_rule_attempt \
         WHERE source_id = ?1 AND channel = ?2 AND created_at >= ?3",
        params![source_id, channel, since],
    )
}

/// The newest ledger row's `created_at`, `None` when Auto never sent.
pub fn last_attempt_at(conn: &Connection) -> Result<Option<String>, AutoRulesRepoError> {
    conn.query_row(
        "SELECT created_at FROM auto_rule_attempt ORDER BY created_at DESC LIMIT 1",
        [],
        |r| r.get(0),
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(repo(other)),
    })
}

impl AutoRulesRepo for Store {
    fn rules(&self) -> Result<Vec<AutoRule>, AutoRulesRepoError> {
        self.read(|conn| Ok(list_rules(conn)))
            .map_err(|e| AutoRulesRepoError::new(e.to_string()))?
    }

    fn rule_upsert(&self, rule: &AutoRule) -> Result<(), AutoRulesRepoError> {
        let rule = rule.clone();
        self.execute(move |conn| Ok(upsert_rule(conn, &rule)))
            .map_err(|e| AutoRulesRepoError::new(e.to_string()))?
    }

    fn rule_delete(&self, id: &str) -> Result<bool, AutoRulesRepoError> {
        let id = id.to_string();
        self.execute(move |conn| Ok(delete_rule(conn, &id)))
            .map_err(|e| AutoRulesRepoError::new(e.to_string()))?
    }

    fn ledger_insert(&self, entry: &AutoAttemptEntry) -> Result<(), AutoRulesRepoError> {
        let entry = entry.clone();
        self.execute(move |conn| Ok(insert_attempt(conn, &entry)))
            .map_err(|e| AutoRulesRepoError::new(e.to_string()))?
    }

    fn ledger_global_since(&self, since: &str) -> Result<u32, AutoRulesRepoError> {
        let since = since.to_string();
        self.read(move |conn| Ok(count_global_since(conn, &since)))
            .map_err(|e| AutoRulesRepoError::new(e.to_string()))?
    }

    fn ledger_rule_since(&self, rule_id: &str, since: &str) -> Result<u32, AutoRulesRepoError> {
        let (rule_id, since) = (rule_id.to_string(), since.to_string());
        self.read(move |conn| Ok(count_rule_since(conn, &rule_id, &since)))
            .map_err(|e| AutoRulesRepoError::new(e.to_string()))?
    }

    fn ledger_company_since(
        &self,
        company_key: &str,
        since: &str,
    ) -> Result<u32, AutoRulesRepoError> {
        let (company_key, since) = (company_key.to_string(), since.to_string());
        self.read(move |conn| Ok(count_company_since(conn, &company_key, &since)))
            .map_err(|e| AutoRulesRepoError::new(e.to_string()))?
    }

    fn ledger_last_at(&self) -> Result<Option<String>, AutoRulesRepoError> {
        self.read(|conn| Ok(last_attempt_at(conn)))
            .map_err(|e| AutoRulesRepoError::new(e.to_string()))?
    }

    fn ledger_source_since(
        &self,
        source_id: &str,
        channel: &str,
        since: &str,
    ) -> Result<u32, AutoRulesRepoError> {
        let (source_id, channel, since) = (
            source_id.to_string(),
            channel.to_string(),
            since.to_string(),
        );
        self.read(move |conn| Ok(count_source_since(conn, &source_id, &channel, &since)))
            .map_err(|e| AutoRulesRepoError::new(e.to_string()))?
    }

    fn ledger_last_at_for_source(
        &self,
        source_id: &str,
    ) -> Result<Option<String>, AutoRulesRepoError> {
        let source_id = source_id.to_string();
        self.read(move |conn| Ok(last_attempt_at_for_source(conn, &source_id)))
            .map_err(|e| AutoRulesRepoError::new(e.to_string()))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate;
    use boba_domain::apply::policy::RuleChannel;

    fn connection() -> Connection {
        let mut conn = Connection::open_in_memory().expect("in-memory sqlite");
        migrate::embedded()
            .to_latest(&mut conn)
            .expect("apply migrations");
        conn
    }

    fn rule(id: &str) -> AutoRule {
        let mut rule = AutoRule::defaults(id, "Email Apply × remote", "2026-10-17T00:00:00Z");
        rule.source_ids = vec!["remotive".into()];
        rule.channels = vec![RuleChannel::Email, RuleChannel::AtsForm];
        rule
    }

    fn entry(id: &str, rule_id: &str, company: &str, at: &str) -> AutoAttemptEntry {
        AutoAttemptEntry {
            id: id.into(),
            rule_id: rule_id.into(),
            application_id: "app_1".into(),
            source_id: "remotive".into(),
            channel: RuleChannel::Email,
            company_key: company.into(),
            created_at: at.into(),
        }
    }

    #[test]
    fn a_rule_round_trips_every_field() {
        let conn = connection();
        let mut stored = rule("rule_1");
        stored.min_rank = 61.5;
        stored.min_fit = 77.0;
        stored.min_confidence = Confidence::High;
        stored.allow_flags = true;
        stored.min_verdict = Verdict::Strong;
        stored.daily_cap = 12;
        stored.per_company_cap = 2;
        stored.per_company_days = 45;
        stored.active_hours = boba_domain::apply::policy::ActiveHours {
            start_minute: 480,
            end_minute: 1020,
        };
        upsert_rule(&conn, &stored).unwrap();

        assert_eq!(list_rules(&conn).unwrap(), vec![stored]);
    }

    #[test]
    fn a_second_upsert_replaces_the_rule_by_id() {
        let conn = connection();
        upsert_rule(&conn, &rule("rule_1")).unwrap();
        let mut edited = rule("rule_1");
        edited.name = "Edited".into();
        edited.enabled = false;
        edited.updated_at = "2026-10-17T01:00:00Z".into();
        upsert_rule(&conn, &edited).unwrap();

        let rules = list_rules(&conn).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0], edited);
    }

    #[test]
    fn deleting_a_rule_reports_whether_a_row_went() {
        let conn = connection();
        upsert_rule(&conn, &rule("rule_1")).unwrap();
        assert!(delete_rule(&conn, "rule_1").unwrap());
        assert!(!delete_rule(&conn, "rule_1").unwrap());
        assert!(list_rules(&conn).unwrap().is_empty());
    }

    #[test]
    fn the_database_rejects_a_cap_outside_the_prd_bounds() {
        let conn = connection();
        let mut rule = rule("rule_1");
        rule.daily_cap = 26;
        assert!(upsert_rule(&conn, &rule).is_err(), "daily_cap ≤ 25");
        rule.daily_cap = 5;
        rule.per_company_cap = 0;
        assert!(
            upsert_rule(&conn, &rule).is_err(),
            "one per company at least"
        );
        rule.per_company_cap = 1;
        rule.min_rank = 101.0;
        assert!(upsert_rule(&conn, &rule).is_err(), "rank is a percentage");
    }

    #[test]
    fn the_ledger_counts_globally_per_rule_and_per_company() {
        let conn = connection();
        upsert_rule(&conn, &rule("rule_1")).unwrap();
        upsert_rule(&conn, &rule("rule_2")).unwrap();
        insert_attempt(
            &conn,
            &entry("aut_1", "rule_1", "acme", "2026-10-17T09:00:00Z"),
        )
        .unwrap();
        insert_attempt(
            &conn,
            &entry("aut_2", "rule_1", "nimbus", "2026-10-17T09:10:00Z"),
        )
        .unwrap();
        insert_attempt(
            &conn,
            &entry("aut_3", "rule_2", "acme", "2026-10-16T09:00:00Z"),
        )
        .unwrap();

        assert_eq!(
            count_global_since(&conn, "2026-10-17T00:00:00Z").unwrap(),
            2
        );
        assert_eq!(
            count_rule_since(&conn, "rule_1", "2026-10-17T00:00:00Z").unwrap(),
            2
        );
        assert_eq!(
            count_rule_since(&conn, "rule_2", "2026-10-17T00:00:00Z").unwrap(),
            0
        );
        assert_eq!(
            count_company_since(&conn, "acme", "2026-10-15T00:00:00Z").unwrap(),
            2,
            "the company window spans rules"
        );
        assert_eq!(
            last_attempt_at(&conn).unwrap().as_deref(),
            Some("2026-10-17T09:10:00Z")
        );
    }

    #[test]
    fn an_empty_ledger_has_no_last_attempt() {
        let conn = connection();
        assert_eq!(last_attempt_at(&conn).unwrap(), None);
        assert_eq!(
            count_global_since(&conn, "2026-10-17T00:00:00Z").unwrap(),
            0
        );
    }

    #[test]
    fn the_session_cap_counts_one_source_one_channel_and_the_local_day() {
        let conn = connection();
        upsert_rule(&conn, &rule("rule_1")).unwrap();
        let attempt = |id: &str, source: &str, channel: RuleChannel, at: &str| AutoAttemptEntry {
            id: id.into(),
            rule_id: "rule_1".into(),
            application_id: "app_1".into(),
            source_id: source.into(),
            channel,
            company_key: "acme".into(),
            created_at: at.into(),
        };
        insert_attempt(
            &conn,
            &attempt(
                "aut_1",
                "linkedin",
                RuleChannel::SessionBoard,
                "2026-10-17T09:00:00Z",
            ),
        )
        .unwrap();
        insert_attempt(
            &conn,
            &attempt(
                "aut_2",
                "linkedin",
                RuleChannel::SessionBoard,
                "2026-10-17T10:00:00Z",
            ),
        )
        .unwrap();
        insert_attempt(
            &conn,
            &attempt(
                "aut_3",
                "linkedin",
                RuleChannel::Email,
                "2026-10-17T11:00:00Z",
            ),
        )
        .unwrap();
        insert_attempt(
            &conn,
            &attempt(
                "aut_4",
                "indeed",
                RuleChannel::SessionBoard,
                "2026-10-17T12:00:00Z",
            ),
        )
        .unwrap();

        assert_eq!(
            count_source_since(&conn, "linkedin", "session_board", "2026-10-17T00:00:00Z").unwrap(),
            2,
            "one Source, one channel, today"
        );
        assert_eq!(
            count_source_since(&conn, "linkedin", "session_board", "2026-10-17T09:30:00Z").unwrap(),
            1,
            "the local-day window is honoured"
        );
        assert_eq!(
            count_source_since(&conn, "indeed", "session_board", "2026-10-17T00:00:00Z").unwrap(),
            1
        );
        assert_eq!(
            count_source_since(&conn, "linkedin", "session_board", "2026-10-18T00:00:00Z").unwrap(),
            0
        );
    }

    #[test]
    fn the_newest_auto_send_to_one_source_is_the_spacing_anchor() {
        let conn = connection();
        // The ledger row names its rule (FK), so the rule exists first.
        upsert_rule(&conn, &rule("rule_1")).unwrap();
        for (id, source, at) in [
            ("aut_1", "remotive", "2026-10-17T09:00:00Z"),
            ("aut_2", "remotive", "2026-10-17T09:20:00Z"),
            ("aut_3", "company_site", "2026-10-17T09:30:00Z"),
        ] {
            let row = AutoAttemptEntry {
                source_id: source.into(),
                ..entry(id, "rule_1", "acme", at)
            };
            insert_attempt(&conn, &row).unwrap();
        }

        assert_eq!(
            last_attempt_at_for_source(&conn, "remotive")
                .unwrap()
                .as_deref(),
            Some("2026-10-17T09:20:00Z")
        );
        assert_eq!(
            last_attempt_at_for_source(&conn, "company_site")
                .unwrap()
                .as_deref(),
            Some("2026-10-17T09:30:00Z")
        );
        assert_eq!(last_attempt_at_for_source(&conn, "never").unwrap(), None);
    }
}
