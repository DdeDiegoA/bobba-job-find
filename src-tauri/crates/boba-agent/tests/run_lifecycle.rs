//! S2.2 run-lifecycle integration tests: the fake CLI replays recorded JSONL,
//! the runtime persists/emits every event, limits and cancellation kill the
//! process tree, and failures are classified so AI work can pause (FR-16/21).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use boba_agent::lockdown::TaskSupport;
use boba_agent::mcp::config::{RunSandbox, SandboxConfig};
use boba_agent::mcp::server::ToolHandler;
use boba_agent::runtime::{AgentRuntime, RunSpec, classify_failure};
use boba_agent::supervisor::CommandSpec;
use boba_app::ports::agent::{
    AgentEvent, AgentPort, AgentTask, AgentTaskKind, CancelToken, FailureReason, InputTrust,
    RunBudget,
};
use boba_app::ports::repo::agent::{
    AgentEventRecord, AgentRepo, AgentRepoError, AgentRunRecord, FinishAgentRun, NewAgentRun,
    RunStatus,
};

/// The fake CLI, built on demand so a cold CI cache still works: `cargo test`
/// builds only the harness in `target/debug/deps`, never the sibling bin.
fn fake_cli() -> PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        if let Ok(path) = std::env::var("BOBA_FAKE_CLI") {
            return PathBuf::from(path);
        }
        let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .find(|dir| dir.join("Cargo.toml").is_file() && dir.join("tools/fake-cli").is_dir())
            .expect("workspace root");
        let status = Command::new(env!("CARGO"))
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
        workspace_root.join("target/debug").join(name)
    })
    .clone()
}

fn spec(env: &[(&str, &str)]) -> RunSpec {
    RunSpec {
        spec: CommandSpec {
            program: fake_cli(),
            args: vec![],
            cwd: None,
            env: env
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
        },
    }
}

/// This suite drives the runtime and the supervisor, not ADR-010's gate, so it
/// mounts one that allows the task it runs. Without a gate a task that reads
/// third-party content is refused (S2.3b), which is not what these tests are
/// about.
fn runtime(repo: Arc<FakeRepo>) -> AgentRuntime<FakeRepo> {
    AgentRuntime::new(repo).with_gate(Arc::new(|_task: &AgentTask| TaskSupport::Supported))
}

fn task(budget: RunBudget) -> AgentTask {
    AgentTask {
        kind: AgentTaskKind::ExtractPostings,
        input_trust: InputTrust::Untrusted,
        cli: "claude_code".to_string(),
        model: Some("sonnet".to_string()),
        prompt: "extract".to_string(),
        target_language: "en".to_string(),
        budget,
        env: vec![],
    }
}

// --- In-memory AgentRepo, so tests stay in-process and deterministic. ---

#[derive(Default)]
struct FakeRepo {
    runs: Mutex<Vec<AgentRunRecord>>,
    events: Mutex<Vec<(String, AgentEventRecord)>>,
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
        run_id: &str,
        seq: i64,
        kind: &str,
        payload_json: &str,
    ) -> Result<(), AgentRepoError> {
        self.events.lock().unwrap().push((
            run_id.to_string(),
            AgentEventRecord {
                seq,
                kind: kind.to_string(),
                payload_json: payload_json.to_string(),
                at: "2026-10-05T00:00:01.000Z".to_string(),
            },
        ));
        Ok(())
    }

    fn finish_run(&self, run_id: &str, finish: FinishAgentRun) -> Result<(), AgentRepoError> {
        let mut runs = self.runs.lock().unwrap();
        let run = runs.iter_mut().find(|r| r.id == run_id).expect("run");
        run.status = finish.status.id().to_string();
        run.failure_reason = finish.failure_reason;
        run.tokens_in = finish.tokens_in;
        run.tokens_out = finish.tokens_out;
        run.cost_usd = finish.cost_usd;
        run.ended_at = Some("2026-10-05T00:00:02.000Z".to_string());
        Ok(())
    }

    fn get_run(&self, run_id: &str) -> Result<Option<AgentRunRecord>, AgentRepoError> {
        Ok(self
            .runs
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.id == run_id)
            .cloned())
    }

    fn list_events(&self, run_id: &str) -> Result<Vec<AgentEventRecord>, AgentRepoError> {
        Ok(self
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|(id, _)| id == run_id)
            .map(|(_, e)| e.clone())
            .collect())
    }

    fn unfinished_runs(&self) -> Result<Vec<AgentRunRecord>, AgentRepoError> {
        Ok(self
            .runs
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.status == "running")
            .cloned()
            .collect())
    }
}

