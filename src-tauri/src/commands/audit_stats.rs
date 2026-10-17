//! Audit Log and funnel stats commands (S9.9, FR-89/FR-97, SM-4/5/7).
//!
//! Thin wrappers: resolve the ports through [`wiring::port`] (ADR-012), call the
//! read-only use case, map the failure (AD-3). Nothing here writes.

use boba_app::error::{AppError, AppResult, ErrorCode};
use boba_app::ports::repo::apply::ApplyRepo;
use boba_app::ports::repo::auto_rules::AutoRulesRepo;
use boba_app::ports::repo::ingestion::IngestionRepo;
use boba_app::ports::repo::outbox_hold::OutboxHoldRepo;
use boba_app::ports::repo::scoring::ScoringRepo;
use boba_app::ports::repo::sources::SourcesRepo;
use boba_app::ports::repo::system::SystemRepo;
use boba_app::stats::audit::{AuditLogPage, AuditLogService, DEFAULT_PAGE};
use boba_app::stats::{FunnelStats, StatsError, StatsService};

use crate::wiring;

fn to_app(error: StatsError) -> AppError {
    AppError::new(ErrorCode::Internal, error.message_key())
}

/// One page of the Audit Log timeline, newest first (AC-1).
///
/// `cursor` is the `next_cursor` of the page before; `limit` is clamped in the
/// core. CH12: the viewer never asks for — or receives — the whole log.
#[tauri::command]
#[specta::specta]
pub fn audit_log(
    app: tauri::AppHandle,
    cursor: Option<String>,
    limit: Option<u32>,
) -> AppResult<AuditLogPage> {
    let service = AuditLogService::new(
        wiring::port::<dyn SystemRepo>(&app)?,
        wiring::port::<dyn ApplyRepo>(&app)?,
        wiring::port::<dyn OutboxHoldRepo>(&app)?,
        wiring::port::<dyn AutoRulesRepo>(&app)?,
        wiring::port::<dyn SourcesRepo>(&app)?,
    );
    service
        .page(cursor.as_deref(), limit.unwrap_or(DEFAULT_PAGE))
        .map_err(to_app)
}

/// Write the Audit Log as CSV to the path the user picked in the native dialog
/// (AC-1: "I can export it as CSV"). Returns the path written.
#[tauri::command]
#[specta::specta]
pub fn audit_log_export_csv(app: tauri::AppHandle, destination: String) -> AppResult<String> {
    if destination.trim().is_empty() {
        return Err(AppError::invalid_input("errors.audit_stats.path"));
    }
    let service = AuditLogService::new(
        wiring::port::<dyn SystemRepo>(&app)?,
        wiring::port::<dyn ApplyRepo>(&app)?,
        wiring::port::<dyn OutboxHoldRepo>(&app)?,
        wiring::port::<dyn AutoRulesRepo>(&app)?,
        wiring::port::<dyn SourcesRepo>(&app)?,
    );
    let csv = service.csv().map_err(to_app)?;
    std::fs::write(&destination, csv)
        .map_err(|_| AppError::internal("errors.audit_stats.write"))?;
    Ok(destination)
}

/// Applications by Stage, reply rate and interview rate, with the Source,
/// Verdict and language breakdowns (AC-2). Computed locally.
#[tauri::command]
#[specta::specta]
pub fn stats_funnel(app: tauri::AppHandle) -> AppResult<FunnelStats> {
    let service = StatsService::new(
        wiring::port::<dyn ApplyRepo>(&app)?,
        wiring::port::<dyn IngestionRepo>(&app)?,
        wiring::port::<dyn ScoringRepo>(&app)?,
        wiring::port::<dyn SourcesRepo>(&app)?,
    );
    service.funnel().map_err(to_app)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_map_to_one_translated_key() {
        let mapped = to_app(StatsError::System(
            boba_app::ports::repo::system::SystemRepoError::new("disk"),
        ));
        assert_eq!(mapped.code, ErrorCode::Internal);
        assert_eq!(mapped.message_key, "errors.audit_stats.store");
    }
}
