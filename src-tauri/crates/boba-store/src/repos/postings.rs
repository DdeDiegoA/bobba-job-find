//! Postings repository (S4.2): the single writer of `posting` and
//! `posting_source` (AD-16, AD-19).
//!
//! The store keeps the typed `boba_domain::posting` records; `locations` and
//! `requirements` are stored as JSON text, and the enums as their `as_str`
//! form, parsed back on read. Dedup merges Offers into one `posting` with one
//! provenance row per source; a source's row is refreshed when that source is
//! seen again and rows are never deleted (AD-19). `IngestionRepo` is implemented
//! for [`Store`], which is `Send + Sync`.

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use boba_app::ports::repo::ingestion::{IngestionRepo, IngestionRepoError};
use boba_domain::dedup::{DedupProbe, description_simhash, job_key_of, probe_matches};
use boba_domain::posting::{ApplyHints, Money, Offer, PayPeriod, Provenance, WorkMode};
use boba_domain::posting_features::{
    AgentRunRef, Claim, FeatureKind, FeatureSource, PendingPosting, PostingFeatures,
    confidence_parse, confidence_text, evidence_columns, evidence_from_columns,
};

use crate::db::{Store, StoreError};

const SELECT: &str = "SELECT id, source_id, external_id, title, company, location_json, \
     work_mode, amount_minor, currency, pay_period, posted_at, deadline, first_seen, last_seen, \
     lang, description_html, description_text, requirements_json, apply_url, apply_email \
     FROM posting";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Offer> {
    let amount_minor: Option<i64> = r.get(7)?;
    let currency: Option<String> = r.get(8)?;
    let period: Option<String> = r.get(9)?;
    let money = match (amount_minor, currency, period) {
        (Some(amount_minor), Some(currency), Some(period)) => Some(Money {
            amount_minor,
            currency,
            period: parse(PayPeriod::parse(&period), period)?,
        }),
        _ => None,
    };
    Ok(Offer {
        id: r.get(0)?,
        source_id: r.get(1)?,
        external_id: r.get(2)?,
        title: r.get(3)?,
        company: r.get(4)?,
        locations: json_list(r.get(5)?)?,
        work_mode: {
            let text: String = r.get(6)?;
            parse(WorkMode::parse(&text), text)?
        },
        money,
        posted_at: r.get(10)?,
        deadline: r.get(11)?,
        first_seen: r.get(12)?,
        last_seen: r.get(13)?,
        lang: r.get(14)?,
        description_html: r.get(15)?,
        description_text: r.get(16)?,
        requirements: json_list(r.get(17)?)?,
        apply: ApplyHints {
            url: r.get(18)?,
            email: r.get(19)?,
        },
    })
}

/// A stored enum back to its type; an unknown value is a corrupt row, not a
/// silent default.
fn parse<T>(value: Option<T>, text: String) -> rusqlite::Result<T> {
    value.ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(UnknownValue(text)),
        )
    })
}

#[derive(Debug, thiserror::Error)]
#[error("unknown stored value '{0}'")]
struct UnknownValue(String);

/// A JSON array column back to `Vec<String>`.
fn json_list(text: String) -> rusqlite::Result<Vec<String>> {
    serde_json::from_str(&text).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn json_array(values: &[String]) -> String {
    Value::from(values).to_string()
}

/// Store one Offer and the provenance row it was normalised from, in one
/// transaction (AD-19).
pub fn insert_offer(
    conn: &Connection,
    offer: &Offer,
    provenance: &Provenance,
) -> Result<(), StoreError> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO posting (id, source_id, external_id, title, company, location_json, \
         work_mode, amount_minor, currency, pay_period, posted_at, deadline, first_seen, \
         last_seen, lang, description_html, description_text, requirements_json, apply_url, \
         apply_email, dedup_key, simhash) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, \
                 ?18, ?19, ?20, ?21, ?22)",
        params![
            offer.id,
            offer.source_id,
            offer.external_id,
            offer.title,
            offer.company,
            json_array(&offer.locations),
            offer.work_mode.as_str(),
            offer.money.as_ref().map(|m| m.amount_minor),
            offer.money.as_ref().map(|m| m.currency.as_str()),
            offer.money.as_ref().map(|m| m.period.as_str()),
            offer.posted_at,
            offer.deadline,
            offer.first_seen,
            offer.last_seen,
            offer.lang,
            offer.description_html,
            offer.description_text,
            json_array(&offer.requirements),
            offer.apply.url,
            offer.apply.email,
            // S4.3 fills the two dedup columns 0008 shipped for it: the indexed
            // job key a later ingest looks this posting up by, and the simhash
            // of its description (`NULL` when the description is too short to
            // compare, AD-19). `simhash` is a `u64` in a SQLite INTEGER; the
            // cast is lossless both ways.
            job_key_of(offer),
            description_simhash(&offer.description_text).map(|hash| hash as i64),
        ],
    )?;
    insert_provenance(&tx, provenance)?;
    tx.commit()?;
    Ok(())
}

