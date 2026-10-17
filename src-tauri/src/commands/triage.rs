//! Inbox triage commands (S6.6, FR-54, FR-66, FR-67): Save, Dismiss (single or
//! bulk, with an optional reason), Undo, the Excluded VETO override and the
//! answer to a Preferences suggestion.
//!
//! The use case is [`TriageService`]; these are thin wrappers that validate the
//! wire shape, call it and map the failure (AD-3). The one thing that happens
//! here and not in the use case is **applying** a suggestion: Preferences are
//! the Settings document (S3.7), which lives above `boba-app`, so the command
//! layer reads it, makes the change the suggestion names, and only then records
//! that the user accepted it.

use std::sync::Arc;

use boba_app::error::{AppError, AppResult, ErrorCode};
use boba_app::ports::repo::apply::ApplyRepo;
use boba_app::ports::repo::ingestion::IngestionRepo;
use boba_app::ports::repo::scoring::ScoringRepo;
use boba_app::ports::repo::system::SystemRepo;
use boba_app::ports::repo::triage::TriageRepo;
use boba_app::system::SystemClock;
use boba_app::triage::{DismissOutcome, OverrideOutcome, SaveOutcome, TriageError, TriageService};
use boba_domain::triage::{DismissReason, Suggestion, SuggestionDecisionStatus, normalize};

use crate::wiring;

use super::preferences::{self, PreferencesError};

/// The use case over the mounted owner ports (ADR-012: ports come only from the
/// helper).
fn service(app: &tauri::AppHandle) -> AppResult<TriageService> {
    Ok(TriageService::new(
        wiring::port::<dyn TriageRepo>(app)?,
        wiring::port::<dyn IngestionRepo>(app)?,
        wiring::port::<dyn ScoringRepo>(app)?,
        wiring::port::<dyn ApplyRepo>(app)?,
        Arc::new(SystemClock),
    ))
}

/// Map a use-case failure onto the typed IPC error (AD-3).
fn to_app(error: TriageError) -> AppError {
    let code = match &error {
        TriageError::UnknownPosting(_) => ErrorCode::NotFound,
        TriageError::NotExcluded(_) | TriageError::UnknownSuggestion(_) => ErrorCode::InvalidInput,
        TriageError::Triage(_)
        | TriageError::Postings(_)
        | TriageError::Scores(_)
        | TriageError::Apply(_) => ErrorCode::Internal,
    };
    AppError::new(code, error.message_key())
}

/// Parse the reason the menu sent. An unknown reason is refused, never guessed:
/// a wrong reason would teach Boba the wrong preference (FR-67).
fn parse_reason(raw: Option<String>) -> AppResult<Option<DismissReason>> {
    match raw {
        None => Ok(None),
        Some(raw) => DismissReason::parse(&raw)
            .map(Some)
            .ok_or_else(|| AppError::invalid_input("errors.inbox_actions.unknown_reason")),
    }
}

fn required(values: &[String]) -> AppResult<()> {
    if values.iter().all(|id| id.trim().is_empty()) || values.is_empty() {
        return Err(AppError::invalid_input(
            "errors.inbox_actions.missing_offer",
        ));
    }
    Ok(())
}

/// Save an Offer: an Application in `Saved` (FR-66).
#[tauri::command]
#[specta::specta]
pub fn inbox_save(app: tauri::AppHandle, posting_id: String) -> AppResult<SaveOutcome> {
    if posting_id.trim().is_empty() {
        return Err(AppError::invalid_input(
            "errors.inbox_actions.missing_offer",
        ));
    }
    service(&app)?.save(&posting_id).map_err(to_app)
}

/// Dismiss one or more Offers, optionally saying why (FR-66/FR-67).
#[tauri::command]
#[specta::specta]
pub fn inbox_dismiss(
    app: tauri::AppHandle,
    posting_ids: Vec<String>,
    reason: Option<String>,
) -> AppResult<DismissOutcome> {
    required(&posting_ids)?;
    service(&app)?
        .dismiss(&posting_ids, parse_reason(reason)?)
        .map_err(to_app)
}

