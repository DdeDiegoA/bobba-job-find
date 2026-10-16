//! `profile` repository (owner module: profile, AD-16) — immutable fact
//! revisions, the profile version and agent proposals (AD-15, FR-11..13).
//!
//! Free functions over a `Connection` keep the SQL testable in isolation; the
//! [`ProfileRepo`] impl on [`Store`] below is the adapter the composition root
//! hands to the app. Facts are append-only: there is no update or delete path,
//! and the migration's triggers reject one if it ever appears.

use boba_app::ports::repo::profile::{ProfileRepo, ProfileRepoError};
use boba_domain::profile::{
    FactDraft, FactKind, FactProposal, NewRevision, ProfileFact, Provenance,
};
use rusqlite::{Connection, OptionalExtension as _, Row, params};

use crate::db::{Store, StoreError};

const FACT_COLUMNS: &str = "id, item_id, kind, text_en, text_es, evidence, detail_json, \
                            supersedes, profile_version, provenance, agent_run_id, created_at";

/// A stored value that parses to no known variant. The migration's CHECK
/// constraints mean this only happens on a hand-edited database.
#[derive(Debug, thiserror::Error)]
#[error("unknown value {value:?} in profile column {column}")]
struct BadValue {
    column: &'static str,
    value: String,
}

fn bad_value(column: &'static str, value: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::new(BadValue { column, value }),
    )
}

fn fact_from_row(row: &Row<'_>) -> rusqlite::Result<ProfileFact> {
    let kind: String = row.get(2)?;
    let provenance: String = row.get(9)?;
    Ok(ProfileFact {
        id: row.get(0)?,
        item_id: row.get(1)?,
        kind: FactKind::parse(&kind).ok_or_else(|| bad_value("kind", kind))?,
        text_en: row.get(3)?,
        text_es: row.get(4)?,
        evidence: row.get(5)?,
        detail_json: row.get(6)?,
        supersedes: row.get(7)?,
        profile_version: row.get::<_, i64>(8)? as u32,
        provenance: Provenance::parse(&provenance)
            .ok_or_else(|| bad_value("provenance", provenance))?,
        agent_run_id: row.get(10)?,
        created_at: row.get(11)?,
    })
}

/// The current profile version; `0` before the first fact (the migration seeds
/// the row, so this never misses).
pub fn profile_version_get(conn: &Connection) -> rusqlite::Result<u32> {
    let version: i64 = conn.query_row(
        "SELECT version FROM profile WHERE id = 'profile'",
        [],
        |row| row.get(0),
    )?;
    Ok(version.max(0) as u32)
}

/// Append one revision and bump the profile version in **one** transaction
/// (AD-15): `supersedes` and the version are resolved here, so concurrent edits
/// cannot fork an item and a crash cannot leave the version behind the facts.
///
/// Fails when `is_new` disagrees with whether the item has a current revision.
pub fn fact_append_revision(
    conn: &mut Connection,
    revision: &NewRevision,
) -> Result<ProfileFact, ProfileRepoError> {
    let tx = conn
        .transaction()
        .map_err(|e| ProfileRepoError::new(e.to_string()))?;
    let current =
        fact_latest(&tx, &revision.item_id).map_err(|e| ProfileRepoError::new(e.to_string()))?;
    let supersedes = match (current, revision.is_new) {
        (Some(existing), false) => Some(existing.id),
        (None, true) => None,
        (Some(_), true) => {
            return Err(ProfileRepoError::new(format!(
                "profile item {} already exists",
                revision.item_id
            )));
        }
        (None, false) => {
            return Err(ProfileRepoError::new(format!(
                "profile item {} has no current revision",
                revision.item_id
            )));
        }
    };
    let version = profile_version_get(&tx)
        .map_err(|e| ProfileRepoError::new(e.to_string()))?
        .checked_add(1)
        .ok_or_else(|| ProfileRepoError::new("the profile version overflowed"))?;
    let fact = revision.clone().into_fact(supersedes, version);
    fact_append(&tx, &fact).map_err(|e| ProfileRepoError::new(e.to_string()))?;
    profile_set_version(&tx, version, &fact.created_at)
        .map_err(|e| ProfileRepoError::new(e.to_string()))?;
    tx.commit()
        .map_err(|e| ProfileRepoError::new(e.to_string()))?;
    Ok(fact)
}

