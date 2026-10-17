//! The on-device semantic match (F3): its status and the opt-in (S6.9, FR-55,
//! AD-8).
//!
//! The decision is [`boba_app::embeddings::EmbeddingsService`], which the
//! composition root mounts; these commands only translate. The download is
//! the user's choice (AC1) — nothing here starts it without `embeddings_opt_in`
//! — and without the opt-in the scoring run asks for no similarity at all
//! (AC2).

use std::sync::Arc;

use boba_app::embeddings::{EmbeddingStatus, EmbeddingsError, EmbeddingsService};
use boba_app::error::{AppError, AppResult};
use tauri::{AppHandle, Manager, Runtime};

/// What F3 can do right now: whether this build carries the encoder, whether
/// the user opted in, where the model lives and what it costs (AC1, AC4).
#[tauri::command]
#[specta::specta]
pub fn embeddings_status(app: AppHandle) -> AppResult<EmbeddingStatus> {
    service(&app)?.status().map_err(to_app)
}

/// The user's consent: download the model (about 487 MB, multilingual-e5-small)
/// if it is not on disk, record the destination, and turn F3 on (AC1).
#[tauri::command]
#[specta::specta]
pub async fn embeddings_opt_in(app: AppHandle) -> AppResult<EmbeddingStatus> {
    // The download blocks for minutes on a slow line; it must not hold the
    // webview's thread (S9.4 review F2's rule).
    tauri::async_runtime::spawn_blocking(move || service(&app)?.opt_in().map_err(to_app))
        .await
        .map_err(|_| AppError::internal("errors.embeddings.download"))?
}

/// Turn F3 off (AC2): the next run scores without it and renormalises its
/// weight. The model files stay on disk.
#[tauri::command]
#[specta::specta]
pub fn embeddings_opt_out(app: AppHandle) -> AppResult<()> {
    service(&app)?.opt_out().map_err(to_app)
}

/// The mounted use case; a locked store answers the typed Locked error.
fn service<R: Runtime>(app: &AppHandle<R>) -> AppResult<Arc<EmbeddingsService>> {
    app.try_state::<Arc<EmbeddingsService>>()
        .map(|state| Arc::clone(state.inner()))
        .ok_or_else(|| AppError::locked("errors.store.locked"))
}

/// Map the use case's failure onto the typed IPC error (AD-3: keys cross IPC).
fn to_app(error: EmbeddingsError) -> AppError {
    match error {
        // Nothing to turn on in this build: the UI disables the control.
        EmbeddingsError::Unavailable => AppError::invalid_input(error.message_key()),
        EmbeddingsError::Port(_) | EmbeddingsError::System(_) => {
            AppError::internal(error.message_key())
        }
    }
}