fn drain(handle: boba_app::ports::agent::RunHandle) -> (String, Vec<AgentEvent>) {
    let run_id = handle.run_id.clone();
    let events: Vec<AgentEvent> = handle.events.iter().collect();
    (run_id, events)
}

// AC1: a run persists and emits every event, attributed to its CLI and model.
#[test]
fn run_persists_and_emits_events_attributed_to_cli_and_model() {
    let repo = Arc::new(FakeRepo::default());
    let emitted = Arc::new(Mutex::new(Vec::<(String, i64, AgentEvent)>::new()));
    let sink = Arc::clone(&emitted);
    let runtime = runtime(Arc::clone(&repo)).with_emitter(Arc::new(
        move |run_id: &str, seq: i64, event: &AgentEvent| {
            sink.lock()
                .unwrap()
                .push((run_id.to_string(), seq, event.clone()));
        },
    ));

    let handle = runtime.start(
        task(RunBudget::default()),
        spec(&[("BOBA_FAKE_MODE", "replay")]),
    );
    let (run_id, events) = drain(handle);

    // Every variant the fake replays is present, in order, ending `Finished`.
    assert!(matches!(events[0], AgentEvent::Started { .. }));
    assert!(events.iter().any(|e| matches!(e, AgentEvent::Text { .. })));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AgentEvent::ToolCall { .. }))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AgentEvent::ToolResult { .. }))
    );
    assert!(events.iter().any(|e| matches!(e, AgentEvent::Usage { .. })));
    assert!(matches!(events.last(), Some(AgentEvent::Finished { .. })));

    // Persisted: run is finished, attributed, with usage summed.
    let run = repo.get_run(&run_id).unwrap().expect("run");
    assert_eq!(run.status, "finished");
    assert_eq!(run.cli, "claude_code");
    assert_eq!(run.model.as_deref(), Some("sonnet"));
    assert_eq!(run.tokens_in, Some(120));
    assert_eq!(run.tokens_out, Some(40));

    // Persisted events mirror the emitted stream 1:1.
    let stored = repo.list_events(&run_id).unwrap();
    assert_eq!(stored.len(), events.len());
    assert_eq!(stored[0].kind, "started");
    assert!(emitted.lock().unwrap().len() == events.len());
    assert!(
        emitted
            .lock()
            .unwrap()
            .iter()
            .all(|(id, _, _)| id == &run_id)
    );
}

// AC2: a wall-clock timeout kills the whole tree and fails `Timeout`.
#[test]
fn timeout_kills_the_process_tree() {
    let repo = Arc::new(FakeRepo::default());
    let runtime = runtime(Arc::clone(&repo));
    let budget = RunBudget {
        wall_clock: Some(Duration::from_millis(200)),
        ..RunBudget::default()
    };
    let handle = runtime.start(
        task(budget),
        spec(&[
            ("BOBA_FAKE_MODE", "spawn-child"),
            ("BOBA_FAKE_DELAY_MS", "5000"),
        ]),
    );
    let (run_id, events) = drain(handle);

    assert!(matches!(
        events.last(),
        Some(AgentEvent::Failed {
            reason: FailureReason::Timeout,
            ..
        })
    ));
    let run = repo.get_run(&run_id).unwrap().expect("run");
    assert_eq!(run.status, "failed");
    assert_eq!(run.failure_reason.as_deref(), Some("timeout"));
}

