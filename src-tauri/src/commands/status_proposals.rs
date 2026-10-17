//! Reply classification and Status Proposal commands (S8.4, FR-100..FR-103).
//!
//! Thin wrappers over [`StatusProposalsService`]: validate, call the use case,
//! map the failure onto the typed IPC error (AD-3). The ports arrive through
//! [`wiring::port`] (ADR-012), so a locked store is `errors.store.locked` and
//! never a panic.
//!
//! The deterministic rules are the `boba-mail` adapter, handed in here because
//! the use case crate cannot depend on it (AD-1): the composition root is the
//! only place that knows the two.

use std::sync::Arc;

use boba_app::error::{AppError, AppResult, ErrorCode};
use boba_app::ports::agent::AgentPort;
use boba_app::ports::repo::apply::ApplyRepo;
use boba_app::ports::repo::ingestion::IngestionRepo;
use boba_app::ports::repo::system::SystemRepo;
use boba_app::status_proposals::repo::StatusProposalRepo;
use boba_app::status_proposals::{
    ApproveOutcome, BatchOutcome, ClassificationMode, ClassifyOutcome, DisclosureView,
    ReplyClassifier, StatusProposalView, StatusProposalsError, StatusProposalsService,
};
use boba_app::system::SystemClock;

use crate::commands::agent::ENGINE_KEY;
use crate::wiring;

/// The service over the mounted ports (ADR-012: ports come only from the
/// helper).
fn service(app: &tauri::AppHandle) -> AppResult<StatusProposalsService> {
    let rules: Arc<dyn ReplyClassifier> = Arc::new(boba_mail::rules::RulesClassifier);
    Ok(StatusProposalsService::new(
        wiring::port::<dyn StatusProposalRepo>(app)?,
        wiring::port::<dyn ApplyRepo>(app)?,
        wiring::port::<dyn IngestionRepo>(app)?,
        wiring::port::<dyn SystemRepo>(app)?,
        wiring::port::<dyn AgentPort>(app)?,
        rules,
        Arc::new(SystemClock),
    ))
}

/// Map a use-case failure onto the typed IPC error (AD-3).
fn to_app(error: StatusProposalsError) -> AppError {
    let code = match &error {
        StatusProposalsError::DisclosureRequired
        | StatusProposalsError::NeedsApplication(_)
        | StatusProposalsError::AlreadyThere
        | StatusProposalsError::NotUndoable(_) => ErrorCode::InvalidInput,
        StatusProposalsError::UnknownProposal(_) => ErrorCode::NotFound,
        StatusProposalsError::NotPending(_) | StatusProposalsError::StaleCard => {
            ErrorCode::Conflict
        }
        StatusProposalsError::Repo(_)
        | StatusProposalsError::Apply(_)
        | StatusProposalsError::Postings(_)
        | StatusProposalsError::Settings(_)
        | StatusProposalsError::Tracker(_)
        | StatusProposalsError::Run(_) => ErrorCode::Internal,
    };
    AppError::new(code, error.message_key())
}

/// The user's engine, when one was chosen (S2.1's stored choice). A row this
/// build cannot read is no choice: the uncertain cases wait for triage.
fn stored_engine(app: &tauri::AppHandle) -> Option<(String, Option<String>)> {
    let repo = wiring::port::<dyn SystemRepo>(app).ok()?;
    let raw = repo.setting_get(ENGINE_KEY).ok()??;
    let choice: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let cli = choice.get("cli")?.as_str()?.trim().to_string();
    if cli.is_empty() {
        return None;
    }
    let model = choice
        .get("model")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_string);
    Some((cli, model))
}

/// What the disclosure screen shows (AC3, AC4).
#[tauri::command]
#[specta::specta]
pub fn status_proposals_disclosure(app: tauri::AppHandle) -> AppResult<DisclosureView> {
    service(&app)?.disclosure().map_err(to_app)
}

/// Save the reply-reading choice (AC3). `acknowledge` records that the user saw
/// the plain-language disclosure.
#[tauri::command]
#[specta::specta]
pub fn status_proposals_save_settings(
    app: tauri::AppHandle,
    mode: String,
    auto_confirm: bool,
    acknowledge: bool,
) -> AppResult<DisclosureView> {
    let mode = ClassificationMode::parse(mode.trim())
        .ok_or_else(|| AppError::invalid_input("errors.status_proposals.unknown_mode"))?;
    service(&app)?
        .save_settings(mode, auto_confirm, acknowledge)
        .map_err(to_app)
}

/// Classify every message no classifier has read yet (AC1, AC4).
///
/// The engine is the stored choice; when none is stored the uncertain cases
/// wait for manual triage rather than failing the pass.
#[tauri::command]
#[specta::specta]
pub fn status_proposals_classify(app: tauri::AppHandle) -> AppResult<ClassifyOutcome> {
    let (cli, model) = match stored_engine(&app) {
        Some((cli, model)) => (Some(cli), model),
        None => (None, None),
    };
    service(&app)?
        .classify(cli.as_deref(), model)
        .map_err(to_app)
}

/// Today's needs-you queue (AC2).
#[tauri::command]
#[specta::specta]
pub fn status_proposals_pending(app: tauri::AppHandle) -> AppResult<Vec<StatusProposalView>> {
    service(&app)?.proposals().map_err(to_app)
}

/// Approve one proposal, moving its card (AC2). `application_id` is the user's
/// pick for an ambiguous match.
#[tauri::command]
#[specta::specta]
pub fn status_proposals_approve(
    app: tauri::AppHandle,
    proposal_id: String,
    application_id: Option<String>,
) -> AppResult<ApproveOutcome> {
    service(&app)?
        .approve(&proposal_id, application_id.as_deref())
        .map_err(to_app)
}

/// Approve a batch; the ambiguous ones are reported, not applied (AC2).
#[tauri::command]
#[specta::specta]
pub fn status_proposals_approve_many(
    app: tauri::AppHandle,
    proposal_ids: Vec<String>,
) -> AppResult<BatchOutcome> {
    service(&app)?.approve_many(&proposal_ids).map_err(to_app)
}

/// Dismiss a proposal: nothing moves (AC2).
#[tauri::command]
#[specta::specta]
pub fn status_proposals_dismiss(app: tauri::AppHandle, proposal_id: String) -> AppResult<()> {
    service(&app)?.dismiss(&proposal_id).map_err(to_app)
}

/// Undo an automatically applied proposal (AC5).
#[tauri::command]
#[specta::specta]
pub fn status_proposals_undo(app: tauri::AppHandle, proposal_id: String) -> AppResult<()> {
    service(&app)?.undo(&proposal_id).map_err(to_app)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_map_to_codes_and_i18n_keys() {
        let mapped = to_app(StatusProposalsError::DisclosureRequired);
        assert_eq!(mapped.code, ErrorCode::InvalidInput);
        assert_eq!(
            mapped.message_key,
            "errors.status_proposals.disclosure_required"
        );

        let mapped = to_app(StatusProposalsError::NotPending("sp_1".to_string()));
        assert_eq!(mapped.code, ErrorCode::Conflict);
        assert_eq!(mapped.message_key, "errors.status_proposals.not_pending");

        let mapped = to_app(StatusProposalsError::UnknownProposal("sp_1".to_string()));
        assert_eq!(mapped.code, ErrorCode::NotFound);

        let mapped = to_app(StatusProposalsError::Repo(
            boba_app::status_proposals::repo::StatusProposalRepoError("disk".to_string()),
        ));
        assert_eq!(mapped.code, ErrorCode::Internal);
        assert_eq!(mapped.message_key, "errors.status_proposals.store");
    }
}
