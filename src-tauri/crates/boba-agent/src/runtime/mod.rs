//! Agent run lifecycle (AD-4, architecture 5.4): spawn a CLI task through the
//! `ProcessSupervisor`, parse its stream into normalised `AgentEvent`s, persist
//! and emit each one, enforce per-task limits, and classify failures so AI work
//! can pause while non-AI work continues (FR-21, NFR-14).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};

use boba_app::ports::agent::{
    AgentEvent, AgentPort, AgentTask, CancelToken, FailureReason, InputTrust, RunHandle,
};
use boba_app::ports::repo::agent::{AgentRepo, FinishAgentRun, NewAgentRun, RunStatus};

use crate::drivers::{self, CliKind, HealthStatus};
use crate::lockdown::{self, TaskSupport, Unverified};
use crate::mcp::config::{self, RunSandbox, SandboxConfig};
use crate::mcp::server::{Limits, McpEndpoint, ToolHandler};
use crate::pack;
use crate::supervisor::{CommandSpec, Stream, SupervisedProcess};

/// How long to wait for a line before re-checking cancellation and limits.
const POLL: Duration = Duration::from_millis(100);

/// The concrete CLI invocation for a task. The runtime adds the run's
/// workspace as `cwd` and its endpoint and token to `env` (AC1); callers that
/// build the command themselves (tests, the composition root) pass one of
/// these, and `AgentPort::run` builds it through [`CommandBuilder`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSpec {
    pub spec: CommandSpec,
}

/// Builds the CLI invocation for one task. The composition root injects it: a
/// `CliDriver` owns detection and argv (AD-4, ADR-012), and the [`RunSandbox`]
/// it is handed carries the workspace, the run's `mcp.json` and the endpoint,
/// so adding the CLI's own MCP-config flag stays the driver's business.
pub type CommandBuilder = dyn Fn(&AgentTask, &RunSandbox) -> CommandSpec + Send + Sync;

/// Event sink, e.g. the Tauri `agent.run.progressed` emitter.
pub type Emitter = dyn Fn(&str, i64, &AgentEvent) + Send + Sync;

/// Whether a task may run at all (ADR-010 §2). The composition root injects it —
/// it owns the probed CLI evidence — and it is consulted before a run gets a
/// workspace, so no caller can reach a CLI the gate holds back. A runtime
/// without a gate runs what it is given; production always mounts one.
pub type TaskGate = dyn Fn(&AgentTask) -> TaskSupport + Send + Sync;

/// Runs tasks on a chosen CLI, persisting each run and its events (AD-4).
pub struct AgentRuntime<R: AgentRepo + 'static> {
    repo: Arc<R>,
    emitter: Option<Arc<Emitter>>,
    /// Optional per-run poll cap, giving deterministic tests a stop condition.
    max_polls: Option<u64>,
    /// The per-run sandbox and MCP endpoint (AC1, AD-6). Absent until the
    /// composition root mounts them (ADR-012) and in tests that drive the
    /// supervisor with their own fixture command.
    sandbox: Option<(SandboxConfig, Arc<dyn ToolHandler>)>,
    /// Turns a task into a CLI command, for [`AgentPort::run`].
    builder: Option<Arc<CommandBuilder>>,
    /// ADR-010's gate, consulted before anything is created for a run.
    gate: Option<Arc<TaskGate>>,
    /// Bounds the endpoint puts on its peers.
    limits: Limits,
}

