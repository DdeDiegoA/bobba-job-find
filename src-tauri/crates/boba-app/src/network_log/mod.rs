//! Network activity log (S10.4, NFR-1).
//!
//! Every outbound request the app makes on the user's behalf records its
//! destination here — the **host only**, never a path, a query, a header or a
//! payload (FR-40 redaction, AD-13). The view groups the log by purpose, and
//! [`Purpose`] enumerates exactly what NFR-1 allows: Sources, email, the Agent
//! CLI, update checks, the model download and FX rates. A telemetry endpoint
//! cannot be represented at all, which is what makes "there are no telemetry
//! endpoints" a property of the type rather than a promise in the UI.
//!
//! The log is deliberately cheap: a [`record`] from a request path takes a
//! mutex for the duration of one map update and never fails the request (a
//! privacy view must not be able to break the network). It lives in memory and
//! dies with the process; destinations the app is *configured* to contact are
//! passed to [`NetworkLog::snapshot`] as [`Declared`] rows, so the view is
//! complete before the first run.

pub mod fx;

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use specta::Type;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// The most destinations the log keeps; the oldest is dropped past this.
pub const MAX_DESTINATIONS: usize = 64;

/// The setting that turns update checks on (off by default, PRD §5.1). S10.5
/// wires the updater to it; until then the view shows no update destination.
pub const UPDATE_CHECK_KEY: &str = "network.update_checks";

/// The updater's release host (`tauri.conf.json` plugin endpoint).
pub const UPDATE_HOST: &str = "github.com";

/// Where `hf-hub` downloads the optional local model from (S6.9).
pub const MODEL_HOST: &str = "huggingface.co";

/// Why the app contacted a destination (NFR-1 a–e, design §9.13).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Type,
)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    /// Enabled Sources, custom feeds and the browser driver's pages.
    Sources,
    /// The connected email provider (OAuth, send, read, IMAP/SMTP).
    Email,
    /// The Agent CLI's own provider: `cli:` labels, because the host is the
    /// CLI's business, not Boba's.
    AgentCli,
    /// Update checks, when the user enabled them.
    Update,
    /// The optional local-model download.
    ModelDownload,
    /// FX rates, when the user enabled them.
    Fx,
}

impl Purpose {
    /// Every purpose, in the order the view groups them.
    pub const ALL: [Self; 6] = [
        Self::Sources,
        Self::Email,
        Self::AgentCli,
        Self::Update,
        Self::ModelDownload,
        Self::Fx,
    ];
}

/// One destination row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Destination {
    /// The host (`remotive.com`) or, where there is no Boba-side HTTP call, the
    /// label the caller recorded (`claude`, `imap.fastmail.com`).
    pub name: String,
    pub purpose: Purpose,
    /// When it was last contacted, UTC RFC 3339; `None` for a destination that
    /// is merely configured (nothing has been sent yet this session).
    pub last_seen: Option<String>,
    /// Requests recorded this session.
    pub requests: u32,
    /// A short, non-secret detail (e.g. the model id); never a path or a token.
    pub note: Option<String>,
}

/// One purpose's rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct PurposeGroup {
    pub purpose: Purpose,
    pub destinations: Vec<Destination>,
}

/// What Settings → Privacy → Network activity renders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct NetworkActivity {
    /// One group per purpose that has a destination, in [`Purpose::ALL`] order.
    pub groups: Vec<PurposeGroup>,
    /// Always `false`: [`Purpose`] has no telemetry variant, so no endpoint
    /// outside NFR-1 can be recorded or shown (AC1).
    pub telemetry: bool,
}

/// Why a destination was refused. Nothing here crosses IPC: a request path
/// records with [`record`], which swallows the error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NetworkLogError {
    #[error("destination is empty")]
    Empty,
    #[error("destination is not an https URL: {0}")]
    NotHttps(String),
    #[error("destination has no host: {0}")]
    MissingHost(String),
    #[error("destination carries a path, a query or whitespace: {0}")]
    Impure(String),
}

/// A destination the app is configured to contact but has not (yet) this
/// session, e.g. the inbox of a connected account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declared {
    pub purpose: Purpose,
    pub name: String,
    /// See [`Destination::note`].
    pub note: Option<String>,
    /// When the app last knew this destination was contacted outside this
    /// session (the model download's completion time), if it knows.
    pub last_seen: Option<String>,
}