/// Undo a Dismiss (FR-66): the reason and the lifecycle dismissal are cleared.
#[tauri::command]
#[specta::specta]
pub fn inbox_undo_dismiss(app: tauri::AppHandle, posting_id: String) -> AppResult<bool> {
    if posting_id.trim().is_empty() {
        return Err(AppError::invalid_input(
            "errors.inbox_actions.missing_offer",
        ));
    }
    service(&app)?.undo_dismiss(&posting_id).map_err(to_app)
}

/// Override the Gate that vetoed an Offer (FR-54).
#[tauri::command]
#[specta::specta]
pub fn inbox_override_veto(
    app: tauri::AppHandle,
    posting_id: String,
) -> AppResult<OverrideOutcome> {
    if posting_id.trim().is_empty() {
        return Err(AppError::invalid_input(
            "errors.inbox_actions.missing_offer",
        ));
    }
    service(&app)?.override_veto(&posting_id).map_err(to_app)
}

/// Accept or ignore the Preferences change Boba suggested (FR-67 AC3).
///
/// Accepting applies the change **first**: recording an accepted suggestion
/// whose change never landed would stop Boba from ever asking again, leaving
/// the user with the pattern and no fix.
///
/// `company` is the spelling the user saw on the cards. The id carries the
/// normalized name (so two spellings are one suggestion); the preference keeps
/// the readable one, but only when it normalizes to the same company — the
/// screen cannot name one company under another's id.
#[tauri::command]
#[specta::specta]
pub fn inbox_decide_suggestion(
    app: tauri::AppHandle,
    id: String,
    accept: bool,
    company: Option<String>,
) -> AppResult<()> {
    let suggestion = Suggestion::parse(&id)
        .ok_or_else(|| AppError::invalid_input("errors.inbox_actions.unknown_suggestion"))?;
    if accept {
        let repo = wiring::port::<dyn SystemRepo>(&app)?;
        apply_change(repo.as_ref(), &display_suggestion(&suggestion, company)?)?;
    }
    service(&app)?
        .decide_suggestion(
            &id,
            if accept {
                SuggestionDecisionStatus::Accepted
            } else {
                SuggestionDecisionStatus::Ignored
            },
        )
        .map_err(to_app)
}

/// The suggestion with the company spelled as the screen showed it, when that
/// spelling is the same company.
fn display_suggestion(suggestion: &Suggestion, company: Option<String>) -> AppResult<Suggestion> {
    match (suggestion, company) {
        (Suggestion::ExcludeCompany(normalized), Some(display)) => {
            if normalize(&display) != *normalized || display.trim().is_empty() {
                return Err(AppError::invalid_input(
                    "errors.inbox_actions.unknown_suggestion",
                ));
            }
            Ok(Suggestion::ExcludeCompany(display))
        }
        (suggestion, _) => Ok(suggestion.clone()),
    }
}

fn prefs_error(error: PreferencesError) -> AppError {
    let code = match error {
        PreferencesError::Repo(_) => ErrorCode::Internal,
        PreferencesError::Invalid(_) | PreferencesError::Weights(_) => ErrorCode::InvalidInput,
    };
    AppError::new(code, error.message_key())
}

