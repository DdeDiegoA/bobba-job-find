//! Agent runtime and per-run sandbox wiring (CH5; AD-4, AD-6, AD-7, ADR-012).
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
//! CLI command built from the driver's evidence. What is deliberately *not*
//! live: the data behind most catalog tools (profile, postings, scores, mail,
//! documents land in later slices) and the CLI argv for the CLIs whose MCP flag
//! is not a plain `--flag <path>`. Both gaps are explicit refusals here rather
//! than silence — see [`RepoTools`] and [`command_builder`].

use std::path::{Path, PathBuf};
use std::sync::Arc;

use boba_agent::drivers::CliKind;
use boba_agent::mcp::config::SandboxConfig;
use boba_agent::mcp::server::ToolHandler;
use boba_agent::runtime::{AgentRuntime, Emitter};
use boba_agent::supervisor::CommandSpec;
use boba_app::ports::agent::{AgentEvent, AgentTask, FailureReason};
use boba_mcp_proto::tool;
use boba_store::db::Store;
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
    AgentRuntime::new(Arc::clone(store))
        .with_emitter(agent_emitter(app))
        .with_sandbox(
            sandbox(app),
            Arc::new(RepoTools {
                _store: Arc::clone(store),
            }),
        )
        .with_command_builder(Arc::new(command_builder))
}

/// The sandbox a run works in: app data owns the workspaces, and the shim is
/// the binary this app actually ships (the same path lands in `mcp.json`).
fn sandbox<R: Runtime>(app: &AppHandle<R>) -> SandboxConfig {
    let app_data = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("boba-job-find"));
    SandboxConfig {
        runs_root: app_data.join("runs"),
        app_data: app_data.clone(),
        shim: resolve_shim(),
    }
}

