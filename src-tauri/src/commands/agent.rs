//! Agent engine commands (S3.2, FR-2/FR-3/FR-17, NFR-3): what this machine
//! has, which engine the user picked, and the round-trip test call.
//!
//! The detected CLI list, its health and the tasks a CLI can run all come from
//! `boba-agent`'s probe and ADR-010's lockdown verdict — never from a list kept
//! here, so a CLI the app cannot verify is reported unsupported instead of
//! being assumed safe. The choice is one `setting` row (S1.7) and is the same
//! row Settings → AI engine edits later (FR-17), so changing the engine never
//! touches any other data.
//!
//! The test call runs the smallest real agent task (the Boba Agent's pack, the
//! run's sandbox, the user's chosen CLI and model) and reports what came back:
//! latency and the model the run started with, or the CLI's own output and the
//! failure reason. It declares user input, so ADR-010's gate lets it run on any
//! CLI: it is a connection test, not a content-reading task.

use std::sync::Arc;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use boba_agent::detect::CliStatus;
use boba_agent::drivers::{CliKind, HealthStatus};
use boba_agent::lockdown;
use boba_app::error::{AppError, AppResult, ErrorCode};
use boba_app::ports::agent::{
    AgentEvent, AgentPort, AgentTask, AgentTaskKind, FailureReason, InputTrust, RunBudget,
};
use boba_app::ports::clock::ClockPort;
use boba_app::ports::repo::system::{SystemRepo, SystemRepoError};
use boba_app::system::SystemClock;
use serde::{Deserialize, Serialize};

use crate::wiring;
use crate::wiring::agent::EngineProbe;

/// The one `setting` row that holds the chosen engine (FR-17).
pub const ENGINE_KEY: &str = "ai_engine";

/// How long the test call waits for the run to end. The run's own wall-clock
/// budget is a little shorter, so the CLI's process tree is killed by the
/// runtime before this deadline fires.
const TEST_DEADLINE: std::time::Duration = std::time::Duration::from_secs(120);
const RUN_WALL_CLOCK: std::time::Duration = std::time::Duration::from_secs(110);

/// The engine the user picked, as stored in [`ENGINE_KEY`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct EngineChoice {
    /// A `CliKind` id, e.g. `claude_code`.
    pub cli: String,
    /// The model the user typed, or the CLI's own default when absent.
    pub model: Option<String>,
}

/// One agent CLI the probe looked at (FR-2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct EngineCard {
    /// A `CliKind` id; the value the user picks.
    pub cli: String,
    /// Human-readable name.
    pub name: String,
    pub installed: bool,
    /// The version the `--version` probe reported (empty when not installed).
    pub version: String,
    /// The binary the probe found (absent when not installed).
    pub path: Option<String>,
    /// `healthy` | `auth_required` | `unhealthy`.
    pub health: String,
    /// Where to get the CLI when it is missing (FR-2: "Not installed" + link).
    pub install_url: String,
    /// Task ids ADR-010's gate refuses on this CLI. Empty means every task can
    /// run (the engine step says which tasks stay unavailable when it is not).
    pub unsupported_tasks: Vec<String>,
}

/// What the engine step and Settings show: every known CLI in registry order,
/// and the choice already stored (if any).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct EngineSnapshot {
    pub engines: Vec<EngineCard>,
    pub chosen: Option<EngineChoice>,
}

/// The result of the round-trip test call (FR-3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct TestCallResult {
    pub ok: bool,
    /// The CLI and model the run started with, as the run reported them.
    pub cli: String,
    pub model: Option<String>,
    /// Round-trip time, measured from dispatch to the run's last event. A
    /// millisecond count stays far below 2^53, so it crosses IPC as a JS number
    /// (specta forbids BigInt-style types).
    #[specta(type = specta_typescript::Number)]
    pub latency_ms: u64,
    /// The CLI's own text output (trimmed and capped). On failure this is where
    /// the CLI's error message lives.
    pub reply: String,
    /// `FailureReason::id` when the run failed.
    pub failure_reason: Option<String>,
    /// The i18n key the step shows for that failure (AD-3).
    pub message_key: Option<String>,
}

