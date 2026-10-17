//! Preferences commands (S3.7, FR-7/FR-14): what the user is looking for, and
//! its version.
//!
//! Stored as one JSON row in the encrypted `setting` table (S1.7, AD-16) read
//! through the mounted [`SystemRepo`] port (ADR-012): preferences never leave
//! the DB and never touch a loose file. A change creates a new prefs version and
//! sets `scores_stale`, so the scoring engine (S6.3) knows to re-score the open
//! Offers and is the only writer that clears the flag back to fresh (FR-14).
//!
//! Tolerance is deliberate on read: unknown fields from a newer build are
//! ignored, and a row this build cannot read falls back to the defaults, so a
//! storage change can never leave the step without values.

use std::sync::Arc;

use boba_app::error::{AppError, AppResult, ErrorCode};
use boba_app::ports::clock::ClockPort;
use boba_app::ports::repo::system::{SystemRepo, SystemRepoError};
use boba_app::system::SystemClock;
use serde::{Deserialize, Serialize};

use crate::wiring;

/// The one `setting` row that holds the preferences document.
pub const KEY: &str = "preferences";

/// Target seniority (FR-7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum Seniority {
    Intern,
    Junior,
    Mid,
    Senior,
    Lead,
    Principal,
}

/// Salary period (AD-17).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum MoneyPeriod {
    Year,
    Month,
    Hour,
}

/// Whether the salary floor excludes Offers or only lowers their score (FR-7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum SalaryFirmness {
    Hard,
    Soft,
}

/// The salary floor, in the AD-17 money shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct SalaryFloor {
    /// Integer minor units (AD-17); still an i64 on the wire and in the DB.
    /// Exported as a TypeScript `number` because specta refuses BigInt-style
    /// types over IPC: minor amounts stay far below 2^53, so no precision is
    /// lost — a float here is impossible either way.
    #[specta(type = specta_typescript::Number)]
    pub amount_minor: i64,
    /// ISO-4217, upper case.
    pub currency: String,
    pub period: MoneyPeriod,
    pub firmness: SalaryFirmness,
}

/// Hours the user is reachable, local time (FR-7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct TimezoneWindow {
    pub from: u8,
    pub to: u8,
}

/// Work modes the user accepts at all (FR-7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct WorkModes {
    pub remote: bool,
    pub hybrid: bool,
    pub onsite: bool,
}

/// Everything the user can edit (FR-7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct Preferences {
    pub roles: Vec<String>,
    pub seniority_min: Option<Seniority>,
    pub seniority_max: Option<Seniority>,
    pub salary_floor: Option<SalaryFloor>,
    pub locations: Vec<String>,
    pub timezone_window: TimezoneWindow,
    pub work_modes: WorkModes,
    /// BCP-47 languages I work in (AD-17).
    pub working_languages: Vec<String>,
    pub deal_breaker_keywords: Vec<String>,
    pub deal_breaker_notes: String,
    pub excluded_companies: Vec<String>,
}

impl Default for Preferences {
    /// Sensible defaults (FR-7): remote allowed, a wide day window, English.
    fn default() -> Self {
        Self {
            roles: Vec::new(),
            seniority_min: None,
            seniority_max: None,
            salary_floor: None,
            locations: Vec::new(),
            timezone_window: TimezoneWindow { from: 8, to: 18 },
            work_modes: WorkModes {
                remote: true,
                hybrid: false,
                onsite: false,
            },
            working_languages: vec!["en".to_string()],
            deal_breaker_keywords: Vec::new(),
            deal_breaker_notes: String::new(),
            excluded_companies: Vec::new(),
        }
    }
}

/// The preferences document plus its version and staleness (FR-14). The same
/// shape is stored as JSON and sent over IPC, so the two can never drift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct PreferencesSnapshot {
    /// Bumped by every change.
    pub version: u32,
    pub values: Preferences,
    /// True until a re-score consumed this version (FR-14).
    pub scores_stale: bool,
}

impl Default for PreferencesSnapshot {
    /// An untouched store: no version, defaults, nothing stale.
    fn default() -> Self {
        Self {
            version: 0,
            values: Preferences::default(),
            scores_stale: false,
        }
    }
}

/// A preferences use-case failure.
#[derive(Debug, Clone, thiserror::Error)]
pub enum PreferencesError {
    #[error(transparent)]
    Repo(#[from] SystemRepoError),
    /// The document the caller sent breaks a preference rule.
    #[error("invalid preferences: {0}")]
    Invalid(String),
}

impl PreferencesError {
    /// The i18n key the UI shows (AD-3: keys cross IPC, never raw text).
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::Repo(_) => "errors.preferences.store",
            Self::Invalid(_) => "errors.preferences.invalid",
        }
    }
}

/// Map a use-case failure onto the typed IPC error (AD-3).
fn to_app(error: PreferencesError) -> AppError {
    let code = match error {
        PreferencesError::Repo(_) => ErrorCode::Internal,
        PreferencesError::Invalid(_) => ErrorCode::InvalidInput,
    };
    AppError::new(code, error.message_key())
}

