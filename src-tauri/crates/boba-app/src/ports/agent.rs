//! `AgentPort` — run an agent-CLI task and stream normalised events (AD-4).
//! Adapter: `boba-agent`. The port is CLI-agnostic: a feature builds an
//! [`AgentTask`] and consumes [`AgentEvent`]s; drivers live behind the adapter.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Trust level of a task's inputs (AD-4/AD-6). Tasks that read postings,
/// company pages or email are `Untrusted`; profile/translation work is `User`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputTrust {
    Untrusted,
    User,
}

/// The capabilities Boba asks of an agent CLI (FR-16, architecture 5.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTaskKind {
    ProfileImport,
    /// The profile gap interview (S3.6): short questions for what the Profile
    /// is missing. Trusted by default; a run whose input carries third-party
    /// text declares `InputTrust::Untrusted` and takes the lockdown gate (AC4).
    ProfileGaps,
    ExtractPostings,
    NarrateScores,
    TailorApplication,
    ReviewApplication,
    AnswerQuestions,
    ClassifyEmails,
    TranslateProfile,
    /// One existing fact rewritten as a suggestion (FX10a). The input is the
    /// user's own fact, but the output is agent text that rewrites it: the run
    /// takes the lockdown gate like untrusted work (ADR-010).
    ImproveFact,
}

impl AgentTaskKind {
    /// Every kind, in the order the UI lists them. Mirrors
    /// `boba_mcp_proto::tool::ALL_TASKS`: a task the catalog does not know is a
    /// task the core cannot run.
    pub const ALL: [AgentTaskKind; 10] = [
        AgentTaskKind::ProfileImport,
        AgentTaskKind::ProfileGaps,
        AgentTaskKind::ExtractPostings,
        AgentTaskKind::NarrateScores,
        AgentTaskKind::TailorApplication,
        AgentTaskKind::ReviewApplication,
        AgentTaskKind::AnswerQuestions,
        AgentTaskKind::ClassifyEmails,
        AgentTaskKind::TranslateProfile,
        AgentTaskKind::ImproveFact,
    ];

    /// Stable id used in the DB (`agent_run.task_kind`) and diagnostics.
    pub const fn id(self) -> &'static str {
        match self {
            AgentTaskKind::ProfileImport => "profile_import",
            AgentTaskKind::ProfileGaps => "profile_gaps",
            AgentTaskKind::ExtractPostings => "extract_postings",
            AgentTaskKind::NarrateScores => "narrate_scores",
            AgentTaskKind::TailorApplication => "tailor_application",
            AgentTaskKind::ReviewApplication => "review_application",
            AgentTaskKind::AnswerQuestions => "answer_questions",
            AgentTaskKind::ClassifyEmails => "classify_emails",
            AgentTaskKind::TranslateProfile => "translate_profile",
            AgentTaskKind::ImproveFact => "improve_fact",
        }
    }

    /// Whether this task reads untrusted content (AD-6/ADR-010). Such tasks run
    /// only on a CLI with verified `tool_lockdown`; that gate is enforced where
    /// the task is dispatched, not here.
    pub const fn reads_untrusted(self) -> bool {
        // FX10a: `ImproveFact` is deliberately outside this allowlist. Its
        // input is the user's own fact, but its output rewrites that fact as
        // agent text, so the run must prove its tool lockdown first.
        !matches!(
            self,
            AgentTaskKind::ProfileImport
                | AgentTaskKind::ProfileGaps
                | AgentTaskKind::TranslateProfile
        )
    }
}

/// Per-task limits (architecture 5.4). `None` means "no limit of this kind".
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RunBudget {
    /// Wall-clock cap; the process tree is killed when it elapses.
    pub wall_clock: Option<Duration>,
    /// Maximum MCP tool calls before the run fails with `LimitExceeded`.
    pub max_tool_calls: Option<u32>,
    /// Maximum agent turns (enforced once drivers report turn boundaries).
    pub max_turns: Option<u32>,
    /// Maximum reported cost in USD (enforced once drivers report `Usage` cost).
    pub max_cost_usd: Option<f64>,
}

/// A run request. `cli` is a `CliKind` id; the adapter resolves it to an
/// installation and builds the CLI-specific command (AD-4).
#[derive(Debug, Clone, PartialEq)]
pub struct AgentTask {
    pub kind: AgentTaskKind,
    pub input_trust: InputTrust,
    /// `CliKind` id, e.g. `claude_code`.
    pub cli: String,
    pub model: Option<String>,
    pub prompt: String,
    /// The **content** language the deliverables are written in (BCP-47,
    /// AD-17), filled into the agent pack for this run. Separate from the UI
    /// locale: it follows the posting, not the user's setting.
    pub target_language: String,
    pub budget: RunBudget,
    /// Extra environment for the run (per-run MCP token, workspace paths, and
    /// test scenarios). Secrets are passed here, never on the command line.
    pub env: Vec<(String, String)>,
}

impl AgentTask {
    /// Whether this task reads content nobody in this app vouched for: its kind
    /// reads third-party text, or it declares untrusted input outright. Such
    /// tasks require a verified tool lockdown (ADR-010 §2); everything that
    /// gates or refuses a run asks this one question.
    pub fn reads_untrusted(&self) -> bool {
        self.kind.reads_untrusted() || self.input_trust == InputTrust::Untrusted
    }
}

