//! Human-readable, upstream-compatible export (S10.1, FR-110, ADR-003).
//!
//! The export is **unencrypted by design**: it is the user's way out of the
//! app, so every file is plain JSON, CSV, Markdown, PDF or DOCX. The UI warns
//! before it runs (AC1); nothing here encrypts, redacts or phones home.
//!
//! The formats are the upstream ones (ADR-003): `job_search_tracker.csv` uses
//! the upstream header and status vocabulary (`saved` rows are left out — they
//! are not applications yet), `seen_jobs.json` is the append-only `{seen: …}`
//! map the upstream `rank_state.py` reads, and each application gets the
//! upstream `applications/<company>_<role>/` folder with its PDF and DOCX.
//! `profile.json` / `profile.md`, `applications.json` and `audit.csv` are ours.

use std::fs;
use std::path::{Path, PathBuf};

use crate::db::Store;

/// The tracker CSV header, byte-for-byte the upstream one (ADR-003). Upstream
/// readers split on commas, so free-form `notes` are sanitised before writing.
pub const TRACKER_HEADER: &str = "date,company,sector,role,role_type,channel,status,contact_person,fit_rating,notes,cv_file,cover_letter_file,source,deadline";

/// Every file name the export writes at the destination root.
pub const PROFILE_JSON: &str = "profile.json";
pub const PROFILE_MARKDOWN: &str = "profile.md";
pub const TRACKER_CSV: &str = "job_search_tracker.csv";
pub const APPLICATIONS_JSON: &str = "applications.json";
pub const SEEN_JOBS_JSON: &str = "seen_jobs.json";
pub const AUDIT_CSV: &str = "audit.csv";
/// The per-application folders (`applications/<company>_<role>/`).
pub const APPLICATIONS_DIR: &str = "applications";

/// Why an export did not complete. Nothing is rolled back: a failed export
/// leaves whatever it wrote for the user to delete.
#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    /// The chosen path is missing or is not a folder.
    #[error("the destination is not a folder")]
    Destination,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("sqlite error: {0}")]
    Sqlite(String),
    #[error("encode error: {0}")]
    Encode(String),
}

impl From<rusqlite::Error> for ExportError {
    fn from(error: rusqlite::Error) -> Self {
        ExportError::Sqlite(error.to_string())
    }
}

impl From<crate::db::StoreError> for ExportError {
    fn from(error: crate::db::StoreError) -> Self {
        ExportError::Sqlite(error.to_string())
    }
}

/// Write the unencrypted export into `destination` and return every file it
/// created (AC1).
pub fn export(store: &Store, destination: &Path) -> Result<Vec<PathBuf>, ExportError> {
    if !destination.is_dir() {
        return Err(ExportError::Destination);
    }
    let data = store.read(load).map_err(ExportError::from)?;
    let mut written = Vec::new();
    write_profile(destination, &data, &mut written)?;
    let folders = write_applications(destination, &data, &mut written)?;
    write_tracker(destination, &data, &folders, &mut written)?;
    write_seen_jobs(destination, &data, &mut written)?;
    write_audit(destination, &data, &mut written)?;
    Ok(written)
}

/// Everything the export writes, read in one pass through the store.
struct ExportData {
    profile_version: u32,
    profile_updated_at: String,
    facts: Vec<boba_domain::profile::ProfileFact>,
    applications: Vec<ApplicationRow>,
    /// The newest document of each `(application, kind)` that has a blob.
    documents: Vec<DocumentRow>,
    postings: Vec<PostingRow>,
    cards: std::collections::HashMap<String, boba_domain::scoring::rank::ScoreCardRow>,
    /// The status of each posting's application, when it has one.
    app_status: std::collections::HashMap<String, boba_domain::application::AppStatus>,
    audit: Vec<boba_domain::audit::AuditRow>,
}

struct ApplicationRow {
    id: String,
    posting_id: String,
    status: boba_domain::application::AppStatus,
    offer_snapshot_json: String,
    notes: Option<String>,
    created_at: String,
    /// When the application reached its current status (the tracker's `date`
    /// column); `created_at` when no status event recorded it.
    status_at: String,
    /// The portal the posting came from (the tracker's `source` column).
    source: Option<String>,
}

struct DocumentRow {
    application_id: String,
    kind: String,
    pdf: Option<Vec<u8>>,
    docx: Option<Vec<u8>>,
}

struct PostingRow {
    id: String,
    title: String,
    company: String,
    apply_url: String,
    deadline: Option<String>,
    posted_at: Option<String>,
    dedup_key: String,
    source: Option<String>,
}

