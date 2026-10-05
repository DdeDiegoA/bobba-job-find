//! S2.2 run-lifecycle integration tests: the fake CLI replays recorded JSONL,
//! the runtime persists/emits every event, limits and cancellation kill the
//! process tree, and failures are classified so AI work can pause (FR-16/21).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use boba_agent::runtime::{AgentRuntime, RunSpec, classify_failure};
use boba_agent::supervisor::CommandSpec;
use boba_app::ports::agent::{
    AgentEvent, AgentTask, AgentTaskKind, CancelToken, FailureReason, InputTrust, RunBudget,
};
use boba_app::ports::repo::agent::{
    AgentEventRecord, AgentRepo, AgentRepoError, AgentRunRecord, FinishAgentRun, NewAgentRun,
    RunStatus,
};

/// The fake CLI built by `cargo test` in the workspace.
fn fake_cli() -> PathBuf {
    if let Ok(path) = std::env::var("BOBA_FAKE_CLI") {
        return PathBuf::from(path);
    }
    let exe = std::env::current_exe().expect("current exe");
    let dir = exe.parent().and_then(Path::parent).expect("target dir");
    let name = if cfg!(windows) {
        "boba-fake-cli.exe"
    } else {
        "boba-fake-cli"
    };
    let candidate = dir.join(name);
    assert!(
        candidate.exists(),
        "build the workspace first: cargo build -p boba-fake-cli ({})",
        candidate.display()
    );
    candidate
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

fn task(budget: RunBudget) -> AgentTask {
    AgentTask {
        kind: AgentTaskKind::ExtractPostings,
        input_trust: InputTrust::Untrusted,
        cli: "claude_code".to_string(),
        model: Some("sonnet".to_string()),
        prompt: "extract".to_string(),
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
    let runtime = AgentRuntime::new(Arc::clone(&repo)).with_emitter(Arc::new(
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
    let runtime = AgentRuntime::new(Arc::clone(&repo));
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
    let runtime = AgentRuntime::new(Arc::clone(&repo));
    let handle = runtime.start(
        task(RunBudget::default()),
        spec(&[("BOBA_FAKE_MODE", "spawn-child")]),
    );
    let cancel: CancelToken = handle.cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        cancel.cancel();
    });
    let (run_id, events) = drain(handle);

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
    let runtime = AgentRuntime::new(Arc::clone(&repo));
    let lines = [
        r#"{"type":"tool_call","name":"get_posting","input":{}}"#,
        r#"{"type":"tool_call","name":"get_posting","input":{}}"#,
        r#"{"type":"tool_call","name":"get_posting","input":{}}"#,
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
    let runtime = AgentRuntime::new(Arc::clone(&repo));
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