/// Resolve the packaged `boba-mcp` sidecar.
///
/// Tauri copies `bundle.externalBin` next to the executable, and a release
/// build keeps the target-triple suffix it was built with, so both spellings
/// are tried. A missing sidecar is not fatal here — the run still starts — but
/// it means the CLI gets no tools, so it is logged loudly.
fn resolve_shim() -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    if let Some(found) = exe_dir.as_deref().and_then(find_shim) {
        return found;
    }
    let expected = exe_dir.map_or_else(
        || PathBuf::from(shim_file_name()),
        |dir| dir.join(shim_file_name()),
    );
    tracing::warn!(
        path = %expected.display(),
        "boba-mcp sidecar not found next to the executable; agent runs will start without tools"
    );
    expected
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
/// at this run's `mcp.json`, and the prompt.
///
/// Only the CLIs whose flag is a plain `--flag <path>` are wired here, and the
/// flag itself comes from the driver ([`CliKind::mcp_config_flag`]), where its
/// recorded `--help` fixture proves it. A CLI without one gets a command that
/// cannot start rather than an invented argv: the run fails fast and the log
/// says why. Tool lock-down flags and per-CLI argv shapes are S2.3b/S2.4.
fn command_builder(task: &AgentTask, sandbox: &boba_agent::mcp::config::RunSandbox) -> CommandSpec {
    let kind = CliKind::ALL
        .into_iter()
        .find(|kind| kind.id() == task.cli)
        .unwrap_or_else(|| {
            tracing::warn!(cli = %task.cli, "unknown agent CLI; the run cannot start");
            CliKind::Aider
        });

    let Some(flag) = kind.mcp_config_flag() else {
        tracing::warn!(
            cli = %kind.id(),
            "no minimal argv for this CLI yet (its MCP flag is not `--flag <path>`); \
             it lands with the driver argv work, so this run fails instead of guessing"
        );
        return CommandSpec {
            program: PathBuf::from(format!("boba-mcp-unsupported-cli-{}", kind.id())),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
        };
    };

    CommandSpec {
        program: PathBuf::from(kind.binary_names()[0]),
        args: vec![
            flag.to_string(),
            sandbox.config.display().to_string(),
            task.prompt.clone(),
        ],
        // The runtime sets `cwd` to the run's workspace and adds the endpoint
        // and token to the environment; argv never carries secrets.
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
        other => panic!("`{other}` is in the catalog but has no owner in RepoTools"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use boba_agent::mcp::config::RunSandbox;
    use boba_agent::mcp::config::Workspace;
    use serde_json::json;

    /// A sandbox whose config path is a fixed string: nothing is written here.
    fn sandbox_config() -> SandboxConfig {
        SandboxConfig {
            runs_root: PathBuf::from("/tmp/boba-ch5/runs"),
            app_data: PathBuf::from("/tmp/boba-ch5"),
            shim: PathBuf::from("/tmp/boba-ch5/boba-mcp"),
        }
    }

    fn task(cli: &str) -> AgentTask {
        AgentTask {
            kind: boba_app::ports::agent::AgentTaskKind::ExtractPostings,
            input_trust: boba_app::ports::agent::InputTrust::Untrusted,
            cli: cli.to_string(),
            model: None,
            prompt: "extract the offers".to_string(),
            budget: boba_app::ports::agent::RunBudget::default(),
            env: Vec::new(),
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
        let dir = tmp_dir(tag);
        let key = boba_store::keys::DbKey::from_hex(
            "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff",
        )
        .expect("test key");
        let store = Arc::new(
            Store::open_with_key(
                boba_store::db::StoreConfig::new(dir.join("boba.db"), dir.join("snapshots")),
                key,
            )
            .expect("store"),
        );
        RepoTools { _store: store }
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

    /// Every catalog tool must route somewhere; an unowned name panics instead
    /// of silently answering.
    #[test]
    #[should_panic(expected = "has no owner in RepoTools")]
    fn an_unowned_catalog_tool_is_a_loud_bug() {
        pending_slice("made_up_tool");
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
    fn the_command_points_the_cli_at_this_runs_mcp_config() {
        for cli in ["claude_code", "gemini_cli"] {
            let config = sandbox_config();
            let workspace = Workspace::create(&config.runs_root, "01RUN").expect("workspace");
            let sandbox = RunSandbox {
                endpoint: "/tmp/boba-ch5/ipc/01RUN.sock".to_string(),
                token: "token".to_string(),
                config: workspace.path().join("mcp.json"),
                workspace,
            };

            let spec = command_builder(&task(cli), &sandbox);
            let flag = CliKind::ALL
                .into_iter()
                .find(|kind| kind.id() == cli)
                .and_then(CliKind::mcp_config_flag)
                .expect("a path flag");
            assert_eq!(
                spec.program,
                PathBuf::from(
                    CliKind::ALL
                        .iter()
                        .find(|k| k.id() == cli)
                        .unwrap()
                        .binary_names()[0]
                )
            );
            assert_eq!(spec.args[0], flag);
            assert_eq!(spec.args[1], sandbox.config.display().to_string());
            assert_eq!(spec.args[2], "extract the offers");
            // Secrets never travel in argv, and the runtime owns cwd.
            assert!(!spec.args.iter().any(|arg| arg.contains("token")));
            assert_eq!(spec.cwd, None);
        }
    }

    #[test]
    fn a_cli_without_a_path_flag_fails_fast_instead_of_guessing_argv() {
        for cli in ["codex", "opencode", "aider"] {
            let workspace =
                Workspace::create(&sandbox_config().runs_root, "01RUN").expect("workspace");
            let sandbox = RunSandbox {
                endpoint: String::new(),
                token: String::new(),
                config: workspace.path().join("mcp.json"),
                workspace,
            };
            let spec = command_builder(&task(cli), &sandbox);
            assert!(
                spec.program.to_string_lossy().contains("unsupported-cli-"),
                "{cli} must not get an invented argv: {:?}",
                spec.program
            );
            assert!(spec.args.is_empty());
        }
    }
}