/// An engine use-case failure.
#[derive(Debug, Clone, thiserror::Error)]
pub enum AgentError {
    #[error(transparent)]
    Repo(#[from] SystemRepoError),
    /// The id names no CLI this build knows how to drive.
    #[error("unknown agent CLI: {0}")]
    UnknownCli(String),
    /// The detection probe did not come back (it panicked or was cancelled).
    #[error("the CLI probe did not finish")]
    Probe,
}

impl AgentError {
    /// The i18n key the UI shows (AD-3).
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::Repo(_) => "errors.agent.store",
            Self::UnknownCli(_) => "errors.agent.unknown_cli",
            Self::Probe => "errors.agent.probe",
        }
    }
}

/// Map a use-case failure onto the typed IPC error (AD-3).
fn to_app(error: AgentError) -> AppError {
    let code = match error {
        AgentError::Repo(_) | AgentError::Probe => ErrorCode::Internal,
        AgentError::UnknownCli(_) => ErrorCode::InvalidInput,
    };
    AppError::new(code, error.message_key())
}

/// The `CliKind` an id names, or nothing when this build cannot drive it.
fn kind_of(id: &str) -> Option<CliKind> {
    CliKind::ALL.into_iter().find(|kind| kind.id() == id)
}

/// The stored choice, or `None` when nothing is stored yet (or the row is one
/// this build cannot read: a corrupt row never traps the user in the step).
fn read_choice(repo: &dyn SystemRepo) -> Result<Option<EngineChoice>, AgentError> {
    let Some(raw) = repo.setting_get(ENGINE_KEY)? else {
        return Ok(None);
    };
    Ok(parse_choice(&raw))
}

/// A stored row: unknown fields are ignored; an unknown CLI is no choice.
fn parse_choice(raw: &str) -> Option<EngineChoice> {
    let choice: EngineChoice = serde_json::from_str(raw).ok()?;
    kind_of(&choice.cli).map(|_| EngineChoice {
        cli: choice.cli,
        model: choice
            .model
            .map(|model| model.trim().to_string())
            .filter(|model| !model.is_empty()),
    })
}

/// Store the choice; every later task reads this row (FR-17).
fn save_choice(
    repo: &dyn SystemRepo,
    clock: &dyn ClockPort,
    choice: EngineChoice,
) -> Result<(), AgentError> {
    let json = serde_json::to_string(&choice)
        .map_err(|error| AgentError::Repo(SystemRepoError::new(error.to_string())))?;
    repo.setting_put(ENGINE_KEY, &json, &clock.now_rfc3339())?;
    Ok(())
}

/// Validate what the step or Settings sent: a known CLI, a trimmed model.
fn make_choice(cli: &str, model: Option<String>) -> Result<EngineChoice, AgentError> {
    let cli = cli.trim();
    if kind_of(cli).is_none() {
        return Err(AgentError::UnknownCli(cli.to_string()));
    }
    Ok(EngineChoice {
        cli: cli.to_string(),
        model: model
            .map(|model| model.trim().to_string())
            .filter(|model| !model.is_empty()),
    })
}

/// The health id the UI badges a CLI with (FR-2).
fn health_id(health: &HealthStatus) -> &'static str {
    match health {
        HealthStatus::Healthy => "healthy",
        HealthStatus::AuthRequired => "auth_required",
        HealthStatus::Unhealthy(_) => "unhealthy",
    }
}