/// Store the profile version with the change's timestamp (UTC RFC 3339, AD-17).
pub fn profile_set_version(
    conn: &Connection,
    version: u32,
    updated_at: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE profile SET version = ?1, updated_at = ?2 WHERE id = 'profile'",
        params![i64::from(version), updated_at],
    )?;
    Ok(())
}

/// Append one immutable revision (AD-15).
pub fn fact_append(conn: &Connection, fact: &ProfileFact) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO profile_fact (id, item_id, kind, text_en, text_es, evidence, detail_json, \
         supersedes, profile_version, provenance, agent_run_id, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            fact.id,
            fact.item_id,
            fact.kind.as_str(),
            fact.text_en,
            fact.text_es,
            fact.evidence,
            fact.detail_json,
            fact.supersedes,
            i64::from(fact.profile_version),
            fact.provenance.as_str(),
            fact.agent_run_id,
            fact.created_at,
        ],
    )?;
    Ok(())
}

/// One revision by row id.
pub fn fact_get(conn: &Connection, id: &str) -> rusqlite::Result<Option<ProfileFact>> {
    conn.query_row(
        &format!("SELECT {FACT_COLUMNS} FROM profile_fact WHERE id = ?1"),
        params![id],
        fact_from_row,
    )
    .optional()
}

/// The latest revision of one item: the one no other revision supersedes.
pub fn fact_latest(conn: &Connection, item_id: &str) -> rusqlite::Result<Option<ProfileFact>> {
    conn.query_row(
        &format!(
            "SELECT {FACT_COLUMNS} FROM profile_fact f \
             WHERE f.item_id = ?1 AND NOT EXISTS \
                 (SELECT 1 FROM profile_fact n WHERE n.supersedes = f.id) \
             ORDER BY f.profile_version DESC LIMIT 1"
        ),
        params![item_id],
        fact_from_row,
    )
    .optional()
}

/// Every revision of one item, newest first (History, AC3).
pub fn fact_history(conn: &Connection, item_id: &str) -> rusqlite::Result<Vec<ProfileFact>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {FACT_COLUMNS} FROM profile_fact WHERE item_id = ?1 \
         ORDER BY profile_version DESC, rowid DESC"
    ))?;
    let rows = stmt.query_map(params![item_id], fact_from_row)?;
    rows.collect()
}

/// The latest revision of every item, in insertion order (editor order is the
/// UI's concern: it groups by kind).
pub fn facts_current(conn: &Connection) -> rusqlite::Result<Vec<ProfileFact>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {FACT_COLUMNS} FROM profile_fact f \
         WHERE NOT EXISTS (SELECT 1 FROM profile_fact n WHERE n.supersedes = f.id) \
         ORDER BY f.rowid"
    ))?;
    let rows = stmt.query_map([], fact_from_row)?;
    rows.collect()
}

/// Store a proposal an agent run made (AC4).
pub fn proposal_append(conn: &Connection, proposal: &FactProposal) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO profile_fact_proposal (id, item_id, kind, text_en, text_es, evidence, \
         detail_json, source_excerpt, untrusted_origin, agent_run_id, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            proposal.id,
            proposal.item_id,
            proposal.draft.kind.as_str(),
            proposal.draft.text_en,
            proposal.draft.text_es,
            proposal.draft.evidence,
            proposal.draft.detail_json,
            proposal.source_excerpt,
            proposal.untrusted_origin_flag(),
            proposal.agent_run_id,
            proposal.created_at,
        ],
    )?;
    Ok(())
}

/// Proposals awaiting review, newest first (S3.5, S3.6).
pub fn proposals_pending(conn: &Connection) -> rusqlite::Result<Vec<FactProposal>> {
    let mut stmt = conn.prepare(
        "SELECT id, item_id, kind, text_en, text_es, evidence, detail_json, source_excerpt, \
         untrusted_origin, agent_run_id, created_at \
         FROM profile_fact_proposal ORDER BY rowid DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        let kind: String = row.get(2)?;
        let kind = FactKind::parse(&kind).ok_or_else(|| bad_value("kind", kind))?;
        Ok(FactProposal {
            id: row.get(0)?,
            item_id: row.get(1)?,
            draft: FactDraft {
                kind,
                text_en: row.get(3)?,
                text_es: row.get(4)?,
                evidence: row.get(5)?,
                detail_json: row.get(6)?,
            },
            source_excerpt: row.get(7)?,
            untrusted_origin: row.get::<_, i64>(8)? != 0,
            agent_run_id: row.get(9)?,
            created_at: row.get(10)?,
        })
    })?;
    rows.collect()
}