/// A normalised event streamed from a run (AD-4). The wire form is JSON with a
/// `type` tag; drivers that emit plain text degrade to [`AgentEvent::Text`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    Started {
        cli: String,
        model: Option<String>,
    },
    Text {
        text: String,
    },
    ToolCall {
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        name: String,
        output: String,
    },
    Usage {
        tokens_in: Option<i64>,
        tokens_out: Option<i64>,
        cost_usd: Option<f64>,
    },
    Finished {
        exit_code: i32,
    },
    Failed {
        reason: FailureReason,
        message_key: String,
    },
}

/// Why a run ended without a result (FR-21, architecture 5.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureReason {
    /// The CLI needs a login or API key; AI work pauses until fixed.
    AuthExpired,
    /// The CLI/provider quota is exhausted; AI work pauses until it resets.
    QuotaExceeded,
    /// The requested model or CLI was not found.
    NotFound,
    /// The wall-clock limit elapsed; the process tree was killed.
    Timeout,
    /// A per-task limit (tool calls, cost) was hit; never a silent truncation.
    LimitExceeded,
    /// The user or the kill switch cancelled the run.
    Cancelled,
    /// The CLI cannot run this task safely (ADR-010's tool-lockdown gate): the
    /// task is unsupported on this install, with no user override.
    Unsupported,
    /// The task's output stayed invalid after its retries: the run stops rather
    /// than storing a payload the core cannot use (AC2).
    InvalidOutput,
    /// The CLI crashed or exited nonzero for an unclassified reason.
    Crashed,
}

impl FailureReason {
    pub const fn id(self) -> &'static str {
        match self {
            FailureReason::AuthExpired => "auth_expired",
            FailureReason::QuotaExceeded => "quota_exceeded",
            FailureReason::NotFound => "not_found",
            FailureReason::Timeout => "timeout",
            FailureReason::LimitExceeded => "limit_exceeded",
            FailureReason::Cancelled => "cancelled",
            FailureReason::Unsupported => "unsupported",
            FailureReason::InvalidOutput => "invalid_output",
            FailureReason::Crashed => "crashed",
        }
    }

    /// i18n key for the plain-language message shown to the user.
    pub const fn message_key(self) -> &'static str {
        match self {
            FailureReason::AuthExpired => "errors.agent.auth_expired",
            FailureReason::QuotaExceeded => "errors.agent.quota_exceeded",
            FailureReason::NotFound => "errors.agent.not_found",
            FailureReason::Timeout => "errors.agent.timeout",
            FailureReason::LimitExceeded => "errors.agent.limit_exceeded",
            FailureReason::Cancelled => "errors.agent.cancelled",
            FailureReason::Unsupported => "errors.agent.unsupported",
            FailureReason::InvalidOutput => "errors.agent.invalid_output",
            FailureReason::Crashed => "errors.agent.crashed",
        }
    }

    /// Whether this failure pauses AI tasks (NFR-14: non-AI work continues).
    pub const fn pauses_ai(self) -> bool {
        matches!(
            self,
            FailureReason::AuthExpired | FailureReason::QuotaExceeded
        )
    }
}

/// A cooperative cancellation token shared by the caller and the run.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    /// Request cancellation. The run kills its process tree and emits `Failed`.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// A running task. Drop it without draining `events` loses the stream but does
/// not stop the run; use [`CancelToken`] to stop.
pub struct RunHandle {
    pub run_id: String,
    /// Normalised events, in order, ending after `Finished` or `Failed`.
    pub events: Receiver<AgentEvent>,
    pub cancel: CancelToken,
}

/// Runs `AgentTask`s and streams normalised events (AD-4).
pub trait AgentPort: Send + Sync {
    fn run(&self, task: AgentTask) -> RunHandle;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untrusted_tasks_are_the_content_reading_ones() {
        assert!(!AgentTaskKind::ProfileImport.reads_untrusted());
        assert!(!AgentTaskKind::TranslateProfile.reads_untrusted());
        assert!(AgentTaskKind::NarrateScores.reads_untrusted());
        assert!(AgentTaskKind::TailorApplication.reads_untrusted());
    }

    #[test]
    fn a_task_that_declares_untrusted_input_is_treated_as_one() {
        let mut task = AgentTask {
            kind: AgentTaskKind::TranslateProfile,
            input_trust: InputTrust::User,
            cli: "claude_code".to_string(),
            model: None,
            prompt: "translate".to_string(),
            target_language: "en".to_string(),
            budget: RunBudget::default(),
            env: Vec::new(),
        };
        assert!(!task.reads_untrusted());
        task.input_trust = InputTrust::Untrusted;
        assert!(task.reads_untrusted());
    }

    #[test]
    fn only_auth_and_quota_pause_ai() {
        assert!(FailureReason::AuthExpired.pauses_ai());
        assert!(FailureReason::QuotaExceeded.pauses_ai());
        assert!(!FailureReason::Timeout.pauses_ai());
        assert!(!FailureReason::Crashed.pauses_ai());
        assert!(
            FailureReason::AuthExpired
                .message_key()
                .starts_with("errors.")
        );
    }

    #[test]
    fn events_round_trip_as_tagged_json() {
        let event = AgentEvent::ToolCall {
            name: "get_posting".to_string(),
            input: serde_json::json!({"id": "01H"}),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"tool_call\""), "{json}");
        assert_eq!(serde_json::from_str::<AgentEvent>(&json).unwrap(), event);
    }

    #[test]
    fn cancel_token_is_shared() {
        let token = CancelToken::new();
        let clone = token.clone();
        assert!(!clone.is_cancelled());
        token.cancel();
        assert!(clone.is_cancelled());
    }
}
