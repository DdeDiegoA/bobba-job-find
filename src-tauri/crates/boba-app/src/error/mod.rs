//! The typed error model shared by the core and the UI (AD-3).
//!
//! Every Tauri command returns `Result<T, AppError>`. An `AppError` is a
//! machine-readable triple: a [`ErrorCode`], an i18n `message_key` and optional
//! structured `details`. The UI never renders `AppError` itself; it translates
//! `message_key` (see `src/lib/ipc/errors.ts`). Raw Rust strings never cross the
//! IPC boundary as user-facing text.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use specta::Type;

/// Stable, machine-readable error category. Adding a variant is additive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Unexpected internal failure.
    Internal,
    /// A caller argument failed validation.
    InvalidInput,
    /// The requested entity does not exist.
    NotFound,
    /// The request conflicts with current state.
    Conflict,
    /// The store is locked (key unavailable); see ADR-009.
    Locked,
    /// The caller is not authenticated/authorised for the action.
    Unauthorized,
    /// A required dependency is temporarily unavailable.
    Unavailable,
    /// The task was cancelled by the user or the kill switch.
    Cancelled,
    /// The capability is not supported on this build/host.
    Unsupported,
}

/// Structured, non-user-facing error context: ids, field names, i18n keys.
///
/// Flat string map on purpose: `serde_json::Value` is recursive and specta
/// cannot inline it into a TypeScript type. Values are diagnostics only and are
/// never rendered as raw text.
///
/// ponytail: flat map; widen to a nested `Details` enum when a slice needs it.
pub type Details = BTreeMap<String, String>;

/// A typed command error: `{code, message_key, details}`.
///
/// `message_key` is an i18n key resolved by the UI. `details` carries optional
/// structured context (field names, ids) for logging and diagnostics; it is
/// never shown as raw text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, thiserror::Error)]
#[error("{code:?}: {message_key}")]
pub struct AppError {
    /// Machine-readable category.
    pub code: ErrorCode,
    /// i18n key resolved by the UI (never a raw user-facing string).
    pub message_key: String,
    /// Optional structured context for diagnostics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Details>,
}

impl AppError {
    /// Build an error from a code and an i18n message key.
    pub fn new(code: ErrorCode, message_key: impl Into<String>) -> Self {
        Self {
            code,
            message_key: message_key.into(),
            details: None,
        }
    }

    /// Replace the structured diagnostic details.
    pub fn with_details(mut self, details: Details) -> Self {
        self.details = Some(details);
        self
    }

    /// Add one structured diagnostic entry.
    pub fn with_detail(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.details
            .get_or_insert_with(Details::new)
            .insert(key.into(), value.into());
        self
    }

    /// An unexpected internal failure.
    pub fn internal(message_key: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message_key)
    }

    /// An invalid caller argument.
    pub fn invalid_input(message_key: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidInput, message_key)
    }

    /// The store is locked (ADR-009).
    pub fn locked(message_key: impl Into<String>) -> Self {
        Self::new(ErrorCode::Locked, message_key)
    }

    /// The request conflicts with current state (S3.8: a run without a
    /// recovery backup).
    pub fn conflict(message_key: impl Into<String>) -> Self {
        Self::new(ErrorCode::Conflict, message_key)
    }

    /// The task was cancelled.
    pub fn cancelled(message_key: impl Into<String>) -> Self {
        Self::new(ErrorCode::Cancelled, message_key)
    }
}

/// Convenience alias for command return types.
pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialises_as_code_message_key_details() {
        let err =
            AppError::new(ErrorCode::NotFound, "errors.offer.not_found").with_detail("id", "01H");
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["code"], "not_found");
        assert_eq!(json["message_key"], "errors.offer.not_found");
        assert_eq!(json["details"]["id"], "01H");
    }

    #[test]
    fn omits_absent_details() {
        let json = serde_json::to_value(AppError::internal("errors.internal")).unwrap();
        assert!(json.get("details").is_none());
    }
}