/// One probe result as the step shows it, with ADR-010's verdict folded in: a
/// task the gate would refuse is listed, so AC2 can say what will not run.
fn card(status: CliStatus) -> EngineCard {
    match status {
        CliStatus::NotInstalled { kind } => EngineCard {
            cli: kind.id().to_string(),
            name: kind.display_name().to_string(),
            installed: false,
            version: String::new(),
            path: None,
            // Nothing ran, so there is no health to report; the step shows the
            // "Not installed" state and its install link instead (FR-2).
            health: "unhealthy".to_string(),
            install_url: kind.install_url().to_string(),
            unsupported_tasks: Vec::new(),
        },
        CliStatus::Installed(install) => {
            let kind = install.kind;
            let unsupported_tasks = AgentTaskKind::ALL
                .into_iter()
                .filter(|task| {
                    !lockdown::support(*task, kind, &install.version, &install.capabilities)
                        .is_supported()
                })
                .map(|task| task.id().to_string())
                .collect();
            EngineCard {
                cli: kind.id().to_string(),
                name: kind.display_name().to_string(),
                installed: true,
                version: install.version,
                path: Some(install.path.display().to_string()),
                health: health_id(&install.health).to_string(),
                install_url: kind.install_url().to_string(),
                unsupported_tasks,
            }
        }
    }
}

/// The cards for a probe, in the order the probe returned them.
fn cards(statuses: Vec<CliStatus>) -> Vec<EngineCard> {
    statuses.into_iter().map(card).collect()
}

/// Every probed CLI in registry order, plus the stored choice.
fn snapshot(statuses: Vec<CliStatus>, chosen: Option<EngineChoice>) -> EngineSnapshot {
    EngineSnapshot {
        engines: cards(statuses),
        chosen,
    }
}

/// The test call's task: one user-trust ping on the picked engine (FR-3). It
/// declares user input because Boba wrote the prompt, so no content gate applies
/// (ADR-010) while the run still goes through the full sandbox and agent pack.
fn test_task(choice: &EngineChoice) -> AgentTask {
    AgentTask {
        kind: AgentTaskKind::ProfileImport,
        input_trust: InputTrust::User,
        cli: choice.cli.clone(),
        model: choice.model.clone(),
        prompt: "This is Boba's connection test. Reply with the single word: pong. \
                 Do not call any tool."
            .to_string(),
        target_language: "en".to_string(),
        budget: RunBudget {
            wall_clock: Some(RUN_WALL_CLOCK),
            ..RunBudget::default()
        },
        env: Vec::new(),
    }
}

/// The events of one run, as the step shows them (FR-3): latency and the model
/// identity on success, the CLI's own output and the reason on failure.
fn outcome(events: &[AgentEvent], elapsed: Duration) -> TestCallResult {
    let (cli, model) = events
        .iter()
        .find_map(|event| match event {
            AgentEvent::Started { cli, model } => Some((cli.clone(), model.clone())),
            _ => None,
        })
        .unwrap_or_default();
    let failure = events.iter().find_map(|event| match event {
        AgentEvent::Failed {
            reason,
            message_key,
        } => Some((*reason, message_key.clone())),
        _ => None,
    });
    let finished_ok = events
        .iter()
        .any(|event| matches!(event, AgentEvent::Finished { exit_code: 0 }));
    // A stream that ends with neither a failure nor a clean exit stopped
    // without saying why: report it as a crash, never as a pass.
    let (ok, reason, message_key) = match (failure, finished_ok) {
        (Some((reason, key)), _) => (false, Some(reason.id().to_string()), Some(key)),
        (None, true) => (true, None, None),
        (None, false) => (
            false,
            Some(FailureReason::Crashed.id().to_string()),
            Some(FailureReason::Crashed.message_key().to_string()),
        ),
    };
    TestCallResult {
        ok,
        cli,
        model,
        latency_ms: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
        reply: reply_of(events),
        failure_reason: reason,
        message_key,
    }
}

/// The CLI's text output, trimmed and capped: it is shown as the CLI's own
/// words, never parsed or trusted.
fn reply_of(events: &[AgentEvent]) -> String {
    let mut text = events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    text = text.trim().to_string();
    const CAP: usize = 2_000;
    if text.chars().count() > CAP {
        text = text.chars().take(CAP).collect();
    }
    text
}