// AC2: cancellation kills the tree and records `cancelled`.
#[test]
fn cancellation_kills_and_records_cancelled() {
    let repo = Arc::new(FakeRepo::default());
    let runtime = runtime(Arc::clone(&repo));
    let handle = runtime.start(
        task(RunBudget::default()),
        spec(&[("BOBA_FAKE_MODE", "spawn-child")]),
    );
    let cancel: CancelToken = handle.cancel.clone();
    // Cancel the moment the run says it started — no sleep, no guess (CH12).
    let run_id = handle.run_id.clone();
    let events: Vec<AgentEvent> = handle
        .events
        .iter()
        .inspect(|event| {
            if matches!(event, AgentEvent::Started { .. }) {
                cancel.cancel();
            }
        })
        .collect();

    assert!(matches!(
        events.last(),
        Some(AgentEvent::Failed {
            reason: FailureReason::Cancelled,
            ..
        })
    ));
    let run = repo.get_run(&run_id).unwrap().expect("run");
    assert_eq!(run.status, "cancelled");
}

// AC2: the max-tool-call limit fails `LimitExceeded` (never a silent truncation).
#[test]
fn tool_call_limit_fails_with_limit_exceeded() {
    let repo = Arc::new(FakeRepo::default());
    let runtime = runtime(Arc::clone(&repo));
    let lines = [
        // A Claude `stream-json` run opens with its init event (ADR-010 §1).
        r#"{"type":"system","subtype":"init","tools":["mcp__boba__get_task_input"]}"#,
        r#"{"type":"tool_call","name":"mcp__boba__get_posting","input":{}}"#,
        r#"{"type":"tool_call","name":"mcp__boba__get_posting","input":{}}"#,
        r#"{"type":"tool_call","name":"mcp__boba__get_posting","input":{}}"#,
        r#"{"type":"finished","exit_code":0}"#,
    ]
    .join("\n");
    let budget = RunBudget {
        max_tool_calls: Some(2),
        ..RunBudget::default()
    };
    let handle = runtime.start(
        task(budget),
        spec(&[("BOBA_FAKE_MODE", "replay"), ("BOBA_FAKE_LINES", &lines)]),
    );
    let (run_id, events) = drain(handle);

    assert!(matches!(
        events.last(),
        Some(AgentEvent::Failed {
            reason: FailureReason::LimitExceeded,
            ..
        })
    ));
    assert_eq!(
        repo.get_run(&run_id).unwrap().unwrap().status,
        "failed",
        "limit hit is a failure, not a success"
    );
}

