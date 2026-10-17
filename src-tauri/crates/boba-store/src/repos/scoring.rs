//! Scoring repository (S6.3): the single writer of `score_card` (AD-8, AD-16).
//!
//! A card is written once and read forever: [`insert_card`] inserts, and nothing
//! here updates a row — a re-score is a new row for new versions (AC3). The
//! schema enforces the same thing (a UNIQUE key over the three versions and a
//! trigger that refuses any UPDATE), so a bug here cannot rewrite what the user
//! was already shown.
//!
//! The typed card lives in [`boba_domain::scoring::rank`]; the two axes are
//! columns (readable, sortable by the Inbox, FR-63) while the eleven factors and
//! the Gates are the JSON the model produced, kept verbatim (FR-61: a card reads
//! back exactly as it was written).

use rusqlite::Connection;

use boba_app::ports::repo::scoring::{ScoringRepo, ScoringRepoError};
use boba_domain::scoring::factors::{Factor, FactorScore};
use boba_domain::scoring::gates::{Confidence, GateReport};
use boba_domain::scoring::narration::ScoreNarration;
use boba_domain::scoring::rank::{CardVersions, ScoreCard, ScoreCardRow, Verdict};

use crate::db::{Store, StoreError};

/// Every column of `score_card`, in the order [`row`] reads them.
const SELECT: &str = "SELECT id, posting_id, profile_version, prefs_version, model_version, \
     rank, fit, quality, verdict, confidence, factors_json, gates_json, created_at \
     FROM score_card";

/// Store one card (AC3: insert only).
pub fn insert_card(conn: &Connection, row: &ScoreCardRow) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO score_card (id, posting_id, profile_version, prefs_version, model_version, \
         rank, fit, quality, verdict, confidence, factors_json, gates_json, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        rusqlite::params![
            row.id,
            row.posting_id,
            row.profile_version,
            row.prefs_version,
            row.model_version,
            row.card.rank,
            row.card.fit,
            row.card.quality,
            row.card.verdict.as_str(),
            confidence_text(row.card.confidence),
            encode(&row.card.factors)?,
            encode(&row.gates)?,
            row.created_at,
        ],
    )?;
    Ok(())
}

/// The card stored for one exact key, `None` when these versions were never
/// scored (FR-61).
pub fn card(
    conn: &Connection,
    posting_id: &str,
    versions: CardVersions,
) -> Result<Option<ScoreCardRow>, StoreError> {
    let sql = format!(
        "{SELECT} WHERE posting_id = ?1 AND profile_version = ?2 AND prefs_version = ?3 \
         AND model_version = ?4"
    );
    let mut statement = conn.prepare(&sql)?;
    let mut rows = statement.query_map(
        rusqlite::params![
            posting_id,
            versions.profile_version,
            versions.prefs_version,
            versions.model_version
        ],
        row,
    )?;
    Ok(rows.next().transpose()?)
}

/// The newest card for an Offer, whatever versions it was computed for.
pub fn latest_card(
    conn: &Connection,
    posting_id: &str,
) -> Result<Option<ScoreCardRow>, StoreError> {
    let sql = format!("{SELECT} WHERE posting_id = ?1 ORDER BY created_at DESC, id DESC LIMIT 1");
    let mut statement = conn.prepare(&sql)?;
    let mut rows = statement.query_map(rusqlite::params![posting_id], row)?;
    Ok(rows.next().transpose()?)
}

/// Every card an Offer has, oldest first (AC3).
pub fn cards_for(conn: &Connection, posting_id: &str) -> Result<Vec<ScoreCardRow>, StoreError> {
    let sql = format!("{SELECT} WHERE posting_id = ?1 ORDER BY created_at ASC, id ASC");
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(rusqlite::params![posting_id], row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<ScoreCardRow>>>()?)
}

/// The narration stored newest-first for one card in one language (S6.5).
///
/// `score_card_id` is part of the lookup, not a filter the caller may forget
/// (AD-8): a narration is bound to the card it explains, and a re-scored Offer
/// has no narration until one is written against the new card.
pub fn latest_narration(
    conn: &Connection,
    posting_id: &str,
    score_card_id: &str,
    lang: &str,
) -> Result<Option<ScoreNarration>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT id, posting_id, score_card_id, lang, text, untrusted_origin, created_at \
         FROM score_narration WHERE posting_id = ?1 AND score_card_id = ?2 AND lang = ?3 \
         ORDER BY created_at DESC, id DESC LIMIT 1",
    )?;
    let mut rows = statement.query_map(
        rusqlite::params![posting_id, score_card_id, lang],
        narration_row,
    )?;
    Ok(rows.next().transpose()?)
}

