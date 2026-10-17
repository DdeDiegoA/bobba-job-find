//! Session Board Auto use cases (S11.3, FR-91, AD-11, AD-12).
//!
//! Auto on a Session Board Source has its own opt-in and its own limits, and
//! both are decided here, never on a screen:
//!
//! * [`SessionAutoService::opt_in`] accepts only the typed Source name and the
//!   warning this build ships, and writes the audit pair before the row lands;
//! * the daily cap is validated here (three by default, ten is the hard
//!   maximum), so no UI can widen it;
//! * [`SessionAutoService::snapshot`] is the one place the state (off, lapsed,
//!   paused, active) is computed: an opt-in older than 30 days or a challenge
//!   pause falls back to Review, and every send re-reads it through the core.
//!
//! The policy itself (numbers, records, state) lives in
//! [`boba_domain::apply::session_policy`]; this module is the IO seam.

use std::sync::Arc;

use boba_domain::apply::session_policy::{
    self, CapViolation, SessionAutoOptIn, SessionAutoRefusal, SessionAutoStatus,
};
use boba_domain::audit::AuditOutcome;
use boba_domain::source::{AccessClass, SourceRecord};
use serde::{Deserialize, Serialize};
use specta::Type;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::ports::clock::ClockPort;
use crate::ports::repo::auto_rules::{AutoRulesRepo, AutoRulesRepoError};
use crate::ports::repo::sources::{SourcesRepo, SourcesRepoError};
use crate::ports::repo::system::{NewIntent, SystemRepo, SystemRepoError};

/// The audit action an accepted Auto opt-in writes (FR-91: with the warning
/// version and the typed name).
pub const OPT_IN_ACTION: &str = "settings.session_auto.opt_in";
/// The audit action revoking it writes.
pub const REVOKE_ACTION: &str = "settings.session_auto.revoke";
/// The audit action a cap change writes.
pub const CAP_ACTION: &str = "settings.session_auto.cap";

/// One Session Board Source's Auto state, as the settings screen shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct SessionAutoView {
    pub source_id: String,
    pub name: String,
    /// The Session Board reading opt-in (S11.1a) — Auto needs it too.
    pub reading_opted_in: bool,
    /// `off`, `expired`, `paused` or `active` (FR-91).
    pub state: String,
    /// When the Auto opt-in was accepted, `None` when never.
    pub accepted_at: Option<String>,
    /// The name the user typed at acceptance.
    pub confirmed_name: Option<String>,
    /// The warning version the user read.
    pub warning_version: Option<String>,
    /// When the opt-in must be re-confirmed (30 days after acceptance).
    pub expires_at: Option<String>,
    /// Until when a challenge keeps Auto paused.
    pub paused_until: Option<String>,
    /// The per-Source daily cap (default 3, maximum 10).
    pub daily_cap: u8,
    /// Automatic submissions to this Source today.
    pub used_today: u32,
    /// The hard maximum this build accepts, so the UI never invents one.
    pub max_daily_cap: u8,
    /// The warning copy version this build ships.
    pub current_warning_version: String,
}

