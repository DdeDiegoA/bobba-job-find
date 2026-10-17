//! Profile gap interview and bilingual drafting commands (S3.6, FR-6, FR-12):
//! thin wrappers over [`InterviewService`] (AD-3).
//!
//! The store, the agent runtime and the setting row arrive through
//! [`wiring::port`] (ADR-012): a locked or unmounted port is the typed error,
//! never a panic. The chosen CLI is the same `setting` row the engine step
//! wrote (S3.2), exactly as the CV import reads it.
//!
//! Nothing here saves an agent's word: the interview's questions are computed
//! by the core, the user's answers are their own facts, and a drafted
//! translation waits as a proposal until [`profile_gaps_confirm`].

use std::sync::Arc;

use boba_app::error::{AppError, AppResult, ErrorCode};
use boba_app::ports::agent::AgentPort;
use boba_app::ports::repo::profile::ProfileRepo;
use boba_app::ports::repo::system::SystemRepo;
use boba_app::profile::ProfileSnapshot;
use boba_app::profile_gaps::gaps::{Gap, GapAnswer};
use boba_app::profile_gaps::{
    InterviewOutcome, InterviewRequest, InterviewService, ProfileGapsError, TranslateOutcome,
    TranslateRequest,
};
use boba_app::profile_import::FactEdit;
use boba_app::system::SystemClock;

use crate::commands::agent::{ENGINE_KEY, EngineChoice};
use crate::wiring;

/// The failures only a command can hit, each with its own key (CH7:
/// `errors.profile_gaps.*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandError {
    Store,
    NoEngine,
    Run,
}

impl CommandError {
    /// The key the UI translates (the gate scans `message_key` fns, AC4).
    const fn message_key(self) -> &'static str {
        match self {
            CommandError::Store => "errors.profile_gaps.store",
            CommandError::NoEngine => "errors.profile_gaps.no_engine",
            CommandError::Run => "errors.profile_gaps.run",
        }
    }

    fn app_error(self) -> AppError {
        let code = match self {
            CommandError::NoEngine => ErrorCode::InvalidInput,
            CommandError::Store | CommandError::Run => ErrorCode::Internal,
        };
        AppError::new(code, self.message_key())
    }
}

/// The service over the mounted ports (ADR-012: ports come only from the
/// helper).
fn service(app: &tauri::AppHandle) -> AppResult<InterviewService> {
    Ok(InterviewService::new(
        wiring::port::<dyn ProfileRepo>(app)?,
        wiring::port::<dyn SystemRepo>(app)?,
        wiring::port::<dyn AgentPort>(app)?,
        Arc::new(SystemClock),
    ))
}

/// Map a use-case failure onto the typed IPC error (AD-3).
fn to_app(error: ProfileGapsError) -> AppError {
    use boba_app::profile::ProfileError;
    let code = match &error {
        ProfileGapsError::BadAnswer => ErrorCode::InvalidInput,
        ProfileGapsError::Profile(ProfileError::Rule(_) | ProfileError::UnknownKind(_)) => {
            ErrorCode::InvalidInput
        }
        ProfileGapsError::UnknownGap(_)
        | ProfileGapsError::UnknownItem(_)
        | ProfileGapsError::UnknownProposal(_) => ErrorCode::NotFound,
        ProfileGapsError::Profile(
            ProfileError::UnknownItem(_) | ProfileError::UnknownRevision(_),
        ) => ErrorCode::NotFound,
        ProfileGapsError::Store
        | ProfileGapsError::Contract
        | ProfileGapsError::Run(_)
        | ProfileGapsError::Profile(_) => ErrorCode::Internal,
    };
    AppError::new(code, error.message_key())
}

/// The engine the user picked in the wizard's engine step (S3.2). Without one,
/// the interview cannot ask the agent; the step says so and offers the manual
/// questions instead of guessing a CLI.
fn chosen_engine(app: &tauri::AppHandle) -> AppResult<EngineChoice> {
    let repo = wiring::port::<dyn SystemRepo>(app)?;
    let raw = repo
        .setting_get(ENGINE_KEY)
        .map_err(|_| CommandError::Store.app_error())?;
    let choice: Option<EngineChoice> = raw
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .filter(|choice: &EngineChoice| !choice.cli.trim().is_empty());
    choice.ok_or_else(|| CommandError::NoEngine.app_error())
}

/// The questions the Profile's facts leave open, without any run. This is what
/// the step shows before the user asks for help, and the whole step in limited
/// mode (S3.3: no working CLI).
#[tauri::command]
#[specta::specta]
pub fn profile_gaps_questions(app: tauri::AppHandle) -> AppResult<Vec<Gap>> {
    service(&app)?.questions().map_err(to_app)
}