/// Drop a pending proposal the user confirmed or discarded (S3.5). Returns
/// whether a row was removed.
pub fn proposal_delete(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    Ok(conn.execute(
        "DELETE FROM profile_fact_proposal WHERE id = ?1",
        params![id],
    )? > 0)
}

/// Run a read against the pooled connections, mapping SQLite errors.
fn read<T, F>(store: &Store, f: F) -> Result<T, ProfileRepoError>
where
    T: Send + 'static,
    F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
{
    store
        .read(|conn| f(conn).map_err(StoreError::from))
        .map_err(|e| ProfileRepoError::new(e.to_string()))
}

/// Run a write on the single writer actor, mapping SQLite errors.
fn write<T, F>(store: &Store, f: F) -> Result<T, ProfileRepoError>
where
    T: Send + 'static,
    F: FnOnce(&mut Connection) -> rusqlite::Result<T> + Send + 'static,
{
    store
        .execute(|conn| f(conn).map_err(StoreError::from))
        .map_err(|e| ProfileRepoError::new(e.to_string()))
}

impl ProfileRepo for Store {
    fn profile_version(&self) -> Result<u32, ProfileRepoError> {
        read(self, profile_version_get)
    }

    fn facts_current(&self) -> Result<Vec<ProfileFact>, ProfileRepoError> {
        read(self, facts_current)
    }

    fn fact_latest(&self, item_id: &str) -> Result<Option<ProfileFact>, ProfileRepoError> {
        let item_id = item_id.to_owned();
        read(self, move |conn| fact_latest(conn, &item_id))
    }

    fn fact_get(&self, id: &str) -> Result<Option<ProfileFact>, ProfileRepoError> {
        let id = id.to_owned();
        read(self, move |conn| fact_get(conn, &id))
    }

    fn fact_history(&self, item_id: &str) -> Result<Vec<ProfileFact>, ProfileRepoError> {
        let item_id = item_id.to_owned();
        read(self, move |conn| fact_history(conn, &item_id))
    }

    fn fact_append_revision(
        &self,
        revision: &NewRevision,
    ) -> Result<ProfileFact, ProfileRepoError> {
        let revision = revision.clone();
        self.execute(move |conn| {
            fact_append_revision(conn, &revision).map_err(|e| StoreError::Sqlite(e.to_string()))
        })
        .map_err(|e| ProfileRepoError::new(e.to_string()))
    }

    fn proposals_pending(&self) -> Result<Vec<FactProposal>, ProfileRepoError> {
        read(self, proposals_pending)
    }

    fn proposal_append(&self, proposal: &FactProposal) -> Result<(), ProfileRepoError> {
        let proposal = proposal.clone();
        write(self, move |conn| proposal_append(conn, &proposal))
    }