/// The postings an Offer could already be (S4.3, AD-19): the same `job_key`, the
/// same canonical apply URL, or the same company.
///
/// The store owns the filtering, the domain owns the rule ([`probe_matches`]),
/// and this reads the postings rather than narrowing them in SQL: 0008 index
/// only `dedup_key`, and this slice owns no migration, so a query could not use
/// an index for the company or the canonical URL anyway. A local database holds
/// one user's postings — thousands of rows — and the answer is a handful.
pub fn dedup_candidates(conn: &Connection, probe: &DedupProbe) -> Result<Vec<Offer>, StoreError> {
    let mut stmt = conn.prepare(SELECT)?;
    let rows = stmt.query_map([], row)?;
    let mut candidates = Vec::new();
    for offer in rows {
        let offer = offer?;
        if probe_matches(probe, &offer) {
            candidates.push(offer);
        }
    }
    Ok(candidates)
}

/// Attach one more source's observation to the posting dedup merged it into, and
/// refresh `last_seen`, in one transaction (S4.3, AD-19). Nothing else on the
/// posting moves — the user already read it as it was first seen — and no row is
/// ever deleted. A posting that is not there is an error: the foreign key on
/// `posting_source.posting_id` refuses the orphan row, and the timestamp goes
/// back with it.
pub fn merge_provenance(
    conn: &Connection,
    posting_id: &str,
    provenance: &Provenance,
    now: &str,
) -> Result<(), StoreError> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE posting SET last_seen = ?2 WHERE id = ?1",
        params![posting_id, now],
    )?;
    insert_provenance(&tx, provenance)?;
    tx.commit()?;
    Ok(())
}

/// Write the row for one source's observation of a posting (AD-19). The key is
/// `(posting, source, source id)`, so re-seeing the same item on the same source
/// refreshes the row with the latest payload and fetch time instead of adding a
/// copy. Rows of other sources are untouched, and nothing is ever deleted.
fn insert_provenance(conn: &Connection, provenance: &Provenance) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO posting_source (posting_id, source_id, external_id, url, raw, fetched_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT(posting_id, source_id, external_id) DO UPDATE SET \
             url = excluded.url, raw = excluded.raw, fetched_at = excluded.fetched_at",
        params![
            provenance.posting_id,
            provenance.source_id,
            provenance.external_id,
            provenance.url,
            provenance.raw,
            provenance.fetched_at,
        ],
    )?;
    Ok(())
}

pub fn get_offer(conn: &Connection, id: &str) -> Result<Option<Offer>, StoreError> {
    Ok(conn
        .query_row(&format!("{SELECT} WHERE id = ?1"), params![id], row)
        .optional()?)
}

