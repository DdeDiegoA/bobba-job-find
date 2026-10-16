//! Agent runtime and per-run sandbox wiring (CH5; AD-4, AD-6, AD-7, ADR-010,
//! ADR-012).
//!
//! The composition root is the only place that knows the app-data paths, where
//! the packaged `boba-mcp` sidecar lives, and which repos stand behind the MCP
//! tool surface. It hands all of that to `boba-agent` through the seams S2.3a
//! defined — `AgentRuntime::new(…).with_emitter(…).with_sandbox(…)
//! .with_command_builder(…)` — and `port_list` mounts the result as
//! `Arc<dyn AgentPort>`. Nothing here edits `lib.rs`, and no `boba-agent` type
//! learns a global path.
//!
//! What is live after this chore: every run gets its own workspace, token and
//! MCP endpoint, its own `mcp.json`, an event stream the UI can follow, and a
//! CLI command built from the driver's evidence **and** ADR-010's gate — the
//! lock-down verdict comes from the probed `cli_install` row, and a task that
//! reads third-party content is refused unless that CLI's lock-down is verified.
//! What is deliberately *not* live: the data behind most catalog tools (profile,
//! postings, scores, mail, documents land in later slices), the CLI argv for the
//! CLIs whose MCP flag is not a plain `--flag <path>`, and the observation that
//! will let a verified CLI run untrusted tasks (S2.4). Gaps are explicit
//! refusals here rather than silence — see [`RepoTools`], [`command_builder`] and
//! [`lock_down`].

use std::path::{Path, PathBuf};
use std::sync::Arc;

use boba_agent::drivers::{Capabilities, CliKind};
use boba_agent::lockdown::{self, LockdownConfirmation, TaskSupport, Unverified};
use boba_agent::mcp::config::{RunSandbox, SandboxConfig};
use boba_agent::mcp::server::ToolHandler;
use boba_agent::runtime::{AgentRuntime, Emitter, TaskGate};
use boba_agent::supervisor::CommandSpec;
use boba_app::ports::agent::{AgentEvent, AgentTask, FailureReason};
use boba_mcp_proto::tool;
use boba_store::db::Store;
use boba_store::repos::cli_install;
use serde_json::Value;
use tauri::{AppHandle, Manager, Runtime};
use tauri_specta::Event as _;

use crate::events::{TaskFinished, TaskOutcome, TaskProgressed};

/// The area the UI keys agent-run progress on (AD-3).
pub const AGENT_AREA: &str = "agent.run";

/// The shim's file name, as declared in `tauri.conf.json`
/// (`bundle.externalBin = ["binaries/boba-mcp"]`).
const SHIM_NAME: &str = "boba-mcp";

/// Build this launch's run engine: the store is the `AgentRepo`, and the
/// sandbox, tool surface, event sink and command builder come from here.
pub fn agent_runtime<R: Runtime>(app: &AppHandle<R>, store: &Arc<Store>) -> AgentRuntime<Store> {
    let shim = resolve_shim();
    let resolved = shim.as_ref().ok().cloned();
    AgentRuntime::new(Arc::clone(store))
        .with_emitter(agent_emitter(app))
        .with_sandbox(
            sandbox_config(app, resolved.as_deref()),
            Arc::new(RepoTools {
                _store: Arc::clone(store),
            }),
        )
        .with_command_builder(Arc::new(command_builder(shim.err())))
        // ADR-010 §2: every task goes through this gate before a run exists.
        .with_gate(task_gate(Arc::clone(store)))
}

/// The sandbox a run works in: app data owns the workspaces, and the shim is
/// the binary this app actually ships (the same path lands in `mcp.json`).
///
/// `pub(crate)` because the startup sweep (ADR-010 §4) needs the same paths and
/// does not care which binary the shim is: it passes `None`.
pub(crate) fn sandbox_config<R: Runtime>(app: &AppHandle<R>, shim: Option<&Path>) -> SandboxConfig {
    let app_data = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("boba-job-find"));
    SandboxConfig {
        runs_root: app_data.join("runs"),
        app_data: app_data.clone(),
        shim: shim.map_or_else(|| PathBuf::from(shim_file_name()), Path::to_path_buf),
    }
}