/// Start one test call and wait for it, bounded by [`TEST_DEADLINE`].
fn test_call(agents: &dyn AgentPort, choice: EngineChoice) -> TestCallResult {
    let started = Instant::now();
    let handle = agents.run(test_task(&choice));
    let deadline = started + TEST_DEADLINE;
    let mut events = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return timed_out(&choice, &events, started);
        }
        match handle.events.recv_timeout(remaining) {
            Ok(event) => {
                let terminal = matches!(
                    event,
                    AgentEvent::Finished { .. } | AgentEvent::Failed { .. }
                );
                events.push(event);
                if terminal {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                handle.cancel.cancel();
                return timed_out(&choice, &events, started);
            }
            // The run ended without sending its last event; `outcome` reports
            // that as a crash rather than a pass.
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    outcome(&events, started.elapsed())
}

/// The run did not answer inside the deadline: stop it and report the timeout.
fn timed_out(choice: &EngineChoice, events: &[AgentEvent], started: Instant) -> TestCallResult {
    let reason = FailureReason::Timeout;
    TestCallResult {
        ok: false,
        cli: choice.cli.clone(),
        model: choice.model.clone(),
        latency_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        reply: reply_of(events),
        failure_reason: Some(reason.id().to_string()),
        message_key: Some(reason.message_key().to_string()),
    }
}

/// The store, through the composition root (ADR-012).
fn repo(app: &tauri::AppHandle) -> AppResult<Arc<dyn SystemRepo>> {
    wiring::port::<dyn SystemRepo>(app)
}

/// Every known CLI, probed now (the probe also refreshes the `cli_install`
/// evidence ADR-010's gate reads), and the stored choice (FR-2, FR-17).
#[tauri::command]
#[specta::specta]
pub async fn agent_engine_snapshot(app: tauri::AppHandle) -> AppResult<EngineSnapshot> {
    let probe = wiring::port::<EngineProbe>(&app)?;
    let system = repo(&app)?;
    let chosen = read_choice(system.as_ref()).map_err(to_app)?;
    // The probe runs bounded subprocesses; never on the UI thread.
    let statuses = tauri::async_runtime::spawn_blocking(move || probe.detect_and_store())
        .await
        .map_err(|_| to_app(AgentError::Probe))?;
    Ok(snapshot(statuses, chosen))
}

/// Store the engine the user picked (AC1, FR-17). Settings calls the same
/// command, so a later change keeps every other row untouched.
#[tauri::command]
#[specta::specta]
pub fn agent_engine_choose(
    app: tauri::AppHandle,
    cli: String,
    model: Option<String>,
) -> AppResult<EngineChoice> {
    let choice = make_choice(&cli, model).map_err(to_app)?;
    save_choice(repo(&app)?.as_ref(), &SystemClock, choice.clone()).map_err(to_app)?;
    Ok(choice)
}

/// Run the round-trip test on the picked CLI and model (FR-3).
#[tauri::command]
#[specta::specta]
pub async fn agent_test_call(
    app: tauri::AppHandle,
    cli: String,
    model: Option<String>,
) -> AppResult<TestCallResult> {
    let choice = make_choice(&cli, model).map_err(to_app)?;
    let agents = wiring::port::<dyn AgentPort>(&app)?;
    tauri::async_runtime::spawn_blocking(move || test_call(agents.as_ref(), choice))
        .await
        .map_err(|_| AppError::new(ErrorCode::Internal, "errors.agent.crashed"))
}

/// How long the model listing may take before the step falls back to free text.
const MODELS_DEADLINE: Duration = Duration::from_secs(5);

/// The models a CLI can list, when its own `--help` documents a subcommand for
/// it (FR-3: "listed where the CLI exposes them; otherwise free text").
///
/// Only OpenCode documents one (`models`); the other four take a model name but
/// never list them, so they always answer with an empty list and the step shows
/// a free-text field.
#[tauri::command]
#[specta::specta]
pub async fn agent_models(cli: String) -> AppResult<Vec<String>> {
    let choice = make_choice(&cli, None).map_err(to_app)?;
    let kind = kind_of(&choice.cli).expect("make_choice rejected every unknown id");
    Ok(
        tauri::async_runtime::spawn_blocking(move || listed_models(kind))
            .await
            .unwrap_or_default(),
    )
}

/// Run the CLI's own model listing, bounded, and parse it. Any failure — not
/// installed, a non-zero exit, a hung process — is an empty list, never an error
/// the user has to answer: the free-text field is always there.
fn listed_models(kind: CliKind) -> Vec<String> {
    const LISTED: &[(CliKind, &[&str])] = &[(CliKind::OpenCode, &["models"])];
    let Some((_, args)) = LISTED.iter().find(|(candidate, _)| *candidate == kind) else {
        return Vec::new();
    };
    let Some(path) = boba_agent::detect::find_binary(kind, &boba_agent::detect::search_dirs())
    else {
        return Vec::new();
    };
    let output = run_listing(&path, args);
    parse_models(&output)
}

/// Run one bounded subprocess and return its stdout (empty on any failure).
///
/// The pipe is drained on its own thread so a talkative CLI can never block on
/// a full buffer, and the wait is bounded so a CLI that wants a terminal is
/// killed instead of stalling the step.
fn run_listing(program: &std::path::Path, args: &[&str]) -> String {
    use std::io::Read as _;
    use std::process::{Command, Stdio};
    let Ok(mut child) = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return String::new();
    };
    let Some(mut stdout) = child.stdout.take() else {
        return String::new();
    };
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    let deadline = Instant::now() + MODELS_DEADLINE;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return String::new();
            }
        }
    }
    reader.join().unwrap_or_default()
}