impl Declared {
    /// A declared row, validated like a recorded one.
    pub fn new(purpose: Purpose, name: &str) -> Result<Self, NetworkLogError> {
        Ok(Self {
            purpose,
            name: clean_label(name)?,
            note: None,
            last_seen: None,
        })
    }

    /// Attach a non-secret detail (the model id, the account label).
    pub fn with_note(mut self, note: &str) -> Self {
        self.note = Some(note.to_string());
        self
    }

    /// Attach the time the last contact is known to have happened.
    pub fn with_last_seen(mut self, at: &str) -> Self {
        if !at.trim().is_empty() {
            self.last_seen = Some(at.to_string());
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    last_seen: String,
    requests: u32,
    note: Option<String>,
}

/// The in-memory log. One instance in production ([`record`]); tests build
/// their own.
#[derive(Debug, Default)]
pub struct NetworkLog {
    entries: Mutex<BTreeMap<(Purpose, String), Entry>>,
}

impl NetworkLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one request to `url`: the host is kept, everything else dropped.
    /// The timestamp is the wall clock; [`Self::record_at`] is the test seam.
    pub fn record(&self, purpose: Purpose, url: &str) -> Result<(), NetworkLogError> {
        let now = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_default();
        self.record_at(purpose, url, &now)
    }

    /// Record one request at an explicit time (tests, and callers that own a
    /// clock).
    pub fn record_at(&self, purpose: Purpose, url: &str, at: &str) -> Result<(), NetworkLogError> {
        let host = host_of(url)?;
        self.record_host(purpose, &host, at)
    }

    /// Record a destination that is not an HTTP URL: an Agent CLI id, an
    /// IMAP/SMTP host. `label` must be a bare host or id.
    pub fn record_host(
        &self,
        purpose: Purpose,
        label: &str,
        at: &str,
    ) -> Result<(), NetworkLogError> {
        let name = clean_label(label)?;
        let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        match entries.get_mut(&(purpose, name.clone())) {
            Some(entry) => {
                entry.requests = entry.requests.saturating_add(1);
                if at > entry.last_seen.as_str() {
                    entry.last_seen = at.to_string();
                }
            }
            None => {
                entries.insert(
                    (purpose, name),
                    Entry {
                        last_seen: at.to_string(),
                        requests: 1,
                        note: None,
                    },
                );
                trim(&mut entries);
            }
        }
        Ok(())
    }

    /// Record a contact the app learned about after the fact (the model
    /// download's completion time, read back from the settings store): the row
    /// gets that time, adds no request to this session's count, and keeps the
    /// newest of the two if it is already there.
    pub fn observe_at(
        &self,
        purpose: Purpose,
        label: &str,
        at: &str,
        note: Option<&str>,
    ) -> Result<(), NetworkLogError> {
        let name = clean_label(label)?;
        let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        match entries.get_mut(&(purpose, name.clone())) {
            Some(entry) => {
                if at > entry.last_seen.as_str() {
                    entry.last_seen = at.to_string();
                }
                if let Some(note) = note {
                    entry.note = Some(note.to_string());
                }
            }
            None => {
                entries.insert(
                    (purpose, name),
                    Entry {
                        last_seen: at.to_string(),
                        requests: 0,
                        note: note.map(str::to_string),
                    },
                );
                trim(&mut entries);
            }
        }
        Ok(())
    }

    /// Add a note to a recorded row (the model id, the account label).
    pub fn annotate(&self, purpose: Purpose, label: &str, note: &str) {
        let Ok(name) = clean_label(label) else {
            return;
        };
        if let Some(entry) = self
            .entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get_mut(&(purpose, name))
        {
            entry.note = Some(note.to_string());
        }
    }

    /// The grouped view: recorded rows (newest first) merged with `declared`
    /// rows that have not been recorded yet.
    pub fn snapshot(&self, declared: &[Declared]) -> NetworkActivity {
        let entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        let mut groups = Vec::new();
        for purpose in Purpose::ALL {
            let mut rows: Vec<Destination> = entries
                .iter()
                .filter(|((entry_purpose, _), _)| *entry_purpose == purpose)
                .map(|((_, name), entry)| Destination {
                    name: name.clone(),
                    purpose,
                    last_seen: Some(entry.last_seen.clone()),
                    requests: entry.requests,
                    note: entry.note.clone(),
                })
                .collect();
            for row in declared.iter().filter(|row| row.purpose == purpose) {
                if !rows.iter().any(|existing| existing.name == row.name) {
                    rows.push(Destination {
                        name: row.name.clone(),
                        purpose,
                        last_seen: row.last_seen.clone(),
                        requests: 0,
                        note: row.note.clone(),
                    });
                }
            }
            // Newest first; configured-but-unreached rows after them, by name.
            rows.sort_by(|left, right| match (&left.last_seen, &right.last_seen) {
                (Some(a), Some(b)) => b.cmp(a).then_with(|| left.name.cmp(&right.name)),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => left.name.cmp(&right.name),
            });
            if !rows.is_empty() {
                groups.push(PurposeGroup {
                    purpose,
                    destinations: rows,
                });
            }
        }
        NetworkActivity {
            groups,
            telemetry: false,
        }
    }

    /// Forget everything (tests, and Reset everything's in-memory half).
    pub fn clear(&self) {
        self.entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
    }
}

/// Drop the oldest rows until the log fits [`MAX_DESTINATIONS`].
fn trim(entries: &mut BTreeMap<(Purpose, String), Entry>) {
    while entries.len() > MAX_DESTINATIONS {
        let oldest = entries
            .iter()
            .min_by(|left, right| left.1.last_seen.cmp(&right.1.last_seen))
            .map(|(key, _)| key.clone());
        let Some(oldest) = oldest else { break };
        entries.remove(&oldest);
    }
}

/// The host of an `https://` URL, lower-cased. A plain `http://` URL is
/// refused: Boba only talks to destinations over TLS (AD-10).
pub fn host_of(url: &str) -> Result<String, NetworkLogError> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err(NetworkLogError::Empty);
    }
    let rest = match trimmed.split_once("://") {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("https") => rest,
        Some((scheme, _)) if scheme.eq_ignore_ascii_case("http") => {
            return Err(NetworkLogError::NotHttps(trimmed.to_string()));
        }
        Some(_) => return Err(NetworkLogError::NotHttps(trimmed.to_string())),
        None => return Err(NetworkLogError::NotHttps(trimmed.to_string())),
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    // Drop userinfo and a port; neither is a destination.
    let host = authority.rsplit('@').next().unwrap_or_default();
    let host = host.split(':').next().unwrap_or_default();
    if host.is_empty() {
        return Err(NetworkLogError::MissingHost(trimmed.to_string()));
    }
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    clean_label(&host)
}