/// The store, through the composition root (ADR-012): locked is a typed error.
fn repo(app: &tauri::AppHandle) -> AppResult<Arc<dyn SystemRepo>> {
    wiring::port::<dyn SystemRepo>(app)
}

/// The current preferences, or the defaults when nothing is stored yet (FR-7).
pub(crate) fn snapshot(repo: &dyn SystemRepo) -> Result<PreferencesSnapshot, PreferencesError> {
    let Some(raw) = repo.setting_get(KEY)? else {
        return Ok(PreferencesSnapshot::default());
    };
    Ok(parse(&raw))
}

/// Save an edit: a real change appends a new version and marks scores stale
/// (AC2, FR-14); an unchanged save returns the stored document untouched.
pub(crate) fn save(
    repo: &dyn SystemRepo,
    clock: &dyn ClockPort,
    values: Preferences,
) -> Result<PreferencesSnapshot, PreferencesError> {
    validate(&values)?;
    let current = snapshot(repo)?;
    if current.values == values {
        return Ok(current);
    }
    let next = PreferencesSnapshot {
        version: current.version + 1,
        values,
        scores_stale: true,
    };
    let json =
        serde_json::to_string(&next).map_err(|e| PreferencesError::Invalid(e.to_string()))?;
    repo.setting_put(KEY, &json, &clock.now_rfc3339())?;
    Ok(next)
}

/// Reject values the engine could not score with (AD-17 money, hour ranges).
fn validate(values: &Preferences) -> Result<(), PreferencesError> {
    if let Some(floor) = &values.salary_floor {
        if floor.amount_minor < 0 {
            return Err(PreferencesError::Invalid("negative_amount".to_string()));
        }
        let currency = &floor.currency;
        if currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_uppercase()) {
            return Err(PreferencesError::Invalid("currency".to_string()));
        }
    }
    let window = values.timezone_window;
    if window.from > 23 || window.to > 23 {
        return Err(PreferencesError::Invalid("timezone_window".to_string()));
    }
    let modes = values.work_modes;
    if !modes.remote && !modes.hybrid && !modes.onsite {
        return Err(PreferencesError::Invalid("work_modes".to_string()));
    }
    Ok(())
}

/// A stored row: unknown fields are ignored, and a row this build cannot read
/// (corrupt, or written by a shape that is gone) reads as version 0 with the
/// defaults, so the step never greets the user with an error screen.
fn parse(raw: &str) -> PreferencesSnapshot {
    serde_json::from_str::<PreferencesSnapshot>(raw).unwrap_or_default()
}

/// The current preferences, version and staleness (AC1/AC2).
#[tauri::command]
#[specta::specta]
pub fn preferences_snapshot(app: tauri::AppHandle) -> AppResult<PreferencesSnapshot> {
    snapshot(repo(&app)?.as_ref()).map_err(to_app)
}

