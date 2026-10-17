//! `OutboxHoldRepo` — owns `apply_outbox` and `apply_rule_dry_run` (S9.7,
//! FR-84/FR-85, AD-16).
//!
//! One row is one queued automatic send, and its `state` is the whole story:
//! `would_send` in a rule's dry run, `held` while it waits, `releasing` once the
//! policy said Auto and the channel is about to send, then `sent`, `cancelled`
//! or `needs_review`. The dry-run table is one rule's progress; a rule without a
//! row has never run and reports the compiled default.
//!
//! The adapter is `boba-store::repos::outbox_hold`.

use boba_domain::apply::outbox::{DryRunState, NewOutboxItem, OutboxItem, OutboxState};

/// A repo-level failure, stringified so the port stays free of the adapter's
/// error types.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("outbox store error: {0}")]
pub struct OutboxHoldRepoError(pub String);

impl OutboxHoldRepoError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// The Outbox hold storage surface (AD-16: the apply area owns its tables).
pub trait OutboxHoldRepo: Send + Sync {
    /// Record one queued item and return it as stored.
    fn outbox_insert(
        &self,
        item: &NewOutboxItem,
        now: &str,
    ) -> Result<OutboxItem, OutboxHoldRepoError>;

    /// One item by id, `None` when unknown.
    fn outbox_get(&self, id: &str) -> Result<Option<OutboxItem>, OutboxHoldRepoError>;

    /// The application's one open item (`would_send`, `held` or `releasing`).
    fn outbox_live_for(
        &self,
        application_id: &str,
    ) -> Result<Option<OutboxItem>, OutboxHoldRepoError>;

    /// The most recent items, newest first, `limit` at most.
    fn outbox_list(&self, limit: usize) -> Result<Vec<OutboxItem>, OutboxHoldRepoError>;

    /// The `held` items whose `release_at` has come, oldest first (FR-85).
    fn outbox_due(&self, now: &str) -> Result<Vec<OutboxItem>, OutboxHoldRepoError>;

    /// The `releasing` items: a launch that finds one is a crash mid-release.
    fn outbox_releasing(&self) -> Result<Vec<OutboxItem>, OutboxHoldRepoError>;

    /// Claim one held item for one releaser: `held` → `releasing` in one
    /// conditional write. Returns whether this caller won, so two sweeps (the
    /// tick and the screen) can never release the same item twice.
    fn outbox_claim(&self, id: &str, now: &str) -> Result<bool, OutboxHoldRepoError>;

    /// Move one item to `state`, recording why when it leaves the path. Returns
    /// the item as stored; an unknown id is an error, never a silent write.
    fn outbox_set_state(
        &self,
        id: &str,
        state: OutboxState,
        note: Option<&str>,
        now: &str,
    ) -> Result<OutboxItem, OutboxHoldRepoError>;

    /// Replace the user's edits (`None` clears one), returning the item.
    fn outbox_set_overrides(
        &self,
        id: &str,
        subject: Option<&str>,
        body: Option<&str>,
        now: &str,
    ) -> Result<OutboxItem, OutboxHoldRepoError>;

    // ---- the dry run -------------------------------------------------------

    /// One rule's dry-run progress, `None` when the rule never ran.
    fn dry_run_get(&self, rule_id: &str) -> Result<Option<DryRunState>, OutboxHoldRepoError>;

    /// Every stored dry-run row, oldest rule first.
    fn dry_run_list(&self) -> Result<Vec<DryRunState>, OutboxHoldRepoError>;

    /// Insert or replace one rule's dry-run progress.
    fn dry_run_upsert(&self, state: &DryRunState, now: &str) -> Result<(), OutboxHoldRepoError>;
}
