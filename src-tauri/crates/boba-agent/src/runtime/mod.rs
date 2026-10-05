//! Agent run lifecycle (AD-4, architecture 5.4): spawn a CLI task through the
//! `ProcessSupervisor`, parse its stream into normalised `AgentEvent`s, persist
//! and emit each one, enforce per-task limits, and classify failures so AI work
//! can pause while non-AI work continues (FR-21, NFR-14).

use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};

use boba_app::ports::agent::{
    AgentEvent, AgentPort, AgentTask, CancelToken, FailureReason, InputTrust, RunHandle,
};
use boba_app::ports::repo::agent::{AgentRepo, FinishAgentRun, NewAgentRun, RunStatus};

use crate::drivers::{CliKind, HealthStatus};
use crate::supervisor::{CommandSpec, Stream, SupervisedProcess};

/// How long to wait for a line before re-checking cancellation and limits.
const POLL: Duration = Duration::from_millis(100);

/// The concrete CLI invocation for a task. S2.3 owns workspace + MCP config
/// rendering; S2.2 tests supply a fake CLI here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSpec {
    pub spec: CommandSpec,
}

/// Event sink, e.g. the Tauri `agent.run.progressed` emitter.
pub type Emitter = dyn Fn(&str, i64, &AgentEvent) + Send + Sync;

/// Runs tasks on a chosen CLI, persisting each run and its events (AD-4).
pub struct AgentRuntime<R: AgentRepo + 'static> {
    repo: Arc<R>,
    emitter: Option<Arc<Emitter>>,
    /// Optional per-run poll cap, giving deterministic tests a stop condition.
    max_polls: Option<u64>,
}

impl<R: AgentRepo + 'static> AgentRuntime<R> {
    pub fn new(repo: Arc<R>) -> Self {
        Self {
            repo,
            emitter: None,
            max_polls: None,
        }
    }

    /// Attach an event sink, e.g. the Tauri `agent.run.progressed` emitter.
    pub fn with_emitter(mut self, emitter: Arc<Emitter>) -> Self {
        self.emitter = Some(emitter);
        self
    }

    /// Bound the poll loop (tests). Production leaves it unbounded.
    pub fn with_max_polls(mut self, max_polls: u64) -> Self {
        self.max_polls = Some(max_polls);
        self
    }

    /// Run `task` against `run_spec`, streaming events on a background thread.
    pub fn start(&self, task: AgentTask, run_spec: RunSpec) -> RunHandle {
        let run_id = ulid::Ulid::generate().to_string();
        let cancel = CancelToken::new();
        let (tx, events) = std::sync::mpsc::channel();

        // Open the run before spawning so a crash can be detected (AD-16).
        let _ = self.repo.start_run(NewAgentRun {
            id: run_id.clone(),
            task_kind: task.kind.id().to_string(),
            input_trust: trust_id(task.input_trust).to_string(),
            cli: task.cli.clone(),
            model: task.model.clone(),
        });

        let cancel_for_thread = cancel.clone();
        let repo = Arc::clone(&self.repo);
        let emitter = self.emitter.clone();
        let max_polls = self.max_polls;
        let state = RunState {
            run_id: run_id.clone(),
            task,
            run_spec,
            repo,
            emitter,
            tx: tx.clone(),
            seq: 0,
        };
        thread::Builder::new()
            .name(format!("boba-agent-run-{run_id}"))
            .spawn(move || drive(state, cancel_for_thread, max_polls))
            .expect("spawn run thread");

        RunHandle {
            run_id,
            events,
            cancel,
        }
    }
}

impl<R: AgentRepo + 'static> AgentPort for AgentRuntime<R> {
    /// Adapter-agnostic entry point used by features. The CLI command is built
    /// by the adapter from the task's `cli`; S2.2's adapter is `boba-agent`
    /// itself, so the command is resolved by the caller as a [`RunSpec`].
    fn run(&self, _task: AgentTask) -> RunHandle {
        panic!("use AgentRuntime::start(task, RunSpec) until S2.3 wires MCP config")
    }
}