/// Resolve the packaged `boba-mcp` sidecar.
///
/// Tauri copies `bundle.externalBin` next to the executable, and a release
/// build keeps the target-triple suffix it was built with, so both spellings
/// are tried. A missing sidecar is an error, not a warning: without it the CLI
/// would run with no MCP tools and only its own native ones, which is exactly
/// what AD-6 exists to prevent, so every run is refused instead.
fn resolve_shim() -> Result<PathBuf, String> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    if let Some(found) = exe_dir.as_deref().and_then(find_shim) {
        return Ok(found);
    }
    let expected = exe_dir.map_or_else(
        || PathBuf::from(shim_file_name()),
        |dir| dir.join(shim_file_name()),
    );
    Err(format!(
        "the boba-mcp sidecar is missing (looked next to the executable, at {})",
        expected.display()
    ))
}

/// The shim next to the executable: the exact name first, then Tauri's
/// triple-suffixed build name.
fn find_shim(exe_dir: &Path) -> Option<PathBuf> {
    let exact = exe_dir.join(shim_file_name());
    if exact.is_file() {
        return Some(exact);
    }
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(exe_dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_shim(path))
        .collect();
    candidates.sort();
    candidates.into_iter().next()
}

fn shim_file_name() -> String {
    if cfg!(windows) {
        format!("{SHIM_NAME}.exe")
    } else {
        SHIM_NAME.to_string()
    }
}

/// Looks like the sidecar: next to it live `*.d` dep files and other binaries.
fn is_shim(path: &Path) -> bool {
    path.is_file()
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(SHIM_NAME) && !name.ends_with(".d"))
}

/// Forward a run's events to the UI as the typed `task.progressed` and
/// `task.finished` facts (AD-3). `message_key` is an i18n key, never prose.
fn agent_emitter<R: Runtime>(app: &AppHandle<R>) -> Arc<Emitter> {
    let app = app.clone();
    Arc::new(move |run_id: &str, seq: i64, event: &AgentEvent| {
        let progress = TaskProgressed {
            task_id: run_id.to_string(),
            area: AGENT_AREA.to_string(),
            done: seq.clamp(0, i64::from(u32::MAX)) as u32,
            total: None,
            message_key: failure_of(event).map(|reason| reason.message_key().to_string()),
        };
        let _ = progress.emit(&app);

        if let Some((outcome, message_key)) = terminal_of(event) {
            let finished = TaskFinished {
                task_id: run_id.to_string(),
                area: AGENT_AREA.to_string(),
                outcome,
                message_key,
            };
            let _ = finished.emit(&app);
        }
    })
}

fn failure_of(event: &AgentEvent) -> Option<FailureReason> {
    match event {
        AgentEvent::Failed { reason, .. } => Some(*reason),
        _ => None,
    }
}

fn terminal_of(event: &AgentEvent) -> Option<(TaskOutcome, Option<String>)> {
    match event {
        AgentEvent::Finished { .. } => Some((TaskOutcome::Completed, None)),
        AgentEvent::Failed { reason, .. } => {
            let outcome = if *reason == FailureReason::Cancelled {
                TaskOutcome::Cancelled
            } else {
                TaskOutcome::Failed
            };
            Some((outcome, Some(reason.message_key().to_string())))
        }
        _ => None,
    }
}