impl<R: AgentRepo + 'static> AgentRuntime<R> {
    pub fn new(repo: Arc<R>) -> Self {
        Self {
            repo,
            emitter: None,
            max_polls: None,
            sandbox: None,
            builder: None,
            gate: None,
            limits: Limits::default(),
        }
    }

    /// Give every run its own workspace, token and MCP endpoint, served by the
    /// injected handler. The composition root owns the paths and implements
    /// `ToolHandler` over the repos (ADR-012): nothing here knows app data.
    pub fn with_sandbox(mut self, sandbox: SandboxConfig, handler: Arc<dyn ToolHandler>) -> Self {
        self.sandbox = Some((sandbox, handler));
        self
    }

    /// How a task becomes a CLI command, for [`AgentPort::run`].
    pub fn with_command_builder(mut self, builder: Arc<CommandBuilder>) -> Self {
        self.builder = Some(builder);
        self
    }

    /// The hard gate every task goes through (ADR-010 §2).
    pub fn with_gate(mut self, gate: Arc<TaskGate>) -> Self {
        self.gate = Some(gate);
        self
    }

    /// Endpoint limits (tests shorten the handshake deadline).
    pub fn with_endpoint_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
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
        self.launch(task, move |_, _| Some(run_spec))
    }

    /// Prepare the run's workspace and endpoint, take its command from `build`,
    /// then spawn. Every run ends by deleting the workspace and revoking the
    /// token, whichever way it ended (AC1).
    fn launch(
        &self,
        task: AgentTask,
        build: impl FnOnce(&AgentTask, Option<&RunSandbox>) -> Option<RunSpec>,
    ) -> RunHandle {
        let run_id = ulid::Ulid::generate().to_string();

        if let Some(refused) = self.held_back(task.clone(), run_id.clone()) {
            return refused;
        }

        let some_sandbox = match self
            .sandbox
            .as_ref()
            .map(|(config, _)| config::prepare(config, &run_id))
        {
            Some(Ok(sandbox)) => Some(sandbox),
            // A run that cannot get its own workspace must not start.
            Some(Err(_)) => return self.failed(task, run_id, FailureReason::Crashed),
            None => None,
        };
        // The workspace is the run's instruction sheet: `system.md` plus this
        // task's skill, in the file the CLI reads, with the run's content
        // language and its provenance (AC1, AC2), plus the CLI's own MCP config
        // when it reads that from the workspace (AD-6).
        if let Some(sandbox) = some_sandbox.as_ref()
            && furnish(sandbox, &task).is_err()
        {
            return self.failed(task, run_id, FailureReason::Crashed);
        }
        let sandbox = some_sandbox;
        let endpoint = match self.endpoint_for(&task, sandbox.as_ref()) {
            Ok(endpoint) => endpoint,
            Err(reason) => return self.failed(task, run_id, reason),
        };
        let Some(run_spec) = build(&task, sandbox.as_ref()) else {
            // No command builder mounted, or no sandbox to run in: the run is
            // still recorded and reported, never a panic (ADR-012).
            return self.failed(task, run_id, FailureReason::NotFound);
        };

        self.spawn(
            task,
            run_id,
            sandboxed(run_spec, sandbox.as_ref()),
            sandbox,
            endpoint,
        )
    }

    /// Bind this run's MCP endpoint; `None` when no sandbox is mounted.
    fn endpoint_for(
        &self,
        task: &AgentTask,
        sandbox: Option<&RunSandbox>,
    ) -> Result<Option<McpEndpoint>, FailureReason> {
        let (Some((_, handler)), Some(sandbox)) = (self.sandbox.as_ref(), sandbox) else {
            return Ok(None);
        };
        McpEndpoint::start(
            &sandbox.endpoint,
            task.kind.id(),
            &sandbox.token,
            &task.prompt,
            task.budget.max_tool_calls,
            Arc::clone(handler),
            self.limits,
        )
        .map(Some)
        .map_err(|_| FailureReason::Crashed)
    }

    /// Open the run, spawn the command, and stream its events.
    fn spawn(
        &self,
        task: AgentTask,
        run_id: String,
        run_spec: RunSpec,
        sandbox: Option<RunSandbox>,
        endpoint: Option<McpEndpoint>,
    ) -> RunHandle {
        let cancel = CancelToken::new();
        let (tx, events) = std::sync::mpsc::channel();

        // Open the run before spawning so a crash can be detected (AD-16).
        let _ = self.repo.start_run(NewAgentRun {
            id: run_id.clone(),
            task_kind: task.kind.id().to_string(),
            input_trust: trust_id(task.input_trust).to_string(),
            cli: task.cli.clone(),
            model: task.model.clone(),
            pack_version: pack::version().to_string(),
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
            endpoint,
            tx: tx.clone(),
            seq: 0,
        };
        thread::Builder::new()
            .name(format!("boba-agent-run-{run_id}"))
            .spawn(move || {
                // Held for the whole run: dropping the sandbox deletes the
                // workspace (AC1). The endpoint lives in the run state.
                let _held = sandbox;
                drive(state, cancel_for_thread, max_polls);
            })
            .expect("spawn run thread");

        RunHandle {
            run_id,
            events,
            cancel,
        }
    }

    /// Hold a task back before it gets a workspace, a token or a CLI (ADR-010
    /// §2). Returns the finished handle when the gate refuses, `None` when the
    /// task may run.
    ///
    /// With no gate mounted — a fixture, or a composition that forgot one — a
    /// task that reads third-party content is refused anyway: nothing has
    /// vouched for it, so the default is refusal, never a quiet run.
    fn held_back(&self, task: AgentTask, run_id: String) -> Option<RunHandle> {
        let verdict = match self.gate.as_ref() {
            Some(gate) => gate(&task),
            None if task.reads_untrusted() => TaskSupport::Unsupported {
                reason: Unverified::NoVerifiedConfig,
                fallback: None,
            },
            None => TaskSupport::Supported,
        };
        let TaskSupport::Unsupported { .. } = verdict else {
            return None;
        };
        // Recorded and reported like any other failure: the reason carries the
        // i18n key the UI explains the refusal with.
        Some(self.failed(task, run_id, FailureReason::Unsupported))
    }

    /// Record and report a run that never started. The port features call has
    /// no panic path: a missing engine is a failure with a reason.
    fn failed(&self, task: AgentTask, run_id: String, reason: FailureReason) -> RunHandle {
        let (tx, events) = std::sync::mpsc::channel();
        let _ = self.repo.start_run(NewAgentRun {
            id: run_id.clone(),
            task_kind: task.kind.id().to_string(),
            input_trust: trust_id(task.input_trust).to_string(),
            cli: task.cli.clone(),
            model: task.model.clone(),
            pack_version: pack::version().to_string(),
        });

        let events_of_the_run = [
            AgentEvent::Started {
                cli: task.cli.clone(),
                model: task.model.clone(),
            },
            failed_event(reason),
        ];
        for (seq, event) in events_of_the_run.iter().enumerate() {
            record_event(
                &*self.repo,
                self.emitter.as_ref(),
                &tx,
                &run_id,
                seq as i64,
                event,
            );
        }
        let _ = self.repo.finish_run(
            &run_id,
            FinishAgentRun {
                status: RunStatus::Failed,
                failure_reason: Some(reason.id().to_string()),
                tokens_in: None,
                tokens_out: None,
                cost_usd: None,
            },
        );

        RunHandle {
            run_id,
            events,
            cancel: CancelToken::new(),
        }
    }
}