fn trust_id(trust: InputTrust) -> &'static str {
    match trust {
        InputTrust::Untrusted => "untrusted",
        InputTrust::User => "user",
    }
}

/// Run state carried through the poll loop.
struct RunState<R: AgentRepo> {
    run_id: String,
    task: AgentTask,
    run_spec: RunSpec,
    repo: Arc<R>,
    emitter: Option<Arc<Emitter>>,
    tx: Sender<AgentEvent>,
    seq: i64,
}

impl<R: AgentRepo> RunState<R> {
    /// Persist + emit one event and advance the sequence number.
    fn emit(&mut self, event: &AgentEvent) {
        let seq = self.seq;
        self.seq += 1;
        let payload = serde_json::to_string(event).unwrap_or_else(|_| "{}".to_string());
        let _ = self
            .repo
            .record_event(&self.run_id, seq, event_kind(event), &payload);
        if let Some(sink) = &self.emitter {
            sink(&self.run_id, seq, event);
        }
        let _ = self.tx.send(event.clone());
    }
}

/// How the loop ended, mapped to a terminal status + reason.
enum Outcome {
    Finished {
        exit_code: i32,
        tokens_in: Option<i64>,
        tokens_out: Option<i64>,
        cost_usd: Option<f64>,
    },
    Cancelled,
    Failed(FailureReason),
}

/// Drive one run to completion: spawn, drain, close the row, emit terminal.
fn drive<R: AgentRepo>(mut state: RunState<R>, cancel: CancelToken, max_polls: Option<u64>) {
    let outcome = run_process(&mut state, &cancel, max_polls);

    let (status, reason) = match &outcome {
        Outcome::Finished { .. } => (RunStatus::Finished, None),
        Outcome::Cancelled => (RunStatus::Cancelled, Some(FailureReason::Cancelled)),
        Outcome::Failed(reason) => (RunStatus::Failed, Some(*reason)),
    };
    let terminal = match &outcome {
        Outcome::Finished { exit_code, .. } => AgentEvent::Finished {
            exit_code: *exit_code,
        },
        Outcome::Cancelled => failed_event(FailureReason::Cancelled),
        Outcome::Failed(reason) => failed_event(*reason),
    };
    state.emit(&terminal);

    let (tokens_in, tokens_out, cost_usd) = match outcome {
        Outcome::Finished {
            tokens_in,
            tokens_out,
            cost_usd,
            ..
        } => (tokens_in, tokens_out, cost_usd),
        _ => (None, None, None),
    };
    let _ = state.repo.finish_run(
        &state.run_id,
        FinishAgentRun {
            status,
            failure_reason: reason.map(|r| r.id().to_string()),
            tokens_in,
            tokens_out,
            cost_usd,
        },
    );
}

fn failed_event(reason: FailureReason) -> AgentEvent {
    AgentEvent::Failed {
        reason,
        message_key: reason.message_key().to_string(),
    }
}