/// Make the Preferences change a suggestion names (FR-67). Idempotent: a
/// company already excluded is not added twice.
fn apply_change(repo: &dyn SystemRepo, suggestion: &Suggestion) -> AppResult<()> {
    let current = preferences::snapshot(repo).map_err(prefs_error)?;
    let mut values = current.values;
    match suggestion {
        Suggestion::ExcludeCompany(company) => {
            let key = normalize(company);
            if !values
                .excluded_companies
                .iter()
                .any(|stored| normalize(stored) == key)
            {
                values.excluded_companies.push(company.clone());
            }
        }
        Suggestion::ExcludeOnsite => {
            // Turning off the last accepted mode would leave no Preferences at
            // all (the editor's own rule); the suggestion says so instead.
            if !values.work_modes.remote && !values.work_modes.hybrid {
                return Err(AppError::invalid_input(
                    "errors.inbox_actions.suggestion_not_applicable",
                ));
            }
            values.work_modes.onsite = false;
        }
    }
    preferences::save(repo, &SystemClock, values).map_err(prefs_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use boba_app::ports::clock::ClockPort;
    use boba_app::system::FixedClock;
    use boba_store::db::{Store, StoreConfig};
    use boba_store::keys::DbKey;
    use boba_store::repos::audit::SqliteSystemRepo;

    use super::*;

    const KEY_HEX: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

    fn tmp_dir(tag: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("boba-triage-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// The real encrypted store over the real `setting` table: no keychain, so
    /// the preference change these tests exercise is the one the app writes.
    fn repo(tag: &str) -> SqliteSystemRepo {
        let dir = tmp_dir(tag);
        let key = DbKey::from_hex(KEY_HEX).expect("test key");
        let config = StoreConfig::new(dir.join("boba.db"), dir.join("snapshots"));
        let store = Arc::new(Store::open_with_key(config, key).expect("store opens"));
        SqliteSystemRepo::new(store, "test")
    }

    fn clock() -> FixedClock {
        FixedClock::new("2026-10-17T09:00:00Z")
    }

    #[test]
    fn an_unknown_reason_is_refused_instead_of_guessed() {
        let error = parse_reason(Some("bored".to_string())).expect_err("unknown");
        assert_eq!(error.code, ErrorCode::InvalidInput);
        assert_eq!(error.message_key, "errors.inbox_actions.unknown_reason");
        assert_eq!(parse_reason(None).expect("no reason"), None);
        assert_eq!(
            parse_reason(Some("company".to_string())).expect("known"),
            Some(DismissReason::Company)
        );
    }

    #[test]
    fn an_empty_batch_is_refused() {
        assert!(required(&[]).is_err());
        assert!(required(&[" ".to_string()]).is_err());
        assert!(required(&["pst_1".to_string()]).is_ok());
    }

    /// FR-67: accepting "exclude this company" writes the company once.
    #[test]
    fn accepting_the_company_suggestion_adds_it_once() {
        let repo = repo("company");
        let suggestion = Suggestion::ExcludeCompany("Boba Labs".to_string());

        apply_change(&repo, &suggestion).expect("apply");
        apply_change(&repo, &suggestion).expect("apply again");

        let saved = preferences::snapshot(&repo).expect("read");
        assert_eq!(
            saved.values.excluded_companies,
            vec!["Boba Labs".to_string()]
        );
        assert_eq!(saved.version, 1);
    }

    /// FR-67: accepting "exclude onsite" turns the mode off.
    #[test]
    fn accepting_the_onsite_suggestion_turns_onsite_off() {
        let repo = repo("onsite");
        apply_change(&repo, &Suggestion::ExcludeOnsite).expect("apply");

        let saved = preferences::snapshot(&repo).expect("read");
        assert!(!saved.values.work_modes.onsite);
        assert!(saved.values.work_modes.remote);
    }

    /// The suggestion is never applied when it would leave no work mode.
    #[test]
    fn the_onsite_suggestion_is_refused_when_it_would_leave_no_mode() {
        let repo = repo("last-mode");
        let mut values = preferences::snapshot(&repo).expect("read").values;
        values.work_modes.remote = false;
        values.work_modes.hybrid = false;
        values.work_modes.onsite = true;
        preferences::save(&repo, &clock(), values).expect("seed");

        let error = apply_change(&repo, &Suggestion::ExcludeOnsite).expect_err("refused");
        assert_eq!(
            error.message_key,
            "errors.inbox_actions.suggestion_not_applicable"
        );
    }

    /// The clock the commands use is the app's own (AD-17).
    #[test]
    fn the_clock_is_the_systems() {
        let now = ClockPort::now_rfc3339(&SystemClock);
        assert!(now.contains('T'));
    }

    /// The preference keeps the spelling the cards showed, not the id's
    /// normalized form — but only for the company the id names.
    #[test]
    fn the_company_keeps_the_spelling_the_screen_showed() {
        let parsed = Suggestion::parse("exclude_company:boba labs").expect("id");
        let display = display_suggestion(&parsed, Some("Boba  Labs".to_string())).expect("display");
        assert_eq!(
            display,
            Suggestion::ExcludeCompany("Boba  Labs".to_string())
        );

        let wrong = display_suggestion(&parsed, Some("Other Co".to_string()));
        assert!(wrong.is_err());
        // No spelling sent: the normalized one is still the right company.
        let fallback = display_suggestion(&parsed, None).expect("fallback");
        assert_eq!(
            fallback,
            Suggestion::ExcludeCompany("boba labs".to_string())
        );
    }
}