    fn proposal_delete(&self, id: &str) -> Result<bool, ProfileRepoError> {
        let id = id.to_owned();
        write(self, move |conn| proposal_delete(conn, &id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::StoreConfig;
    use crate::keys::DbKey;
    use boba_domain::profile::Lang;
    use std::path::Path;
    use tempfile::TempDir;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open");
        conn.execute_batch(include_str!("../../migrations/0007_profile.sql"))
            .expect("apply 0007");
        conn
    }

    fn fact(
        item_id: &str,
        id: &str,
        version: u32,
        supersedes: Option<&str>,
        en: Option<&str>,
        es: Option<&str>,
    ) -> ProfileFact {
        ProfileFact {
            id: id.to_string(),
            item_id: item_id.to_string(),
            kind: FactKind::Experience,
            text_en: en.map(str::to_string),
            text_es: es.map(str::to_string),
            evidence: Some("role “Nimbus”".to_string()),
            detail_json: "{\"org\":\"Nimbus\"}".to_string(),
            supersedes: supersedes.map(str::to_string),
            profile_version: version,
            provenance: Provenance::User,
            agent_run_id: None,
            created_at: "2026-10-15T12:00:00.000Z".to_string(),
        }
    }

    // AC3: the version lives in `profile`, seeded at 0 by the migration.
    #[test]
    fn profile_version_starts_at_zero_and_round_trips() {
        let conn = conn();
        assert_eq!(profile_version_get(&conn).unwrap(), 0);
        profile_set_version(&conn, 7, "2026-10-15T12:00:00.000Z").unwrap();
        assert_eq!(profile_version_get(&conn).unwrap(), 7);
        let updated_at: String = conn
            .query_row("SELECT updated_at FROM profile", [], |row| row.get(0))
            .unwrap();
        assert_eq!(updated_at, "2026-10-15T12:00:00.000Z");
    }

    // AC1/AC3: an item keeps its ULID and gains a revision; only the newest is
    // "current", and no revision is lost.
    #[test]
    fn facts_current_keeps_only_the_latest_revision() {
        let conn = conn();
        let item = "01ITEM";
        fact_append(
            &conn,
            &fact(item, "01REV1", 1, None, Some("Senior Backend Eng"), None),
        )
        .unwrap();
        fact_append(
            &conn,
            &fact(
                item,
                "01REV2",
                2,
                Some("01REV1"),
                Some("Senior Backend Engineer"),
                None,
            ),
        )
        .unwrap();

        let current = facts_current(&conn).unwrap();
        assert_eq!(current.len(), 1, "only one revision is current");
        assert_eq!(current[0].id, "01REV2");
        assert_eq!(current[0].item_id, item, "the item ULID is stable");
        assert_eq!(current[0].profile_version, 2);

        let history = fact_history(&conn, item).unwrap();
        assert_eq!(
            history.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(),
            vec!["01REV2", "01REV1"],
            "history is newest first and keeps every revision"
        );
        assert_eq!(fact_latest(&conn, item).unwrap().unwrap().id, "01REV2");
        assert!(fact_latest(&conn, "01OTHER").unwrap().is_none());
    }

    #[test]
    fn draft_columns_round_trip() {
        let conn = conn();
        let mut document = fact(
            "01ITEM",
            "01REV1",
            1,
            None,
            Some("Led the dbt/Airflow migration"),
            Some("Lideré la migración de dbt/Airflow"),
        );
        document.kind = FactKind::Skill;
        document.detail_json = "{\"cefr\":null}".to_string();
        fact_append(&conn, &document).unwrap();

        let stored = fact_get(&conn, "01REV1").unwrap().unwrap();
        assert_eq!(stored, document);
        assert_eq!(stored.kind, FactKind::Skill);
        assert_eq!(
            stored.text(Lang::Es).unwrap(),
            "Lideré la migración de dbt/Airflow"
        );
        assert_eq!(stored.missing_variant(), None);
    }

    // AC3 (AD-15): the migration itself refuses to rewrite or erase a fact.
    #[test]
    fn update_and_delete_are_rejected_by_the_migration() {
        let conn = conn();
        fact_append(
            &conn,
            &fact("01ITEM", "01REV1", 1, None, Some("Nimbus"), None),
        )
        .unwrap();

        let updated = conn.execute(
            "UPDATE profile_fact SET text_en = 'rewritten' WHERE id = '01REV1'",
            [],
        );
        assert!(updated.is_err(), "a fact must never be updated in place");

        let deleted = conn.execute("DELETE FROM profile_fact WHERE id = '01REV1'", []);
        assert!(deleted.is_err(), "a fact must never be deleted");

        assert_eq!(facts_current(&conn).unwrap().len(), 1);
    }

    // AC2: the schema keeps facts with at least one variant, and flags the gap.
    #[test]
    fn a_fact_without_any_variant_is_rejected() {
        let conn = conn();
        let mut empty = fact("01ITEM", "01REV1", 1, None, None, None);
        empty.text_en = None;
        empty.text_es = None;
        assert!(
            fact_append(&conn, &empty).is_err(),
            "at least one variant is required"
        );

        fact_append(
            &conn,
            &fact("01ITEM", "01REV2", 1, None, Some("Rust"), None),
        )
        .unwrap();
        let stored = fact_get(&conn, "01REV2").unwrap().unwrap();
        assert_eq!(
            stored.missing_variant(),
            Some(Lang::Es),
            "the gap is visible, not invented"
        );
    }

    // AC4 (ADR-010): a pending proposal carries the run's trust level as 0/1.
    #[test]
    fn proposal_untrusted_origin_round_trips() {
        let conn = conn();
        let proposal = |id: &str, untrusted: bool| FactProposal {
            id: id.to_string(),
            item_id: Some("01ITEM".to_string()),
            draft: FactDraft {
                text_en: Some("AWS Certified".to_string()),
                ..FactDraft::new(FactKind::Certification)
            },
            source_excerpt: Some("AWS Certified".to_string()),
            untrusted_origin: untrusted,
            agent_run_id: "run_01".to_string(),
            created_at: "2026-10-15T12:00:00.000Z".to_string(),
        };
        proposal_append(&conn, &proposal("01P1", true)).unwrap();
        proposal_append(&conn, &proposal("01P2", false)).unwrap();

        let pending = proposals_pending(&conn).unwrap();
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].id, "01P2", "newest first");
        assert!(
            pending[1].untrusted_origin,
            "a posting-sourced proposal is untrusted"
        );
        assert!(
            !pending[0].untrusted_origin,
            "the user's own CV is trusted input"
        );

        assert!(proposal_delete(&conn, "01P2").unwrap());
        assert!(!proposal_delete(&conn, "01P2").unwrap());
        assert_eq!(proposals_pending(&conn).unwrap().len(), 1);
    }