/// The minimal CLI invocation for a task: the CLI, its MCP-config flag pointed
/// at this run's `mcp.json`, the pinned lockdown config, its non-interactive
/// flags, and the prompt.
///
/// Two refusals come first, both fail-closed:
/// 1. **No sidecar.** Without `boba-mcp` the CLI would run with its own native
///    tools and none of ours, so no run starts (AD-6).
/// 2. **A prompt that could be parsed as an option.** Neither CLI documents an
///    end-of-options `--`, so a prompt starting with `-` is refused instead of
///    guessed at.
///
/// What a task is allowed to *do* is not decided here: ADR-010's gate runs in
/// the runtime, before a run exists ([`task_gate`]).
///
/// A CLI whose argv is not wired yet (Codex, OpenCode, Aider — their MCP flag
/// is not a plain `--flag <path>`, or they take no prompt) is refused here too;
/// that stays the driver's work (S2.4).
fn command_builder(
    shim_error: Option<String>,
) -> impl Fn(&AgentTask, &RunSandbox) -> CommandSpec + Send + Sync {
    move |task: &AgentTask, sandbox: &RunSandbox| {
        if let Some(why) = &shim_error {
            return refusal(task, why);
        }
        if task.prompt.starts_with('-') {
            return refusal(
                task,
                "the prompt starts with '-', which this CLI would read as an option",
            );
        }
        let Some(kind) = CliKind::ALL.into_iter().find(|kind| kind.id() == task.cli) else {
            return refusal(task, "unknown agent CLI");
        };
        let (Some(mcp_flag), Some(argv)) = (
            kind.mcp_config_flag(),
            kind.non_interactive_argv(&task.prompt),
        ) else {
            return refusal(
                task,
                "no minimal argv for this CLI yet (its MCP flag is not `--flag <path>`, or it \
                 takes no prompt); that arrives with the driver argv work",
            );
        };
        // The flags that pin this CLI's lockdown for this task. A CLI the gate
        // lets an untrusted task through always has them (`lockdown`'s own
        // invariant); a CLI that cannot be pinned only ever runs the user's own
        // input, with the security badge.
        let pinned = lockdown::pinned_args(kind, task.kind.id());

        CommandSpec {
            program: PathBuf::from(kind.binary_names()[0]),
            args: [mcp_flag.to_string(), sandbox.config.display().to_string()]
                .into_iter()
                .chain(pinned.unwrap_or_default())
                .chain(argv)
                .collect(),
            // The runtime sets `cwd` to the run's workspace and adds the endpoint
            // and token to the environment; argv never carries secrets.
            cwd: None,
            env: Vec::new(),
        }
    }
}

/// ADR-010's gate, in the shape the runtime takes it: the composition root
/// supplies the probed `cli_install` evidence, and `boba-agent` decides. The
/// runtime consults it before a run gets a workspace, a token or a CLI, so no
/// caller can route around it.
///
/// Until S2.4 wires the run-time observation of a pinned lockdown,
/// [`lock_down`] refuses every untrusted task on every CLI (orquestador ruling
/// (c), 2026-10-16) — a hard gate, with no user override.
fn task_gate(store: Arc<Store>) -> Arc<TaskGate> {
    Arc::new(move |task: &AgentTask| {
        let Some(kind) = CliKind::ALL.into_iter().find(|kind| kind.id() == task.cli) else {
            // Nothing ever probed this CLI, so nothing vouches for any task on
            // it.
            return TaskSupport::Unsupported {
                reason: Unverified::NoVerifiedConfig,
                fallback: None,
            };
        };
        lock_down(&store, kind, task)
    })
}

/// ADR-010's verdict for a task, from the probed install the store holds: the
/// composition root owns the `cli_install` row (ADR-012), the decision itself
/// belongs to `boba-agent`.
///
/// No probed install (a CLI the user added by hand, or an unreadable store) is
/// no evidence at all, so the verdict is taken from an empty one — which still
/// lets the user's own input through and refuses everything untrusted.
fn lock_down(store: &Store, kind: CliKind, task: &AgentTask) -> TaskSupport {
    let probed = store
        .read({
            let kind_id = kind.id().to_string();
            move |conn| {
                let rows = cli_install::list(conn).unwrap_or_default();
                Ok(rows.into_iter().find(|row| row.kind == kind_id))
            }
        })
        .ok()
        .flatten();
    let (version, capabilities) = probed.map_or_else(
        || (String::new(), Capabilities::default()),
        |row| {
            (
                row.version,
                serde_json::from_str(&row.capabilities_json).unwrap_or_default(),
            )
        },
    );
    // `NotObserved` until S2.4's runtime watches the CLI's init event (ruling
    // (c), 2026-10-16): the pinned config alone never lifts the gate.
    lockdown::support(
        task.kind,
        kind,
        &version,
        &capabilities,
        LockdownConfirmation::NotObserved,
    )
}

