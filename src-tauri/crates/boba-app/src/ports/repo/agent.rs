//! `AgentRepo` — owns `agent_run`, `agent_event`, `cli_install` (AD-4, AD-16).
//! Adapter: `boba-store`. The runtime records each run and its event stream here.
//!
//! Events are passed as already-serialised JSON so the adapter needs no serde
//! knowledge of the event model.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A stored run's terminal state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Finished,
    Failed,
    Cancelled,
}

impl RunStatus {
    pub const fn id(self) -> &'static str {
        match self {
            RunStatus::Running => "running",
            RunStatus::Finished => "finished",
            RunStatus::Failed => "failed",
            RunStatus::Cancelled => "cancelled",
        }
    }
}

/// A run to open before its process starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAgentRun {
    pub id: String,
    /// `AgentTaskKind::id`.
    pub task_kind: String,
    /// `InputTrust` id (`untrusted`/`user`).
    pub input_trust: String,
    /// `CliKind::id`.
    pub cli: String,
    pub model: Option<String>,
}

/// The terminal update for a run (architecture 5.4).
#[derive(Debug, Clone, PartialEq)]
pub struct FinishAgentRun {
    pub status: RunStatus,
    /// `FailureReason::id` when `status` is `failed`/`cancelled`.
    pub failure_reason: Option<String>,
    pub tokens_in: Option<i64>,
    pub tokens_out: Option<i64>,
    pub cost_usd: Option<f64>,
}

/// A stored run, for the run report and crash hygiene.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentRunRecord {
    pub id: String,
    pub task_kind: String,
    pub input_trust: String,
    pub cli: String,
    pub model: Option<String>,
    pub status: String,
    pub failure_reason: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub tokens_in: Option<i64>,
    pub tokens_out: Option<i64>,
    #[allow(clippy::struct_field_names)]
    pub cost_usd: Option<f64>,
}

/// A stored event, in run order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentEventRecord {
    pub seq: i64,
    pub kind: String,
    pub payload_json: String,
    pub at: String,
}

/// Repository failure, stringly-typed so the port stays storage-agnostic.
#[derive(Debug, Error)]
#[error("{0}")]
pub struct AgentRepoError(pub String);

/// Owns the agent tables (AD-16). Only this port writes them.
pub trait AgentRepo: Send + Sync {
    /// Open a run with `status = running`.
    fn start_run(&self, run: NewAgentRun) -> Result<(), AgentRepoError>;

    /// Append one event at `seq`. `kind` is the event variant id.
    fn record_event(
        &self,
        run_id: &str,
        seq: i64,
        kind: &str,
        payload_json: &str,
    ) -> Result<(), AgentRepoError>;

    /// Close a run with its terminal status and reported usage.
    fn finish_run(&self, run_id: &str, finish: FinishAgentRun) -> Result<(), AgentRepoError>;

    /// Fetch a run, if present.
    fn get_run(&self, run_id: &str) -> Result<Option<AgentRunRecord>, AgentRepoError>;

    /// All events for a run, ordered by `seq`.
    fn list_events(&self, run_id: &str) -> Result<Vec<AgentEventRecord>, AgentRepoError>;

    /// Runs left `running` by a crash, oldest first (crash hygiene).
    fn unfinished_runs(&self) -> Result<Vec<AgentRunRecord>, AgentRepoError>;
}