    // AC4: a confirmed fact is the user's own claim — provenance, no untrusted flag.
    #[test]
    fn confirmed_facts_have_provenance_and_no_untrusted_origin_column() {
        let conn = conn();
        let mut confirmed = fact("01ITEM", "01REV1", 1, None, Some("Rust"), None);
        confirmed.provenance = Provenance::AgentProposal;
        confirmed.agent_run_id = Some("run_01".to_string());
        fact_append(&conn, &confirmed).unwrap();
        assert_eq!(
            fact_get(&conn, "01REV1")
                .unwrap()
                .unwrap()
                .agent_run_id
                .as_deref(),
            Some("run_01"),
            "the run that proposed it is kept for audit"
        );

        for table in ["profile_fact", "profile"] {
            let mut stmt = conn
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap();
            let columns: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            assert!(
                !columns.iter().any(|c| c == "untrusted_origin"),
                "{table} must not carry untrusted_origin (AD-15)"
            );
        }
        assert!(
            conn.prepare("PRAGMA table_info(profile_fact_proposal)")
                .unwrap()
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<rusqlite::Result<Vec<String>>>()
                .unwrap()
                .iter()
                .any(|c| c == "untrusted_origin")
        );
    }

    /// A fact an agent proposed must name its run (ADR-010).
    #[test]
    fn an_agent_fact_without_a_run_is_rejected() {
        let conn = conn();
        let mut orphan = fact("01ITEM", "01REV1", 1, None, Some("Rust"), None);
        orphan.provenance = Provenance::AgentProposal;
        assert!(fact_append(&conn, &orphan).is_err());
    }

    /// The adapter the composition root hands to the app: real encrypted store,
    /// writer actor and read pool, no keychain.
    #[test]
    fn store_adapter_writes_through_the_actor_and_reads_it_back() {
        let dir = TempDir::new().expect("tempdir");
        let store = open_store(&dir);
        assert_eq!(store.profile_version().unwrap(), 0);

        let created = store
            .fact_append_revision(&revision("01ITEM", "01REV1", true, Some("Nimbus"), None))
            .unwrap();
        assert_eq!(created.profile_version, 1);
        assert_eq!(created.supersedes, None);

        assert_eq!(store.profile_version().unwrap(), 1);
        assert_eq!(store.facts_current().unwrap(), vec![created.clone()]);
        assert_eq!(store.fact_latest("01ITEM").unwrap().unwrap().id, "01REV1");
        assert_eq!(store.fact_history("01ITEM").unwrap().len(), 1);
        assert_eq!(store.fact_get("01REV1").unwrap().unwrap().item_id, "01ITEM");
        assert!(store.fact_get("01NOPE").unwrap().is_none());

        let proposal = FactProposal {
            id: "01P1".to_string(),
            item_id: None,
            draft: FactDraft {
                text_en: Some("Authorised to work in the EU".to_string()),
                ..FactDraft::new(FactKind::Eligibility)
            },
            source_excerpt: None,
            untrusted_origin: true,
            agent_run_id: "run_01".to_string(),
            created_at: created.created_at.clone(),
        };
        store.proposal_append(&proposal).unwrap();
        assert_eq!(store.proposals_pending().unwrap(), vec![proposal]);
        assert!(store.proposal_delete("01P1").unwrap());
        assert!(store.proposals_pending().unwrap().is_empty());
    }