/// A command that cannot start: the run fails fast, is recorded and is reported,
/// and no CLI is launched with the wrong tools.
fn refusal(task: &AgentTask, why: &str) -> CommandSpec {
    tracing::warn!(cli = %task.cli, reason = %why, "refusing this agent run");
    CommandSpec {
        program: PathBuf::from(format!("boba-refused-{}", task.cli)),
        args: Vec::new(),
        cwd: None,
        env: Vec::new(),
    }
}

/// The repos behind the MCP tool surface (AD-5: the agent proposes, the core
/// disposes).
///
/// The catalog's data lands slice by slice (profile S3.4, postings S4.2,
/// scoring S6.3, documents S7.2, mail S8.1), so today every tool answers with an
/// explicit refusal naming the slice it belongs to. That is deliberate: a tool
/// that is not implemented must fail loudly, never return empty data that a run
/// reads as "nothing to do". Policy — the task's tool subset, the call cap and
/// the `untrusted_content` envelope — is enforced by the endpoint before this
/// handler is reached (S2.3a), so nothing here has to re-check it.
pub struct RepoTools {
    _store: Arc<Store>,
}

impl ToolHandler for RepoTools {
    fn call(&self, _task: &str, tool_name: &str, _args: &Value) -> Result<Value, String> {
        match tool::find(tool_name) {
            // Defence in depth: the endpoint already refuses unknown names.
            None => Err(format!("no tool `{tool_name}`")),
            Some(spec) => Err(format!(
                "`{}` has no data source yet: it lands with {}",
                spec.name,
                pending_slice(spec.name)
            )),
        }
    }
}

/// The slice that brings each tool's data. Every catalog tool must appear here:
/// [`tests::every_catalog_tool_is_answered_explicitly`] fails otherwise.
fn pending_slice(tool_name: &str) -> &'static str {
    match tool_name {
        "get_task_input" => "S2.4 (task registry: per-run inputs)",
        "get_profile_facts"
        | "get_preferences"
        | "propose_profile_facts"
        | "propose_translations" => "S3.4 (profile facts)",
        "get_pending_postings" | "get_posting" | "get_company_research" => "S4.2 (postings)",
        "propose_posting_features" => "S6.2 (posting features)",
        "get_score_card" => "S6.3 (scoring)",
        "save_score_narration" => "S6.5 (narration)",
        "save_document_draft" => "S7.2 (documents)",
        "save_review" => "S7.4 (reviewer pass)",
        "get_email_batch" => "S8.1 (mail)",
        "propose_status_change" => "S8.4 (status proposals)",
        // Never panic on a request path: an unmapped catalog tool is a bug, and
        // the test below fails when one appears.
        _ => UNOWNED,
    }
}

/// Answer for a catalog tool nobody claimed yet; the tests refuse to let a
/// catalog entry reach it.
const UNOWNED: &str = "no slice owns it yet (unassigned)";

#[cfg(test)]
mod tests {
    use super::*;
    use boba_agent::mcp::config::Workspace;
    use boba_app::ports::agent::InputTrust;
    use serde_json::json;

    /// Claude Code's recorded-probe capabilities: MCP plus a tool allowlist,
    /// which is what its pinned lockdown config needs.
    fn claude_caps() -> Capabilities {
        Capabilities {
            stream_json: true,
            mcp: true,
            tool_allowlist: true,
            ..Capabilities::default()
        }
    }

    fn task(cli: &str) -> AgentTask {
        task_of(
            cli,
            boba_app::ports::agent::AgentTaskKind::ExtractPostings,
            InputTrust::Untrusted,
        )
    }

    fn task_of(
        cli: &str,
        kind: boba_app::ports::agent::AgentTaskKind,
        input_trust: InputTrust,
    ) -> AgentTask {
        AgentTask {
            kind,
            input_trust,
            cli: cli.to_string(),
            model: None,
            prompt: "extract the offers".to_string(),
            budget: boba_app::ports::agent::RunBudget::default(),
            env: Vec::new(),
        }
    }

    /// A run's sandbox under its own temp root, so parallel tests never share a
    /// workspace directory.
    fn run_sandbox(tag: &str) -> RunSandbox {
        let root = tmp_dir(tag);
        let config = SandboxConfig {
            runs_root: root.join("runs"),
            app_data: root.clone(),
            shim: root.join("boba-mcp"),
        };
        let workspace = Workspace::create(&config.runs_root, "01RUN").expect("workspace");
        RunSandbox {
            endpoint: config.app_data.join("ipc/01RUN.sock").display().to_string(),
            token: "a-token".to_string(),
            config: workspace.path().join("mcp.json"),
            workspace,
        }
    }