/// Spawn and drain the process, returning how the loop ended.
fn run_process<R: AgentRepo>(
    state: &mut RunState<R>,
    cancel: &CancelToken,
    max_polls: Option<u64>,
) -> Outcome {
    let mut proc = match SupervisedProcess::spawn(&state.run_spec.spec) {
        Ok(proc) => proc,
        Err(_) => {
            // A missing/unlaunchable binary is a not-found failure.
            return Outcome::Failed(FailureReason::NotFound);
        }
    };

    state.emit(&AgentEvent::Started {
        cli: state.task.cli.clone(),
        model: state.task.model.clone(),
    });

    let deadline = state
        .task
        .budget
        .wall_clock
        .map(|limit| Instant::now() + limit);
    let mut usage = Usage::default();
    let mut tool_calls: u32 = 0;
    let mut polls: u64 = 0;

    loop {
        if cancel.is_cancelled() {
            proc.kill();
            return Outcome::Cancelled;
        }
        if deadline.is_some_and(|d| Instant::now() >= d) {
            proc.kill();
            return Outcome::Failed(FailureReason::Timeout);
        }
        if state
            .task
            .budget
            .max_tool_calls
            .is_some_and(|cap| tool_calls >= cap)
        {
            proc.kill();
            return Outcome::Failed(FailureReason::LimitExceeded);
        }
        if max_polls.is_some_and(|cap| polls >= cap) {
            proc.kill();
            return Outcome::Failed(FailureReason::Crashed);
        }
        polls += 1;

        match proc.recv_line(POLL) {
            Stream::Line(line) => {
                for event in parse_line(&line) {
                    match &event {
                        AgentEvent::Usage {
                            tokens_in,
                            tokens_out,
                            cost_usd,
                        } => usage.absorb(*tokens_in, *tokens_out, *cost_usd),
                        AgentEvent::ToolCall { .. } => tool_calls += 1,
                        _ => {}
                    }
                    state.emit(&event);
                }
            }
            Stream::Idle => continue,
            Stream::Eof => break,
        }
    }

    match proc.wait() {
        Some(status) if status.success() => Outcome::Finished {
            exit_code: status.code().unwrap_or(0),
            tokens_in: usage.tokens_in,
            tokens_out: usage.tokens_out,
            cost_usd: usage.cost_usd,
        },
        Some(status) => {
            let stderr = proc.stderr_so_far();
            Outcome::Failed(classify_failure(&stderr, status.code().unwrap_or(1)))
        }
        None => Outcome::Failed(FailureReason::Crashed),
    }
}

/// Parse one stdout line as an `AgentEvent`; non-JSON is plain `Text`.
pub fn parse_line(line: &str) -> Vec<AgentEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if trimmed.starts_with('{')
        && let Ok(event) = serde_json::from_str::<AgentEvent>(trimmed)
    {
        return vec![event];
    }
    vec![AgentEvent::Text {
        text: trimmed.to_string(),
    }]
}

/// The event variant id used as `agent_event.kind`.
fn event_kind(event: &AgentEvent) -> &'static str {
    match event {
        AgentEvent::Started { .. } => "started",
        AgentEvent::Text { .. } => "text",
        AgentEvent::ToolCall { .. } => "tool_call",
        AgentEvent::ToolResult { .. } => "tool_result",
        AgentEvent::Usage { .. } => "usage",
        AgentEvent::Finished { .. } => "finished",
        AgentEvent::Failed { .. } => "failed",
    }
}

/// Classify a nonzero exit into a `FailureReason` from its stderr (FR-21).
pub fn classify_failure(stderr: &str, exit_code: i32) -> FailureReason {
    let lower = stderr.to_ascii_lowercase();
    const QUOTA_HINTS: [&str; 6] = [
        "quota",
        "rate limit",
        "rate_limit",
        "429",
        "too many requests",
        "usage limit",
    ];
    const AUTH_HINTS: [&str; 6] = [
        "log in",
        "login",
        "not authenticated",
        "api key",
        "api_key",
        "unauthorized",
    ];
    const NOT_FOUND_HINTS: [&str; 3] = ["not found", "no such file", "command not found"];
    if QUOTA_HINTS.iter().any(|hint| lower.contains(hint)) {
        FailureReason::QuotaExceeded
    } else if AUTH_HINTS.iter().any(|hint| lower.contains(hint)) {
        FailureReason::AuthExpired
    } else if NOT_FOUND_HINTS.iter().any(|hint| lower.contains(hint)) || exit_code == 127 {
        FailureReason::NotFound
    } else {
        FailureReason::Crashed
    }
}

/// Classify a detection health probe into a failure reason, if unhealthy (S2.1).
pub fn health_failure(health: &HealthStatus) -> Option<FailureReason> {
    match health {
        HealthStatus::Healthy => None,
        HealthStatus::AuthRequired => Some(FailureReason::AuthExpired),
        HealthStatus::Unhealthy(_) => Some(FailureReason::Crashed),
    }
}

