//! Postings repository (S4.2): the single writer of `posting` and
//! `posting_source` (AD-16, AD-19).
//!
//! The store keeps the typed `boba_domain::posting` records; `locations` and
//! `requirements` are stored as JSON text, and the enums as their `as_str`
//! form, parsed back on read. Dedup merges Offers into one `posting` with N
//! provenance rows, so a row is appended, never deleted (AD-19). `IngestionRepo`
//! is implemented for [`Store`], which is `Send + Sync`.

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use boba_app::ports::repo::ingestion::{IngestionRepo, IngestionRepoError};
use boba_domain::posting::{ApplyHints, Money, Offer, PayPeriod, Provenance, WorkMode};

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
         apply_email) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, \
                 ?18, ?19, ?20)",
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
        ],
    )?;
    insert_provenance(&tx, provenance)?;
    tx.commit()?;
    Ok(())
}

/// Append (or refresh) one observation of a posting (AD-19: provenance is never
/// rewritten or deleted — the same source observation is the same row).
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
}

#[cfg(test)]
mod tests {
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
}