/// Save the preferences step or the Settings page (AC1/AC2, FR-14).
#[tauri::command]
#[specta::specta]
pub fn preferences_save(
    app: tauri::AppHandle,
    values: Preferences,
) -> AppResult<PreferencesSnapshot> {
    save(repo(&app)?.as_ref(), &SystemClock, values).map_err(to_app)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use boba_app::system::FixedClock;
    use boba_store::db::{Store, StoreConfig};
    use boba_store::keys::DbKey;
    use boba_store::repos::audit::SqliteSystemRepo;

    use super::*;

    const KEY_HEX: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

    fn tmp_dir(tag: &str) -> PathBuf {
        // One store file per call: parallel tests must not share it.
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("boba-prefs-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// The real encrypted store over the real `setting` table: no keychain, so
    /// the round trip these tests exercise is the one the app uses (S1.2a path).
    fn repo(tag: &str) -> SqliteSystemRepo {
        let dir = tmp_dir(tag);
        let key = DbKey::from_hex(KEY_HEX).expect("test key");
        let config = StoreConfig::new(dir.join("boba.db"), dir.join("snapshots"));
        let store = Arc::new(Store::open_with_key(config, key).expect("store opens"));
        SqliteSystemRepo::new(store, "test")
    }

    fn clock() -> FixedClock {
        FixedClock::new("2026-10-16T01:00:00.000Z")
    }

    fn repo_with(tag: &str, values: Preferences) -> (SqliteSystemRepo, PreferencesSnapshot) {
        let repo = repo(tag);
        let saved = save(&repo, &clock(), values).expect("save");
        (repo, saved)
    }

    #[test]
    fn an_absent_row_reads_as_defaults_at_version_zero() {
        let stored = snapshot(&repo("absent")).expect("snapshot");
        assert_eq!(stored.version, 0);
        assert!(!stored.scores_stale);
        assert_eq!(stored.values, Preferences::default());
        assert!(stored.values.work_modes.remote);
        assert!(!stored.values.work_modes.onsite);
        assert_eq!(stored.values.timezone_window.from, 8);
        assert_eq!(stored.values.timezone_window.to, 18);
        assert_eq!(stored.values.working_languages, vec!["en".to_string()]);
    }

    #[test]
    fn a_corrupt_row_reads_as_defaults_instead_of_failing() {
        let corrupt = repo("corrupt");
        corrupt.setting_put(KEY, "{not json", "seed").expect("seed");
        assert_eq!(
            snapshot(&corrupt).expect("snapshot"),
            snapshot(&repo("clean")).expect("snapshot")
        );
    }

    #[test]
    fn fields_from_a_newer_build_are_ignored() {
        let (repo, stored) = repo_with(
            "forward",
            Preferences {
                roles: vec!["SRE".to_string()],
                ..Preferences::default()
            },
        );
        // A row a newer build wrote: an extra field in the document and in the
        // values, both of which this build must ignore rather than choke on.
        let mut doc: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&stored).expect("json")).expect("value");
        doc["schema"] = serde_json::json!(99);
        doc["values"]["theme"] = serde_json::json!("dark");
        repo.setting_put(KEY, &doc.to_string(), "seed")
            .expect("seed");

        assert_eq!(snapshot(&repo).expect("snapshot"), stored);
    }

    #[test]
    fn a_change_bumps_the_version_and_marks_scores_stale() {
        let values = Preferences {
            roles: vec!["Backend Engineer".to_string()],
            ..Preferences::default()
        };
        let (repo, saved) = repo_with("change", values.clone());

        assert_eq!(saved.version, 1);
        assert!(saved.scores_stale);
        assert_eq!(snapshot(&repo).expect("snapshot"), saved);

        let again = save(
            &repo,
            &clock(),
            Preferences {
                roles: vec![
                    "Backend Engineer".to_string(),
                    "Platform Engineer".to_string(),
                ],
                ..values
            },
        )
        .expect("save");
        assert_eq!(again.version, 2);
    }

    #[test]
    fn an_unchanged_save_creates_no_new_version() {
        let values = Preferences {
            locations: vec!["Berlin".to_string()],
            ..Preferences::default()
        };
        let (repo, saved) = repo_with("unchanged", values.clone());
        let again = save(&repo, &clock(), values).expect("save");

        assert_eq!(saved.version, 1);
        assert_eq!(again, saved);
    }

    #[test]
    fn a_re_score_consumes_the_version_without_losing_it() {
        let (repo, stored) = repo_with(
            "rescored",
            Preferences {
                roles: vec!["SRE".to_string()],
                ..Preferences::default()
            },
        );
        let fresh = PreferencesSnapshot {
            scores_stale: false,
            ..stored
        };
        let json = serde_json::to_string(&fresh).expect("json");
        repo.setting_put(KEY, &json, "2026-10-16T02:00:00.000Z")
            .expect("put");

        let stored = snapshot(&repo).expect("snapshot");
        assert!(!stored.scores_stale);
        assert_eq!(stored.version, 1);
    }

    #[test]
    fn invalid_values_are_rejected_before_the_store_is_touched() {
        let repo = repo("invalid");

        let negative = Preferences {
            salary_floor: Some(SalaryFloor {
                amount_minor: -1,
                currency: "USD".to_string(),
                period: MoneyPeriod::Year,
                firmness: SalaryFirmness::Hard,
            }),
            ..Preferences::default()
        };
        assert!(matches!(
            save(&repo, &clock(), negative),
            Err(PreferencesError::Invalid(_))
        ));

        let currency = Preferences {
            salary_floor: Some(SalaryFloor {
                amount_minor: 1,
                currency: "usd".to_string(),
                period: MoneyPeriod::Year,
                firmness: SalaryFirmness::Soft,
            }),
            ..Preferences::default()
        };
        assert!(save(&repo, &clock(), currency).is_err());

        let hours = Preferences {
            timezone_window: TimezoneWindow { from: 8, to: 24 },
            ..Preferences::default()
        };
        assert!(save(&repo, &clock(), hours).is_err());

        let no_mode = Preferences {
            work_modes: WorkModes {
                remote: false,
                hybrid: false,
                onsite: false,
            },
            ..Preferences::default()
        };
        assert!(save(&repo, &clock(), no_mode).is_err());

        // Nothing was written for a rejected edit: still the untouched defaults.
        assert_eq!(snapshot(&repo).expect("snapshot").version, 0);
    }

    #[test]
    fn the_salary_floor_round_trips_in_minor_units() {
        let floor = SalaryFloor {
            amount_minor: 6_000_000,
            currency: "USD".to_string(),
            period: MoneyPeriod::Month,
            firmness: SalaryFirmness::Soft,
        };
        let (repo, _) = repo_with(
            "money",
            Preferences {
                salary_floor: Some(floor.clone()),
                ..Preferences::default()
            },
        );

        assert_eq!(
            snapshot(&repo).expect("snapshot").values.salary_floor,
            Some(floor)
        );
    }

    #[test]
    fn errors_map_to_codes_and_i18n_keys() {
        let mapped = to_app(PreferencesError::Invalid("currency".to_string()));
        assert_eq!(mapped.code, ErrorCode::InvalidInput);
        assert_eq!(mapped.message_key, "errors.preferences.invalid");

        let mapped = to_app(PreferencesError::Repo(SystemRepoError::new("disk")));
        assert_eq!(mapped.code, ErrorCode::Internal);
        assert_eq!(mapped.message_key, "errors.preferences.store");
    }
}
