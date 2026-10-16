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

use boba_agent::drivers::{Capabilities, CliKind};
use boba_agent::lockdown::{self, TaskSupport};
use boba_agent::mcp::config::{RunSandbox, SandboxConfig};
use boba_agent::mcp::server::ToolHandler;
use boba_agent::runtime::{AgentRuntime, Emitter};
use boba_agent::supervisor::CommandSpec;
use boba_app::ports::agent::{AgentEvent, AgentTask, FailureReason, InputTrust};
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
        .with_command_builder(Arc::new(command_builder(shim.err(), Arc::clone(store))))
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
/// Three refusals come first, all fail-closed:
/// 1. **No sidecar.** Without `boba-mcp` the CLI would run with its own native
///    tools and none of ours, so no run starts (AD-6).
/// 2. **Untrusted input without a verified tool lockdown.** ADR-010 allows a
///    task that reads postings, mail or company pages only on a CLI whose
///    lockdown this build can verify — for the version the driver was verified
///    against and with the config pinned in `boba-agent/src/lockdown`. That
///    gate is a hard one, with no user override; a CLI that fails it is refused
///    rather than "run anyway and hope".
/// 3. **A prompt that could be parsed as an option.** Neither CLI documents an
///    end-of-options `--`, so a prompt starting with `-` is refused instead of
///    guessed at.
///
/// A CLI whose argv is not wired yet (Codex, OpenCode, Aider — their MCP flag
/// is not a plain `--flag <path>`, or they take no prompt) is refused too; that
/// stays the driver's work (S2.4).
fn command_builder(
    shim_error: Option<String>,
    store: Arc<Store>,
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
        if reads_untrusted(task) && !lock_down(&store, kind, task).is_supported() {
            return refusal(
                task,
                "this task reads untrusted content and this CLI has no verified tool lockdown \
                 for its version (ADR-010; there is no user override)",
            );
        }
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
        // The flags that pin this CLI's lockdown for this task. A verified CLI
        // always has them (`lockdown`'s own invariant), and an untrusted task
        // without them must not start at all; a user-input task may run without
        // them on a CLI that cannot be pinned, with the security badge.
        let pinned = lockdown::pinned_args(kind, task.kind.id());
        if reads_untrusted(task) && pinned.is_none() {
            return refusal(task, "this CLI's lockdown config is not pinned");
        }

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
    lockdown::support(task.kind, kind, &version, &capabilities)
}

/// A task that must not run without the ADR-010 gate: either its kind reads
/// third-party content, or it declares untrusted input outright.
fn reads_untrusted(task: &AgentTask) -> bool {
    task.kind.reads_untrusted() || task.input_trust == InputTrust::Untrusted
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
        // A probed Full CLI, so its runs carry the pinned lockdown config.
        let builder = command_builder(
            None,
            store_with_cli("argv", CliKind::ClaudeCode, "2.1.289", claude_caps()),
        );
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

    /// P1: a task that reads third-party text must not start on a CLI whose
    /// lockdown this build cannot verify (ADR-010, "no override").
    #[test]
    fn an_untrusted_task_is_refused_on_an_unverified_cli() {
        // Aider has no verified lockdown at all, and no probed install here.
        let builder = command_builder(None, store("unverified"));
        let sandbox = run_sandbox("unverified");
        let mut untrusted = task("claude_code");
        untrusted.prompt = "summarise this posting".to_string();
        let spec = builder(&untrusted, &sandbox);
        assert!(
            spec.program.to_string_lossy().contains("boba-refused-"),
            "an untrusted task got a live argv: {spec:?}"
        );
        assert!(spec.args.is_empty());

        // A CLI outside its driver's verified version range loses lockdown too.
        let outdated = store_with_cli("outdated", CliKind::ClaudeCode, "2.2.0", claude_caps());
        let builder = command_builder(None, outdated);
        let spec = builder(&untrusted, &sandbox);
        assert!(
            spec.program.to_string_lossy().contains("boba-refused-"),
            "an out-of-range CLI got a live argv: {spec:?}"
        );
    }

    /// The user's own input does not need lockdown, so those tasks still run,
    /// with the security badge the UI shows (ADR-010 §2).
    #[test]
    fn a_user_input_task_still_runs_on_an_unverified_cli() {
        let outdated = store_with_cli("outdated-user", CliKind::ClaudeCode, "2.2.0", claude_caps());
        let builder = command_builder(None, outdated);
        let sandbox = run_sandbox("outdated-user");

        let spec = builder(
            &task_of(
                "claude_code",
                boba_app::ports::agent::AgentTaskKind::ProfileImport,
                InputTrust::User,
            ),
            &sandbox,
        );
        assert_eq!(spec.program, PathBuf::from("claude"), "{spec:?}");
    }

    /// P1: on a Full CLI the untrusted tasks run, with the pinned lockdown
    /// config from `--help` and this task's own tool subset.
    #[test]
    fn a_verified_full_cli_runs_an_untrusted_task_with_its_lockdown_pinned() {
        let full = store_with_cli("full", CliKind::ClaudeCode, "2.1.289", claude_caps());
        let builder = command_builder(None, full);
        let sandbox = run_sandbox("full");

        let mut untrusted = task("claude_code");
        untrusted.prompt = "extract the offers".to_string();
        let spec = builder(&untrusted, &sandbox);

        assert_eq!(spec.program, PathBuf::from("claude"), "{spec:?}");
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
        assert_eq!(
            allowed,
            "mcp__boba__get_task_input,mcp__boba__get_pending_postings,mcp__boba__propose_posting_features",
            "the allowlist must be this task's own tools, never more"
        );
    }

    /// P1: no sidecar means no tools, so no run may start.
    #[test]
    fn a_missing_shim_refuses_every_run() {
        let builder = command_builder(
            Some("the boba-mcp sidecar is missing".to_string()),
            store("no-shim-store"),
        );
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
        let builder = command_builder(None, store("dash-prompt-store"));
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
        let builder = command_builder(None, store("no-argv"));
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
