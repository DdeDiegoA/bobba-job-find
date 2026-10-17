//! `AutoRulesRepo` — owns `auto_rule` and `auto_rule_attempt` (S9.6, FR-83,
//! AD-16).
//!
//! The rules are stored rows, never a loose JSON file (Consistency Conventions:
//! config), so core validates and the UI only proposes. The attempt ledger is
//! what the three caps count: it is written when Auto decides to send, before
//! the channel does anything, so a send that dies mid-flight still counts.
//!
//! The adapter is `boba-store::repos::auto_rules`.

use boba_domain::apply::policy::{AutoAttemptEntry, AutoRule};

/// A repo-level failure, stringified so the port stays free of the adapter's
/// error types.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("auto rule store error: {0}")]
pub struct AutoRulesRepoError(pub String);

impl AutoRulesRepoError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// The Auto Rules storage surface (AD-16: apply owns its own tables).
pub trait AutoRulesRepo: Send + Sync {
    /// Every rule, oldest first.
    fn rules(&self) -> Result<Vec<AutoRule>, AutoRulesRepoError>;

    /// Insert or replace one rule by id.
    fn rule_upsert(&self, rule: &AutoRule) -> Result<(), AutoRulesRepoError>;

    /// Remove one rule. Returns whether a row was deleted.
    fn rule_delete(&self, id: &str) -> Result<bool, AutoRulesRepoError>;

    // ---- the cap ledger ----------------------------------------------------

    /// Record one automatic submission before it is attempted.
    fn ledger_insert(&self, entry: &AutoAttemptEntry) -> Result<(), AutoRulesRepoError>;

    /// Automatic submissions recorded at or after `since` (RFC 3339, UTC).
    fn ledger_global_since(&self, since: &str) -> Result<u32, AutoRulesRepoError>;

    /// One rule's automatic submissions at or after `since`.
    fn ledger_rule_since(&self, rule_id: &str, since: &str) -> Result<u32, AutoRulesRepoError>;

    /// Automatic submissions to one company at or after `since`.
    fn ledger_company_since(
        &self,
        company_key: &str,
        since: &str,
    ) -> Result<u32, AutoRulesRepoError>;

    /// The newest ledger row's `created_at`, `None` when Auto never sent.
    fn ledger_last_at(&self) -> Result<Option<String>, AutoRulesRepoError>;

    /// The newest automatic send to one Source, `None` when that Source never
    /// took one. S9.7's per-Source spacing reads it (FR-91).
    fn ledger_last_at_for_source(
        &self,
        source_id: &str,
    ) -> Result<Option<String>, AutoRulesRepoError>;
}