fn load(conn: &rusqlite::Connection) -> Result<ExportData, crate::db::StoreError> {
    let profile_version = crate::repos::profile::profile_version_get(conn)?;
    let profile_updated_at: String = conn
        .query_row(
            "SELECT updated_at FROM profile WHERE id = 'profile'",
            [],
            |r| r.get(0),
        )
        .unwrap_or_default();
    let facts = crate::repos::profile::facts_current(conn)?;

    let mut statement = conn.prepare(
        "SELECT a.id, a.posting_id, a.status, a.offer_snapshot_json, a.notes, a.created_at, \
         COALESCE((SELECT ev.created_at FROM status_event ev WHERE ev.application_id = a.id \
          AND ev.to_status = a.status ORDER BY ev.created_at DESC, ev.id DESC LIMIT 1), \
          a.created_at), \
         (SELECT s.name FROM posting_source ps JOIN source s ON s.id = ps.source_id \
          WHERE ps.posting_id = a.posting_id ORDER BY ps.fetched_at DESC, ps.external_id LIMIT 1) \
         FROM application a ORDER BY a.created_at, a.id",
    )?;
    let applications = statement
        .query_map([], |row| {
            let status: String = row.get(2)?;
            Ok(ApplicationRow {
                id: row.get(0)?,
                posting_id: row.get(1)?,
                status: boba_domain::application::AppStatus::parse(&status)
                    .unwrap_or(boba_domain::application::AppStatus::Saved),
                offer_snapshot_json: row.get(3)?,
                notes: row.get(4)?,
                created_at: row.get(5)?,
                status_at: row.get(6)?,
                source: row.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut statement = conn.prepare(
        "SELECT application_id, kind, pdf_blob, docx_blob FROM document \
         WHERE pdf_blob IS NOT NULL OR docx_blob IS NOT NULL \
         ORDER BY application_id, kind, version",
    )?;
    let mut documents: Vec<DocumentRow> = Vec::new();
    for row in statement.query_map([], |row| {
        Ok(DocumentRow {
            application_id: row.get(0)?,
            kind: row.get(1)?,
            pdf: row.get(2)?,
            docx: row.get(3)?,
        })
    })? {
        let row = row?;
        // Later versions overwrite earlier ones: the folder carries the newest.
        match documents
            .iter_mut()
            .find(|d| d.application_id == row.application_id && d.kind == row.kind)
        {
            Some(existing) => *existing = row,
            None => documents.push(row),
        }
    }

    let mut statement = conn.prepare(
        "SELECT p.id, p.title, p.company, p.apply_url, p.deadline, p.posted_at, p.dedup_key, \
         (SELECT s.name FROM posting_source ps JOIN source s ON s.id = ps.source_id \
          WHERE ps.posting_id = p.id ORDER BY ps.fetched_at DESC, ps.external_id LIMIT 1) \
         FROM posting p ORDER BY p.last_seen DESC, p.id",
    )?;
    let postings = statement
        .query_map([], |row| {
            Ok(PostingRow {
                id: row.get(0)?,
                title: row.get(1)?,
                company: row.get(2)?,
                apply_url: row.get(3)?,
                deadline: row.get(4)?,
                posted_at: row.get(5)?,
                dedup_key: row.get(6)?,
                source: row.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let ids: Vec<String> = postings.iter().map(|p| p.id.clone()).collect();
    let cards = crate::repos::scoring::latest_cards(conn, &ids)?;
    let app_status = applications
        .iter()
        .map(|a| (a.posting_id.clone(), a.status))
        .collect();
    let audit = crate::repos::audit::all_rows(conn)
        .map_err(|error| crate::db::StoreError::Sqlite(error.to_string()))?;

    Ok(ExportData {
        profile_version,
        profile_updated_at,
        facts,
        applications,
        documents,
        postings,
        cards,
        app_status,
        audit,
    })
}

fn write_file(path: &Path, bytes: &[u8], written: &mut Vec<PathBuf>) -> Result<(), ExportError> {
    fs::write(path, bytes)?;
    written.push(path.to_path_buf());
    Ok(())
}

fn write_profile(
    destination: &Path,
    data: &ExportData,
    written: &mut Vec<PathBuf>,
) -> Result<(), ExportError> {
    let json = serde_json::json!({
        "version": data.profile_version,
        "updated_at": data.profile_updated_at,
        "facts": data.facts,
    });
    let text = serde_json::to_string_pretty(&json)
        .map_err(|error| ExportError::Encode(error.to_string()))?;
    write_file(&destination.join(PROFILE_JSON), text.as_bytes(), written)?;
    write_file(
        &destination.join(PROFILE_MARKDOWN),
        profile_markdown(data).as_bytes(),
        written,
    )
}

/// The Profile as one readable Markdown page: sections in editor order, both
/// language variants, and the evidence a fact carries.
fn profile_markdown(data: &ExportData) -> String {
    use boba_domain::profile::FactKind;

    let mut out = String::from("# Profile\n\n");
    out.push_str(&format!(
        "Profile version {}. Updated {}.\n",
        data.profile_version, data.profile_updated_at
    ));
    for kind in FactKind::ALL {
        let mut facts = data
            .facts
            .iter()
            .filter(|fact| fact.kind == kind)
            .peekable();
        if facts.peek().is_none() {
            continue;
        }
        out.push_str(&format!("\n## {}\n\n", title_case(kind.as_str())));
        for fact in facts {
            let mut line = String::from("- ");
            match fact.text_en.as_deref() {
                Some(text) => line.push_str(text),
                None => line.push_str(fact.text_es.as_deref().unwrap_or("")),
            }
            if let Some(es) = fact.text_es.as_deref()
                && fact.text_en.as_deref() != Some(es)
            {
                line.push_str(&format!(" (ES: {es})"));
            }
            if let Some(evidence) = fact.evidence.as_deref() {
                line.push_str(&format!(" — evidence: {evidence}"));
            }
            line.push_str(&format!(" [{}]", fact.provenance.as_str()));
            out.push_str(&line);
            out.push('\n');
        }
    }
    out
}

fn title_case(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Write one folder per application and return the folder path keyed by the
/// application id.
fn write_applications(
    destination: &Path,
    data: &ExportData,
    written: &mut Vec<PathBuf>,
) -> Result<std::collections::HashMap<String, PathBuf>, ExportError> {
    use boba_app::applications::OfferSnapshot;
    use std::collections::HashMap;

    let root = destination.join(APPLICATIONS_DIR);
    let mut folders: HashMap<String, PathBuf> = HashMap::new();
    let mut used: HashMap<String, usize> = HashMap::new();
    for application in &data.applications {
        let snapshot: OfferSnapshot =
            serde_json::from_str(&application.offer_snapshot_json).unwrap_or_default();
        let mut slug = format!("{}_{}", slug(&snapshot.company), slug(&snapshot.title));
        if slug == "_" {
            slug = "application".to_string();
        }
        let count = used.entry(slug.clone()).or_insert(0);
        *count += 1;
        if *count > 1 {
            slug = format!("{slug}_{count}");
        }
        let folder = root.join(&slug);
        fs::create_dir_all(&folder)?;
        for document in data
            .documents
            .iter()
            .filter(|document| document.application_id == application.id)
        {
            if let Some(pdf) = &document.pdf {
                write_file(&folder.join(format!("{}.pdf", document.kind)), pdf, written)?;
            }
            if let Some(docx) = &document.docx {
                write_file(
                    &folder.join(format!("{}.docx", document.kind)),
                    docx,
                    written,
                )?;
            }
        }
        folders.insert(application.id.clone(), folder);
    }

    let applications: Vec<serde_json::Value> = data
        .applications
        .iter()
        .map(|application| {
            let snapshot: OfferSnapshot =
                serde_json::from_str(&application.offer_snapshot_json).unwrap_or_default();
            serde_json::json!({
                "id": application.id,
                "posting_id": application.posting_id,
                "status": application.status.as_str(),
                "title": snapshot.title,
                "company": snapshot.company,
                "apply_url": snapshot.apply_url.as_deref().map(public_url),
                "lang": snapshot.lang,
                "deadline": snapshot.deadline,
                "notes": application.notes,
                "created_at": application.created_at,
            })
        })
        .collect();
    let text = serde_json::to_string_pretty(&serde_json::json!({ "applications": applications }))
        .map_err(|error| ExportError::Encode(error.to_string()))?;
    write_file(
        &destination.join(APPLICATIONS_JSON),
        text.as_bytes(),
        written,
    )?;
    Ok(folders)
}

/// A path-safe folder segment: lower-case alphanumerics, everything else one
/// underscore, trimmed (upstream's `<company>_<role>` shape).
fn slug(value: &str) -> String {
    let mut slug = String::new();
    let mut last_underscore = false;
    for ch in value.chars() {
        if ch.is_alphanumeric() {
            slug.extend(ch.to_lowercase());
            last_underscore = false;
        } else if !last_underscore {
            slug.push('_');
            last_underscore = true;
        }
    }
    slug.trim_matches('_').to_string()
}

/// One CSV field: quoted only when it has to be (RFC 4180).
fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn csv_line(fields: &[String]) -> String {
    let mut line = fields
        .iter()
        .map(|field| csv_field(field))
        .collect::<Vec<_>>()
        .join(",");
    line.push('\n');
    line
}

/// Free text upstream readers split by comma: drop what would break the row.
fn one_line(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if matches!(ch, ',' | '"' | '\n' | '\r') {
                ' '
            } else {
                ch
            }
        })
        .collect::<String>()
        .trim()
        .to_string()
}

/// A URL as it may leave the app: no query, no fragment and no embedded
/// credentials. A signed or session token can live in the query of an apply
/// URL; the export is unencrypted, so only the path ships (S10.1 review).
fn public_url(url: &str) -> String {
    let without_fragment = url.split('#').next().unwrap_or_default();
    let without_query = without_fragment.split('?').next().unwrap_or_default();
    match without_query.split_once("://") {
        Some((scheme, rest)) => {
            let authority_end = rest.find('/').unwrap_or(rest.len());
            let (authority, path) = rest.split_at(authority_end);
            let host = authority.rsplit('@').next().unwrap_or(authority);
            format!("{scheme}://{host}{path}")
        }
        None => without_query.to_string(),
    }
}

/// The `YYYY-MM-DD` of a stored RFC 3339 timestamp, empty when it is not one.
fn date_part(value: &str) -> String {
    let date = value.get(..10).unwrap_or_default();
    let shaped = date.len() == 10
        && date.as_bytes()[4] == b'-'
        && date.as_bytes()[7] == b'-'
        && date
            .bytes()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit());
    if shaped {
        date.to_string()
    } else {
        String::new()
    }
}

/// The upstream band name for a stored score (upstream `rank_state.py` BANDS).
fn band(card: &boba_domain::scoring::rank::ScoreCardRow) -> String {
    use boba_domain::scoring::rank::Verdict;
    match card.card.verdict {
        Verdict::Strong => "Strong Fit",
        Verdict::Good => "Good Fit",
        Verdict::Moderate => "Moderate Fit",
        Verdict::Weak => "Weak Fit",
        Verdict::Poor => "Poor Fit",
    }
    .to_string()
}

fn write_tracker(
    destination: &Path,
    data: &ExportData,
    folders: &std::collections::HashMap<String, PathBuf>,
    written: &mut Vec<PathBuf>,
) -> Result<(), ExportError> {
    use boba_app::applications::OfferSnapshot;

    let mut csv = String::from(TRACKER_HEADER);
    csv.push('\n');
    for application in &data.applications {
        // AC1: `saved` rows are not applications yet, so the tracker leaves
        // them out; the Applications JSON keeps them.
        if application.status == boba_domain::application::AppStatus::Saved {
            continue;
        }
        let snapshot: OfferSnapshot =
            serde_json::from_str(&application.offer_snapshot_json).unwrap_or_default();
        let folder = folders
            .get(&application.id)
            .and_then(|path| path.file_name())
            .map(|name| format!("{APPLICATIONS_DIR}/{}", name.to_string_lossy()))
            .unwrap_or_default();
        let has = |kind: &str| {
            data.documents
                .iter()
                .any(|document| document.application_id == application.id && document.kind == kind)
        };
        let deadline = snapshot
            .deadline
            .clone()
            .or_else(|| {
                data.postings
                    .iter()
                    .find(|posting| posting.id == application.posting_id)
                    .and_then(|posting| posting.deadline.clone())
            })
            .map(|value| date_part(&value))
            .unwrap_or_default();
        let fit = data
            .cards
            .get(&application.posting_id)
            .map(band)
            .unwrap_or_default();
        let row = vec![
            date_part(&application.status_at),
            snapshot.company,
            String::new(),
            snapshot.title,
            String::new(),
            String::new(),
            application.status.as_str().to_string(),
            String::new(),
            fit,
            one_line(application.notes.as_deref().unwrap_or_default()),
            if has("cv") {
                format!("{folder}/cv.pdf")
            } else {
                String::new()
            },
            if has("cover_letter") {
                format!("{folder}/cover_letter.pdf")
            } else {
                String::new()
            },
            application.source.clone().unwrap_or_default(),
            deadline,
        ];
        csv.push_str(&csv_line(&row));
    }
    write_file(&destination.join(TRACKER_CSV), csv.as_bytes(), written)
}

/// The upstream `PASS` / `FLAG` / `FAIL` spelling of a gate verdict.
fn verdict_text(verdict: boba_domain::scoring::gates::Verdict) -> &'static str {
    use boba_domain::scoring::gates::Verdict;
    match verdict {
        Verdict::Pass => "PASS",
        Verdict::Flag => "FLAG",
        Verdict::Veto => "FAIL",
    }
}

fn gate(
    card: &boba_domain::scoring::rank::ScoreCardRow,
    kind: boba_domain::scoring::gates::GateKind,
) -> Option<&boba_domain::scoring::gates::GateResult> {
    card.gates.results.iter().find(|result| result.gate == kind)
}

fn write_seen_jobs(
    destination: &Path,
    data: &ExportData,
    written: &mut Vec<PathBuf>,
) -> Result<(), ExportError> {
    use boba_domain::scoring::gates::GateKind;
    use serde_json::{Map, Value, json};
    use std::collections::BTreeMap;

    let mut seen: BTreeMap<String, Value> = BTreeMap::new();
    for posting in &data.postings {
        let key = if posting.dedup_key.is_empty() {
            let fallback = format!("{}-{}", slug(&posting.company), slug(&posting.title));
            if fallback == "-" {
                posting.id.clone()
            } else {
                fallback
            }
        } else {
            posting.dedup_key.clone()
        };
        let card = data.cards.get(&posting.id);
        let status = if card.is_some() || data.app_status.contains_key(&posting.id) {
            "ranked"
        } else {
            "new"
        };
        let mut entry = Map::new();
        entry.insert("key".into(), json!(key.clone()));
        entry.insert("title".into(), json!(posting.title));
        entry.insert("company".into(), json!(posting.company));
        entry.insert("url".into(), json!(public_url(&posting.apply_url)));
        entry.insert(
            "portal".into(),
            json!(posting.source.clone().unwrap_or_default()),
        );
        entry.insert("status".into(), json!(status));
        entry.insert(
            "posted_date".into(),
            json!(
                posting
                    .posted_at
                    .as_deref()
                    .map(date_part)
                    .unwrap_or_default()
            ),
        );
        entry.insert(
            "deadline".into(),
            json!(
                posting
                    .deadline
                    .as_deref()
                    .map(date_part)
                    .unwrap_or_default()
            ),
        );
        if let Some(card) = card {
            entry.insert("rank_score".into(), json!(card.card.rank.round() as i64));
            entry.insert("rank_verdict".into(), json!(band(card)));
            entry.insert("rank_date".into(), json!(date_part(&card.created_at)));
            if let Some(location) = gate(card, GateKind::Location) {
                entry.insert(
                    "location_verdict".into(),
                    json!(verdict_text(location.verdict)),
                );
            }
            if let Some(language) = gate(card, GateKind::Language) {
                entry.insert(
                    "language_gate".into(),
                    json!(verdict_text(language.verdict)),
                );
                if language.verdict != boba_domain::scoring::gates::Verdict::Pass {
                    entry.insert("language_note".into(), json!(reason_text(language.reason)));
                }
            }
            let collect = |missing: bool| {
                let mut out: Vec<String> = Vec::new();
                for factor in &card.card.factors {
                    let values = if missing {
                        &factor.missing
                    } else {
                        &factor.matched
                    };
                    for value in values {
                        if !out.contains(value) {
                            out.push(value.clone());
                        }
                    }
                }
                out.truncate(3);
                out
            };
            entry.insert("strengths".into(), json!(collect(false)));
            entry.insert("gaps".into(), json!(collect(true)));
        }
        seen.insert(key, Value::Object(entry));
    }
    let text = serde_json::to_string_pretty(&json!({ "seen": seen }))
        .map_err(|error| ExportError::Encode(error.to_string()))?;
    write_file(&destination.join(SEEN_JOBS_JSON), text.as_bytes(), written)
}

/// A gate reason in its stored snake_case form (what upstream stores).
fn reason_text(reason: boba_domain::scoring::gates::GateReason) -> String {
    serde_json::to_value(reason)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn write_audit(
    destination: &Path,
    data: &ExportData,
    written: &mut Vec<PathBuf>,
) -> Result<(), ExportError> {
    let mut csv = String::from(
        "seq,id,created_at,phase,action,outcome,correlation_id,intent_id,actor,target_kind,\
         target_id,detail_json,payload_blake3,prev_hash,hash\n",
    );
    for row in &data.audit {
        let fields = vec![
            row.seq.to_string(),
            row.id.clone(),
            row.created_at.clone(),
            row.phase.as_str().to_string(),
            row.action.clone(),
            row.outcome
                .map(|outcome| outcome.as_str().to_string())
                .unwrap_or_default(),
            row.correlation_id.clone(),
            row.intent_id.clone().unwrap_or_default(),
            row.actor.clone(),
            row.target_kind.clone(),
            row.target_id.clone().unwrap_or_default(),
            row.detail_json.clone(),
            row.payload_blake3.clone().unwrap_or_default(),
            row.prev_hash.clone(),
            row.hash.clone(),
        ];
        csv.push_str(&csv_line(&fields));
    }
    write_file(&destination.join(AUDIT_CSV), csv.as_bytes(), written)
}

/// The [`ExportPort`](boba_app::ports::export::ExportPort) adapter (ADR-012):
/// the open store holds every row the files are built from.
impl boba_app::ports::export::ExportPort for Store {
    fn export(
        &self,
        destination: &Path,
    ) -> Result<Vec<PathBuf>, boba_app::ports::export::ExportError> {
        use boba_app::ports::export::ExportError as PortError;

        crate::export::export(self, destination).map_err(|error| match error {
            ExportError::Destination => PortError::Destination,
            ExportError::Io(_) | ExportError::Sqlite(_) | ExportError::Encode(_) => {
                PortError::Internal
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::db::StoreConfig;
    use crate::keys::DbKey;

    fn temp() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp dir")
    }

    fn store(dir: &Path) -> Store {
        let config = StoreConfig::new(dir.join("boba.db"), dir.join("snapshots"));
        Store::open_with_key(config, DbKey::generate().expect("key")).expect("store")
    }

    fn exec(store: &Store, sql: impl Into<String>) {
        let sql = sql.into();
        store
            .execute(move |conn| {
                conn.execute_batch(&sql)?;
                Ok(())
            })
            .expect("seed");
    }

    fn seed_source(store: &Store) {
        exec(
            store,
            "INSERT INTO source (id, name, access_class, legal_tier, apply_capability, \
             min_interval_ms, enabled, added_at, updated_at) \
             VALUES ('src_1', 'linkedin', 'rss', 'safe', 'link_out', 1000, 1, \
             '2026-10-01T00:00:00Z', '2026-10-01T00:00:00Z');",
        );
    }

    fn seed_posting(store: &Store, id: &str, title: &str, company: &str, dedup_key: &str) {
        let sql = format!(
            "INSERT INTO posting (id, source_id, external_id, title, company, work_mode, \
             posted_at, deadline, first_seen, last_seen, lang, apply_url, dedup_key) \
             VALUES ('{id}', 'src_1', 'ext_{id}', '{title}', '{company}', 'remote', \
             '2026-10-01T00:00:00Z', '2026-11-01T00:00:00Z', '2026-10-01T00:00:00Z', \
             '2026-10-02T00:00:00Z', 'en', 'https://acme.test/jobs/{id}', '{dedup_key}'); \
             INSERT INTO posting_source (posting_id, source_id, external_id, url, raw, fetched_at) \
             VALUES ('{id}', 'src_1', 'ext_{id}', 'https://acme.test/jobs/{id}', '{{}}', \
             '2026-10-02T00:00:00Z');"
        );
        exec(store, &sql);
    }

    fn snapshot(title: &str, company: &str) -> String {
        format!(
            r#"{{"title":"{title}","company":"{company}","apply_url":"https://acme.test/jobs/1","lang":"en","deadline":"2026-11-01T00:00:00Z"}}"#
        )
    }

    fn seed_application(
        store: &Store,
        id: &str,
        posting_id: &str,
        status: &str,
        company: &str,
        title: &str,
        notes: Option<&str>,
    ) {
        let snapshot = snapshot(title, company);
        let notes = notes
            .map(|value| format!("'{value}'"))
            .unwrap_or_else(|| "NULL".to_string());
        let sql = format!(
            "INSERT INTO application (id, posting_id, status, offer_snapshot_json, notes, \
             next_action, created_at, updated_at) \
             VALUES ('{id}', '{posting_id}', '{status}', '{snapshot}', {notes}, NULL, \
             '2026-10-03T09:00:00Z', '2026-10-04T09:00:00Z');"
        );
        exec(store, &sql);
    }

    fn seed_document(store: &Store, id: &str, application_id: &str, kind: &str) {
        let sql = format!(
            "INSERT INTO document (id, application_id, kind, lang, template, version, \
             model_json, pdf_blob, docx_blob, created_at, updated_at) \
             VALUES ('{id}', '{application_id}', '{kind}', 'en', 'classic', 1, '{{}}', \
             x'25504446', x'504b0304', '2026-10-04T09:00:00Z', '2026-10-04T09:00:00Z');"
        );
        exec(store, &sql);
    }

    fn seed_profile(store: &Store) {
        exec(
            store,
            "UPDATE profile SET version = 1, updated_at = '2026-10-02T00:00:00Z' \
             WHERE id = 'profile'; \
             INSERT INTO profile_fact (id, item_id, kind, text_en, text_es, evidence, \
             detail_json, profile_version, provenance, created_at) \
             VALUES ('pf_1', 'pit_1', 'skill', 'Rust', 'Rust', 'Led a Rust service', '{}', 1, \
             'user', '2026-10-01T00:00:00Z');",
        );
    }

    fn seed_audit(store: &Store) {
        exec(
            store,
            "INSERT INTO audit_event (seq, id, created_at, phase, action, outcome, \
             correlation_id, intent_id, actor, target_kind, target_id, detail_json, \
             payload_blake3, prev_hash, hash) VALUES \
             (1, 'aud_1', '2026-10-04T09:00:00Z', 'intent', 'apply.submitted', NULL, 'corr_1', \
             NULL, 'core', 'application', 'app_1', '{\"count\":1}', NULL, \
             '0000000000000000000000000000000000000000000000000000000000000000', 'aa'), \
             (2, 'aud_2', '2026-10-04T09:00:01Z', 'outcome', 'apply.submitted', 'ok', 'corr_1', \
             'aud_1', 'core', 'application', 'app_1', '{\"ok\":true}', 'deadbeef', 'aa', 'bb');",
        );
    }

    fn seed_card(store: &Store, posting_id: &str) {
        use boba_domain::scoring::factors::{Factor, FactorScore};
        use boba_domain::scoring::gates::{
            Confidence, GateKind, GateReason, GateReport, GateResult, Verdict,
        };

        let factors: Vec<FactorScore> = Factor::ALL
            .iter()
            .map(|factor| FactorScore {
                factor: *factor,
                score: Some(70.0),
                weight: 1.0 / 11.0,
                effective_weight: 1.0 / 11.0,
                matched: if *factor == Factor::Skills {
                    vec!["Rust".to_string()]
                } else {
                    Vec::new()
                },
                missing: if *factor == Factor::Skills {
                    vec!["Kubernetes".to_string()]
                } else {
                    Vec::new()
                },
                evidence: Vec::new(),
                confidence: Confidence::High,
            })
            .collect();
        let gates = GateReport {
            results: GateKind::ALL
                .iter()
                .map(|kind| GateResult {
                    gate: *kind,
                    verdict: if *kind == GateKind::Location {
                        Verdict::Flag
                    } else {
                        Verdict::Pass
                    },
                    reason: if *kind == GateKind::Location {
                        GateReason::Unverified
                    } else {
                        GateReason::Satisfied
                    },
                    evidence: None,
                    confidence: Confidence::High,
                })
                .collect(),
        };
        let factors = serde_json::to_string(&factors).expect("factors json");
        let gates = serde_json::to_string(&gates).expect("gates json");
        let sql = format!(
            "INSERT INTO score_card (id, posting_id, profile_version, prefs_version, \
             model_version, rank, fit, quality, verdict, confidence, factors_json, gates_json, \
             created_at) VALUES ('sc_1', '{posting_id}', 1, 1, 1, 78.0, 85.0, 70.0, 'strong', \
             'high', '{factors}', '{gates}', '2026-10-05T09:00:00Z');"
        );
        exec(store, &sql);
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).expect("read export file")
    }

    #[test]
    fn refuses_a_destination_that_is_not_a_folder() {
        let dir = temp();
        let store = store(dir.path());

        let error = export(&store, &dir.path().join("missing")).expect_err("not a folder");

        assert!(matches!(error, ExportError::Destination));
    }

    #[test]
    fn writes_the_profile_as_json_and_markdown() {
        let dir = temp();
        let store = store(dir.path());
        seed_profile(&store);
        let out = dir.path().join("out");
        fs::create_dir(&out).expect("out");

        export(&store, &out).expect("export");

        let json: serde_json::Value =
            serde_json::from_str(&read(&out.join(PROFILE_JSON))).expect("profile json");
        assert_eq!(json["version"], 1);
        assert_eq!(json["facts"][0]["text_en"], "Rust");
        assert_eq!(json["facts"][0]["kind"], "skill");
        assert_eq!(json["facts"][0]["provenance"], "user");

        let markdown = read(&out.join(PROFILE_MARKDOWN));
        assert!(markdown.contains("## Skill"), "{markdown}");
        assert!(markdown.contains("Rust"), "{markdown}");
        assert!(markdown.contains("Led a Rust service"), "{markdown}");
    }

    #[test]
    fn writes_the_tracker_csv_with_the_upstream_header_and_leaves_saved_rows_out() {
        let dir = temp();
        let store = store(dir.path());
        let out = dir.path().join("out");
        fs::create_dir(&out).expect("out");
        seed_source(&store);
        seed_posting(
            &store,
            "pst_1",
            "Data Scientist",
            "Acme",
            "acme-data-scientist",
        );
        seed_posting(&store, "pst_2", "Designer", "Globex", "globex-designer");
        seed_application(
            &store,
            "app_1",
            "pst_1",
            "applied",
            "Acme",
            "Data Scientist",
            Some("Great, \"culture\"\nnext week"),
        );
        seed_application(
            &store, "app_2", "pst_2", "saved", "Globex", "Designer", None,
        );
        seed_document(&store, "doc_cv", "app_1", "cv");
        exec(
            &store,
            "INSERT INTO status_event (id, application_id, from_status, to_status, source, \
             reason, created_at) VALUES ('evt_1', 'app_1', 'saved', 'applied', 'user', NULL, \
             '2026-10-06T12:00:00Z');",
        );

        export(&store, &out).expect("export");

        let csv = read(&out.join(TRACKER_CSV));
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines[0], TRACKER_HEADER, "the upstream header, verbatim");
        assert_eq!(lines.len(), 2, "the saved row is left out");
        let fields: Vec<&str> = lines[1].split(',').collect();
        assert_eq!(fields.len(), 14, "one row stays fourteen columns wide");
        assert_eq!(
            fields[0], "2026-10-06",
            "the date column is the day the status was reached"
        );
        assert_eq!(fields[1], "Acme");
        assert_eq!(fields[3], "Data Scientist");
        assert_eq!(fields[6], "applied", "the canonical upstream spelling");
        assert!(
            !fields[9].contains(',') && !fields[9].contains('"') && !fields[9].contains('\n'),
            "notes must not break an upstream comma split: {:?}",
            fields[9]
        );
        assert!(fields[9].contains("Great"), "{:?}", fields[9]);
        assert_eq!(fields[10], "applications/acme_data_scientist/cv.pdf");
        assert_eq!(fields[13], "2026-11-01");
    }

    #[test]
    fn writes_applications_json_with_every_status_and_its_history() {
        let dir = temp();
        let store = store(dir.path());
        let out = dir.path().join("out");
        fs::create_dir(&out).expect("out");
        seed_source(&store);
        seed_posting(
            &store,
            "pst_1",
            "Data Scientist",
            "Acme",
            "acme-data-scientist",
        );
        seed_application(
            &store,
            "app_1",
            "pst_1",
            "saved",
            "Acme",
            "Data Scientist",
            None,
        );

        export(&store, &out).expect("export");

        let json: serde_json::Value =
            serde_json::from_str(&read(&out.join(APPLICATIONS_JSON))).expect("applications json");
        let applications = json["applications"].as_array().expect("array");
        assert_eq!(applications.len(), 1, "saved rows are included here");
        assert_eq!(applications[0]["status"], "saved");
        assert_eq!(applications[0]["title"], "Data Scientist");
        assert_eq!(applications[0]["company"], "Acme");
    }

    #[test]
    fn writes_application_folders_with_the_pdf_and_docx() {
        let dir = temp();
        let store = store(dir.path());
        let out = dir.path().join("out");
        fs::create_dir(&out).expect("out");
        seed_source(&store);
        seed_posting(
            &store,
            "pst_1",
            "Data Scientist",
            "Acme",
            "acme-data-scientist",
        );
        seed_application(
            &store,
            "app_1",
            "pst_1",
            "applied",
            "Acme",
            "Data Scientist",
            None,
        );
        seed_document(&store, "doc_cv", "app_1", "cv");
        seed_document(&store, "doc_letter", "app_1", "cover_letter");

        export(&store, &out).expect("export");

        let folder = out.join(APPLICATIONS_DIR).join("acme_data_scientist");
        assert_eq!(fs::read(folder.join("cv.pdf")).expect("cv pdf"), b"%PDF");
        assert_eq!(
            fs::read(folder.join("cv.docx")).expect("cv docx"),
            b"PK\x03\x04"
        );
        assert!(folder.join("cover_letter.pdf").is_file());
        assert!(folder.join("cover_letter.docx").is_file());
    }

    #[test]
    fn writes_seen_jobs_in_the_upstream_seen_map() {
        let dir = temp();
        let store = store(dir.path());
        let out = dir.path().join("out");
        fs::create_dir(&out).expect("out");
        seed_source(&store);
        seed_posting(
            &store,
            "pst_1",
            "Data Scientist",
            "Acme",
            "acme-data-scientist",
        );
        seed_card(&store, "pst_1");

        export(&store, &out).expect("export");

        let json: serde_json::Value =
            serde_json::from_str(&read(&out.join(SEEN_JOBS_JSON))).expect("seen jobs json");
        let entry = &json["seen"]["acme-data-scientist"];
        assert_eq!(entry["key"], "acme-data-scientist");
        // The ranked-entry schema upstream's `rank_state.py` reads and writes
        // (`docs/research/upstream-analysis.md`, `tests/test_rank_state.py`).
        let expected: std::collections::BTreeSet<&str> = [
            "key",
            "title",
            "company",
            "url",
            "portal",
            "status",
            "posted_date",
            "deadline",
            "rank_score",
            "rank_verdict",
            "rank_date",
            "location_verdict",
            "language_gate",
            "strengths",
            "gaps",
        ]
        .into_iter()
        .collect();
        let actual: std::collections::BTreeSet<&str> = entry
            .as_object()
            .expect("entry object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(actual, expected, "the upstream entry schema");
        assert_eq!(entry["title"], "Data Scientist");
        assert_eq!(entry["company"], "Acme");
        assert_eq!(entry["portal"], "linkedin");
        assert_eq!(entry["status"], "ranked");
        assert_eq!(entry["rank_score"], 78);
        assert_eq!(entry["rank_verdict"], "Strong Fit");
        assert_eq!(entry["location_verdict"], "FLAG");
        assert_eq!(entry["language_gate"], "PASS");
        assert_eq!(entry["strengths"][0], "Rust");
        assert_eq!(entry["gaps"][0], "Kubernetes");
        assert_eq!(entry["posted_date"], "2026-10-01");
        assert_eq!(entry["deadline"], "2026-11-01");
    }

    #[test]
    fn strips_tokens_from_exported_urls() {
        let dir = temp();
        let store = store(dir.path());
        let out = dir.path().join("out");
        fs::create_dir(&out).expect("out");
        seed_source(&store);
        exec(
            &store,
            "INSERT INTO posting (id, source_id, external_id, title, company, work_mode, \
             posted_at, deadline, first_seen, last_seen, lang, apply_url, dedup_key) \
             VALUES ('pst_1', 'src_1', 'ext_1', 'Data Scientist', 'Acme', 'remote', \
             '2026-10-01T00:00:00Z', NULL, '2026-10-01T00:00:00Z', '2026-10-02T00:00:00Z', \
             'en', 'https://acme.test/jobs/1?token=super-secret#apply', \
             'acme-data-scientist'); \
             INSERT INTO posting_source (posting_id, source_id, external_id, url, raw, fetched_at) \
             VALUES ('pst_1', 'src_1', 'ext_1', 'https://acme.test/jobs/1', '{}', \
             '2026-10-02T00:00:00Z'); \
             INSERT INTO application (id, posting_id, status, offer_snapshot_json, notes, \
             next_action, created_at, updated_at) VALUES ('app_1', 'pst_1', 'applied', \
             '{\"title\":\"Data Scientist\",\"company\":\"Acme\",\
             \"apply_url\":\"https://user:pw@acme.test/jobs/1?session=super-secret\",\
             \"lang\":\"en\",\"deadline\":null}', NULL, NULL, \
             '2026-10-03T09:00:00Z', '2026-10-04T09:00:00Z');",
        );

        export(&store, &out).expect("export");

        let applications = read(&out.join(APPLICATIONS_JSON));
        assert!(
            !applications.contains("super-secret") && !applications.contains("user:pw"),
            "a signed or session token must not leave the app: {applications}"
        );
        assert!(applications.contains("https://acme.test/jobs/1"));
        let seen = read(&out.join(SEEN_JOBS_JSON));
        assert!(
            !seen.contains("super-secret"),
            "the seen map must not carry the query: {seen}"
        );
        assert!(seen.contains("https://acme.test/jobs/1"));
    }

    #[test]
    fn never_writes_credential_bearing_tables() {
        let dir = temp();
        let store = store(dir.path());
        let out = dir.path().join("out");
        fs::create_dir(&out).expect("out");
        seed_source(&store);
        seed_posting(
            &store,
            "pst_1",
            "Data Scientist",
            "Acme",
            "acme-data-scientist",
        );
        exec(
            &store,
            "INSERT INTO email_account (id, provider, address, mode, client_id, \
             client_id_kind, scopes, secret_ref, connected_at, updated_at) \
             VALUES ('em_1', 'imap_smtp', 'secret@example.com', 'read_send', 'cid', 'byo', \
             'read', 'keychain://boba/email/em_1', '2026-10-01T00:00:00Z', \
             '2026-10-01T00:00:00Z'); \
             INSERT INTO setting (key, value, updated_at) \
             VALUES ('email.token', 'tok_super_secret', '2026-10-01T00:00:00Z');",
        );

        export(&store, &out).expect("export");

        let text = all_text(&out);
        assert!(!text.contains("secret@example.com"), "no mailbox address");
        assert!(!text.contains("tok_super_secret"), "no token value");
        assert!(!text.contains("keychain://"), "no keychain reference");
    }

    /// Every exported file, decoded as text (PDF/DOCX blobs are not text; the
    /// search strings here only matter in the text files).
    fn all_text(root: &Path) -> String {
        let mut out = String::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).expect("read dir") {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    out.push_str(&String::from_utf8_lossy(
                        &fs::read(&path).expect("read file"),
                    ));
                    out.push('\n');
                }
            }
        }
        out
    }

    #[test]
    fn writes_the_audit_log_as_csv() {
        let dir = temp();
        let store = store(dir.path());
        let out = dir.path().join("out");
        fs::create_dir(&out).expect("out");
        seed_audit(&store);

        export(&store, &out).expect("export");

        let csv = read(&out.join(AUDIT_CSV));
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(
            lines[0],
            "seq,id,created_at,phase,action,outcome,correlation_id,intent_id,actor,\
             target_kind,target_id,detail_json,payload_blake3,prev_hash,hash"
        );
        assert_eq!(lines.len(), 3, "header plus both audit rows");
        assert!(lines[1].contains("apply.submitted"), "{}", lines[1]);
        assert!(lines[2].starts_with("2,aud_2,"), "{}", lines[2]);
    }
}