impl<R: AgentRepo + 'static> AgentPort for AgentRuntime<R> {
    /// The AD-4 entry point features call. The task's CLI command comes from the
    /// injected [`CommandBuilder`]; the workspace, the token, the endpoint and
    /// the tool policy are the runtime's (AC1, AD-6).
    fn run(&self, task: AgentTask) -> RunHandle {
        self.launch(task, |task, sandbox| {
            let (builder, sandbox) = (self.builder.as_ref()?, sandbox?);
            Some(RunSpec {
                spec: builder(task, sandbox),
            })
        })
    }
}

/// Fill a run's workspace with what the CLI reads there: the agent pack (AC1)
/// and, for a CLI that reads its own MCP config from the workspace, that config
/// (AD-6). Both are per run, so both are written now and leave with the
/// workspace.
fn furnish(sandbox: &RunSandbox, task: &AgentTask) -> std::io::Result<()> {
    let Some(kind) = cli_kind(&task.cli) else {
        return Err(std::io::Error::other("unknown agent CLI"));
    };
    pack::write_into(
        sandbox.workspace.path(),
        task.kind,
        kind,
        task.model.as_deref(),
        &task.target_language,
    )?;
    let Some((file, config)) =
        kind.workspace_mcp_config(&shim_path(sandbox), &sandbox.endpoint, &sandbox.token)
    else {
        return Ok(());
    };
    std::fs::write(sandbox.workspace.path().join(file), config.to_string())
}

/// The `boba-mcp` shim this run points at, read back from the run's own
/// `mcp.json`. The workspace config of a CLI that keeps its MCP servers in its
/// own file names the same binary, so one source of truth serves both.
fn shim_path(sandbox: &RunSandbox) -> PathBuf {
    std::fs::read_to_string(&sandbox.config)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|config| {
            config["mcpServers"][boba_mcp_proto::tool::SERVER_NAME]["command"]
                .as_str()
                .map(PathBuf::from)
        })
        .unwrap_or_else(|| PathBuf::from("boba-mcp"))
}

/// Put the run inside its workspace: `cwd` is the run's own directory, and the
/// endpoint and token travel in the environment, never in argv.
fn sandboxed(mut run_spec: RunSpec, sandbox: Option<&RunSandbox>) -> RunSpec {
    if let Some(sandbox) = sandbox {
        run_spec.spec.cwd = Some(sandbox.workspace.path().to_path_buf());
        run_spec
            .spec
            .env
            .extend(config::run_env(&sandbox.endpoint, &sandbox.token));
    }
    run_spec
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
    /// The run's MCP endpoint, held for as long as the run: dropping it revokes
    /// the token, and its invalid-output flag ends the run (AC2).
    endpoint: Option<McpEndpoint>,
    tx: Sender<AgentEvent>,
    seq: i64,
}

impl<R: AgentRepo> RunState<R> {
    /// Persist + emit one event and advance the sequence number.
    fn emit(&mut self, event: &AgentEvent) {
        let seq = self.seq;
        self.seq += 1;
        record_event(
            &*self.repo,
            self.emitter.as_ref(),
            &self.tx,
            &self.run_id,
            seq,
            event,
        );
    }
}