/// Resolve a `CliKind` id (as carried on `AgentTask.cli`) to its kind.
pub fn cli_kind(id: &str) -> Option<CliKind> {
    CliKind::ALL.into_iter().find(|kind| kind.id() == id)
}

/// Locate (building if needed) the fake CLI used by runtime/supervisor tests.
///
/// `cargo test` builds only the test harness in `target/debug/deps`, never
/// sibling bins, so a cold CI cache has no `target/debug/boba-fake-cli`. This
/// builds it once on first use. `BOBA_FAKE_CLI` short-circuits (packaged E2E).
#[cfg(test)]
pub(crate) fn fake_cli_path() -> std::path::PathBuf {
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        if let Ok(path) = std::env::var("BOBA_FAKE_CLI") {
            return PathBuf::from(path);
        }
        let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .find(|dir| dir.join("Cargo.toml").is_file() && dir.join("tools/fake-cli").is_dir())
            .expect("workspace root");
        let status = std::process::Command::new(env!("CARGO"))
            .args(["build", "-p", "boba-fake-cli"])
            .current_dir(workspace_root)
            .status()
            .expect("run cargo build -p boba-fake-cli");
        assert!(status.success(), "cargo build -p boba-fake-cli failed");
        let name = if cfg!(windows) {
            "boba-fake-cli.exe"
        } else {
            "boba-fake-cli"
        };
        let bin = workspace_root.join("target/debug").join(name);
        assert!(bin.exists(), "fake CLI not built at {}", bin.display());
        bin
    })
    .clone()
}

#[derive(Default, Clone, Copy)]
struct Usage {
    tokens_in: Option<i64>,
    tokens_out: Option<i64>,
    cost_usd: Option<f64>,
}

impl Usage {
    fn absorb(&mut self, tokens_in: Option<i64>, tokens_out: Option<i64>, cost_usd: Option<f64>) {
        if let Some(v) = tokens_in {
            *self.tokens_in.get_or_insert(0) += v;
        }
        if let Some(v) = tokens_out {
            *self.tokens_out.get_or_insert(0) += v;
        }
        if let Some(v) = cost_usd {
            *self.cost_usd.get_or_insert(0.0) += v;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_json_lines_parse_as_text() {
        assert_eq!(
            parse_line("hello world"),
            vec![AgentEvent::Text {
                text: "hello world".to_string()
            }]
        );
        assert!(parse_line("   ").is_empty());
    }

    #[test]
    fn json_lines_parse_to_events() {
        let line = r#"{"type":"usage","tokens_in":10,"tokens_out":5,"cost_usd":0.01}"#;
        assert_eq!(
            parse_line(line),
            vec![AgentEvent::Usage {
                tokens_in: Some(10),
                tokens_out: Some(5),
                cost_usd: Some(0.01)
            }]
        );
    }

    #[test]
    fn failure_classification_covers_auth_quota_notfound() {
        assert_eq!(
            classify_failure("Error: 429 too many requests", 1),
            FailureReason::QuotaExceeded
        );
        assert_eq!(
            classify_failure("Please log in to continue", 1),
            FailureReason::AuthExpired
        );
        assert_eq!(
            classify_failure("model not found: gpt-x", 1),
            FailureReason::NotFound
        );
        assert_eq!(
            classify_failure("segmentation fault", 139),
            FailureReason::Crashed
        );
    }

    #[test]
    fn cli_kind_resolves_task_ids() {
        assert_eq!(cli_kind("claude_code"), Some(CliKind::ClaudeCode));
        assert_eq!(cli_kind("nope"), None);
    }

    #[test]
    fn usage_accumulates_reported_values_only() {
        let mut usage = Usage::default();
        usage.absorb(Some(10), None, Some(0.5));
        usage.absorb(Some(5), Some(3), None);
        assert_eq!(usage.tokens_in, Some(15));
        assert_eq!(usage.tokens_out, Some(3));
        assert_eq!(usage.cost_usd, Some(0.5));
    }
}