/// Why a Session Board Auto use case was refused.
#[derive(Debug, thiserror::Error)]
pub enum SessionAutoError {
    #[error("unknown source '{0}'")]
    UnknownSource(String),
    #[error("source '{0}' is not a Session Board Source")]
    NotSessionBoard(String),
    /// AD-9/FR-91: Auto is a second opt-in; the first one is required.
    #[error("source '{0}' has no Session Board opt-in")]
    ReadingOptInRequired(String),
    #[error(transparent)]
    Refused(#[from] SessionAutoRefusal),
    #[error(transparent)]
    Cap(#[from] CapViolation),
    #[error(transparent)]
    Sources(#[from] SourcesRepoError),
    #[error(transparent)]
    Rules(#[from] AutoRulesRepoError),
    #[error(transparent)]
    System(#[from] SystemRepoError),
    /// A stored timestamp this build cannot read.
    #[error("the stored timestamp cannot be read")]
    Clock,
}

impl SessionAutoError {
    /// The translated message key the UI shows (CH7: `errors.session.*`).
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::UnknownSource(_) => "errors.session.unknown_source",
            Self::NotSessionBoard(_) => "errors.session.not_session_board",
            Self::ReadingOptInRequired(_) => "errors.session.auto_needs_reading",
            Self::Refused(SessionAutoRefusal::NameMismatch) => "errors.session.auto_name_mismatch",
            Self::Refused(_) => "errors.session.auto_warning_changed",
            Self::Cap(_) => "errors.session.auto_cap_invalid",
            Self::Sources(_) | Self::Rules(_) | Self::System(_) | Self::Clock => {
                "errors.session.store"
            }
        }
    }
}

/// The Session Board Auto use cases.
pub struct SessionAutoService {
    sources: Arc<dyn SourcesRepo>,
    rules: Arc<dyn AutoRulesRepo>,
    system: Arc<dyn SystemRepo>,
    clock: Arc<dyn ClockPort>,
}

impl SessionAutoService {
    pub fn new(
        sources: Arc<dyn SourcesRepo>,
        rules: Arc<dyn AutoRulesRepo>,
        system: Arc<dyn SystemRepo>,
        clock: Arc<dyn ClockPort>,
    ) -> Self {
        Self {
            sources,
            rules,
            system,
            clock,
        }
    }

    /// Every Session Board Source and where its Auto stands (AC1, FR-91).
    pub fn snapshot(&self) -> Result<Vec<SessionAutoView>, SessionAutoError> {
        let mut views = Vec::new();
        for source in self.sources.list_sources()? {
            if source.access_class == AccessClass::SessionBrowser {
                views.push(self.view_of(&source)?);
            }
        }
        views.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(views)
    }

    /// One Source's Auto state.
    pub fn view(&self, source_id: &str) -> Result<SessionAutoView, SessionAutoError> {
        let source = self.session_source(source_id)?;
        self.view_of(&source)
    }

    /// Accept the separate Auto opt-in (FR-91 (a)+(b)+(c)).
    ///
    /// The core compares the typed name and the warning version; it records the
    /// attempt in the Audit Log **before** the row lands, so an opt-in the log
    /// never heard of cannot exist. Re-confirming after 30 days is the same
    /// call: the row is replaced and the previous acceptance is not trusted.
    pub fn opt_in(
        &self,
        source_id: &str,
        typed_name: &str,
        warning_version: &str,
    ) -> Result<SessionAutoView, SessionAutoError> {
        let source = self.session_source(source_id)?;
        // FR-91 (c): the separate Auto opt-in sits on top of the reading one.
        if self.sources.get_optin(source_id)?.is_none() {
            return Err(SessionAutoError::ReadingOptInRequired(
                source_id.to_string(),
            ));
        }
        let now = self.now()?;
        let optin =
            SessionAutoOptIn::accept(source_id, &source.name, typed_name, warning_version, now)?;
        let detail = serde_json::json!({
            "source_id": source_id,
            "confirmed_name": optin.confirmed_name,
            "warning_version": optin.warning_version,
        });
        let stamp = session_policy::format_time(now);
        self.audited(
            OPT_IN_ACTION,
            source_id,
            &stamp,
            detail,
            || {
                self.sources.record_auto_opt_in(&optin)?;
                Ok(())
            },
            |_| serde_json::json!({}),
        )?;
        self.view_of(&source)
    }

    /// Revoke the Auto opt-in (FR-91: revocable at any time). The reading
    /// opt-in and the challenge pause are untouched.
    pub fn revoke(&self, source_id: &str) -> Result<SessionAutoView, SessionAutoError> {
        let source = self.session_source(source_id)?;
        let now = session_policy::format_time(self.now()?);
        let detail = serde_json::json!({ "source_id": source_id });
        self.audited(
            REVOKE_ACTION,
            source_id,
            &now,
            detail,
            || {
                self.sources.remove_auto_opt_in(source_id)?;
                Ok(())
            },
            |_| serde_json::json!({}),
        )?;
        self.view_of(&source)
    }

    /// Set the Source's daily cap: three by default, ten is the hard maximum
    /// (FR-91). Validated here, never by the UI.
    pub fn set_cap(&self, source_id: &str, cap: u8) -> Result<SessionAutoView, SessionAutoError> {
        let source = self.session_source(source_id)?;
        session_policy::validate_daily_cap(cap)?;
        let now = session_policy::format_time(self.now()?);
        let detail = serde_json::json!({ "source_id": source_id, "cap": cap });
        self.audited(
            CAP_ACTION,
            source_id,
            &now,
            detail,
            || {
                self.system.setting_put(
                    &session_policy::cap_key(source_id),
                    &cap.to_string(),
                    &now,
                )?;
                Ok(())
            },
            |_| serde_json::json!({}),
        )?;
        self.view_of(&source)
    }

    // ---- internals ---------------------------------------------------------

    fn view_of(&self, source: &SourceRecord) -> Result<SessionAutoView, SessionAutoError> {
        let now = self.now()?;
        let opt_in = self.sources.auto_opt_in(&source.id)?;
        let paused_until = self.sources.auto_paused_until(&source.id)?;
        // Auto builds on the reading opt-in (FR-91 (c)): turning that off ends
        // Auto too, even if the Auto row is still there.
        let reading_opted_in = self.sources.get_optin(&source.id)?.is_some();
        let status = if reading_opted_in {
            session_policy::status(opt_in.as_ref(), paused_until.as_deref(), now)
        } else {
            SessionAutoStatus::Off
        };
        let cap = session_policy::stored_daily_cap(
            self.system
                .setting_get(&session_policy::cap_key(&source.id))?
                .as_deref(),
        );
        Ok(SessionAutoView {
            source_id: source.id.clone(),
            name: source.name.clone(),
            reading_opted_in,
            state: status.as_str().to_string(),
            accepted_at: opt_in.as_ref().map(|row| row.accepted_at.clone()),
            confirmed_name: opt_in.as_ref().map(|row| row.confirmed_name.clone()),
            warning_version: opt_in.as_ref().map(|row| row.warning_version.clone()),
            expires_at: opt_in
                .as_ref()
                .and_then(SessionAutoOptIn::expires_at)
                .map(session_policy::format_time),
            paused_until,
            daily_cap: cap,
            used_today: self.used_today(&source.id, now)?,
            max_daily_cap: session_policy::MAX_SESSION_DAILY_CAP,
            current_warning_version: session_policy::AUTO_WARNING_VERSION.to_string(),
        })
    }

    /// Automatic submissions to `source_id` today, in the user's local day —
    /// the same clock the S9.6 caps and the active hours use.
    fn used_today(&self, source_id: &str, now: OffsetDateTime) -> Result<u32, SessionAutoError> {
        let offset =
            time::Duration::minutes(i64::from(crate::apply::rules::local_offset_minutes()));
        let day_start = (now + offset).replace_time(time::Time::MIDNIGHT) - offset;
        Ok(self.rules.ledger_source_since(
            source_id,
            "session_board",
            &session_policy::format_time(day_start),
        )?)
    }

    fn session_source(&self, source_id: &str) -> Result<SourceRecord, SessionAutoError> {
        let source = self
            .sources
            .get_source(source_id)?
            .ok_or_else(|| SessionAutoError::UnknownSource(source_id.to_string()))?;
        if source.access_class != AccessClass::SessionBrowser {
            return Err(SessionAutoError::NotSessionBoard(source_id.to_string()));
        }
        Ok(source)
    }

    fn now(&self) -> Result<OffsetDateTime, SessionAutoError> {
        OffsetDateTime::parse(&self.clock.now_rfc3339(), &Rfc3339)
            .map_err(|_| SessionAutoError::Clock)
    }

    /// The AD-12 pair: the intent is on record before the change, and the
    /// outcome says what it was. A failed intent write means nothing changes.
    fn audited<T>(
        &self,
        action: &str,
        source_id: &str,
        now: &str,
        detail: serde_json::Value,
        change: impl FnOnce() -> Result<T, SessionAutoError>,
        outcome: impl FnOnce(&T) -> serde_json::Value,
    ) -> Result<T, SessionAutoError> {
        let intent = self.system.audit_append_intent(&NewIntent {
            action: action.to_string(),
            correlation_id: format!("{action}_{source_id}_{}", now.replace([':', '+', '.'], "-")),
            actor: "user".to_string(),
            target_kind: "source".to_string(),
            target_id: Some(source_id.to_string()),
            detail_json: detail.to_string(),
            payload_blake3: None,
            created_at: now.to_string(),
        })?;
        match change() {
            Ok(value) => {
                self.system.audit_append_outcome(
                    &intent,
                    AuditOutcome::Ok,
                    &outcome(&value).to_string(),
                    now,
                )?;
                Ok(value)
            }
            Err(error) => {
                let _ = self.system.audit_append_outcome(
                    &intent,
                    AuditOutcome::Failed,
                    &serde_json::json!({ "error": error.to_string() }).to_string(),
                    now,
                );
                Err(error)
            }
        }
    }
}

/// Whether Auto may send on `source_id` right now (FR-91). Callers that build
/// a [`boba_domain::apply::policy::PolicyCandidate`] ask this: the bool is the
/// `source_auto_opt_in` hard limit, and the facts are the per-Source cap.
pub fn auto_state(
    sources: &dyn SourcesRepo,
    source_id: &str,
    now: OffsetDateTime,
) -> Result<SessionAutoStatus, SourcesRepoError> {
    // The reading opt-in is the floor Auto stands on: without it, nothing on
    // that Source may be automated, whatever the second opt-in says.
    if sources.get_optin(source_id)?.is_none() {
        return Ok(SessionAutoStatus::Off);
    }
    let opt_in = sources.auto_opt_in(source_id)?;
    let paused_until = sources.auto_paused_until(source_id)?;
    Ok(session_policy::status(
        opt_in.as_ref(),
        paused_until.as_deref(),
        now,
    ))
}

#[cfg(test)]
mod tests;