/// Persist and emit one event. Both the running path and the failure paths use
/// it, so a run looks the same to the UI whichever way it ended.
fn record_event(
    repo: &dyn AgentRepo,
    emitter: Option<&Arc<Emitter>>,
    tx: &Sender<AgentEvent>,
    run_id: &str,
    seq: i64,
    event: &AgentEvent,
) {
    let payload = serde_json::to_string(event).unwrap_or_else(|_| "{}".to_string());
    let _ = repo.record_event(run_id, seq, event_kind(event), &payload);
    if let Some(sink) = emitter {
        sink(run_id, seq, event);
    }
    let _ = tx.send(event.clone());
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
    // The CLI's own stream format (AD-4), and its init event as ADR-010 §1's
    // evidence. An unknown CLI id parses as plain text: no init event, so
    // nothing gets verified for it.
    let mut reader = cli_kind(&state.task.cli)
        .map(drivers::StreamReader::for_cli)
        .unwrap_or_default();
    let mut usage = Usage::default();
    let mut tool_calls: u32 = 0;
    let mut polls: u64 = 0;
    // Whether this run has been opened by its init event (ADR-010 §1).
    let mut opened = false;

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
        if state
            .endpoint
            .as_ref()
            .is_some_and(McpEndpoint::failed_output)
        {
            // The agent's output stayed invalid after its retries: stop now, and
            // report why (AC2), rather than let the CLI finish a run whose work
            // the core has already refused.
            proc.kill();
            return Outcome::Failed(FailureReason::InvalidOutput);
        }
        if max_polls.is_some_and(|cap| polls >= cap) {
            proc.kill();
            return Outcome::Failed(FailureReason::Crashed);
        }
        polls += 1;

        match proc.recv_line(POLL) {
            Stream::Line(line) => {
                let parsed = reader.line(&line);
                if let Some(tools) = parsed.init_tools {
                    // ADR-010 §1, the run-time observation: the CLI's own first
                    // word on which tools it has. Only this build's tools, or the
                    // pinned lockdown did not hold.
                    opened = true;
                    if !lockdown::init_event_is_locked_down(&tools) && state.task.reads_untrusted()
                    {
                        // Fail closed: a run that reads third-party content
                        // must not continue with tools the envelope does not
                        // protect. (A user-input task may continue with the
                        // security badge, ADR-010 §2.)
                        proc.kill();
                        return Outcome::Failed(FailureReason::Unsupported);
                    }
                } else if !parsed.events.is_empty() {
                    // An untrusted run is opened by its init event, always: a
                    // stream that starts with the agent's own work means the pin
                    // — or the stream that reports what the CLI may call — did
                    // not hold, so the run stops instead of continuing
                    // unverified.
                    if state.task.reads_untrusted() && !opened {
                        proc.kill();
                        return Outcome::Failed(FailureReason::Unsupported);
                    }
                    opened = true;
                }
                for event in parsed.events {
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

    // ADR-010 §1, ruling (d): the stream is over, so whether it ever showed the
    // init event is now settled. A run that reads third-party content and never
    // did is not a verified one.
    let unverified = state.task.reads_untrusted() && !opened;

    let status = proc.wait();

    let outcome = match status {
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
    };

    // An unverified run never just reports "finished": nothing vouched for the
    // tools it had, so the run says so whatever the CLI claimed. The one
    // carve-out is a failure that pauses AI work — auth expired, quota — because
    // FR-21 needs that signal ("sign in again") to reach the user, and reading it
    // as "this CLI is unsupported" would both misdiagnose and un-pause the daily
    // run (orquestador ruling (d) + reviewer C).
    if unverified && !matches!(&outcome, Outcome::Failed(reason) if reason.pauses_ai()) {
        return Outcome::Failed(FailureReason::Unsupported);
    }

    outcome
}

/// Parse one stdout line as an `AgentEvent`; non-JSON is plain `Text`. The
/// CLI-agnostic half of the stream readers in `drivers`.
pub fn parse_line(line: &str) -> Vec<AgentEvent> {
    drivers::plain_line(line).events
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
    use boba_app::ports::agent::{AgentTaskKind, RunBudget};
    use boba_app::ports::repo::agent::{
        AgentEventRecord, AgentRepoError, AgentRunRecord, RunStatus,
    };
    use std::path::PathBuf;
    use std::sync::Mutex;

    /// Minimal in-memory `AgentRepo`: enough to drive a real run in-process.
    #[derive(Default)]
    struct FakeRepo {
        runs: Mutex<Vec<AgentRunRecord>>,
    }

    impl AgentRepo for FakeRepo {
        fn start_run(&self, run: NewAgentRun) -> Result<(), AgentRepoError> {
            self.runs.lock().unwrap().push(AgentRunRecord {
                id: run.id,
                task_kind: run.task_kind,
                input_trust: run.input_trust,
                cli: run.cli,
                model: run.model,
                pack_version: run.pack_version,
                status: RunStatus::Running.id().to_string(),
                failure_reason: None,
                started_at: "2026-10-05T00:00:00.000Z".to_string(),
                ended_at: None,
                tokens_in: None,
                tokens_out: None,
                cost_usd: None,
            });
            Ok(())
        }

        fn record_event(
            &self,
            _run_id: &str,
            _seq: i64,
            _kind: &str,
            _payload_json: &str,
        ) -> Result<(), AgentRepoError> {
            Ok(())
        }

        fn finish_run(&self, run_id: &str, finish: FinishAgentRun) -> Result<(), AgentRepoError> {
            let mut runs = self.runs.lock().unwrap();
            let run = runs.iter_mut().find(|run| run.id == run_id).expect("run");
            run.status = finish.status.id().to_string();
            run.failure_reason = finish.failure_reason;
            run.ended_at = Some("2026-10-05T00:00:02.000Z".to_string());
            Ok(())
        }

        fn get_run(&self, run_id: &str) -> Result<Option<AgentRunRecord>, AgentRepoError> {
            Ok(self
                .runs
                .lock()
                .unwrap()
                .iter()
                .find(|run| run.id == run_id)
                .cloned())
        }

        fn list_events(&self, _run_id: &str) -> Result<Vec<AgentEventRecord>, AgentRepoError> {
            Ok(Vec::new())
        }

        fn unfinished_runs(&self) -> Result<Vec<AgentRunRecord>, AgentRepoError> {
            Ok(Vec::new())
        }
    }

    /// Accepts every call: this test is about the sandbox, not the tools.
    struct NoTools;

    impl ToolHandler for NoTools {
        fn call(
            &self,
            _task: &str,
            _tool: &str,
            _args: &serde_json::Value,
        ) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({}))
        }
    }

    /// A short root: a Unix socket path is capped at ~104 bytes, and the
    /// environment's temp directory is long enough to eat most of that.
    fn scratch() -> PathBuf {
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let dir = PathBuf::from(format!(
            "/tmp/boba-rt-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::remove_dir_all(&dir).ok();
        dir
    }

    fn task() -> AgentTask {
        AgentTask {
            kind: AgentTaskKind::ExtractPostings,
            input_trust: boba_app::ports::agent::InputTrust::Untrusted,
            cli: "claude_code".to_string(),
            model: Some("sonnet".to_string()),
            prompt: "extract".to_string(),
            target_language: "en".to_string(),
            budget: RunBudget::default(),
            env: vec![],
        }
    }

    /// The run's child reports its cwd and its token; both come from the run's
    /// own sandbox, and the workspace is gone once the run ends (AC1).
    #[cfg(unix)]
    #[test]
    fn a_run_runs_inside_its_workspace_and_cleans_up_afterwards() {
        let root = scratch();
        let repo = Arc::new(FakeRepo::default());
        let runtime = AgentRuntime::new(Arc::clone(&repo))
            // This fixture is about the sandbox, not the gate: it allows the
            // untrusted task it runs.
            .with_gate(Arc::new(|_task: &AgentTask| TaskSupport::Supported))
            .with_sandbox(
                SandboxConfig {
                    runs_root: root.join("runs"),
                    app_data: root.clone(),
                    shim: PathBuf::from("/nonexistent/boba-mcp"),
                },
                Arc::new(NoTools),
            )
            .with_command_builder(Arc::new(|_task, _sandbox| CommandSpec {
                program: PathBuf::from("/bin/sh"),
                args: vec![
                    "-c".to_string(),
                    // Opened by the init event an untrusted run now requires.
                    "printf '%s\\n' '{\"type\":\"system\",\"subtype\":\"init\",\"tools\":[\"mcp__boba__get_task_input\"]}'; pwd; printf 'token=%s' \"$BOBA_RUN_TOKEN\"".to_string(),
                ],
                cwd: None,
                env: vec![],
            }));

        let handle = runtime.run(task());
        let run_id = handle.run_id.clone();

        let output: String = handle
            .events
            .iter()
            .filter_map(|event| match event {
                AgentEvent::Text { text } => Some(text),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        let workspace = root.join("runs").join(&run_id);
        assert!(
            output.contains(&workspace.display().to_string()),
            "the child did not run in {workspace:?}: {output:?}"
        );
        let token = output
            .split("token=")
            .nth(1)
            .expect("BOBA_RUN_TOKEN was not in the child's environment");
        assert_eq!(token.trim().len(), 64, "token: {token:?}");
        // The workspace is deleted as the run's thread unwinds, just after the
        // last event: wait for it rather than racing it.
        let mut left = workspace.exists();
        for _ in 0..200 {
            if !left {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
            left = workspace.exists();
        }
        assert!(!left, "the workspace outlived the run: {workspace:?}");

        std::fs::remove_dir_all(root).ok();
    }

    /// A task the gate holds back is reported unsupported, and nothing is
    /// created or spawned for it (ADR-010 §2: a hard gate, before any workspace).
    #[test]
    fn a_task_the_gate_holds_back_never_reaches_a_cli() {
        let root = scratch();
        let repo = Arc::new(FakeRepo::default());
        let runtime = AgentRuntime::new(Arc::clone(&repo))
            .with_sandbox(
                SandboxConfig {
                    runs_root: root.join("runs"),
                    app_data: root.clone(),
                    shim: PathBuf::from("/nonexistent/boba-mcp"),
                },
                Arc::new(NoTools),
            )
            .with_gate(Arc::new(|_task: &AgentTask| TaskSupport::Unsupported {
                reason: Unverified::NoVerifiedConfig,
                fallback: None,
            }))
            .with_command_builder(Arc::new(|_task, _sandbox| CommandSpec {
                program: PathBuf::from("/bin/echo"),
                args: vec!["should not run".to_string()],
                cwd: None,
                env: vec![],
            }));

        let handle = runtime.run(task());

        let reason = handle
            .events
            .iter()
            .find_map(|event| match event {
                AgentEvent::Failed { reason, .. } => Some(reason),
                _ => None,
            })
            .expect("a failed event");
        assert_eq!(reason, FailureReason::Unsupported);
        assert_eq!(
            repo.get_run(&handle.run_id)
                .unwrap()
                .unwrap()
                .failure_reason,
            Some("unsupported".to_string()),
            "the refusal is recorded, not silent"
        );
        assert!(
            !root.join("runs").exists(),
            "a refused task must not get a workspace"
        );
        std::fs::remove_dir_all(root).ok();
    }

    /// No gate mounted is not "no gate to pass": a task that reads third-party
    /// content is refused, because nothing has vouched for it.
    #[test]
    fn without_a_gate_an_untrusted_task_is_still_refused() {
        let repo = Arc::new(FakeRepo::default());
        let runtime = AgentRuntime::new(Arc::clone(&repo));

        let handle = runtime.run(task());
        assert!(
            handle.events.iter().any(|event| matches!(
                event,
                AgentEvent::Failed {
                    reason: FailureReason::Unsupported,
                    ..
                }
            )),
            "an ungated untrusted task was not refused"
        );
    }

    /// The gate lets the ordinary task through: it is not a blanket refusal.
    #[test]
    fn a_task_the_gate_allows_runs() {
        let root = scratch();
        let repo = Arc::new(FakeRepo::default());
        let runtime = AgentRuntime::new(Arc::clone(&repo))
            .with_sandbox(
                SandboxConfig {
                    runs_root: root.join("runs"),
                    app_data: root.clone(),
                    shim: PathBuf::from("/nonexistent/boba-mcp"),
                },
                Arc::new(NoTools),
            )
            .with_gate(Arc::new(|_task: &AgentTask| TaskSupport::Supported))
            .with_max_polls(200);

        let handle = runtime.run(task());
        let finished = handle
            .events
            .iter()
            .any(|event| matches!(event, AgentEvent::Started { .. }));
        assert!(finished, "an allowed task still starts");
        assert!(
            !handle.events.iter().any(|event| matches!(
                event,
                AgentEvent::Failed {
                    reason: FailureReason::Unsupported,
                    ..
                }
            )),
            "an allowed task must not be refused"
        );
        std::fs::remove_dir_all(root).ok();
    }

    /// Without a command builder the port reports a failure instead of the
    /// panic it used to carry (ADR-012).
    #[test]
    fn a_run_without_an_engine_is_reported_not_panicked() {
        let repo = Arc::new(FakeRepo::default());
        let runtime = AgentRuntime::new(Arc::clone(&repo))
            // About the missing engine, not the gate.
            .with_gate(Arc::new(|_task: &AgentTask| TaskSupport::Supported));

        let handle = runtime.run(task());
        let failed = handle
            .events
            .iter()
            .find_map(|event| match event {
                AgentEvent::Failed { reason, .. } => Some(reason),
                _ => None,
            })
            .expect("a failed event");
        assert_eq!(failed, FailureReason::NotFound);
        assert_eq!(
            repo.get_run(&handle.run_id).unwrap().unwrap().status,
            RunStatus::Failed.id()
        );
    }

    /// The pack is written into the run's workspace before the CLI starts: the
    /// system prompt plus this task's skill, in the file this CLI reads (AC1).
    #[test]
    fn the_runs_workspace_holds_the_rendered_agent_pack() {
        let root = scratch();
        let repo = Arc::new(FakeRepo::default());
        let runtime = AgentRuntime::new(Arc::clone(&repo))
            .with_gate(Arc::new(|_task: &AgentTask| TaskSupport::Supported))
            .with_sandbox(
                SandboxConfig {
                    runs_root: root.join("runs"),
                    app_data: root.clone(),
                    shim: PathBuf::from("/nonexistent/boba-mcp"),
                },
                Arc::new(NoTools),
            )
            .with_command_builder(Arc::new(|_, _sandbox| CommandSpec {
                // Report the workspace's own contents, then exit.
                program: PathBuf::from("/bin/sh"),
                args: vec![
                    "-c".to_string(),
                    "printf '%s\\n' '{\"type\":\"system\",\"subtype\":\"init\",\"tools\":[\"mcp__boba__get_task_input\"]}'; cat CLAUDE.md 2>/dev/null || echo NO-PACK".to_string(),
                ],
                cwd: None,
                env: vec![],
            }));

        let handle = runtime.run(task());
        let output: String = handle
            .events
            .iter()
            .filter_map(|event| match event {
                AgentEvent::Text { text } => Some(text),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            output.contains("Boba Job Find"),
            "the system prompt is missing: {output}"
        );
        assert!(
            output.contains("# extract_postings"),
            "the task's skill is missing: {output}"
        );
        std::fs::remove_dir_all(root).ok();
    }

    /// ADR-010 §1: an untrusted run whose own init event lists only this app's
    /// tools runs to the end, and that verification is per run — nothing is
    /// cached (orquestador ruling, 2026-10-16).
    #[test]
    fn a_locked_down_init_event_lets_an_untrusted_run_finish() {
        let root = scratch();
        let repo = Arc::new(FakeRepo::default());
        let runtime = AgentRuntime::new(Arc::clone(&repo))
            .with_gate(Arc::new(|_task: &AgentTask| TaskSupport::Supported))
            .with_sandbox(
                SandboxConfig {
                    runs_root: root.join("runs"),
                    app_data: root.clone(),
                    shim: PathBuf::from("/nonexistent/boba-mcp"),
                },
                Arc::new(NoTools),
            );

        // `start` drives a fixture command, so the CLI id only decides which
        // stream reader reads it: claude_code reads the recorded stream shape.
        let handle = runtime.start(
            task(),
            fake_run_spec(
                "claude-stream",
                "mcp__boba__get_task_input,mcp__boba__propose_posting_features",
            ),
        );
        let events: Vec<AgentEvent> = handle.events.iter().collect();

        assert!(
            matches!(events.last(), Some(AgentEvent::Finished { .. })),
            "{events:?}"
        );
        // And the run's prose is prose, not raw JSON.
        assert!(
            events.iter().any(|event| matches!(
                event,
                AgentEvent::Text { text } if text == "done"
            )),
            "{events:?}"
        );
        std::fs::remove_dir_all(root).ok();
    }

    /// A CLI that says nothing at all cannot vouch for the tool set, so the run
    /// is unverified however it exits — cleanly, or not (ruling (d), reviewer C).
    #[test]
    fn a_silent_cli_leaves_an_untrusted_run_unverified() {
        for mode in ["silent", "silent-fail"] {
            let root = scratch();
            let repo = Arc::new(FakeRepo::default());
            let runtime = AgentRuntime::new(Arc::clone(&repo))
                .with_gate(Arc::new(|_task: &AgentTask| TaskSupport::Supported))
                .with_sandbox(
                    SandboxConfig {
                        runs_root: root.join("runs"),
                        app_data: root.clone(),
                        shim: PathBuf::from("/nonexistent/boba-mcp"),
                    },
                    Arc::new(NoTools),
                );

            let handle = runtime.start(
                task(),
                RunSpec {
                    spec: CommandSpec {
                        program: fake_cli_path(),
                        args: vec![],
                        cwd: None,
                        env: vec![("BOBA_FAKE_MODE".to_string(), mode.to_string())],
                    },
                },
            );
            let run_id = handle.run_id.clone();
            let events: Vec<AgentEvent> = handle.events.iter().collect();

            assert!(
                matches!(
                    events.last(),
                    Some(AgentEvent::Failed {
                        reason: FailureReason::Unsupported,
                        ..
                    })
                ),
                "{mode}: {events:?}"
            );
            assert!(
                !events
                    .iter()
                    .any(|event| matches!(event, AgentEvent::Finished { .. })),
                "{mode}: an unverified run must not report success: {events:?}"
            );
            let run = repo.get_run(&run_id).unwrap().expect("run");
            assert_eq!(run.status, RunStatus::Failed.id(), "{mode}");
            assert_eq!(run.failure_reason.as_deref(), Some("unsupported"), "{mode}");
            std::fs::remove_dir_all(root).ok();
        }
    }

    /// A stream whose first work arrives before any init event cannot vouch for
    /// the tool set either (a CLI that lost `--output-format stream-json`, say):
    /// the untrusted run stops at the first line of its own work.
    #[test]
    fn an_untrusted_run_whose_stream_never_opens_with_its_init_event_stops() {
        let root = scratch();
        let repo = Arc::new(FakeRepo::default());
        let runtime = AgentRuntime::new(Arc::clone(&repo))
            .with_gate(Arc::new(|_task: &AgentTask| TaskSupport::Supported))
            .with_sandbox(
                SandboxConfig {
                    runs_root: root.join("runs"),
                    app_data: root.clone(),
                    shim: PathBuf::from("/nonexistent/boba-mcp"),
                },
                Arc::new(NoTools),
            );

        let handle = runtime.start(
            task(),
            RunSpec {
                spec: CommandSpec {
                    program: fake_cli_path(),
                    args: vec![],
                    cwd: None,
                    env: vec![("BOBA_FAKE_MODE".to_string(), "echo".to_string())],
                },
            },
        );
        let events: Vec<AgentEvent> = handle.events.iter().collect();

        assert!(
            matches!(
                events.last(),
                Some(AgentEvent::Failed {
                    reason: FailureReason::Unsupported,
                    ..
                })
            ),
            "{events:?}"
        );
        assert!(
            !events.iter().any(|event| matches!(
                event,
                AgentEvent::ToolCall { .. } | AgentEvent::ToolResult { .. }
            )),
            "the run did work before it was verified: {events:?}"
        );
        std::fs::remove_dir_all(root).ok();
    }

    /// An unverified run reports itself unverified however it ended — except
    /// when it failed for a reason that pauses AI work (FR-21), where that
    /// signal is the actionable one.
    #[test]
    fn an_unverified_run_reports_itself_unverified_whatever_the_exit() {
        let root = scratch();
        let repo = Arc::new(FakeRepo::default());
        let runtime = AgentRuntime::new(Arc::clone(&repo))
            .with_gate(Arc::new(|_task: &AgentTask| TaskSupport::Supported))
            .with_sandbox(
                SandboxConfig {
                    runs_root: root.join("runs"),
                    app_data: root.clone(),
                    shim: PathBuf::from("/nonexistent/boba-mcp"),
                },
                Arc::new(NoTools),
            );

        let handle = runtime.start(
            task(),
            RunSpec {
                spec: CommandSpec {
                    program: fake_cli_path(),
                    args: vec![],
                    cwd: None,
                    env: vec![("BOBA_FAKE_MODE".to_string(), "fail-auth".to_string())],
                },
            },
        );
        let events: Vec<AgentEvent> = handle.events.iter().collect();

        // Auth expires before any init event can arrive, and the user needs to
        // read that, not "unsupported CLI".
        assert!(
            matches!(
                events.last(),
                Some(AgentEvent::Failed {
                    reason: FailureReason::AuthExpired,
                    ..
                })
            ),
            "{events:?}"
        );
        assert!(
            FailureReason::AuthExpired.pauses_ai(),
            "the carve-out only makes sense for a pausing reason"
        );
        std::fs::remove_dir_all(root).ok();
    }

    /// A run that reads third-party content must not continue when the CLI
    /// reports a native tool: the tool set is the whole basis of ADR-010's
    /// envelope, so the run fails visibly instead.
    #[test]
    fn an_init_event_with_a_native_tool_aborts_an_untrusted_run() {
        let root = scratch();
        let repo = Arc::new(FakeRepo::default());
        let runtime = AgentRuntime::new(Arc::clone(&repo))
            .with_gate(Arc::new(|_task: &AgentTask| TaskSupport::Supported))
            .with_sandbox(
                SandboxConfig {
                    runs_root: root.join("runs"),
                    app_data: root.clone(),
                    shim: PathBuf::from("/nonexistent/boba-mcp"),
                },
                Arc::new(NoTools),
            );

        let handle = runtime.start(
            task(),
            fake_run_spec("claude-stream", "mcp__boba__get_task_input,Bash"),
        );
        let events: Vec<AgentEvent> = handle.events.iter().collect();

        assert!(
            matches!(
                events.last(),
                Some(AgentEvent::Failed {
                    reason: FailureReason::Unsupported,
                    ..
                })
            ),
            "{events:?}"
        );
        std::fs::remove_dir_all(root).ok();
    }

    /// The same init event on a task that only reads the user's own input is the
    /// security badge, not a refusal (ADR-010 §2): it still runs, because the
    /// envelope is what protects third-party content and this run has none.
    #[test]
    fn a_native_tool_does_not_abort_a_user_input_run() {
        let root = scratch();
        let repo = Arc::new(FakeRepo::default());
        let runtime = AgentRuntime::new(Arc::clone(&repo))
            .with_gate(Arc::new(|_task: &AgentTask| TaskSupport::Supported))
            .with_sandbox(
                SandboxConfig {
                    runs_root: root.join("runs"),
                    app_data: root.clone(),
                    shim: PathBuf::from("/nonexistent/boba-mcp"),
                },
                Arc::new(NoTools),
            );

        let mut user_task = task();
        user_task.kind = AgentTaskKind::ProfileImport;
        user_task.input_trust = boba_app::ports::agent::InputTrust::User;

        let handle = runtime.start(
            user_task,
            fake_run_spec("claude-stream", "mcp__boba__get_task_input,Bash"),
        );
        let events: Vec<AgentEvent> = handle.events.iter().collect();
        assert!(
            matches!(events.last(), Some(AgentEvent::Finished { .. })),
            "{events:?}"
        );
        std::fs::remove_dir_all(root).ok();
    }

    /// The fake CLI replaying a Claude stream, with its init event's tool list.
    fn fake_run_spec(mode: &str, tools: &str) -> RunSpec {
        let env = vec![
            ("BOBA_FAKE_MODE".to_string(), mode.to_string()),
            ("BOBA_FAKE_TOOLS".to_string(), tools.to_string()),
        ];
        RunSpec {
            spec: CommandSpec {
                program: fake_cli_path(),
                args: vec![],
                cwd: None,
                env,
            },
        }
    }

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
