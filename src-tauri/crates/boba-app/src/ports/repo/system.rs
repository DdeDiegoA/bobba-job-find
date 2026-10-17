//! `SystemRepo` — owns `setting`, `job_run`, `audit_event`, `secret_ref`
//! (AD-16, AD-18). Only this area writes those tables; other modules read
//! through its queries. `job_run`/`secret_ref` land with their slices (S5.1,
//! S8.1) — this slice adds `setting` and `audit_event` (AD-12).
//!
//! The keychain `audit-head` anchor (AD-12) is not a port method: it is
//! keychain IO, so it stays inside the `boba-store` adapter (architect,
//! 2026-10-05).

use boba_domain::audit::{AuditOutcome, AuditRow, ChainStatus};
use thiserror::Error;

/// One `setting` row (Consistency Conventions: typed config in the DB).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingRecord {
    pub key: String,
    pub value: String,
    pub updated_at: String,
}

/// An `intent` row to append (AD-12). `created_at` is supplied by the caller
/// from the injected clock so time is deterministic under test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewIntent {
    pub action: String,
    pub correlation_id: String,
    pub actor: String,
    pub target_kind: String,
    pub target_id: Option<String>,
    pub detail_json: String,
    pub payload_blake3: Option<String>,
    pub created_at: String,
}

/// A repo-level failure, stringified so the port stays free of the adapter's
/// error types.
#[derive(Debug, Clone, Error)]
#[error("system store error: {0}")]
pub struct SystemRepoError(pub String);

impl SystemRepoError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// The `core/system` owner area (AD-16).
pub trait SystemRepo: Send + Sync {
    // ---- setting -----------------------------------------------------------

    fn setting_get(&self, key: &str) -> Result<Option<String>, SystemRepoError>;

    fn setting_put(&self, key: &str, value: &str, updated_at: &str) -> Result<(), SystemRepoError>;

    fn setting_list(&self) -> Result<Vec<SettingRecord>, SystemRepoError>;

    /// Spend one unit from a capped counter at `key` (S11.1b): increment it
    /// only while it is below `cap`, atomically. Returns the new total, or
    /// `None` when the cap is already spent.
    ///
    /// The default is read-then-write, which is enough for an in-memory fake;
    /// the SQL adapter overrides it with one conditional statement so that two
    /// overlapping runs cannot both slip past the cap.
    fn setting_spend_below(
        &self,
        key: &str,
        cap: u32,
        updated_at: &str,
    ) -> Result<Option<u32>, SystemRepoError> {
        let stored = self
            .setting_get(key)?
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0);
        if stored >= cap {
            return Ok(None);
        }
        let total = stored + 1;
        self.setting_put(key, &total.to_string(), updated_at)?;
        Ok(Some(total))
    }

    fn setting_delete(&self, key: &str) -> Result<bool, SystemRepoError>;

    // ---- audit_event (AD-12) ----------------------------------------------

    /// Append an `intent` row, chaining it to the current head and advancing the
    /// keychain `audit-head` anchor to the new head (AD-12).
    fn audit_append_intent(&self, intent: &NewIntent) -> Result<AuditRow, SystemRepoError>;

    /// Append an `outcome` row completing `intent`, advancing the keychain
    /// `audit-head` anchor to the new head (AD-12).
    fn audit_append_outcome(
        &self,
        intent: &AuditRow,
        outcome: AuditOutcome,
        detail_json: &str,
        created_at: &str,
    ) -> Result<AuditRow, SystemRepoError>;

    /// `hash` of the last row, `None` for an empty log.
    fn audit_head_hash(&self) -> Result<Option<String>, SystemRepoError>;

    /// Walk the whole chain, re-hashing every row (AD-12 tamper check).
    fn audit_verify_chain(&self) -> Result<ChainStatus, SystemRepoError>;

    /// `intent` rows with no matching `outcome` (crash recovery, FR-89).
    fn audit_unresolved_intents(&self) -> Result<Vec<AuditRow>, SystemRepoError>;

    /// Read the keychain `audit-head` anchor, `None` when never written.
    fn audit_anchor_get(&self) -> Result<Option<String>, SystemRepoError>;

    /// Whether `hash` names a row in the chain (a known prefix head). Startup
    /// uses it to tell a crash-lagged anchor from a truncated one (AD-12).
    fn audit_anchor_is_known(&self, hash: &str) -> Result<bool, SystemRepoError>;

    /// Mirror the latest hash to the keychain `audit-head` anchor.
    fn audit_anchor_set(&self, hash: &str) -> Result<(), SystemRepoError>;
}