/// Store one narration (S6.5). It is bound to the card it explains.
pub fn insert_narration(conn: &Connection, row: &ScoreNarration) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO score_narration (id, posting_id, score_card_id, lang, text, \
         untrusted_origin, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            row.id,
            row.posting_id,
            row.score_card_id,
            row.lang,
            row.text,
            i64::from(row.untrusted_origin),
            row.created_at,
        ],
    )?;
    Ok(())
}

/// One `score_narration` row back into the typed narration.
fn narration_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ScoreNarration> {
    Ok(ScoreNarration {
        id: r.get(0)?,
        posting_id: r.get(1)?,
        score_card_id: r.get(2)?,
        lang: r.get(3)?,
        text: r.get(4)?,
        untrusted_origin: r.get::<_, i64>(5)? != 0,
        created_at: r.get(6)?,
    })
}

/// One of the card's JSON columns, as the text the row keeps.
fn encode<T: serde::Serialize>(value: &T) -> Result<String, StoreError> {
    serde_json::to_string(value).map_err(|error| StoreError::Sqlite(error.to_string()))
}

/// One `score_card` row back into the typed card.
fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ScoreCardRow> {
    let model_version: u32 = r.get(4)?;
    let factors: Vec<FactorScore> =
        serde_json::from_str(&r.get::<_, String>(10)?).map_err(encode_error)?;
    for factor in Factor::ALL {
        if !factors.iter().any(|score| score.factor == factor) {
            // A card that has lost a factor is a corrupt row, not a card with a
            // missing factor: renormalising over what is left would score the
            // Offer differently than the run that wrote it (FR-61).
            return Err(corrupt(format!(
                "score_card factors_json is missing {}",
                factor.as_str()
            )));
        }
    }
    let gates: GateReport = serde_json::from_str(&r.get::<_, String>(11)?).map_err(encode_error)?;
    Ok(ScoreCardRow {
        id: r.get(0)?,
        posting_id: r.get(1)?,
        profile_version: r.get(2)?,
        prefs_version: r.get(3)?,
        model_version,
        card: ScoreCard {
            model_version,
            factors,
            fit: r.get(6)?,
            quality: r.get(7)?,
            rank: r.get(5)?,
            verdict: verdict(r.get::<_, String>(8)?)?,
            confidence: confidence(&r.get::<_, String>(9)?)?,
        },
        gates,
        created_at: r.get(12)?,
    })
}

/// A stored enum back to its type; an unknown value is a corrupt row, not a
/// silent default.
fn verdict(text: String) -> rusqlite::Result<Verdict> {
    Verdict::parse(&text).ok_or_else(|| corrupt(format!("unknown verdict: {text}")))
}

fn confidence(text: &str) -> rusqlite::Result<Confidence> {
    match text {
        "low" => Ok(Confidence::Low),
        "medium" => Ok(Confidence::Medium),
        "high" => Ok(Confidence::High),
        other => Err(corrupt(format!("unknown confidence: {other}"))),
    }
}

/// The stored form of a confidence, one of the three the `CHECK` accepts.
fn confidence_text(confidence: Confidence) -> &'static str {
    match confidence {
        Confidence::Low => "low",
        Confidence::Medium => "medium",
        Confidence::High => "high",
    }
}

/// A value this build cannot read: JSON that is not the model's, or an enum
/// outside the stored vocabulary. A corrupt row is an error, never a default.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct CorruptRow(String);

fn corrupt(message: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::new(CorruptRow(message)),
    )
}

fn encode_error(error: serde_json::Error) -> rusqlite::Error {
    corrupt(error.to_string())
}

fn map_err(error: StoreError) -> ScoringRepoError {
    ScoringRepoError::new(error.to_string())
}

impl ScoringRepo for Store {
    fn insert_card(&self, row: &ScoreCardRow) -> Result<(), ScoringRepoError> {
        let row = row.clone();
        self.execute(move |conn| insert_card(conn, &row))
            .map_err(map_err)
    }

    fn card(
        &self,
        posting_id: &str,
        versions: CardVersions,
    ) -> Result<Option<ScoreCardRow>, ScoringRepoError> {
        let posting_id = posting_id.to_string();
        self.read(move |conn| card(conn, &posting_id, versions))
            .map_err(map_err)
    }

    fn latest_card(&self, posting_id: &str) -> Result<Option<ScoreCardRow>, ScoringRepoError> {
        let posting_id = posting_id.to_string();
        self.read(move |conn| latest_card(conn, &posting_id))
            .map_err(map_err)
    }