/// Run one `profile_gaps` pass: the agent writes the questions, the core keeps
/// the ones that are really missing (AC1). Runs off the UI thread: the run
/// waits on an agent CLI and can take minutes (S3.5 review F1).
#[tauri::command]
#[specta::specta]
pub async fn profile_gaps_start(
    app: tauri::AppHandle,
    extra_context: Option<String>,
    language: String,
) -> AppResult<InterviewOutcome> {
    tauri::async_runtime::spawn_blocking(move || start(&app, extra_context, language))
        .await
        .map_err(|_| CommandError::Run.app_error())?
}

fn start(
    app: &tauri::AppHandle,
    extra_context: Option<String>,
    language: String,
) -> AppResult<InterviewOutcome> {
    let engine = chosen_engine(app)?;
    let extra_context = extra_context.filter(|text| !text.trim().is_empty());
    service(app)?
        .start(InterviewRequest {
            cli: engine.cli,
            model: engine.model,
            target_language: language,
            extra_context,
        })
        .map_err(to_app)
}

/// Answer one question: the fact it states is saved as the user's own (AC1).
#[tauri::command]
#[specta::specta]
pub fn profile_gaps_answer(
    app: tauri::AppHandle,
    key: String,
    answer: GapAnswer,
    language: String,
) -> AppResult<ProfileSnapshot> {
    service(&app)?
        .answer(&key, answer, &language)
        .map_err(to_app)
}

/// Skip one question: the Gates that needed it lose confidence (AC1).
#[tauri::command]
#[specta::specta]
pub fn profile_gaps_skip(app: tauri::AppHandle, key: String) -> AppResult<()> {
    service(&app)?.skip(&key).map_err(to_app)
}

/// Run one `translate_profile` pass for a fact with one variant (AC2). The
/// draft waits as a proposal; nothing is saved before the user confirms.
#[tauri::command]
#[specta::specta]
pub async fn profile_gaps_translate(
    app: tauri::AppHandle,
    item_id: String,
    lang: String,
    language: String,
) -> AppResult<TranslateOutcome> {
    tauri::async_runtime::spawn_blocking(move || translate(&app, item_id, lang, language))
        .await
        .map_err(|_| CommandError::Run.app_error())?
}

fn translate(
    app: &tauri::AppHandle,
    item_id: String,
    lang: String,
    language: String,
) -> AppResult<TranslateOutcome> {
    let engine = chosen_engine(app)?;
    service(app)?
        .translate(TranslateRequest {
            item_id,
            lang,
            cli: engine.cli,
            model: engine.model,
            target_language: language,
        })
        .map_err(to_app)
}

/// Confirm a drafted variant — as proposed, or with the user's edit (AC3).
#[tauri::command]
#[specta::specta]
pub fn profile_gaps_confirm(
    app: tauri::AppHandle,
    id: String,
    edit: Option<FactEdit>,
) -> AppResult<ProfileSnapshot> {
    service(&app)?.confirm(&id, edit).map_err(to_app)
}

/// Delete a drafted variant: it never becomes a fact (AC3).
#[tauri::command]
#[specta::specta]
pub fn profile_gaps_discard(app: tauri::AppHandle, id: String) -> AppResult<()> {
    service(&app)?.discard(&id).map_err(to_app)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_map_to_codes_and_i18n_keys() {
        let mapped = to_app(ProfileGapsError::BadAnswer);
        assert_eq!(mapped.code, ErrorCode::InvalidInput);
        assert_eq!(mapped.message_key, "errors.profile_gaps.bad_answer");

        let mapped = to_app(ProfileGapsError::UnknownGap("cefr:en".to_string()));
        assert_eq!(mapped.code, ErrorCode::NotFound);
        assert_eq!(mapped.message_key, "errors.profile_gaps.unknown_gap");

        let mapped = to_app(ProfileGapsError::Store);
        assert_eq!(mapped.code, ErrorCode::Internal);
        assert_eq!(mapped.message_key, "errors.profile_gaps.store");

        let mapped = to_app(ProfileGapsError::Contract);
        assert_eq!(mapped.message_key, "errors.profile_gaps.contract");
    }

    #[test]
    fn command_errors_carry_their_own_keys() {
        assert_eq!(
            CommandError::NoEngine.app_error().message_key,
            "errors.profile_gaps.no_engine"
        );
        assert_eq!(
            CommandError::Store.app_error().message_key,
            "errors.profile_gaps.store"
        );
    }
}