    fn tmp_dir(tag: &str) -> PathBuf {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("boba-ch5-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// A handler over a real (encrypted, temp) store: the same construction the
    /// composition root does, without the keychain.
    fn repo_tools(tag: &str) -> RepoTools {
        RepoTools { _store: store(tag) }
    }

    /// An open store with every migration applied.
    fn store(tag: &str) -> Arc<Store> {
        let dir = tmp_dir(tag);
        let key = boba_store::keys::DbKey::from_hex(
            "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff",
        )
        .expect("test key");
        Arc::new(
            Store::open_with_key(
                boba_store::db::StoreConfig::new(dir.join("boba.db"), dir.join("snapshots")),
                key,
            )
            .expect("store"),
        )
    }

    /// A store holding one probed CLI installation, exactly as detection and
    /// S2.1 leave it: the evidence ADR-010's gate is decided from.
    fn store_with_cli(
        tag: &str,
        kind: CliKind,
        version: &str,
        capabilities: Capabilities,
    ) -> Arc<Store> {
        let store = store(tag);
        let (kind_id, path, version, caps_json) = (
            kind.id().to_string(),
            format!("/usr/local/bin/{}", kind.binary_names()[0]),
            version.to_string(),
            capabilities.to_json(),
        );
        store
            .execute(move |conn| {
                boba_store::repos::cli_install::upsert(conn, &kind_id, &path, &version, &caps_json)
                    .map_err(|e| boba_store::db::StoreError::Sqlite(e.to_string()))?;
                Ok(())
            })
            .expect("store the probed install");
        store
    }

    #[test]
    fn every_catalog_tool_is_answered_explicitly() {
        let tools = repo_tools("store");

        for spec in tool::TOOLS {
            let answer = tools.call("extract_postings", spec.name, &json!({}));
            // Never "unknown tool": every catalog entry has a real answer.
            let message = answer.expect_err("no tool has data yet");
            assert!(
                !message.contains("no tool"),
                "{} fell through the handler: {message}",
                spec.name
            );
            assert!(
                message.contains(spec.name) || message.contains("no data source"),
                "unhelpful answer for {}: {message}",
                spec.name
            );
        }
    }

    /// Every catalog tool must route somewhere: reaching the fallback is a bug.
    #[test]
    fn no_catalog_tool_falls_through_to_the_unowned_answer() {
        for spec in tool::TOOLS {
            assert_ne!(
                pending_slice(spec.name),
                UNOWNED,
                "{} is in the catalog with no owner in RepoTools",
                spec.name
            );
        }
        // And an unknown name simply answers with the fallback, no panic.
        assert_eq!(pending_slice("made_up_tool"), UNOWNED);
    }

    #[test]
    fn dangerous_tool_names_are_refused_and_are_not_in_the_catalog() {
        let tools = repo_tools("danger");
        for name in [
            "send_email",
            "submit_application",
            "delete_run",
            "update_settings",
            "fetch_url",
        ] {
            assert!(
                tool::find(name).is_none(),
                "{name} must not be in the catalog"
            );
            assert!(
                tools.call("extract_postings", name, &json!({})).is_err(),
                "{name} must be refused"
            );
        }
    }

    /// The endpoint wraps third-party text (AD-7); a handler that pre-wrapped it
    /// would double-envelope every payload, so nothing here may return the key.
    #[test]
    fn no_handler_answer_pre_wraps_untrusted_content() {
        for spec in tool::TOOLS {
            let answer = repo_tools("wrap").call("extract_postings", spec.name, &json!({}));
            if let Ok(value) = answer {
                assert!(
                    value.get("untrusted_content").is_none(),
                    "{} pre-wrapped its payload",
                    spec.name
                );
            }
        }
    }

    #[test]
    fn the_shim_is_found_beside_the_executable() {
        let dir = tmp_dir("shim");
        assert_eq!(find_shim(&dir), None, "nothing there yet");

        // A stale dep file and another binary must not be mistaken for the shim.
        std::fs::write(dir.join("boba-mcp.d"), b"dep").unwrap();
        std::fs::write(dir.join("other-binary"), b"x").unwrap();
        assert_eq!(find_shim(&dir), None);

        // Tauri's triple-suffixed build name.
        let suffixed = dir.join(format!("{SHIM_NAME}-aarch64-apple-darwin"));
        std::fs::write(&suffixed, b"shim").unwrap();
        assert_eq!(find_shim(&dir), Some(suffixed));

        // The exact name wins when both exist.
        let exact = dir.join(shim_file_name());
        std::fs::write(&exact, b"shim").unwrap();
        assert_eq!(find_shim(&dir), Some(exact));
    }

    #[test]
    fn a_trusted_task_gets_the_cli_argv_with_this_runs_mcp_config() {
        // Trusted kinds are the ones whose input is the user's own (AD-4/ADR-010).
        // A Full CLI, so its runs carry the pinned lockdown config.
        let builder = command_builder(None);
        for cli in ["claude_code", "gemini_cli"] {
            let sandbox = run_sandbox(&format!("argv-{cli}"));
            let trusted = task_of(
                cli,
                boba_app::ports::agent::AgentTaskKind::ProfileImport,
                InputTrust::User,
            );

            let spec = builder(&trusted, &sandbox);
            let kind = CliKind::ALL.into_iter().find(|k| k.id() == cli).unwrap();
            assert_eq!(spec.program, PathBuf::from(kind.binary_names()[0]));
            // Exactly this run's MCP config, this CLI's pinned lockdown flags
            // (none for Gemini: it has no verified pinning yet), then the CLI's
            // own documented non-interactive argv.
            let expected: Vec<String> = [
                kind.mcp_config_flag().unwrap().to_string(),
                sandbox.config.display().to_string(),
            ]
            .into_iter()
            .chain(lockdown::pinned_args(kind, "profile_import").unwrap_or_default())
            .chain(kind.non_interactive_argv("extract the offers").unwrap())
            .collect();
            assert_eq!(spec.args, expected, "{cli}");
            // Secrets never travel in argv, and the runtime owns cwd.
            assert!(!spec.args.iter().any(|arg| arg.contains("a-token")));
            assert_eq!(spec.cwd, None);
        }
    }

    /// P1 (ruling (c), 2026-10-16): ADR-010's gate refuses every untrusted task
    /// until S2.4 observes a pinned lockdown — on an unprobed CLI, on a version
    /// outside its driver's range, and on Claude Code with a pinned config.
    #[test]
    fn untrusted_tasks_are_refused_by_the_gate() {
        let unterprobed = task_gate(store("unprobed"));
        let untrusted = task("aider");
        assert_eq!(
            unterprobed(&untrusted),
            TaskSupport::Unsupported {
                reason: Unverified::NoVerifiedConfig,
                fallback: None,
            }
        );

        let outdated = task_gate(store_with_cli(
            "outdated",
            CliKind::ClaudeCode,
            "2.2.0",
            claude_caps(),
        ));
        assert_eq!(
            outdated(&task("claude_code")),
            TaskSupport::Unsupported {
                reason: Unverified::VersionOutsideVerifiedRange { verified: (2, 1) },
                fallback: None,
            }
        );

        // A Full CLI with its proof complete but no observation yet.
        let full = task_gate(store_with_cli(
            "full",
            CliKind::ClaudeCode,
            "2.1.289",
            claude_caps(),
        ));
        assert_eq!(
            full(&task("claude_code")),
            TaskSupport::Unsupported {
                reason: Unverified::Unobserved,
                fallback: None,
            }
        );
        // `classify_emails` still has its deterministic fallback.
        let mut classify = task("claude_code");
        classify.kind = boba_app::ports::agent::AgentTaskKind::ClassifyEmails;
        assert_eq!(
            full(&classify),
            TaskSupport::Unsupported {
                reason: Unverified::Unobserved,
                fallback: Some(lockdown::Fallback::DeterministicRules),
            }
        );
    }

    /// The user's own input does not need lockdown, so the gate lets it through
    /// with the badge the UI shows, and the CLI still gets a live argv.
    #[test]
    fn the_gate_lets_the_users_own_input_through_with_a_badge() {
        let gate = task_gate(store_with_cli(
            "badge",
            CliKind::ClaudeCode,
            "2.2.0",
            claude_caps(),
        ));
        let user = task_of(
            "claude_code",
            boba_app::ports::agent::AgentTaskKind::ProfileImport,
            InputTrust::User,
        );
        assert_eq!(
            gate(&user),
            TaskSupport::SupportedWithBadge(Unverified::VersionOutsideVerifiedRange {
                verified: (2, 1)
            })
        );

        let builder = command_builder(None);
        let spec = builder(&user, &run_sandbox("badge"));
        assert_eq!(spec.program, PathBuf::from("claude"), "{spec:?}");
        assert!(
            !spec.program.to_string_lossy().contains("boba-refused-"),
            "a user-input task must still run: {spec:?}"
        );
    }

    /// On a Full CLI the user's own input runs with its lockdown pinned: only
    /// our MCP server, and only that task's own tools.
    #[test]
    fn a_user_task_on_a_full_cli_carries_its_pinned_lockdown() {
        let full = store_with_cli("pinned", CliKind::ClaudeCode, "2.1.289", claude_caps());
        let builder = command_builder(None);
        let sandbox = run_sandbox("pinned");

        let user = task_of(
            "claude_code",
            boba_app::ports::agent::AgentTaskKind::ProfileImport,
            InputTrust::User,
        );
        let spec = builder(&user, &sandbox);

        // The gate is what allows it; the argv is what pins it.
        assert!(task_gate(full)(&user).is_supported());
        assert_eq!(spec.args[0], "--mcp-config");
        assert_eq!(spec.args[1], sandbox.config.display().to_string());
        assert!(
            spec.args.iter().any(|arg| arg == "--strict-mcp-config"),
            "the run is not restricted to our MCP server: {spec:?}"
        );
        let allowed = spec
            .args
            .iter()
            .position(|arg| arg == "--allowedTools")
            .map(|at| spec.args[at + 1].clone())
            .expect("the allowlist is pinned");
        let expected: Vec<String> = boba_mcp_proto::tool::tools_for_task("profile_import")
            .into_iter()
            .map(|spec| format!("mcp__boba__{}", spec.name))
            .collect();
        assert_eq!(allowed, expected.join(","), "never more than its own tools");
    }

    /// P1: no sidecar means no tools, so no run may start.
    #[test]
    fn a_missing_shim_refuses_every_run() {
        let builder = command_builder(Some("the boba-mcp sidecar is missing".to_string()));
        let sandbox = run_sandbox("no-shim");

        for task in [
            task_of(
                "claude_code",
                boba_app::ports::agent::AgentTaskKind::ProfileImport,
                InputTrust::User,
            ),
            task("claude_code"),
        ] {
            let spec = builder(&task, &sandbox);
            assert!(
                spec.program.to_string_lossy().contains("boba-refused-"),
                "a run started without the sidecar: {spec:?}"
            );
        }
    }

    #[test]
    fn a_prompt_that_could_be_an_option_is_refused() {
        let builder = command_builder(None);
        let sandbox = run_sandbox("dash-prompt");
        let mut hostile = task_of(
            "claude_code",
            boba_app::ports::agent::AgentTaskKind::ProfileImport,
            InputTrust::User,
        );
        hostile.prompt = "--dangerously-looking-flag value".to_string();

        let spec = builder(&hostile, &sandbox);
        assert!(spec.program.to_string_lossy().contains("boba-refused-"));
    }

    #[test]
    fn a_cli_without_wired_argv_fails_fast_instead_of_guessing() {
        let builder = command_builder(None);
        for cli in ["codex", "opencode", "aider"] {
            let sandbox = run_sandbox(&format!("unsupported-{cli}"));
            let trusted = task_of(
                cli,
                boba_app::ports::agent::AgentTaskKind::ProfileImport,
                InputTrust::User,
            );
            let spec = builder(&trusted, &sandbox);
            assert!(
                spec.program.to_string_lossy().contains("boba-refused-"),
                "{cli} must not get an invented argv: {:?}",
                spec.program
            );
            assert!(spec.args.is_empty());
        }
    }
}