/// A bare host or id: no whitespace, no path, no query, no scheme.
fn clean_label(label: &str) -> Result<String, NetworkLogError> {
    let trimmed = label.trim();
    if trimmed.is_empty() {
        return Err(NetworkLogError::Empty);
    }
    if trimmed.chars().any(char::is_whitespace) || trimmed.contains(['/', '?', '#', '@', '\\']) {
        return Err(NetworkLogError::Impure(trimmed.to_string()));
    }
    Ok(trimmed.to_string())
}

/// The process-wide log every request path writes to.
fn global() -> &'static NetworkLog {
    static LOG: OnceLock<NetworkLog> = OnceLock::new();
    LOG.get_or_init(NetworkLog::new)
}

/// Record one request. Best effort by design: a refused or failed record never
/// fails the request that made it.
pub fn record(purpose: Purpose, url: &str) {
    let _ = global().record(purpose, url);
}

/// Record a non-URL destination (an Agent CLI id, an IMAP host). Best effort.
pub fn record_host(purpose: Purpose, label: &str) {
    let now = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_default();
    let _ = global().record_host(purpose, label, &now);
}

/// Record a contact learned after the fact (S6.9's model download). Best
/// effort, like every record: the view never fails an action.
pub fn observe(purpose: Purpose, label: &str, at: &str, note: Option<&str>) {
    let _ = global().observe_at(purpose, label, at, note);
}

/// Attach a non-secret note to the last recorded row. Best effort.
pub fn annotate(purpose: Purpose, label: &str, note: &str) {
    global().annotate(purpose, label, note);
}