    fn cards_for(&self, posting_id: &str) -> Result<Vec<ScoreCardRow>, ScoringRepoError> {
        let posting_id = posting_id.to_string();
        self.read(move |conn| cards_for(conn, &posting_id))
            .map_err(map_err)
    }

    fn latest_narration(
        &self,
        posting_id: &str,
        score_card_id: &str,
        lang: &str,
    ) -> Result<Option<ScoreNarration>, ScoringRepoError> {
        let posting_id = posting_id.to_string();
        let score_card_id = score_card_id.to_string();
        let lang = lang.to_string();
        self.read(move |conn| latest_narration(conn, &posting_id, &score_card_id, &lang))
            .map_err(map_err)
    }

    fn insert_narration(&self, row: &ScoreNarration) -> Result<(), ScoringRepoError> {
        let row = row.clone();
        self.execute(move |conn| insert_narration(conn, &row))
            .map_err(map_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate;
    use boba_domain::posting::{ApplyHints, Money, Offer, PayPeriod, Provenance, WorkMode};
    use boba_domain::scoring::factors::FactorWeights;
    use boba_domain::scoring::gates::Evidence;
    use boba_domain::scoring::rank::MODEL_VERSION;
    use boba_domain::scoring::rank::card as build_card;
    use rusqlite::params;

    const POSTING: &str = "pst_01SCORING";

    fn connection() -> Connection {
        let mut conn = Connection::open_in_memory().expect("in-memory sqlite");
        migrate::embedded()
            .to_latest(&mut conn)
            .expect("apply migrations");
        conn.execute(
            "INSERT INTO source (id, name, access_class, legal_tier, apply_capability, \
             min_interval_ms, enabled, added_at, updated_at) \
             VALUES ('greenhouse', 'Greenhouse', 'public_json', 'safe', 'link_out', 2000, 1, 't', 't')",
            [],
        )
        .expect("seed the source");
        let offer = Offer {
            id: POSTING.to_string(),
            source_id: "greenhouse".to_string(),
            external_id: "gh-42".to_string(),
            title: "Senior Data Engineer".to_string(),
            company: "Boba Labs".to_string(),
            locations: vec!["Remote, Spain".to_string()],
            work_mode: WorkMode::Remote,
            money: Some(Money {
                amount_minor: 5_500_000,
                currency: "EUR".to_string(),
                period: PayPeriod::Year,
            }),
            posted_at: Some("2026-10-04T00:00:00Z".to_string()),
            deadline: None,
            first_seen: "2026-10-04T00:00:00Z".to_string(),
            last_seen: "2026-10-16T00:00:00Z".to_string(),
            lang: "en".to_string(),
            description_html: "<p>dbt, Airflow.</p>".to_string(),
            description_text: "dbt, Airflow.".to_string(),
            requirements: vec!["dbt".to_string()],
            apply: ApplyHints {
                url: "https://boards.example.com/boba/gh-42".to_string(),
                email: None,
            },
        };
        let provenance = Provenance {
            posting_id: POSTING.to_string(),
            source_id: "greenhouse".to_string(),
            external_id: "gh-42".to_string(),
            url: offer.apply.url.clone(),
            raw: "{}".to_string(),
            fetched_at: "2026-10-04T00:00:00Z".to_string(),
        };
        crate::repos::postings::insert_offer(&conn, &offer, &provenance).expect("seed the offer");
        conn
    }

    /// One stored card, for the versions given.
    fn row(prefs_version: u32, created_at: &str) -> ScoreCardRow {
        let factors: Vec<FactorScore> = Factor::ALL
            .into_iter()
            .map(|factor| {
                if factor == Factor::Semantic {
                    FactorScore::missing_input(factor, 0.15, "no embeddings installed")
                } else {
                    FactorScore::scored(
                        factor,
                        88.0,
                        FactorWeights::DEFAULT.of(factor),
                        Confidence::High,
                        vec![Evidence::Field("description_text".to_string())],
                    )
                }
            })
            .collect();
        let card = build_card(MODEL_VERSION, factors, &FactorWeights::DEFAULT).expect("a card");
        ScoreCardRow::new(
            format!("sc_{prefs_version}"),
            POSTING,
            CardVersions {
                profile_version: 3,
                prefs_version,
                model_version: MODEL_VERSION,
            },
            card,
            GateReport {
                results: Vec::new(),
            },
            created_at,
        )
    }

    #[test]
    fn the_migration_creates_the_score_card_table_its_trigger_and_its_index() {
        let conn = connection();
        let names: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE name LIKE 'score_card%' ORDER BY name")
            .expect("prepare")
            .query_map([], |row| row.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("rows");
        assert_eq!(
            names,
            vec![
                "score_card".to_string(),
                "score_card_immutable".to_string(),
                "score_card_latest_idx".to_string(),
            ]
        );
    }

    #[test]
    fn a_card_round_trips_through_its_columns_and_json() {
        let conn = connection();
        let stored = row(2, "2026-10-16T09:00:00Z");
        insert_card(&conn, &stored).expect("insert");

        let read = card(&conn, POSTING, stored.versions())
            .expect("read")
            .expect("a card");
        assert_eq!(read, stored);
        assert_eq!(read.card.factors.len(), 11);
        assert_eq!(read.card.verdict, Verdict::Strong);
        assert_eq!(
            read.card.factor(Factor::Semantic).map(|f| f.score),
            Some(None),
            "a factor with no input stays without one"
        );
        assert_eq!(read.gates.results.len(), 0);
    }

    #[test]
    fn asking_for_a_version_that_was_never_scored_is_none() {
        let conn = connection();
        insert_card(&conn, &row(2, "2026-10-16T09:00:00Z")).expect("insert");
        assert_eq!(
            card(&conn, POSTING, row(5, "x").versions()).expect("read"),
            None
        );
        assert_eq!(
            card(&conn, "pst_other", row(2, "x").versions()).expect("read"),
            None
        );
    }

    // AC3: a card is written once. The same key is refused, not overwritten.
    #[test]
    fn a_second_card_for_the_same_versions_is_refused() {
        let conn = connection();
        let stored = row(2, "2026-10-16T09:00:00Z");
        insert_card(&conn, &stored).expect("insert");
        let mut again = stored.clone();
        again.id = "sc_other".to_string();
        again.card.rank = 0.0;
        assert!(insert_card(&conn, &again).is_err(), "the key is unique");
        assert_eq!(cards_for(&conn, POSTING).expect("read"), vec![stored]);
    }

    #[test]
    fn a_later_card_leaves_the_earlier_one_exactly_as_it_was() {
        let conn = connection();
        let earlier = row(2, "2026-10-16T09:00:00Z");
        insert_card(&conn, &earlier).expect("insert");
        let later = row(3, "2026-10-17T09:00:00Z");
        insert_card(&conn, &later).expect("insert");

        assert_eq!(
            cards_for(&conn, POSTING).expect("read"),
            vec![earlier.clone(), later]
        );
        assert_eq!(
            card(&conn, POSTING, earlier.versions()).expect("read"),
            Some(earlier)
        );
    }

    #[test]
    fn cards_are_read_oldest_first_and_the_newest_is_the_latest() {
        let conn = connection();
        insert_card(&conn, &row(3, "2026-10-17T09:00:00Z")).expect("insert");
        insert_card(&conn, &row(2, "2026-10-16T09:00:00Z")).expect("insert");

        let history = cards_for(&conn, POSTING).expect("read");
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].versions().prefs_version, 2, "oldest first");
        assert_eq!(
            latest_card(&conn, POSTING)
                .expect("read")
                .expect("a card")
                .versions()
                .prefs_version,
            3
        );
    }

    // AC3 is enforced by the schema, not only by the code path that writes.
    #[test]
    fn the_immutability_trigger_refuses_an_update() {
        let conn = connection();
        insert_card(&conn, &row(2, "2026-10-16T09:00:00Z")).expect("insert");
        let updated = conn.execute(
            "UPDATE score_card SET rank = 0.0 WHERE posting_id = ?1",
            params![POSTING],
        );
        assert!(updated.is_err(), "a stored card is immutable");
    }

    #[test]
    fn deleting_the_offer_takes_its_cards_with_it() {
        let conn = connection();
        insert_card(&conn, &row(2, "2026-10-16T09:00:00Z")).expect("insert");
        conn.execute("DELETE FROM posting WHERE id = ?1", params![POSTING])
            .expect("delete the offer");
        assert!(cards_for(&conn, POSTING).expect("read").is_empty());
    }

    #[test]
    fn a_card_that_has_lost_a_factor_is_a_corrupt_row_not_a_score() {
        let conn = connection();
        insert_card(&conn, &row(2, "2026-10-16T09:00:00Z")).expect("insert");
        // A row written by something that is not this build.
        conn.execute(
            "INSERT INTO score_card (id, posting_id, profile_version, prefs_version, \
             model_version, rank, fit, quality, verdict, confidence, factors_json, gates_json, \
             created_at) VALUES ('sc_bad', ?1, 9, 9, 9, 50.0, 50.0, 50.0, 'moderate', 'high', \
             '[]', '{}', '2026-10-16T09:00:00Z')",
            params![POSTING],
        )
        .expect("insert the corrupt row");
        let corrupt_versions = CardVersions {
            profile_version: 9,
            prefs_version: 9,
            model_version: 9,
        };
        assert!(card(&conn, POSTING, corrupt_versions).is_err());
    }

    /// A narration row for one card and language.
    fn narration(id: &str, lang: &str, text: &str, created_at: &str) -> ScoreNarration {
        ScoreNarration {
            id: id.to_string(),
            posting_id: POSTING.to_string(),
            score_card_id: "sc_2".to_string(),
            lang: lang.to_string(),
            text: text.to_string(),
            untrusted_origin: true,
            created_at: created_at.to_string(),
        }
    }

    #[test]
    fn the_migration_creates_the_narration_table_with_its_untrusted_origin() {
        let conn = connection();
        insert_card(&conn, &row(2, "2026-10-16T09:00:00Z")).expect("insert the card");
        let names: Vec<String> = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE name LIKE 'score_narration%' ORDER BY name",
            )
            .expect("prepare")
            .query_map([], |row| row.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("rows");
        assert_eq!(
            names,
            vec![
                "score_narration".to_string(),
                "score_narration_latest_idx".to_string(),
            ]
        );
        // AC4: the column exists from the start, defaults to untrusted, and
        // refuses anything but 0/1.
        conn.execute(
            "INSERT INTO score_narration (id, posting_id, score_card_id, lang, text, created_at) \
             VALUES ('nrt_plain', ?1, 'sc_2', 'en', 'Two sentences. Of text.', 't')",
            params![POSTING],
        )
        .expect("insert without the column");
        let untrusted: i64 = conn
            .query_row(
                "SELECT untrusted_origin FROM score_narration WHERE id = 'nrt_plain'",
                [],
                |row| row.get(0),
            )
            .expect("read");
        assert_eq!(untrusted, 1, "the default is untrusted");
        assert!(
            conn.execute(
                "INSERT INTO score_narration (id, posting_id, score_card_id, lang, text, \
                 untrusted_origin, created_at) VALUES ('nrt_bad', ?1, 'sc_2', 'en', 'x', 2, 't')",
                params![POSTING],
            )
            .is_err(),
            "only 0 and 1 are allowed"
        );
    }

    #[test]
    fn a_narration_round_trips_and_the_newest_wins_per_language() {
        let conn = connection();
        insert_card(&conn, &row(2, "2026-10-16T09:00:00Z")).expect("insert");
        insert_narration(
            &conn,
            &narration("nrt_1", "en", "First. Text.", "2026-10-16T10:00:00Z"),
        )
        .expect("first");
        insert_narration(
            &conn,
            &narration("nrt_2", "en", "Second. Text.", "2026-10-16T11:00:00Z"),
        )
        .expect("second");
        insert_narration(
            &conn,
            &narration("nrt_3", "es", "Hola. Mundo.", "2026-10-16T12:00:00Z"),
        )
        .expect("spanish");

        let latest = latest_narration(&conn, POSTING, "sc_2", "en")
            .expect("read")
            .expect("a narration");
        assert_eq!(latest.id, "nrt_2");
        assert_eq!(latest.text, "Second. Text.");
        assert!(latest.untrusted_origin);
        assert_eq!(latest.score_card_id, "sc_2");

        let spanish = latest_narration(&conn, POSTING, "sc_2", "es")
            .expect("read")
            .expect("a narration");
        assert_eq!(spanish.id, "nrt_3");
        assert_eq!(
            latest_narration(&conn, POSTING, "sc_2", "fr").expect("read"),
            None
        );
        // A narration never answers for a card it does not explain (AD-8).
        assert_eq!(
            latest_narration(&conn, POSTING, "sc_other", "en").expect("read"),
            None
        );
    }

    #[test]
    fn deleting_the_offer_takes_its_narrations_with_it() {
        let conn = connection();
        insert_card(&conn, &row(2, "2026-10-16T09:00:00Z")).expect("insert the card");
        insert_narration(&conn, &narration("nrt_1", "en", "First. Text.", "t")).expect("insert");
        conn.execute("DELETE FROM posting WHERE id = ?1", params![POSTING])
            .expect("delete the offer");
        assert_eq!(
            latest_narration(&conn, POSTING, "sc_2", "en").expect("read"),
            None
        );
    }
}