    // AD-15: the append resolves the parent and the version in one transaction,
    // so a second save for the same item chains instead of forking.
    #[test]
    fn a_second_append_chains_instead_of_forking() {
        let mut conn = conn();
        let first = fact_append_revision(
            &mut conn,
            &revision("01ITEM", "01REV1", true, Some("v1"), None),
        )
        .unwrap();
        let second = fact_append_revision(
            &mut conn,
            &revision("01ITEM", "01REV2", false, Some("v2"), None),
        )
        .unwrap();

        assert_eq!(second.supersedes.as_deref(), Some("01REV1"));
        assert_eq!(second.profile_version, first.profile_version + 1);
        assert_eq!(profile_version_get(&conn).unwrap(), second.profile_version);
        let current = facts_current(&conn).unwrap();
        assert_eq!(current.len(), 1, "one item, one current revision: no fork");
        assert_eq!(current[0].id, "01REV2");
    }

    // AD-15 in the schema: a fork, a duplicated version or a second root row is
    // rejected even if some other code path tries to write one.
    #[test]
    fn forks_and_duplicate_versions_are_rejected_by_the_schema() {
        let conn = conn();
        fact_append(&conn, &fact("01ITEM", "01REV1", 1, None, Some("v1"), None)).unwrap();

        // The legitimate successor of 01REV1.
        fact_append(
            &conn,
            &fact("01ITEM", "01REV2", 2, Some("01REV1"), Some("v2"), None),
        )
        .unwrap();

        // A second row claiming to supersede 01REV1: the fork the schema rejects.
        let forked = fact("01ITEM", "01FORK", 3, Some("01REV1"), Some("fork"), None);
        assert!(
            fact_append(&conn, &forked).is_err(),
            "one child per revision"
        );
        // The same item and version twice.
        let duplicate = fact("01ITEM", "01DUP", 2, Some("01REV2"), Some("dup"), None);
        assert!(
            fact_append(&conn, &duplicate).is_err(),
            "one row per item version"
        );
        assert_eq!(facts_current(&conn).unwrap().len(), 1);
    }

    #[test]
    fn an_append_rejects_a_wrong_is_new_flag() {
        let mut conn = conn();
        fact_append_revision(
            &mut conn,
            &revision("01ITEM", "01REV1", true, Some("v1"), None),
        )
        .unwrap();

        let again = fact_append_revision(
            &mut conn,
            &revision("01ITEM", "01REV2", true, Some("v2"), None),
        );
        assert!(again.is_err(), "is_new on an existing item");

        let missing = fact_append_revision(
            &mut conn,
            &revision("01OTHER", "01REV3", false, Some("v3"), None),
        );
        assert!(missing.is_err(), "is_new = false on an unknown item");
        assert_eq!(profile_version_get(&conn).unwrap(), 1, "nothing drifted");
        assert_eq!(facts_current(&conn).unwrap().len(), 1);
    }

    fn revision(
        item_id: &str,
        id: &str,
        is_new: bool,
        en: Option<&str>,
        es: Option<&str>,
    ) -> NewRevision {
        NewRevision {
            id: id.to_string(),
            item_id: item_id.to_string(),
            is_new,
            draft: FactDraft {
                kind: FactKind::Experience,
                text_en: en.map(str::to_string),
                text_es: es.map(str::to_string),
                evidence: None,
                detail_json: "{}".to_string(),
            },
            provenance: Provenance::User,
            agent_run_id: None,
            created_at: "2026-10-15T12:00:00.000Z".to_string(),
        }
    }

    fn open_store(dir: &TempDir) -> Store {
        let key =
            DbKey::from_hex("00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff")
                .expect("test key");
        let config = StoreConfig::dev(db_path(dir), dir.path().join("snapshots"));
        Store::open_with_key(config, key).expect("open store")
    }

    fn db_path(dir: &TempDir) -> std::path::PathBuf {
        let path: &Path = dir.path();
        path.join("boba.db")
    }
}