// AC3: an auth error is classified and flagged to pause AI (non-AI continues).
#[test]
fn auth_error_classifies_and_pauses_ai() {
    let repo = Arc::new(FakeRepo::default());
    let runtime = runtime(Arc::clone(&repo));
    let handle = runtime.start(
        task(RunBudget::default()),
        spec(&[("BOBA_FAKE_MODE", "fail-auth")]),
    );
    let (run_id, events) = drain(handle);

    let last = events.last().expect("terminal event");
    match last {
        AgentEvent::Failed { reason, .. } => {
            assert_eq!(*reason, FailureReason::AuthExpired);
            assert!(reason.pauses_ai(), "auth failure must pause AI work");
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    let run = repo.get_run(&run_id).unwrap().expect("run");
    assert_eq!(run.failure_reason.as_deref(), Some("auth_expired"));
}

// AC3 (S2.2 regression): the run must not classify until the stderr pipe is
// closed. `reap()` used to leave the stderr reader racing the classification,
// so a stderr that closes late (a grandchild still holding it) made an
// auth-expired run come back `Unsupported` instead of `AuthExpired`. The hold
// is well under the supervisor's reader join grace, so the reader still
// finishes.
#[cfg(unix)]
#[test]
fn auth_error_is_classified_when_stderr_closes_late() {
    let repo = Arc::new(FakeRepo::default());
    let runtime = runtime(Arc::clone(&repo));
    let handle = runtime.start(
        task(RunBudget::default()),
        spec(&[
            ("BOBA_FAKE_MODE", "fail-auth"),
            ("BOBA_FAKE_STDERR_HOLD_MS", "150"),
        ]),
    );
    let (run_id, events) = drain(handle);

    match events.last() {
        Some(AgentEvent::Failed { reason, .. }) => {
            assert_eq!(
                *reason,
                FailureReason::AuthExpired,
                "a late stderr close must not downgrade auth to Unsupported"
            );
        }
        other => panic!("expected Failed(AuthExpired), got {other:?}"),
    }
    let run = repo.get_run(&run_id).unwrap().expect("run");
    assert_eq!(run.failure_reason.as_deref(), Some("auth_expired"));
}

// AC3: classification covers quota and not-found as well.
#[test]
fn classify_failure_maps_quota_and_not_found() {
    assert_eq!(
        classify_failure("429 too many requests", 1),
        FailureReason::QuotaExceeded
    );
    assert!(FailureReason::QuotaExceeded.pauses_ai());
    assert_eq!(
        classify_failure("model not found", 1),
        FailureReason::NotFound
    );
    assert!(!FailureReason::NotFound.pauses_ai());
}

// --- FX10a: an improve_fact run proves its lockdown before it proposes. ---

/// The improve task: agent text that rewrites one existing fact. Its input is
/// the user's own fact, but the run still takes the lockdown gate (ADR-010).
fn improve_task(budget: RunBudget) -> AgentTask {
    AgentTask {
        kind: AgentTaskKind::ImproveFact,
        input_trust: InputTrust::Untrusted,
        cli: "claude_code".to_string(),
        model: Some("sonnet".to_string()),
        prompt: r#"{"item_id":"01ITEM","revision_id":"01REV1"}"#.to_string(),
        target_language: "en".to_string(),
        budget,
        env: vec![],
    }
}

/// AC1: an init event that opens a tool outside `mcp__boba__*` aborts the run
/// before its proposal is ever accepted — the run fails closed.
#[test]
fn an_improve_run_aborts_when_its_init_event_opens_other_tools() {
    let repo = Arc::new(FakeRepo::default());
    let runtime = runtime(Arc::clone(&repo));
    let lines = [
        r#"{"type":"system","subtype":"init","tools":["mcp__boba__get_task_input","Bash"]}"#,
        r#"{"type":"tool_call","name":"mcp__boba__propose_fact_improvements","input":{"improvements":[]}}"#,
        r#"{"type":"finished","exit_code":0}"#,
    ]
    .join("\n");
    let handle = runtime.start(
        improve_task(RunBudget::default()),
        spec(&[("BOBA_FAKE_MODE", "replay"), ("BOBA_FAKE_LINES", &lines)]),
    );
    let (run_id, events) = drain(handle);

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
    assert_eq!(
        repo.get_run(&run_id)
            .unwrap()
            .expect("run")
            .failure_reason
            .as_deref(),
        Some("unsupported")
    );
}

/// The same run with only our shim's tools opens and finishes: the gate is not
/// a blanket refusal of the improve task.
#[test]
fn an_improve_run_whose_init_event_lists_only_boba_tools_finishes() {
    let repo = Arc::new(FakeRepo::default());
    let runtime = runtime(Arc::clone(&repo));
    let lines = [
        r#"{"type":"system","subtype":"init","tools":["mcp__boba__get_task_input","mcp__boba__propose_fact_improvements"]}"#,
        r#"{"type":"tool_call","name":"mcp__boba__propose_fact_improvements","input":{"improvements":[]}}"#,
        r#"{"type":"finished","exit_code":0}"#,
    ]
    .join("\n");
    let handle = runtime.start(
        improve_task(RunBudget::default()),
        spec(&[("BOBA_FAKE_MODE", "replay"), ("BOBA_FAKE_LINES", &lines)]),
    );
    let (run_id, events) = drain(handle);

    assert!(
        matches!(events.last(), Some(AgentEvent::Finished { .. })),
        "{events:?}"
    );
    assert_eq!(
        repo.get_run(&run_id).unwrap().expect("run").status,
        "finished"
    );
}

// --- CH8: the run's own MCP endpoint, driven end to end by the fake CLI. ---

/// Accepts every call: this test is about the path, not the tool data.
struct NoTools;

impl ToolHandler for NoTools {
    fn call(
        &self,
        _task: &str,
        _application_id: Option<&str>,
        _tool: &str,
        _args: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({}))
    }
}

/// A short root: the run's Unix socket lives here, and a `sockaddr_un` path is
/// capped (S2.2 uses the same trick).
fn scratch(tag: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = PathBuf::from(format!("/tmp/boba-e2e-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// The text the run reported, joined, for a readable assertion message.
fn text_of(events: &[AgentEvent]) -> String {
    events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// CH8 end-to-end through `AgentPort::run` and the real MCP endpoint: the
/// runtime furnishes the run's workspace, the endpoint authenticates the CLI and
/// serves the run's own input, an untrusted run passes its init-event check, and
/// a write is validated against the catalog's schema before it could be stored.
#[test]
fn a_run_drives_its_own_mcp_endpoint_end_to_end() {
    let root = scratch("mcp");
    let repo = Arc::new(FakeRepo::default());
    let calls = serde_json::json!([
        {"name": "get_task_input"},
        {"name": "propose_posting_features", "arguments": {"features": {}}},
    ])
    .to_string();
    let runtime = AgentRuntime::new(Arc::clone(&repo))
        // The gate allows the untrusted task here; the run's own init event is
        // still what the runtime checks (ADR-010 §1).
        .with_gate(Arc::new(|_task: &AgentTask| TaskSupport::Supported))
        .with_sandbox(
            SandboxConfig {
                runs_root: root.join("runs"),
                app_data: root.clone(),
                shim: root.join("boba-mcp"),
            },
            Arc::new(NoTools),
        )
        .with_command_builder(Arc::new(move |_task: &AgentTask, _sandbox: &RunSandbox| {
            CommandSpec {
                program: fake_cli(),
                args: vec![],
                cwd: None,
                env: vec![
                    ("BOBA_FAKE_MODE".to_string(), "mcp-run".to_string()),
                    (
                        "BOBA_FAKE_TOOLS".to_string(),
                        "mcp__boba__get_task_input".to_string(),
                    ),
                    ("BOBA_FAKE_CALLS".to_string(), calls.clone()),
                    // A clean run: no limit to fire, so it exits by itself.
                    ("BOBA_FAKE_HOLD_MS".to_string(), "0".to_string()),
                ],
            }
        }));

    let mut run_task = task(RunBudget::default());
    run_task.budget.wall_clock = Some(Duration::from_secs(15));
    let handle = runtime.run(run_task);
    let (run_id, events) = drain(handle);

    assert!(
        matches!(events.last(), Some(AgentEvent::Finished { .. })),
        "{events:?}"
    );
    assert_eq!(
        repo.get_run(&run_id).unwrap().expect("run").status,
        "finished"
    );

    let text = text_of(&events);
    // Furnished workspace: the agent pack and the run's own MCP config.
    assert!(
        text.contains("pack=true"),
        "the pack was not furnished: {text}"
    );
    assert!(
        text.contains("mcp=true"),
        "the run config was not written: {text}"
    );
    // The endpoint authenticated this run and served its own input.
    assert!(text.contains("auth -> ok"), "{text}");
    assert!(text.contains("get_task_input -> ok"), "{text}");
    // The write was validated: the schema refused it (never stored).
    assert!(
        text.contains("propose_posting_features -> -32602"),
        "an invalid output was not refused: {text}"
    );

    std::fs::remove_dir_all(root).ok();
}