/// One model id per line, trimmed, blanks dropped, capped so a noisy listing
/// cannot flood the step.
fn parse_models(output: &str) -> Vec<String> {
    const CAP: usize = 100;
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(CAP)
        .map(str::to_string)
        .collect()
}

/// The engine the user stored, for the callers that run tasks without asking
/// (the manual-intake extraction trigger). `None` on any failure: no choice, a
/// locked store or a row this build cannot read all mean "no engine to use",
/// never a panic.
pub(crate) fn stored_choice(app: &tauri::AppHandle) -> Option<EngineChoice> {
    read_choice(repo(app).ok()?.as_ref()).ok().flatten()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use boba_agent::detect::CliStatus;
    use boba_agent::drivers::{Capabilities, CliInstall, CliKind, HealthStatus};
    use boba_store::db::{Store, StoreConfig};
    use boba_store::keys::DbKey;
    use boba_store::repos::audit::SqliteSystemRepo;

    use boba_app::system::FixedClock;

    use super::*;

    const KEY_HEX: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

    fn tmp_dir(tag: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("boba-engine-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// The real encrypted store over the real `setting` table: no keychain.
    fn repo(tag: &str) -> SqliteSystemRepo {
        let dir = tmp_dir(tag);
        let key = DbKey::from_hex(KEY_HEX).expect("test key");
        let config = StoreConfig::new(dir.join("boba.db"), dir.join("snapshots"));
        let store = Arc::new(Store::open_with_key(config, key).expect("store opens"));
        SqliteSystemRepo::new(store, "test")
    }

    fn clock() -> FixedClock {
        FixedClock::new("2026-10-17T01:00:00.000Z")
    }

    fn locked_down_caps() -> Capabilities {
        Capabilities {
            stream_json: true,
            mcp: true,
            tool_allowlist: true,
            ..Capabilities::default()
        }
    }

    fn installed(kind: CliKind, version: &str, capabilities: Capabilities) -> CliStatus {
        CliStatus::Installed(CliInstall {
            kind,
            path: PathBuf::from(format!("/usr/local/bin/{}", kind.binary_names()[0])),
            version: version.to_string(),
            health: HealthStatus::Healthy,
            capabilities,
        })
    }

    fn choice() -> EngineChoice {
        EngineChoice {
            cli: "claude_code".to_string(),
            model: Some("sonnet".to_string()),
        }
    }

    #[test]
    fn a_stored_choice_round_trips_and_ignores_unknown_fields() {
        let repo = repo("round-trip");
        save_choice(&repo, &clock(), choice()).expect("save");
        assert_eq!(read_choice(&repo).expect("read"), Some(choice()));

        // A row a newer build wrote: this build ignores what it does not know.
        repo.setting_put(ENGINE_KEY, r#"{"cli":"codex","budget":30}"#, "seed")
            .expect("seed");
        assert_eq!(
            read_choice(&repo).expect("read"),
            Some(EngineChoice {
                cli: "codex".to_string(),
                model: None
            })
        );
    }

    #[test]
    fn a_corrupt_or_unknown_choice_reads_as_no_choice() {
        let repo = repo("corrupt");
        repo.setting_put(ENGINE_KEY, "{not json", "seed")
            .expect("seed");
        assert_eq!(read_choice(&repo).expect("read"), None);

        repo.setting_put(ENGINE_KEY, r#"{"cli":"not_a_cli"}"#, "seed")
            .expect("seed");
        assert_eq!(read_choice(&repo).expect("read"), None);
    }

    #[test]
    fn an_unknown_cli_is_rejected_and_the_model_is_trimmed() {
        assert!(matches!(
            make_choice("nope", None),
            Err(AgentError::UnknownCli(_))
        ));
        let made = make_choice("  codex ", Some("  gpt-5  ".to_string())).expect("choice");
        assert_eq!(made.cli, "codex");
        assert_eq!(made.model.as_deref(), Some("gpt-5"));

        // An empty model means "the CLI's own default", never an empty string.
        let made = make_choice("codex", Some("   ".to_string())).expect("choice");
        assert_eq!(made.model, None);
    }

    #[test]
    fn every_card_reports_what_the_lockdown_gate_would_refuse() {
        let statuses = vec![
            installed(
                CliKind::ClaudeCode,
                "2.1.301 (Claude Code)",
                locked_down_caps(),
            ),
            installed(CliKind::Aider, "0.86.0", Capabilities::default()),
            CliStatus::NotInstalled {
                kind: CliKind::Codex,
            },
        ];
        let cards = cards(statuses);

        let claude = &cards[0];
        assert!(claude.installed);
        assert_eq!(claude.health, "healthy");
        assert_eq!(claude.path.as_deref(), Some("/usr/local/bin/claude"));
        assert!(claude.unsupported_tasks.is_empty(), "{claude:?}");

        // Aider has no verified lockdown: every third-party-content task is
        // refused, and the engine step lists exactly those (AC2).
        let aider = &cards[1];
        assert_eq!(
            aider.unsupported_tasks,
            vec![
                "extract_postings",
                "narrate_scores",
                "tailor_application",
                "review_application",
                "answer_questions",
                "classify_emails",
            ]
        );

        let codex = &cards[2];
        assert!(!codex.installed);
        assert_eq!(codex.version, "");
        assert_eq!(codex.path, None);
        assert!(codex.unsupported_tasks.is_empty());
        assert!(codex.install_url.starts_with("https://"));
    }

    #[test]
    fn the_snapshot_keeps_registry_order_and_the_stored_choice() {
        let statuses = CliKind::ALL
            .into_iter()
            .map(|kind| CliStatus::NotInstalled { kind })
            .collect();
        let snapshot = snapshot(statuses, Some(choice()));
        let ids: Vec<&str> = snapshot
            .engines
            .iter()
            .map(|card| card.cli.as_str())
            .collect();
        assert_eq!(
            ids,
            vec!["claude_code", "codex", "opencode", "gemini_cli", "aider"]
        );
        assert_eq!(snapshot.chosen, Some(choice()));
    }

    #[test]
    fn the_test_task_is_a_user_trust_ping_on_the_picked_engine() {
        let task = test_task(&choice());
        assert_eq!(task.kind, AgentTaskKind::ProfileImport);
        assert_eq!(task.input_trust, InputTrust::User);
        assert_eq!(task.cli, "claude_code");
        assert_eq!(task.model.as_deref(), Some("sonnet"));
        assert_eq!(task.target_language, "en");
        assert_eq!(task.budget.wall_clock, Some(RUN_WALL_CLOCK));
        // The test call may not run longer than the caller waits for it.
        assert!(RUN_WALL_CLOCK < TEST_DEADLINE);
    }

    #[test]
    fn a_successful_run_reports_latency_and_the_model_identity() {
        let events = vec![
            AgentEvent::Started {
                cli: "claude_code".to_string(),
                model: Some("sonnet".to_string()),
            },
            AgentEvent::Text {
                text: "pong".to_string(),
            },
            AgentEvent::Finished { exit_code: 0 },
        ];
        let result = outcome(&events, Duration::from_millis(812));
        assert!(result.ok);
        assert_eq!(result.latency_ms, 812);
        assert_eq!(result.cli, "claude_code");
        assert_eq!(result.model.as_deref(), Some("sonnet"));
        assert_eq!(result.reply, "pong");
        assert_eq!(result.failure_reason, None);
        assert_eq!(result.message_key, None);
    }

    #[test]
    fn a_failed_run_reports_the_reason_the_key_and_the_cli_output() {
        let events = vec![
            AgentEvent::Started {
                cli: "codex".to_string(),
                model: None,
            },
            AgentEvent::Text {
                text: "error: not logged in. Run `codex login`.".to_string(),
            },
            AgentEvent::Failed {
                reason: FailureReason::AuthExpired,
                message_key: "errors.agent.auth_expired".to_string(),
            },
        ];
        let result = outcome(&events, Duration::from_millis(40));
        assert!(!result.ok);
        assert_eq!(result.failure_reason.as_deref(), Some("auth_expired"));
        assert_eq!(
            result.message_key.as_deref(),
            Some("errors.agent.auth_expired")
        );
        assert!(result.reply.contains("codex login"));
    }

    #[test]
    fn a_stream_that_ends_without_finishing_is_a_crash_not_a_pass() {
        let events = vec![AgentEvent::Started {
            cli: "aider".to_string(),
            model: None,
        }];
        let result = outcome(&events, Duration::from_millis(5));
        assert!(!result.ok);
        assert_eq!(result.failure_reason.as_deref(), Some("crashed"));
        assert_eq!(result.message_key.as_deref(), Some("errors.agent.crashed"));
    }

    #[test]
    fn a_model_listing_is_one_id_per_line_and_blanks_are_dropped() {
        let listed = parse_models("  anthropic/claude-sonnet-4-5 \n\nopenai/gpt-5\n");
        assert_eq!(
            listed,
            vec![
                "anthropic/claude-sonnet-4-5".to_string(),
                "openai/gpt-5".to_string()
            ]
        );
    }

    #[test]
    fn only_the_cli_that_documents_a_listing_subcommand_runs_one() {
        // The recorded `--help` fixtures are evidence: only OpenCode lists one.
        for kind in [
            CliKind::ClaudeCode,
            CliKind::Codex,
            CliKind::GeminiCli,
            CliKind::Aider,
        ] {
            assert!(listed_models(kind).is_empty(), "{}", kind.display_name());
        }
    }

    #[test]
    fn a_long_reply_is_capped_so_the_ipc_payload_stays_small() {
        let events = vec![AgentEvent::Text {
            text: "é".repeat(5_000),
        }];
        assert_eq!(
            outcome(&events, Duration::ZERO).reply.chars().count(),
            2_000
        );
    }
}