pub fn provenance_for(conn: &Connection, posting_id: &str) -> Result<Vec<Provenance>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT posting_id, source_id, external_id, url, raw, fetched_at FROM posting_source \
         WHERE posting_id = ?1 ORDER BY fetched_at, source_id, external_id",
    )?;
    let rows = stmt.query_map(params![posting_id], |r| {
        Ok(Provenance {
            posting_id: r.get(0)?,
            source_id: r.get(1)?,
            external_id: r.get(2)?,
            url: r.get(3)?,
            raw: r.get(4)?,
            fetched_at: r.get(5)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn map_err(error: StoreError) -> IngestionRepoError {
    IngestionRepoError::Store(error.to_string())
}

// --- features (S6.2, FR-55/FR-56 inputs, FR-61, AD-5) -------------------------

const FEATURES_SELECT: &str = "SELECT kind, value, source, confidence, evidence_kind, \
     evidence_text, evidence_start, evidence_end, agent_run_id, agent_cli, agent_model \
     FROM posting_features WHERE posting_id = ?1 AND profile_version = ?2";

/// The features of one (posting, profile version), `None` when the version has
/// never been read (the `posting_features_run` marker is what tells the two
/// apart — an extracted-but-empty set is a real answer).
pub fn features(
    conn: &Connection,
    posting_id: &str,
    profile_version: u32,
) -> Result<Option<PostingFeatures>, StoreError> {
    let marked: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM posting_features_run WHERE posting_id = ?1 AND profile_version = ?2",
            params![posting_id, profile_version],
            |r| r.get(0),
        )
        .optional()?;
    if marked.is_none() {
        return Ok(None);
    }

    let mut stmt = conn.prepare(FEATURES_SELECT)?;
    let rows = stmt.query_map(params![posting_id, profile_version], |r| {
        let kind: String = r.get(0)?;
        let source: String = r.get(2)?;
        let confidence: String = r.get(3)?;
        let evidence_kind: String = r.get(4)?;
        let evidence_text: String = r.get(5)?;
        let evidence_start: Option<i64> = r.get(6)?;
        let evidence_end: Option<i64> = r.get(7)?;
        Ok((
            kind,
            source,
            confidence,
            evidence_kind,
            evidence_text,
            evidence_start,
            evidence_end,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(8)?,
            r.get::<_, Option<String>>(9)?,
            r.get::<_, Option<String>>(10)?,
        ))
    })?;

    let mut set = PostingFeatures::new(posting_id.to_string(), profile_version);
    for row in rows {
        let (
            kind,
            source,
            confidence,
            evidence_kind,
            evidence_text,
            start,
            end,
            value,
            run_id,
            cli,
            model,
        ) = row?;
        let claim = Claim {
            kind: parse(FeatureKind::parse(&kind), kind)?,
            value,
            source: parse(FeatureSource::parse(&source), source)?,
            confidence: parse(confidence_parse(&confidence), confidence)?,
            evidence: parse(
                evidence_from_columns(
                    &evidence_kind,
                    evidence_text,
                    start.map(|value| value as usize),
                    end.map(|value| value as usize),
                ),
                evidence_kind,
            )?,
            run: run_ref(run_id, cli, model)?,
        };
        set.claims.push(claim);
    }
    // The rows come back in the store's text order; the set's own order is what
    // a re-run produces, so a read-back equals a fresh extraction (FR-61).
    set.sort();
    Ok(Some(set))
}

/// The run behind an `llm` row, from its three columns. The migration keeps the
/// pairing exact (`source = 'llm'` iff a run id is present), so a row the CHECK
/// let through and this cannot read is a corrupt row, not a default.
fn run_ref(
    run_id: Option<String>,
    cli: Option<String>,
    model: Option<String>,
) -> Result<Option<AgentRunRef>, StoreError> {
    match (run_id, cli) {
        (None, None) => Ok(None),
        (Some(run_id), Some(cli)) => Ok(Some(AgentRunRef { run_id, cli, model })),
        (run_id, cli) => Err(StoreError::Sqlite(format!(
            "posting_features row names half a run: id {run_id:?}, cli {cli:?}"
        ))),
    }
}

/// Upsert one feature set: the claims, the cache marker and — for the
/// deterministic pass — the posting's `requirements_json`, in one transaction.
///
/// `agent_done` is never cleared here: the deterministic pass runs first, and a
/// later pass must not undo it.
fn save_features(
    conn: &Connection,
    features: &PostingFeatures,
    extracted_at: &str,
    agent_done: i64,
) -> Result<(), StoreError> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO posting_features_run (posting_id, profile_version, agent_done, extracted_at) \
         VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(posting_id, profile_version) DO UPDATE SET \
             extracted_at = excluded.extracted_at, \
             agent_done = max(posting_features_run.agent_done, excluded.agent_done)",
        params![
            features.posting_id,
            features.profile_version,
            agent_done,
            extracted_at
        ],
    )?;
    for claim in &features.claims {
        let (evidence_kind, evidence_text, start, end) = evidence_columns(&claim.evidence);
        tx.execute(
            "INSERT INTO posting_features (posting_id, profile_version, kind, value, source, \
             confidence, evidence_kind, evidence_text, evidence_start, evidence_end, \
             untrusted_origin, agent_run_id, agent_cli, agent_model) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1, ?11, ?12, ?13) \
             ON CONFLICT(posting_id, profile_version, kind, value, source) DO UPDATE SET \
                 confidence = excluded.confidence, \
                 evidence_kind = excluded.evidence_kind, \
                 evidence_text = excluded.evidence_text, \
                 evidence_start = excluded.evidence_start, \
                 evidence_end = excluded.evidence_end, \
                 agent_run_id = excluded.agent_run_id, \
                 agent_cli = excluded.agent_cli, \
                 agent_model = excluded.agent_model",
            params![
                features.posting_id,
                features.profile_version,
                claim.kind.as_str(),
                claim.value,
                claim.source.as_str(),
                confidence_text(claim.confidence),
                evidence_kind,
                evidence_text,
                start.map(|value| value as i64),
                end.map(|value| value as i64),
                claim.run().map(|run| run.run_id.as_str()),
                claim.run().map(|run| run.cli.as_str()),
                claim.run().and_then(|run| run.model.as_deref()),
            ],
        )?;
    }
    if agent_done == 0 {
        // S4.2's `requirements_json` is this slice's to fill (0008): the skills
        // the posting asks for, as written.
        tx.execute(
            "UPDATE posting SET requirements_json = ?2 WHERE id = ?1",
            params![features.posting_id, json_array(&features.skill_values())],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Whether an agent pass has been served for one (posting, profile version).
pub fn features_agent_done(
    conn: &Connection,
    posting_id: &str,
    profile_version: u32,
) -> Result<bool, StoreError> {
    Ok(conn
        .query_row(
            "SELECT agent_done FROM posting_features_run \
             WHERE posting_id = ?1 AND profile_version = ?2",
            params![posting_id, profile_version],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
        .is_some_and(|done| done == 1))
}

/// Postings still waiting for an agent pass at this Profile version, newest
/// first: no marker row, or one that says no run has been served.
pub fn features_pending(
    conn: &Connection,
    profile_version: u32,
    limit: usize,
) -> Result<Vec<PendingPosting>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.title, p.company, p.lang, p.description_text FROM posting p \
         LEFT JOIN posting_features_run r \
             ON r.posting_id = p.id AND r.profile_version = ?1 \
         WHERE coalesce(r.agent_done, 0) = 0 \
         ORDER BY p.first_seen DESC, p.id LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![profile_version, limit as i64], |r| {
        Ok(PendingPosting {
            id: r.get(0)?,
            title: r.get(1)?,
            company: r.get(2)?,
            lang: r.get(3)?,
            text: r.get(4)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

impl IngestionRepo for Store {
    fn insert_offer(
        &self,
        offer: &Offer,
        provenance: &Provenance,
    ) -> Result<(), IngestionRepoError> {
        let offer = offer.clone();
        let provenance = provenance.clone();
        self.execute(move |conn| insert_offer(conn, &offer, &provenance))
            .map_err(map_err)
    }

    fn get_offer(&self, id: &str) -> Result<Option<Offer>, IngestionRepoError> {
        let id = id.to_string();
        self.read(move |conn| get_offer(conn, &id)).map_err(map_err)
    }

    fn provenance(&self, posting_id: &str) -> Result<Vec<Provenance>, IngestionRepoError> {
        let posting_id = posting_id.to_string();
        self.read(move |conn| provenance_for(conn, &posting_id))
            .map_err(map_err)
    }

    fn dedup_candidates(&self, probe: &DedupProbe) -> Result<Vec<Offer>, IngestionRepoError> {
        let probe = probe.clone();
        self.read(move |conn| dedup_candidates(conn, &probe))
            .map_err(map_err)
    }

    fn merge_provenance(
        &self,
        posting_id: &str,
        provenance: &Provenance,
        now: &str,
    ) -> Result<(), IngestionRepoError> {
        let posting_id = posting_id.to_string();
        let provenance = provenance.clone();
        let now = now.to_string();
        self.execute(move |conn| merge_provenance(conn, &posting_id, &provenance, &now))
            .map_err(map_err)
    }

    fn features(
        &self,
        posting_id: &str,
        profile_version: u32,
    ) -> Result<Option<PostingFeatures>, IngestionRepoError> {
        let posting_id = posting_id.to_string();
        self.read(move |conn| features(conn, &posting_id, profile_version))
            .map_err(map_err)
    }

    fn features_agent_done(
        &self,
        posting_id: &str,
        profile_version: u32,
    ) -> Result<bool, IngestionRepoError> {
        let posting_id = posting_id.to_string();
        self.read(move |conn| features_agent_done(conn, &posting_id, profile_version))
            .map_err(map_err)
    }

    fn save_deterministic_features(
        &self,
        features: &PostingFeatures,
        extracted_at: &str,
    ) -> Result<(), IngestionRepoError> {
        let features = features.clone();
        let extracted_at = extracted_at.to_string();
        self.execute(move |conn| save_features(conn, &features, &extracted_at, 0))
            .map_err(map_err)
    }

    fn save_agent_features(
        &self,
        features: &PostingFeatures,
        extracted_at: &str,
    ) -> Result<(), IngestionRepoError> {
        let features = features.clone();
        let extracted_at = extracted_at.to_string();
        self.execute(move |conn| save_features(conn, &features, &extracted_at, 1))
            .map_err(map_err)
    }

    fn features_pending(
        &self,
        profile_version: u32,
        limit: usize,
    ) -> Result<Vec<PendingPosting>, IngestionRepoError> {
        self.read(move |conn| features_pending(conn, profile_version, limit))
            .map_err(map_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate;
    use boba_domain::scoring::gates::{Confidence, Evidence, QuotedSpan};

    fn connection() -> Connection {
        let mut conn = Connection::open_in_memory().expect("in-memory sqlite");
        migrate::embedded()
            .to_latest(&mut conn)
            .expect("apply migrations");
        conn.execute(
            "INSERT INTO source (id, name, access_class, legal_tier, apply_capability, \
             min_interval_ms, enabled, added_at, updated_at) \
             VALUES ('remotive', 'Remotive', 'public_json', 'safe', 'link_out', 2000, 1, 't', 't'), \
                    ('himalayas', 'Himalayas', 'public_json', 'safe', 'link_out', 2000, 1, 't', 't')",
            [],
        )
        .expect("seed sources");
        conn
    }

    fn offer(id: &str) -> Offer {
        Offer {
            id: id.into(),
            source_id: "remotive".into(),
            external_id: "abc-1".into(),
            title: "Backend Engineer".into(),
            company: "Boba Labs".into(),
            locations: vec!["Remote".into(), "Bogotá, CO".into()],
            work_mode: WorkMode::Remote,
            money: Some(Money {
                amount_minor: 5_000_000,
                currency: "USD".into(),
                period: PayPeriod::Year,
            }),
            posted_at: Some("2026-10-01T00:00:00Z".into()),
            deadline: None,
            first_seen: "2026-10-16T00:00:00Z".into(),
            last_seen: "2026-10-16T00:00:00Z".into(),
            lang: "en".into(),
            description_html: "<p>APIs</p>".into(),
            description_text: "APIs".into(),
            requirements: vec!["Rust".into(), "SQL".into()],
            apply: ApplyHints {
                url: "https://jobs.example.com/abc-1".into(),
                email: Some("hiring@boba.dev".into()),
            },
        }
    }

    fn provenance(posting_id: &str, source_id: &str, external_id: &str) -> Provenance {
        Provenance {
            posting_id: posting_id.into(),
            source_id: source_id.into(),
            external_id: external_id.into(),
            url: format!("https://jobs.example.com/{external_id}"),
            raw: format!(r#"{{"id":"{external_id}"}}"#),
            fetched_at: "2026-10-16T00:00:00Z".into(),
        }
    }

    /// Long enough that the description simhash has something to compare.
    const LONG: &str = "About the role. We are looking for a Backend Engineer to join our \
        platform team in a remote first company. You will design and build APIs in Rust and \
        PostgreSQL, own services end to end, and work with product managers to ship features \
        that our customers use every day. About you. You have at least four years building \
        production backends and you care about observability, queues and clean interfaces.";

    /// A second source's view of the same job: another id, another URL.
    fn other_source(id: &str, source_id: &str, url: &str) -> Offer {
        Offer {
            id: id.into(),
            source_id: source_id.into(),
            external_id: format!("{source_id}-1"),
            apply: ApplyHints {
                url: url.into(),
                email: None,
            },
            ..offer(id)
        }
    }

    #[test]
    fn an_insert_writes_the_dedup_key_and_the_simhash() {
        let conn = connection();
        let mut stored = offer("pst_1");
        stored.description_text = LONG.into();
        insert_offer(&conn, &stored, &provenance("pst_1", "remotive", "abc-1")).unwrap();

        let (key, simhash): (String, Option<i64>) = conn
            .query_row(
                "SELECT dedup_key, simhash FROM posting WHERE id = 'pst_1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();

        assert_eq!(key, job_key_of(&stored), "S4.3 fills the indexed key");
        assert_eq!(
            simhash.map(|value| value as u64),
            description_simhash(LONG),
            "and the simhash later runs compare against"
        );
    }

    #[test]
    fn a_short_description_stores_no_simhash() {
        let conn = connection();
        let mut stored = offer("pst_1");
        stored.description_text = "Apply now.".into();
        insert_offer(&conn, &stored, &provenance("pst_1", "remotive", "abc-1")).unwrap();

        let simhash: Option<i64> = conn
            .query_row("SELECT simhash FROM posting WHERE id = 'pst_1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(simhash, None, "nothing comparable, nothing stored");
    }

    #[test]
    fn candidates_are_the_postings_with_the_same_key_url_or_company() {
        let conn = connection();
        insert_offer(
            &conn,
            &offer("pst_1"),
            &provenance("pst_1", "remotive", "abc-1"),
        )
        .unwrap();
        // The same job on another board: another id, another URL.
        let elsewhere = other_source("pst_2", "himalayas", "https://himalayas.app/x9");
        insert_offer(&conn, &elsewhere, &provenance("pst_2", "himalayas", "x9")).unwrap();
        // A stranger: another company, another title, another URL.
        let mut stranger = other_source("pst_3", "remotive", "https://jobs.example.com/zz");
        stranger.company = "Globex".into();
        stranger.title = "Data Analyst".into();
        insert_offer(&conn, &stranger, &provenance("pst_3", "remotive", "zz")).unwrap();

        let probe = DedupProbe::of(&elsewhere);
        let mut ids: Vec<String> = dedup_candidates(&conn, &probe)
            .unwrap()
            .into_iter()
            .map(|offer| offer.id)
            .collect();
        ids.sort();

        assert_eq!(
            ids,
            ["pst_1", "pst_2"],
            "the stranger is not worth a comparison"
        );
    }

    #[test]
    fn merging_adds_a_provenance_row_and_refreshes_last_seen_only() {
        let conn = connection();
        insert_offer(
            &conn,
            &offer("pst_1"),
            &provenance("pst_1", "remotive", "abc-1"),
        )
        .unwrap();

        merge_provenance(
            &conn,
            "pst_1",
            &provenance("pst_1", "himalayas", "xyz-9"),
            "2026-10-17T00:00:00Z",
        )
        .unwrap();

        let rows = provenance_for(&conn, "pst_1").unwrap();
        assert_eq!(rows.len(), 2, "both sources are kept (AD-19)");
        assert_eq!(rows[0].source_id, "himalayas", "nothing was deleted");

        let (first_seen, last_seen, title): (String, String, String) = conn
            .query_row(
                "SELECT first_seen, last_seen, title FROM posting WHERE id = 'pst_1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            last_seen, "2026-10-17T00:00:00Z",
            "the posting was seen again"
        );
        assert_eq!(
            first_seen, "2026-10-16T00:00:00Z",
            "it was still seen first then"
        );
        assert_eq!(title, "Backend Engineer", "the posting itself is untouched");
    }

    #[test]
    fn merging_into_a_posting_that_is_not_there_writes_nothing() {
        let conn = connection();
        let error = merge_provenance(
            &conn,
            "pst_ghost",
            &provenance("pst_ghost", "remotive", "abc-1"),
            "2026-10-17T00:00:00Z",
        );
        assert!(error.is_err(), "the FK refuses an orphan provenance row");
        assert_eq!(provenance_for(&conn, "pst_ghost").unwrap().len(), 0);
    }

    #[test]
    fn migration_creates_the_posting_tables() {
        let conn = connection();
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name IN ('posting', 'posting_source')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn offer_round_trips_with_money_locations_and_hints() {
        let conn = connection();
        let stored = offer("pst_1");
        insert_offer(&conn, &stored, &provenance("pst_1", "remotive", "abc-1")).unwrap();

        assert_eq!(get_offer(&conn, "pst_1").unwrap(), Some(stored));
        assert!(get_offer(&conn, "pst_missing").unwrap().is_none());
        assert_eq!(provenance_for(&conn, "pst_missing").unwrap().len(), 0);
    }

    #[test]
    fn provenance_accumulates_one_row_per_source_observation() {
        let conn = connection();
        let stored = offer("pst_1");
        insert_offer(&conn, &stored, &provenance("pst_1", "remotive", "abc-1")).unwrap();
        insert_provenance(&conn, &provenance("pst_1", "himalayas", "xyz-9")).unwrap();

        let rows = provenance_for(&conn, "pst_1").unwrap();
        assert_eq!(rows.len(), 2, "both sources are kept (AD-19)");
        assert_eq!(rows[0].source_id, "himalayas");
        assert_eq!(rows[0].raw, r#"{"id":"xyz-9"}"#, "raw payload is retained");

        // Seeing the same posting on the same source again refreshes its row.
        let mut again = provenance("pst_1", "himalayas", "xyz-9");
        again.raw = r#"{"id":"xyz-9","updated":true}"#.into();
        again.fetched_at = "2026-10-17T00:00:00Z".into();
        insert_provenance(&conn, &again).unwrap();

        let rows = provenance_for(&conn, "pst_1").unwrap();
        assert_eq!(rows.len(), 2, "no duplicate row");
        assert_eq!(rows[1].raw, again.raw);
    }

    #[test]
    fn money_columns_are_all_or_nothing() {
        let conn = connection();
        conn.execute(
            "INSERT INTO posting (id, source_id, external_id, title, company, location_json, \
             work_mode, amount_minor, currency, first_seen, last_seen, lang, description_html, \
             description_text, requirements_json, apply_url) \
             VALUES ('pst_bad', 'remotive', 'x', 't', 'c', '[]', 'remote', 5000000, NULL, \
                     't', 't', 'en', '', '', '[]', 'https://x')",
            [],
        )
        .expect_err("half a Money must be rejected");

        conn.execute(
            "INSERT INTO posting (id, source_id, external_id, title, company, location_json, \
             work_mode, first_seen, last_seen, lang, description_html, description_text, \
             requirements_json, apply_url) \
             VALUES ('pst_null', 'remotive', 'x', 't', 'c', '[]', 'telepathy', 't', 't', 'en', \
                     '', '', '[]', 'https://x')",
            [],
        )
        .expect_err("an unknown work mode must be rejected");
    }

    #[test]
    fn an_offer_needs_a_known_source() {
        let conn = connection();
        let mut orphan = offer("pst_1");
        orphan.source_id = "ghost".into();
        assert!(
            insert_offer(&conn, &orphan, &provenance("pst_1", "ghost", "abc-1")).is_err(),
            "FK must reject (AD-16)"
        );
        assert!(
            get_offer(&conn, "pst_1").unwrap().is_none(),
            "nothing half-written"
        );
    }

    // --- features (S6.2) ---

    fn stored(id: &str) -> (Connection, Offer) {
        let conn = connection();
        let stored = offer(id);
        insert_offer(&conn, &stored, &provenance(id, "remotive", "abc-1")).unwrap();
        (conn, stored)
    }

    fn quoted(kind: FeatureKind, value: &str, span: &str, start: usize) -> Claim {
        Claim::deterministic(
            kind,
            value,
            Confidence::High,
            Evidence::Quoted(QuotedSpan {
                text: span.into(),
                start,
                end: start + span.len(),
            }),
        )
    }

    /// The run behind an `llm` claim, as the use case threads it in.
    fn run() -> AgentRunRef {
        AgentRunRef {
            run_id: "01RUN".to_string(),
            cli: "claude_code".to_string(),
            model: Some("haiku".to_string()),
        }
    }

    fn proposed(kind: FeatureKind, value: &str, span: &str, start: usize) -> Claim {
        Claim {
            source: FeatureSource::Llm,
            confidence: Confidence::Medium,
            run: Some(run()),
            ..quoted(kind, value, span, start)
        }
    }

    fn feature_set(id: &str, profile_version: u32, claims: Vec<Claim>) -> PostingFeatures {
        let mut set = PostingFeatures::new(id, profile_version);
        set.extend(claims);
        set
    }

    #[test]
    fn migration_creates_the_feature_tables() {
        let conn = connection();
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name IN \
                 ('posting_features', 'posting_features_run')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn features_round_trip_and_fill_the_postings_requirements() {
        let (conn, _) = stored("pst_1");
        let set = feature_set(
            "pst_1",
            4,
            vec![
                quoted(FeatureKind::Skill, "Rust", "Rust services", 4),
                quoted(FeatureKind::Skill, "SQL", "SQL and Rust", 0),
                Claim::deterministic(
                    FeatureKind::Compensation,
                    "USD 5000000 year",
                    Confidence::High,
                    Evidence::Field("money".into()),
                ),
            ],
        );
        save_features(&conn, &set, "2026-10-16T00:00:00Z", 0).unwrap();

        let read_back = features(&conn, "pst_1", 4).unwrap().expect("cached");
        assert_eq!(
            read_back, set,
            "a read-back equals what was written (FR-61)"
        );

        let requirements: String = conn
            .query_row(
                "SELECT requirements_json FROM posting WHERE id = 'pst_1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(requirements, r#"["Rust","SQL"]"#);
    }

    #[test]
    fn an_empty_deterministic_set_is_cached_and_still_awaits_the_agent() {
        let (conn, _) = stored("pst_1");
        save_features(&conn, &PostingFeatures::new("pst_1", 1), "t", 0).unwrap();

        assert_eq!(
            features(&conn, "pst_1", 1).unwrap(),
            Some(PostingFeatures::new("pst_1", 1)),
            "an empty set is a real answer, not a cache miss"
        );
        assert_eq!(
            features_pending(&conn, 1, 10).unwrap().len(),
            1,
            "a posting the lexicon found nothing in still deserves the agent's look"
        );
    }

    #[test]
    fn the_cache_is_per_profile_version() {
        let (conn, _) = stored("pst_1");
        let set = feature_set(
            "pst_1",
            1,
            vec![quoted(FeatureKind::Skill, "Rust", "Rust", 0)],
        );
        save_features(&conn, &set, "t", 0).unwrap();

        assert!(features(&conn, "pst_1", 2).unwrap().is_none());
        assert_eq!(features_pending(&conn, 2, 10).unwrap().len(), 1);
    }

    #[test]
    fn re_running_an_extraction_upserts_instead_of_duplicating() {
        let (conn, _) = stored("pst_1");
        let set = feature_set(
            "pst_1",
            1,
            vec![quoted(FeatureKind::Skill, "Rust", "Rust", 0)],
        );
        save_features(&conn, &set, "2026-10-16T00:00:00Z", 0).unwrap();
        save_features(&conn, &set, "2026-10-17T00:00:00Z", 0).unwrap();

        let rows: i64 = conn
            .query_row("SELECT count(*) FROM posting_features", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1, "one claim per identity");
        assert_eq!(features(&conn, "pst_1", 1).unwrap(), Some(set));
    }

    #[test]
    fn an_agent_pass_adds_its_claims_and_marks_the_run_done() {
        let (conn, _) = stored("pst_1");
        save_features(
            &conn,
            &feature_set(
                "pst_1",
                1,
                vec![quoted(FeatureKind::Skill, "Rust", "Rust", 0)],
            ),
            "t",
            0,
        )
        .unwrap();
        assert_eq!(features_pending(&conn, 1, 10).unwrap().len(), 1);

        let merged = feature_set(
            "pst_1",
            1,
            vec![
                quoted(FeatureKind::Skill, "Rust", "Rust", 0),
                proposed(FeatureKind::Note, "remote within the EU", "APIs", 0),
            ],
        );
        save_features(&conn, &merged, "t", 1).unwrap();

        let read_back = features(&conn, "pst_1", 1).unwrap().expect("cached");
        assert_eq!(read_back.claims.len(), 2, "{read_back:?}");
        assert!(read_back.has_llm());
        // AD-5: the agent's claim names the run, CLI and model behind it.
        let agent_claim = read_back
            .claims
            .iter()
            .find(|claim| claim.source == FeatureSource::Llm)
            .expect("the agent claim");
        assert_eq!(agent_claim.run(), Some(&run()));
        let deterministic = read_back
            .claims
            .iter()
            .find(|claim| claim.source == FeatureSource::Det)
            .expect("the core's claim");
        assert_eq!(deterministic.run(), None, "no run wrote it");
        assert!(
            features_pending(&conn, 1, 10).unwrap().is_empty(),
            "a served pass is not pending again (FR-61: no second run)"
        );

        let untrusted: i64 = conn
            .query_row(
                "SELECT count(*) FROM posting_features WHERE untrusted_origin = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(untrusted, 2, "ADR-010: every feature row is untrusted text");
    }

    #[test]
    fn the_agent_pass_marker_reads_back_at_every_stage() {
        let (conn, _) = stored("pst_1");
        assert!(!features_agent_done(&conn, "pst_1", 1).unwrap());
        save_features(&conn, &PostingFeatures::new("pst_1", 1), "t", 0).unwrap();
        assert!(!features_agent_done(&conn, "pst_1", 1).unwrap());
        save_features(&conn, &PostingFeatures::new("pst_1", 1), "t", 1).unwrap();
        assert!(features_agent_done(&conn, "pst_1", 1).unwrap());
        assert!(
            !features_agent_done(&conn, "pst_1", 2).unwrap(),
            "the marker is per Profile version"
        );
    }

    #[test]
    fn the_deterministic_pass_never_undoes_a_served_agent_pass() {
        let (conn, _) = stored("pst_1");
        save_features(&conn, &PostingFeatures::new("pst_1", 1), "t", 1).unwrap();
        save_features(
            &conn,
            &feature_set(
                "pst_1",
                1,
                vec![quoted(FeatureKind::Skill, "Rust", "Rust", 0)],
            ),
            "t",
            0,
        )
        .unwrap();
        assert!(features_pending(&conn, 1, 10).unwrap().is_empty());
    }

    #[test]
    fn pending_postings_come_back_newest_first_and_respect_the_limit() {
        let conn = connection();
        for (id, seen) in [
            ("pst_1", "2026-10-01T00:00:00Z"),
            ("pst_2", "2026-10-02T00:00:00Z"),
        ] {
            let mut one = offer(id);
            one.first_seen = seen.into();
            one.description_text = format!("body of {id}");
            insert_offer(&conn, &one, &provenance(id, "remotive", id)).unwrap();
        }

        let pending = features_pending(&conn, 1, 1).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "pst_2");
        assert_eq!(pending[0].text, "body of pst_2");
        assert_eq!(pending[0].lang, "en");

        let all = features_pending(&conn, 1, 10).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn a_value_outside_the_contract_cannot_be_stored_at_all() {
        let (conn, _) = stored("pst_1");
        let set = feature_set(
            "pst_1",
            1,
            vec![quoted(FeatureKind::Skill, "Rust", "Rust", 0)],
        );
        save_features(&conn, &set, "t", 0).unwrap();

        for statement in [
            "UPDATE posting_features SET confidence = 'certain'",
            "UPDATE posting_features SET kind = 'salary'",
            "UPDATE posting_features SET source = 'agent'",
            "UPDATE posting_features SET untrusted_origin = 2",
            "UPDATE posting_features SET evidence_start = NULL",
            // An agent claim without its run is not a claim (AD-5).
            "UPDATE posting_features SET source = 'llm'",
            "UPDATE posting_features SET source = 'llm', agent_run_id = '01RUN'",
        ] {
            assert!(
                conn.execute(statement, []).is_err(),
                "the store must refuse `{statement}`"
            );
        }
        assert_eq!(features(&conn, "pst_1", 1).unwrap(), Some(set));
    }
}