/// The grouped view over the process-wide log.
pub fn snapshot(declared: &[Declared]) -> NetworkActivity {
    global().snapshot(declared)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> String {
        text.to_string()
    }

    #[test]
    fn an_https_url_is_recorded_as_its_host_only() {
        let log = NetworkLog::new();
        log.record(
            Purpose::Sources,
            "https://boards-api.greenhouse.io/v1/boards/acme/jobs?token=a",
        )
        .expect("valid");

        let activity = log.snapshot(&[]);
        let sources = &activity.groups[0];
        assert_eq!(sources.purpose, Purpose::Sources);
        assert_eq!(sources.destinations.len(), 1);
        assert_eq!(sources.destinations[0].name, "boards-api.greenhouse.io");
        assert_eq!(sources.destinations[0].requests, 1);
        // Host only: no path, no query, no token.
        let json = serde_json::to_string(&activity).expect("json");
        assert!(!json.contains("token"));
        assert!(!json.contains("/v1/boards"));
    }

    #[test]
    fn no_query_no_token_and_no_path_ever_lands_in_the_log() {
        let log = NetworkLog::new();
        log.record(
            Purpose::Email,
            "https://graph.microsoft.com/v1.0/me/messages?$filter=from/emailAddress/address eq 'me@example.com'&access_token=SECRET",
        )
        .expect("valid");
        log.record(
            Purpose::Fx,
            "https://api.frankfurter.dev/v1/latest?base=EUR&api_key=SECRET",
        )
        .expect("valid");

        let json = serde_json::to_string(&log.snapshot(&[])).expect("json");
        for secret in [
            "SECRET",
            "access_token",
            "api_key",
            "filter",
            "me@example.com",
        ] {
            assert!(!json.contains(secret), "{secret} must never be recorded");
        }
        assert!(json.contains("graph.microsoft.com"));
    }

    #[test]
    fn plain_http_and_empty_urls_are_refused() {
        let log = NetworkLog::new();
        assert!(
            log.record(Purpose::Sources, "http://remotive.com/api")
                .is_err()
        );
        assert!(log.record(Purpose::Sources, "").is_err());
        assert!(log.record(Purpose::Sources, "not a url").is_err());
        assert!(log.snapshot(&[]).groups.is_empty());
    }

    #[test]
    fn a_repeated_destination_is_one_row_with_a_count() {
        let log = NetworkLog::new();
        log.record(Purpose::Sources, "https://remotive.com/api")
            .expect("valid");
        log.record(Purpose::Sources, "https://www.remotive.com/other")
            .expect("valid");

        let activity = log.snapshot(&[]);
        let destinations = &activity.groups[0].destinations;
        assert_eq!(destinations.len(), 2, "www. is a different host");
        assert_eq!(destinations[0].requests + destinations[1].requests, 2);
    }

    #[test]
    fn destinations_are_grouped_by_purpose_in_nfr_order() {
        let log = NetworkLog::new();
        log.record(Purpose::Fx, "https://api.frankfurter.dev/v1/latest")
            .expect("valid");
        log.record(Purpose::Email, "https://graph.microsoft.com/v1.0/me")
            .expect("valid");
        log.record(Purpose::Sources, "https://remotive.com/api")
            .expect("valid");

        let activity = log.snapshot(&[]);
        let purposes: Vec<_> = activity.groups.iter().map(|group| group.purpose).collect();
        assert_eq!(
            purposes,
            vec![Purpose::Sources, Purpose::Email, Purpose::Fx]
        );
    }

    #[test]
    fn a_declared_destination_appears_without_a_timestamp() {
        let log = NetworkLog::new();
        let declared = [Declared::new(Purpose::Email, "gmail.googleapis.com").expect("valid")];

        let activity = log.snapshot(&declared);
        let email = &activity.groups[0];
        assert_eq!(email.purpose, Purpose::Email);
        assert_eq!(email.destinations[0].name, "gmail.googleapis.com");
        assert_eq!(email.destinations[0].last_seen, None);
        assert_eq!(email.destinations[0].requests, 0);
    }

    #[test]
    fn a_record_lifts_a_declared_row() {
        let log = NetworkLog::new();
        let declared = [Declared::new(Purpose::Email, "graph.microsoft.com").expect("valid")];
        log.record(Purpose::Email, "https://graph.microsoft.com/v1.0/me")
            .expect("valid");

        let activity = log.snapshot(&declared);
        let destination = &activity.groups[0].destinations[0];
        assert_eq!(activity.groups[0].destinations.len(), 1);
        assert!(destination.last_seen.is_some());
        assert_eq!(destination.requests, 1);
    }

    #[test]
    fn a_declared_row_can_carry_the_time_it_was_last_known_contacted() {
        let log = NetworkLog::new();
        let declared = [Declared::new(Purpose::ModelDownload, MODEL_HOST)
            .expect("valid")
            .with_note("multilingual-e5-small")
            .with_last_seen("2026-10-16T08:00:00Z")];

        let activity = log.snapshot(&declared);
        let row = &activity.groups[0].destinations[0];
        assert_eq!(row.last_seen.as_deref(), Some("2026-10-16T08:00:00Z"));
        assert_eq!(row.note.as_deref(), Some("multilingual-e5-small"));
        assert_eq!(row.requests, 0);
    }

    #[test]
    fn snapshot_never_reports_telemetry() {
        let log = NetworkLog::new();
        log.record(Purpose::Sources, "https://remotive.com/api")
            .expect("valid");

        let activity = log.snapshot(&[]);
        assert!(!activity.telemetry);
        // The allowlist is exactly NFR-1's; nothing can name a telemetry host.
        assert_eq!(
            Purpose::ALL,
            [
                Purpose::Sources,
                Purpose::Email,
                Purpose::AgentCli,
                Purpose::Update,
                Purpose::ModelDownload,
                Purpose::Fx,
            ]
        );
    }

    #[test]
    fn the_newest_destinations_come_first() {
        let log = NetworkLog::new();
        log.record_at(
            Purpose::Sources,
            "https://remotive.com/api",
            "2026-10-17T06:00:00Z",
        )
        .expect("valid");
        log.record_at(
            Purpose::Sources,
            "https://remoteok.com/api",
            "2026-10-17T09:00:00Z",
        )
        .expect("valid");

        let activity = log.snapshot(&[]);
        let names: Vec<_> = activity.groups[0]
            .destinations
            .iter()
            .map(|destination| destination.name.as_str())
            .collect();
        assert_eq!(names, vec!["remoteok.com", "remotive.com"]);
    }

    #[test]
    fn the_log_drops_the_oldest_destination_when_it_is_full() {
        let log = NetworkLog::new();
        for index in 0..=MAX_DESTINATIONS {
            let url = format!("https://host{index}.example/jobs");
            let at = format!("2026-10-17T{:02}:00:00Z", index);
            log.record_at(Purpose::Sources, &url, &at).expect("valid");
        }

        let activity = log.snapshot(&[]);
        let destinations = &activity.groups[0].destinations;
        assert_eq!(destinations.len(), MAX_DESTINATIONS);
        assert!(
            destinations
                .iter()
                .all(|destination| destination.name != "host0.example"),
            "the oldest entry is dropped"
        );
    }

    #[test]
    fn a_cli_id_is_recorded_without_a_url() {
        let log = NetworkLog::new();
        log.record_host(Purpose::AgentCli, "claude", "2026-10-17T09:00:00Z")
            .expect("valid");

        let activity = log.snapshot(&[]);
        assert_eq!(activity.groups[0].purpose, Purpose::AgentCli);
        assert_eq!(activity.groups[0].destinations[0].name, "claude");
    }

    #[test]
    fn a_label_with_a_path_is_refused() {
        let log = NetworkLog::new();
        assert!(
            log.record_host(Purpose::AgentCli, "claude --secret", "2026-10-17T09:00:00Z")
                .is_err()
        );
        assert!(
            log.record_host(Purpose::AgentCli, "", "2026-10-17T09:00:00Z")
                .is_err()
        );
        assert!(Declared::new(Purpose::Email, "host/path").is_err());
    }

    #[test]
    fn a_bad_url_without_a_host_is_refused() {
        let log = NetworkLog::new();
        assert!(
            log.record(Purpose::Sources, "https:///only-a-path")
                .is_err()
        );
        assert_eq!(at("2026-10-17T09:00:00Z").len(), 20);
    }
}
