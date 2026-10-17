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
//! What is live after this slice: every run gets its own workspace, token and
//! MCP endpoint, its own CLI config, the rendered agent pack, an event stream the
//! UI can follow, and a CLI command built from the driver's evidence **and**
//! ADR-010's two halves — the gate decides whether an untrusted task may start
//! from the probed `cli_install` row ([`lock_down`]), and the runtime refuses the
//! run unless its own init event lists only this app's tools.
//! What is deliberately *not* live: the data behind most catalog tools (profile,
//! postings, scores, mail, documents land in later slices), and a verified
//! lock-down for the CLIs whose driver has nothing to pin them with. Gaps are
//! explicit refusals here rather than silence — see [`RepoTools`] and
//! [`command_builder`].

use std::path::{Path, PathBuf};
use std::sync::Arc;

use boba_agent::drivers::{Capabilities, CliKind, McpWiring};
use boba_agent::lockdown::{self, TaskSupport, Unverified};
use boba_agent::mcp::config::{RunSandbox, SandboxConfig};
use boba_agent::mcp::server::ToolHandler;
use boba_agent::runtime::{AgentRuntime, Emitter, TaskGate};
use boba_agent::supervisor::CommandSpec;
use boba_app::ports::agent::{AgentEvent, AgentTask, FailureReason};
use boba_app::ports::clock::ClockPort;
use boba_app::ports::repo::apply::ApplyRepo;
use boba_app::ports::repo::ingestion::IngestionRepo;
use boba_app::ports::repo::profile::ProfileRepo;
use boba_app::ports::repo::scoring::ScoringRepo;
use boba_app::posting_features;
use boba_app::tailoring;
use boba_mcp_proto::tool;
use boba_store::db::Store;
use boba_store::repos::cli_install;
use serde_json::{Value, json};
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
                store: Arc::clone(store),
            }),
        )
        .with_command_builder(Arc::new(command_builder(shim.clone())))
        // ADR-010 §2: every task goes through this gate before a run exists.
        // The gate is only the first half; the run's own init event is the
        // second, and the runtime enforces it before any tool call.
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

/// The minimal CLI invocation for a task: the CLI, how it reaches this run's
/// `boba-mcp` server, the pinned lockdown config, its non-interactive flags, and
/// the prompt.
///
/// Two refusals come first, both fail-closed:
/// 1. **No sidecar.** Without `boba-mcp` the CLI would run with its own native
///    tools and none of ours, so no run starts (AD-6).
/// 2. **A prompt that could be parsed as an option.** No CLI documents an
///    end-of-options `--`, so a prompt starting with `-` is refused instead of
///    guessed at.
///
/// What a task is allowed to *do* is not decided here: ADR-010's gate runs in
/// the runtime, before a run exists ([`task_gate`]), and the runtime re-checks
/// the CLI's own init event before an untrusted run continues.
///
/// A CLI that cannot reach our MCP server at all (Aider) is refused here: a run
/// on it could neither read its input nor write its output.
fn command_builder(
    shim: Result<PathBuf, String>,
) -> impl Fn(&AgentTask, &RunSandbox) -> CommandSpec + Send + Sync {
    move |task: &AgentTask, sandbox: &RunSandbox| {
        let shim = match &shim {
            Ok(path) => path,
            Err(why) => return refusal(task, why),
        };
        if task.prompt.starts_with('-') {
            return refusal(
                task,
                "the prompt starts with '-', which this CLI would read as an option",
            );
        }
        let Some(kind) = CliKind::ALL.into_iter().find(|kind| kind.id() == task.cli) else {
            return refusal(task, "unknown agent CLI");
        };
        let Some(argv) = kind.non_interactive_argv(&task.prompt) else {
            return refusal(task, "this CLI has no non-interactive invocation");
        };
        // How this run reaches `boba-mcp` (AD-6). Two of the four need the CLI
        // to be told where the server is; the token and the endpoint always
        // travel in the process environment, never in argv.
        let wiring = match kind.mcp_wiring() {
            McpWiring::ConfigFile(flag) => {
                vec![flag.to_string(), sandbox.config.display().to_string()]
            }
            // Codex takes config overrides; its own help names this one.
            McpWiring::Overrides => vec![
                "-c".to_string(),
                format!(
                    "mcp_servers.{}.command={}",
                    tool::SERVER_NAME,
                    shim.display()
                ),
            ],
            // The CLI reads its own config file from the run's workspace, which
            // the runtime wrote when it prepared the run.
            McpWiring::WorkspaceFile(_) => Vec::new(),
            McpWiring::Unsupported => {
                return refusal(
                    task,
                    "this CLI has no MCP support, so a run could not reach Boba's tools",
                );
            }
        };
        // The flags that pin this CLI's lockdown for this task. A CLI the gate
        // lets an untrusted task through always has them (`lockdown`'s own
        // invariant); a CLI that cannot be pinned only ever runs the user's own
        // input, with the security badge.
        let pinned = lockdown::pinned_args(kind, task.kind.id());

        CommandSpec {
            program: PathBuf::from(kind.binary_names()[0]),
            args: wiring
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
/// This answers one question — may an untrusted task **start** on this CLI? The
/// question that follows, "did the CLI keep to the pin?", is the run's own init
/// event, checked by the runtime before any tool call (ADR-010 §1). Nothing is
/// cached, so a scheduled run after a restart is checked just as strictly.
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
///
/// A CLI whose version leaves its driver's verified range loses `tool_lockdown`
/// however well it behaved last time (ADR-010 §1).
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
    store: Arc<Store>,
}

impl RepoTools {
    /// `get_pending_postings` (S6.2): the postings still awaiting an agent pass
    /// at the current Profile version. The endpoint puts them in the untrusted
    /// envelope (AD-7).
    fn pending(&self, args: &Value) -> Result<Value, String> {
        let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
        let version = self.store.profile_version().map_err(|e| e.to_string())?;
        let postings = self
            .store
            .features_pending(version, limit)
            .map_err(|e| e.to_string())?;
        Ok(json!({ "postings": postings }))
    }

    /// `propose_posting_features` (S6.2, AD-5): the tool boundary. The payload
    /// is checked here — the contract's shape, and every quote against the
    /// posting's own text — so an invalid call goes back to the agent and costs
    /// it a retry, instead of being dropped silently at the end of the run.
    /// **Storing** happens in `boba_app::posting_features`, which reads this very
    /// call off the run's event stream: what a proposal is stored against (the
    /// Profile version, and the merge with the deterministic claims) is not this
    /// layer's to know.
    fn validate_proposal(&self, args: &Value) -> Result<Value, String> {
        let id = posting_features::proposal_id(args)
            .ok_or_else(|| "`propose_posting_features` needs the posting's `id`".to_string())?;
        let offer = self
            .store
            .get_offer(id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("no posting `{id}`"))?;
        let (accepted, dropped) =
            posting_features::validate_proposal(&offer, args).map_err(|e| e.to_string())?;
        Ok(json!({ "accepted": accepted, "dropped": dropped }))
    }
}

impl RepoTools {
    /// `get_posting` (S7.7): the Offer the run drafts for, as the studio shows
    /// it. Third-party text: the endpoint puts it in the untrusted envelope
    /// (AD-7), so it can never arrive as instructions.
    fn posting(&self, args: &Value) -> Result<Value, String> {
        let id = args
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| "`get_posting` needs the posting's `id`".to_string())?;
        let offer = self
            .store
            .get_offer(id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("no posting `{id}`"))?;
        Ok(json!({
            "id": offer.id,
            "title": offer.title,
            "company": offer.company,
            "lang": offer.lang,
            "locations": offer.locations,
            "work_mode": offer.work_mode.as_str(),
            "money": offer.money,
            "description_text": offer.description_text,
            "requirements": offer.requirements,
            "apply_url": offer.apply.url,
        }))
    }

    /// `get_score_card` (S7.7): the deterministic score the core computed for
    /// this Offer. The numbers are the core's — a run reads them and never
    /// restates them (the tailor skill says so), so only the newest card is
    /// served.
    fn score_card(&self, args: &Value) -> Result<Value, String> {
        let id = args
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| "`get_score_card` needs the posting's `id`".to_string())?;
        let card = self
            .store
            .latest_card(id)
            .map_err(|error| error.to_string())?;
        Ok(json!({ "score_card": card }))
    }
}

impl ToolHandler for RepoTools {
    fn call(
        &self,
        _task: &str,
        application_id: Option<&str>,
        tool_name: &str,
        args: &Value,
    ) -> Result<Value, String> {
        match tool::find(tool_name) {
            // Defence in depth: the endpoint already refuses unknown names.
            None => Err(format!("no tool `{tool_name}`")),
            // And it already answers this one: the run's own input is the run's,
            // not the app's (S2.4).
            Some(spec) if spec.name == "get_task_input" => {
                Err("the run's own input is served by the MCP endpoint itself".to_string())
            }
            // S7.4: the reviewer's critique is stored on the drafts it judged.
            Some(spec) if spec.name == "save_review" => {
                save_review(&self.store, application_id, args)
            }
            // S7.8: the screening answers a run drafted, stored as drafts for
            // the user to approve — for the application the run was started
            // for, whichever one its arguments name.
            Some(spec) if spec.name == "save_answers" => {
                save_answers(&self.store, application_id, args)
            }
            // S6.2: the two tools an `extract_postings` run reads and writes.
            Some(spec) if spec.name == "get_pending_postings" => self.pending(args),
            Some(spec) if spec.name == posting_features::PROPOSE_TOOL => {
                self.validate_proposal(args)
            }
            // S7.7: what a `tailor_application` run reads, and the two drafts it
            // writes. The user's own data (facts, preferences) is not untrusted
            // and the posting is (the endpoint decides the envelope, AD-7).
            Some(spec) if spec.name == "get_profile_facts" => profile_facts(&self.store),
            Some(spec) if spec.name == "get_preferences" => preferences(&self.store),
            Some(spec) if spec.name == "get_posting" => self.posting(args),
            Some(spec) if spec.name == "get_score_card" => self.score_card(args),
            Some(spec) if spec.name == "save_document_draft" => {
                save_document_draft(&self.store, application_id, args)
            }
            Some(spec) => Err(format!(
                "`{}` has no data source yet: it lands with {}",
                spec.name,
                pending_slice(spec.name)
            )),
        }
    }
}

/// `get_profile_facts` (S7.7): the user's confirmed facts, with their stable
/// ids — the only source a tailor run may say anything about the user from
/// (AD-15). Both language variants travel, so the run can see a missing one.
fn profile_facts(store: &Arc<Store>) -> Result<Value, String> {
    let facts = store.facts_current().map_err(|error| error.to_string())?;
    let facts: Vec<Value> = facts
        .iter()
        .map(|fact| {
            json!({
                "id": fact.id,
                "item_id": fact.item_id,
                "kind": fact.kind.as_str(),
                "text_en": fact.text_en,
                "text_es": fact.text_es,
            })
        })
        .collect();
    Ok(json!({ "facts": facts }))
}

/// `get_preferences` (S7.7): the search and application preferences the user
/// saved, exactly as the Preferences screen stores them. A row this build cannot
/// parse is `null`, never an error: the run drafts without them rather than
/// failing on a settings row nobody can read.
fn preferences(store: &Arc<Store>) -> Result<Value, String> {
    let key = crate::commands::preferences::KEY.to_string();
    let raw = store
        .read(move |conn| {
            boba_store::repos::settings::get(conn, &key)
                .map_err(|error| boba_store::db::StoreError::Sqlite(error.to_string()))
        })
        .map_err(|error| error.to_string())?;
    let Some(raw) = raw else {
        return Ok(json!({ "preferences": null }));
    };
    let snapshot: crate::commands::preferences::PreferencesSnapshot =
        serde_json::from_str(&raw).map_err(|error| error.to_string())?;
    Ok(json!({ "preferences": snapshot.values }))
}

/// `save_document_draft` (S7.7): store one draft a tailor run wrote — the CV,
/// then the cover letter — for the application the run was started for.
///
/// The parsing, the no-fabrication backstop (a claim with no `fact_ids` does not
/// deserialize) and the taint live in `boba_app::tailoring::save_agent_draft`;
/// this handler resolves the posting's language for the fallback notice and
/// nothing else.
fn save_document_draft(
    store: &Arc<Store>,
    application_id: Option<&str>,
    args: &Value,
) -> Result<Value, String> {
    let Some(application_id) = application_id else {
        return Err("this run was started for no application".to_string());
    };
    let offer_lang = store
        .application_get(application_id)
        .ok()
        .flatten()
        .and_then(|application| store.get_offer(&application.posting_id).ok().flatten())
        .map(|offer| offer.lang);
    let saved = tailoring::save_agent_draft(
        store.as_ref(),
        application_id,
        args,
        offer_lang.as_deref(),
        &boba_app::system::SystemClock.now_rfc3339(),
    )
    .map_err(|error| error.to_string())?;
    Ok(json!({
        "app_id": application_id,
        "kind": saved.kind.as_str(),
        "version": saved.version,
    }))
}

/// The slice that brings each tool's data, for the tools still waiting for one.
/// A tool answered in [`RepoTools::call`] — `get_task_input`, `save_review`,
/// `get_pending_postings`, `propose_posting_features` — is not listed, and the
/// tests keep both sets in step.
fn pending_slice(tool_name: &str) -> &'static str {
    match tool_name {
        "get_task_input" => "the MCP endpoint (S2.4 serves the run's own input there)",
        "propose_profile_facts" | "propose_translations" => "S3.4 (profile facts)",
        "get_company_research" => {
            "no slice yet (company research is not built, and the tailor skill reads it only when it exists)"
        }
        "save_score_narration" => "S6.5 (narration)",
        "get_email_batch" => "S8.1 (mail)",
        "propose_status_change" => "S8.4 (status proposals)",
        // Never panic on a request path: an unmapped catalog tool is a bug, and
        // the test below fails when one appears.
        _ => UNOWNED,
    }
}

/// `save_review` (S7.4, FR-22/FR-72): store the fresh-context reviewer's
/// critique on the drafts it judged. The core decides that — the agent only
/// describes what is wrong — and a payload it cannot use is refused whole, so
/// the run retries instead of the studio half-showing a critique.
///
/// `application_id` is the application the **run** was started for: a review run
/// cannot annotate another application's drafts, which is refused here as well
/// as at the endpoint.
fn save_review(
    store: &Arc<Store>,
    application_id: Option<&str>,
    args: &Value,
) -> Result<Value, String> {
    let Some(application_id) = application_id else {
        return Err("this run was started for no application".to_string());
    };
    let review = boba_app::review::save_critique(
        store.as_ref(),
        application_id,
        args,
        &boba_app::system::SystemClock.now_rfc3339(),
    )
    .map_err(|error| error.to_string())?;
    Ok(json!({
        "app_id": review.application_id,
        "findings": review.findings.len(),
    }))
}

/// `save_answers` (S7.8, FR-80): store the screening answers one run drafted,
/// as drafts the user approves.
///
/// `application_id` is the application the **run** was started for, and the
/// answers are stored under it: a payload naming another application is refused
/// here as well as at the endpoint, so neither half of that boundary has to
/// trust the other. The defaults a pay or eligibility question is answered with
/// are the user's own — the Salary Floor from Preferences, the eligibility the
/// Profile declares — and the facts an answer may stand on are the profile's
/// **current** ones, so a run can neither invent a value for those questions nor
/// ground an answer in a fact the profile does not have.
fn save_answers(
    store: &Arc<Store>,
    application_id: Option<&str>,
    args: &Value,
) -> Result<Value, String> {
    let Some(application_id) = application_id else {
        return Err("this run was started for no application".to_string());
    };
    let facts = store.facts_current().map_err(|error| error.to_string())?;
    let fact_ids: Vec<String> = facts.iter().map(|fact| fact.id.clone()).collect();
    let defaults = boba_app::answers::Defaults::from_user_settings(&facts, salary_floor(store)?);

    let drafted = boba_app::answers::save_answers(
        store.as_ref(),
        application_id,
        args,
        &defaults,
        &fact_ids,
        &boba_app::system::SystemClock.now_rfc3339(),
    )
    .map_err(|error| error.to_string())?;

    Ok(json!({
        "app_id": drafted.application_id,
        "answers": drafted.answers.len(),
    }))
}

/// The Salary Floor the user set, as `(amount_minor, currency, period)` in the
/// AD-17 money shape. `None` when they set none — the salary question then drops
/// to review instead of being answered.
///
/// Read exactly as the Preferences step reads it: a row this build cannot parse
/// is the defaults, never an error.
fn salary_floor(store: &Arc<Store>) -> Result<Option<(i64, String, String)>, String> {
    let key = crate::commands::preferences::KEY.to_string();
    let raw = store
        .read(move |conn| {
            boba_store::repos::settings::get(conn, &key)
                .map_err(|e| boba_store::db::StoreError::Sqlite(e.to_string()))
        })
        .map_err(|error| error.to_string())?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    let snapshot: crate::commands::preferences::PreferencesSnapshot =
        serde_json::from_str(&raw).unwrap_or_default();
    Ok(snapshot.values.salary_floor.map(|floor| {
        let period = match floor.period {
            crate::commands::preferences::MoneyPeriod::Year => "year",
            crate::commands::preferences::MoneyPeriod::Month => "month",
            crate::commands::preferences::MoneyPeriod::Hour => "hour",
        };
        (floor.amount_minor, floor.currency, period.to_string())
    }))
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
            target_language: "en".to_string(),
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

    /// Whether a catalog tool is answered by this handler instead of refused
    /// with the slice that still owes it.
    fn served(name: &str) -> bool {
        matches!(
            name,
            "get_task_input"
                | "save_review"
                | "save_answers"
                | "get_pending_postings"
                | "get_profile_facts"
                | "get_preferences"
                | "get_posting"
                | "get_score_card"
                | "save_document_draft"
        ) || name == posting_features::PROPOSE_TOOL
    }

    /// A handler over a real (encrypted, temp) store: the same construction the
    /// composition root does, without the keychain.
    fn repo_tools(tag: &str) -> RepoTools {
        repo_tools_over(store(tag))
    }

    /// The same handler over a store the test already holds.
    fn repo_tools_over(store: Arc<Store>) -> RepoTools {
        RepoTools { store }
    }

    /// Store both drafts of one application, as a tailoring run leaves them.
    /// Raw SQL on purpose: the document repository's own tests cover the SQL,
    /// and seeding specific rows is not what this module is being tested for.
    fn drafts(store: &Arc<Store>, application_id: &str) {
        for (id, kind) in [("doc_cv_1", "cv"), ("doc_cover_1", "cover_letter")] {
            let (id, kind, application_id) =
                (id.to_string(), kind.to_string(), application_id.to_string());
            store
                .execute(move |conn| {
                    conn.execute(
                        "INSERT INTO document (id, application_id, kind, lang, template, \
                         model_json, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, 'en', 'cv-classic', '{\"schema_version\":1}', 't', 't')",
                        [id.as_str(), application_id.as_str(), kind.as_str()],
                    )
                    .map(|_| ())
                    .map_err(|e| boba_store::db::StoreError::Sqlite(e.to_string()))
                })
                .expect("the draft is stored");
        }
    }

    /// The critique stored on a document, read straight from its column.
    fn critique(store: &Arc<Store>, id: &str) -> Option<String> {
        let id = id.to_string();
        store
            .read(move |conn| {
                conn.query_row(
                    "SELECT review_json FROM document WHERE id = ?1",
                    [id.as_str()],
                    |r| r.get::<_, Option<String>>(0),
                )
                .map_err(|e| boba_store::db::StoreError::Sqlite(e.to_string()))
            })
            .expect("read the critique")
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

        // save_review (S7.4), save_answers (S7.8) and the two feature tools
        // (S6.2) are implemented, and each has its own tests below; every other
        // catalog tool is still an explicit refusal.
        for spec in tool::TOOLS.iter().filter(|spec| !served(spec.name)) {
            let answer = tools.call("extract_postings", None, spec.name, &json!({}));
            // Never "unknown tool": every catalog entry has a real answer.
            let message = answer.expect_err("no tool has data yet");
            assert!(
                !message.contains("no tool"),
                "{} fell through the handler: {message}",
                spec.name
            );
            assert!(
                message.contains(spec.name)
                    || message.contains("no data source")
                    || message.contains("served by the MCP endpoint"),
                "unhelpful answer for {}: {message}",
                spec.name
            );
        }
    }

    /// A `save_document_draft` payload: the CV of a tailor run.
    fn cv_draft() -> Value {
        json!({
            "app_id": "app_1",
            "doc": {
                "schema_version": 1,
                "lang": "en",
                "contact": {"full_name": "Ada Lovelace"},
                "summary": {"text": "Backend developer.", "fact_ids": ["fact_1"]},
                "sections": [{"heading": "Experience",
                              "items": [{"text": "Shipped an API.", "fact_ids": ["fact_1"]}]}],
                "declared_gaps": []
            }
        })
    }

    /// The document rows one application holds, as `(kind, version, untrusted)`.
    fn document_rows(store: &Arc<Store>, application_id: &str) -> Vec<(String, i64, bool)> {
        let application_id = application_id.to_string();
        store
            .read(move |conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT kind, version, untrusted_origin FROM document \
                         WHERE application_id = ?1 ORDER BY kind, version",
                    )
                    .map_err(|e| boba_store::db::StoreError::Sqlite(e.to_string()))?;
                let rows = stmt
                    .query_map([application_id.as_str()], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    })
                    .map_err(|e| boba_store::db::StoreError::Sqlite(e.to_string()))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| boba_store::db::StoreError::Sqlite(e.to_string()))?;
                Ok(rows
                    .into_iter()
                    .map(|(kind, version, untrusted)| (kind, version, untrusted == 1))
                    .collect())
            })
            .expect("read the documents")
    }

    /// S7.7: the tailor run's draft reaches the store, as the next version of the
    /// application, tainted (ADR-010) and bound to the run's application.
    #[test]
    fn save_document_draft_stores_the_run_draft_and_refuses_another_application() {
        let store = store("tailor-drafts");
        application(&store, "app_1");
        let tools = repo_tools_over(Arc::clone(&store));

        let answer = tools
            .call(
                "tailor_application",
                Some("app_1"),
                "save_document_draft",
                &cv_draft(),
            )
            .expect("the draft is stored");
        assert_eq!(answer["app_id"], "app_1");
        assert_eq!(answer["kind"], "cv");
        assert_eq!(answer["version"], 1);
        assert_eq!(
            document_rows(&store, "app_1"),
            vec![("cv".to_string(), 1, true)],
            "the draft is the application's first CV version, and it is untrusted"
        );

        // A run bound to no application cannot write, whatever its payload says.
        let unbound = tools
            .call(
                "tailor_application",
                None,
                "save_document_draft",
                &cv_draft(),
            )
            .expect_err("no run application, no draft");
        assert!(unbound.contains("no application"), "{unbound}");

        // And a payload naming another application is refused by the use case.
        let mut elsewhere = cv_draft();
        elsewhere["app_id"] = json!("app_2");
        let refused = tools
            .call(
                "tailor_application",
                Some("app_1"),
                "save_document_draft",
                &elsewhere,
            )
            .expect_err("a run disposes only of its own job");
        assert!(refused.contains("app_2"), "{refused}");
        assert_eq!(
            document_rows(&store, "app_1").len(),
            1,
            "nothing else was stored"
        );
    }

    /// S7.7: the facts, the posting and the score card a tailor run reads.
    #[test]
    fn the_tailor_run_reads_the_facts_the_posting_and_the_score_card() {
        let store = store("tailor-reads");
        application(&store, "app_1");
        let fact = boba_domain::profile::ProfileFact {
            id: "fact_1".to_string(),
            item_id: "item_1".to_string(),
            kind: boba_domain::profile::FactKind::Experience,
            text_en: Some("Shipped a billing API.".to_string()),
            text_es: Some("Entregué una API de facturación.".to_string()),
            evidence: None,
            detail_json: "{}".to_string(),
            supersedes: None,
            profile_version: 1,
            provenance: boba_domain::profile::Provenance::User,
            agent_run_id: None,
            created_at: "t".to_string(),
        };
        store
            .execute(move |conn| {
                boba_store::repos::profile::fact_append(conn, &fact)
                    .map_err(|e| boba_store::db::StoreError::Sqlite(e.to_string()))
            })
            .expect("the fact is stored");
        let tools = repo_tools_over(Arc::clone(&store));

        let facts = tools
            .call(
                "tailor_application",
                Some("app_1"),
                "get_profile_facts",
                &json!({}),
            )
            .expect("the facts");
        assert_eq!(facts["facts"][0]["id"], "fact_1");
        assert_eq!(facts["facts"][0]["kind"], "experience");
        assert_eq!(facts["facts"][0]["text_en"], "Shipped a billing API.");

        let posting = tools
            .call(
                "tailor_application",
                Some("app_1"),
                "get_posting",
                &json!({"id": "pst_1"}),
            )
            .expect("the posting");
        assert_eq!(posting["id"], "pst_1");
        assert_eq!(posting["title"], "Backend Engineer");
        assert_eq!(posting["lang"], "en");

        let missing = tools
            .call(
                "tailor_application",
                Some("app_1"),
                "get_posting",
                &json!({"id": "pst_nope"}),
            )
            .expect_err("no such posting");
        assert!(missing.contains("pst_nope"), "{missing}");

        // Nothing has scored this Offer yet: an explicit `null`, not a fake card.
        let card = tools
            .call(
                "tailor_application",
                Some("app_1"),
                "get_score_card",
                &json!({"id": "pst_1"}),
            )
            .expect("the score card slot");
        assert!(card["score_card"].is_null(), "{card}");
    }

    /// S7.7: the preferences the user saved, and `null` when they saved none.
    #[test]
    fn the_tailor_run_reads_the_users_preferences() {
        let store = store("tailor-preferences");
        let tools = repo_tools_over(Arc::clone(&store));

        let none = tools
            .call(
                "tailor_application",
                Some("app_1"),
                "get_preferences",
                &json!({}),
            )
            .expect("no preferences saved");
        assert!(none["preferences"].is_null(), "{none}");

        let key = crate::commands::preferences::KEY.to_string();
        let mut snapshot = crate::commands::preferences::PreferencesSnapshot::default();
        snapshot.values.roles = vec!["Backend engineer".to_string()];
        let json = serde_json::to_string(&snapshot).expect("json");
        store
            .execute(move |conn| {
                boba_store::repos::settings::put(conn, &key, &json, "t")
                    .map_err(|e| boba_store::db::StoreError::Sqlite(e.to_string()))
            })
            .expect("the preferences are stored");

        let saved = tools
            .call(
                "tailor_application",
                Some("app_1"),
                "get_preferences",
                &json!({}),
            )
            .expect("the preferences");
        assert_eq!(saved["preferences"]["roles"][0], "Backend engineer");
    }

    /// S7.4: `save_review` stores the fresh-context reviewer's critique on the
    /// drafts it judged, where the studio reads it back with them.
    #[test]
    fn save_review_stores_the_critique_on_the_drafts() {
        let store = store("review-ok");
        drafts(&store, "app_1");

        let answer = repo_tools_over(Arc::clone(&store))
            .call("review_application",
                Some("app_1"),
                "save_review", &json!({
                "app_id": "app_1",
                "findings": [{"severity": "high", "target": "cv", "message": "unsupported claim"}],
            }))
            .expect("the critique is stored");

        assert_eq!(answer["app_id"], "app_1");
        assert_eq!(answer["findings"], 1, "the critique it stored");
        let cv = critique(&store, "doc_cv_1").expect("the CV carries the critique");
        assert!(cv.contains("unsupported claim"), "{cv}");
        assert!(
            critique(&store, "doc_cover_1").is_some(),
            "both drafts carry the critique"
        );
    }

    /// The app_id-binding rule, per tool (orquestador ruling): a review run
    /// started for one application cannot annotate another, and a run the core
    /// bound to no application cannot review at all.
    #[test]
    fn save_review_refuses_another_application() {
        let store = store("review-binding");
        drafts(&store, "app_1");
        let tools = repo_tools_over(Arc::clone(&store));

        let elsewhere = tools
            .call(
                "review_application",
                Some("app_1"),
                "save_review",
                &json!({
                    "app_id": "app_2",
                    "findings": [{"severity": "high", "target": "cv", "message": "unsupported claim"}],
                }),
            )
            .expect_err("a run may not review another application");
        assert!(elsewhere.contains("app_2"), "{elsewhere}");

        let unbound = tools
            .call(
                "review_application",
                None,
                "save_review",
                &json!({
                    "app_id": "app_1",
                    "findings": [{"severity": "high", "target": "cv", "message": "unsupported claim"}],
                }),
            )
            .expect_err("no run application, no review");
        assert!(unbound.contains("no application"), "{unbound}");

        assert!(
            critique(&store, "doc_cv_1").is_none(),
            "neither refusal wrote anything"
        );
    }

    /// A payload the core cannot use is refused whole, and a critique of an
    /// application with no drafts is refused too: neither writes anything.
    #[test]
    fn save_review_refuses_what_it_cannot_store() {
        let store = store("review-refused");
        drafts(&store, "app_1");
        let tools = repo_tools_over(Arc::clone(&store));

        let unusable = tools
            .call(
                "review_application",
                Some("app_1"),
                "save_review",
                &json!({
                    "app_id": "app_1",
                    "findings": [{"severity": "urgent", "target": "cv", "message": "x"}],
                }),
            )
            .expect_err("an unknown severity is refused");
        assert!(unusable.contains("not usable"), "{unusable}");

        let nothing_to_review = tools
            .call(
                "review_application",
                Some("app_unknown"),
                "save_review",
                &json!({
                    "app_id": "app_unknown",
                    "findings": [],
                }),
            )
            .expect_err("no drafts to review");
        assert!(
            nothing_to_review.contains("no drafts"),
            "{nothing_to_review}"
        );

        assert!(
            critique(&store, "doc_cv_1").is_none(),
            "nothing was written by either refusal"
        );
    }

    /// One application, as S9.1 leaves it: the bank's row hangs off it (AD-16).
    fn application(store: &Arc<Store>, id: &str) {
        let id = id.to_string();
        store
            .execute(move |conn| {
                conn.execute(
                    "INSERT OR IGNORE INTO source (id, name, access_class, legal_tier, \
                     apply_capability, min_interval_ms, enabled, added_at, updated_at) \
                     VALUES ('remotive', 'Remotive', 'public_json', 'safe', 'link_out', 2000, 1, \
                     't', 't')",
                    [],
                )
                .map_err(|e| boba_store::db::StoreError::Sqlite(e.to_string()))?;
                conn.execute(
                    "INSERT OR IGNORE INTO posting (id, source_id, external_id, title, company, \
                     location_json, work_mode, posted_at, deadline, first_seen, last_seen, lang, \
                     description_html, description_text, requirements_json, apply_url, apply_email, \
                     dedup_key, simhash) VALUES ('pst_1', 'remotive', 'abc-1', 'Backend Engineer', \
                     'Boba Labs', '[]', 'remote', NULL, NULL, 't', 't', 'en', '', '', '[]', \
                     'https://jobs.example.com/abc-1', NULL, 'k1', NULL)",
                    [],
                )
                .map_err(|e| boba_store::db::StoreError::Sqlite(e.to_string()))?;
                conn.execute(
                    "INSERT INTO application (id, posting_id, status, offer_snapshot_json, \
                     created_at, updated_at) VALUES (?1, 'pst_1', 'saved', '{}', 't', 't')",
                    [id.as_str()],
                )
                .map(|_| ())
                .map_err(|e| boba_store::db::StoreError::Sqlite(e.to_string()))
            })
            .expect("the application is stored");
    }

    /// One confirmed profile fact, as S3.4 leaves it.
    fn fact(
        store: &Arc<Store>,
        id: &str,
        kind: &str,
        text_en: Option<&str>,
        text_es: Option<&str>,
    ) {
        // An absent variant is the empty string: the profile treats
        // whitespace-only text as no text.
        let (id, kind) = (id.to_string(), kind.to_string());
        let (text_en, text_es) = (
            text_en.unwrap_or_default().to_string(),
            text_es.unwrap_or_default().to_string(),
        );
        store
            .execute(move |conn| {
                conn.execute(
                    "INSERT INTO profile_fact (id, item_id, kind, text_en, text_es, detail_json, \
                     profile_version, provenance, created_at) \
                     VALUES (?1, ?1, ?2, ?3, ?4, '{}', 1, 'user', 't')",
                    [
                        id.as_str(),
                        kind.as_str(),
                        text_en.as_str(),
                        text_es.as_str(),
                    ],
                )
                .map(|_| ())
                .map_err(|e| boba_store::db::StoreError::Sqlite(e.to_string()))
            })
            .expect("the fact is stored");
    }

    /// The preferences document, exactly as the Preferences step stores it
    /// (S3.7): one JSON row in `setting`.
    fn preferences(store: &Arc<Store>, salary_minor: i64) {
        let document = json!({
            "version": 1,
            "values": {
                "roles": ["Backend Engineer"],
                "seniority_min": null,
                "seniority_max": null,
                "salary_floor": {
                    "amount_minor": salary_minor,
                    "currency": "USD",
                    "period": "month",
                    "firmness": "soft"
                },
                "locations": [],
                "timezone_window": {"from": 8, "to": 18},
                "work_modes": {"remote": true, "hybrid": false, "onsite": false},
                "working_languages": ["en"],
                "deal_breaker_keywords": [],
                "deal_breaker_notes": "",
                "excluded_companies": []
            },
            "scores_stale": false
        })
        .to_string();
        store
            .execute(move |conn| {
                conn.execute(
                    "INSERT INTO setting (key, value, updated_at) VALUES (?1, ?2, 't') \
                     ON CONFLICT(key) DO UPDATE SET value = ?2",
                    [crate::commands::preferences::KEY, document.as_str()],
                )
                .map(|_| ())
                .map_err(|e| boba_store::db::StoreError::Sqlite(e.to_string()))
            })
            .expect("the preferences are stored");
    }

    /// The bank rows, read straight from the table as one JSON array.
    fn stored_answers(store: &Arc<Store>) -> Vec<serde_json::Value> {
        let raw: String = store
            .read(|conn| {
                conn.query_row(
                    "SELECT coalesce(json_group_array(json_object('question', question, \
                     'class', class, 'answer', answer, 'needs_review', needs_review, \
                     'untrusted_origin', untrusted_origin, \
                     'approved', approved_at IS NOT NULL)), '[]') \
                     FROM application_answer_bank",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| boba_store::db::StoreError::Sqlite(e.to_string()))
            })
            .expect("read the bank");
        serde_json::from_str(&raw).expect("the bank as JSON")
    }

    /// S7.8: `save_answers` stores what one run drafted and disposes of it the
    /// way FR-80 says — a salary answer is the user's own Salary Floor rather
    /// than the run's prose, a free-text answer stands on the profile's facts,
    /// and every row lands a flagged, untrusted draft the user has to approve.
    #[test]
    fn save_answers_stores_the_drafts_and_the_core_disposes() {
        let store = store("answers-ok");
        application(&store, "app_1");
        fact(&store, "fact_1", "skill", Some("Rust"), None);
        fact(
            &store,
            "fact_2",
            "experience",
            None,
            Some("Seis años en backend"),
        );
        preferences(&store, 600_000);

        let answer = repo_tools_over(Arc::clone(&store))
            .call(
                "answer_questions",
                Some("app_1"),
                "save_answers",
                &json!({
                    "app_id": "app_1",
                    "answers": [
                        {
                            "question": "What are your salary expectations?",
                            "answer": "I would like USD 9999 per month."
                        },
                        {
                            "question": "Do you require visa sponsorship?",
                            "answer": "Yes, please sponsor me."
                        },
                        {
                            "question": "What is your notice period?",
                            "answer": "Four weeks, per my contract.",
                            "fact_ids": ["fact_2"]
                        }
                    ],
                }),
            )
            .expect("the drafts are stored");

        assert_eq!(answer["app_id"], "app_1");
        assert_eq!(answer["answers"], 3);

        let stored = stored_answers(&store);
        assert_eq!(stored.len(), 3, "one row per question");

        // FR-80: the salary value is the Preferences Salary Floor, not the run's
        // own words, and it is flagged for review.
        assert_eq!(stored[0]["class"], "salary");
        assert_eq!(stored[0]["answer"], "USD 6000 per month");
        assert_eq!(stored[0]["needs_review"], 1, "always flagged");

        // Eligibility: no eligibility fact in the profile, so no answer is
        // invented and the question drops to review.
        assert_eq!(stored[1]["class"], "eligibility");
        assert!(stored[1]["answer"].is_null(), "nothing is invented");
        assert_eq!(stored[1]["needs_review"], 1);

        // A free-text answer stands on the fact it names.
        assert_eq!(stored[2]["class"], "other");
        assert_eq!(stored[2]["answer"], "Four weeks, per my contract.");
        assert_eq!(stored[2]["needs_review"], 0);

        // AC3: every row is the run's untrusted draft, and none is approved.
        for row in &stored {
            assert_eq!(row["untrusted_origin"], 1, "ADR-010: the run wrote it");
            assert_eq!(row["approved"], 0, "AC2: only the user approves");
        }
    }

    /// AC1: the eligibility default is the eligibility the Profile declares, in
    /// the language the user wrote it in.
    #[test]
    fn an_eligibility_answer_comes_from_the_profiles_own_declaration() {
        let store = store("answers-eligibility");
        application(&store, "app_1");
        fact(
            &store,
            "fact_1",
            "eligibility",
            None,
            Some("Ciudadanía UE, sin patrocinio"),
        );

        repo_tools_over(Arc::clone(&store))
            .call(
                "answer_questions",
                Some("app_1"),
                "save_answers",
                &json!({
                    "app_id": "app_1",
                    "answers": [{
                        "question": "Are you authorized to work in the EU?",
                        "answer": "I believe so."
                    }],
                }),
            )
            .expect("the draft is stored");

        let stored = stored_answers(&store);
        assert_eq!(stored[0]["class"], "eligibility");
        assert_eq!(stored[0]["answer"], "Ciudadanía UE, sin patrocinio");
        assert_eq!(
            stored[0]["needs_review"], 1,
            "eligibility is always flagged"
        );
    }

    /// A payload the core cannot use is refused whole, and an application
    /// nobody holds cannot be answered for: neither writes anything.
    #[test]
    fn save_answers_refuses_what_it_cannot_store() {
        let store = store("answers-refused");
        application(&store, "app_1");
        fact(&store, "fact_1", "skill", Some("Rust"), None);
        let tools = repo_tools_over(Arc::clone(&store));

        let ungrounded = tools
            .call("answer_questions",
                Some("app_1"),
                "save_answers",
                &json!({
                    "app_id": "app_1",
                    "answers": [{"question": "What is your notice period?", "answer": "Four weeks."}],
                }),
            )
            .expect_err("an answer with no facts is refused");
        assert!(ungrounded.contains("fact_ids"), "{ungrounded}");

        let invented = tools
            .call(
                "answer_questions",
                Some("app_1"),
                "save_answers",
                &json!({
                    "app_id": "app_1",
                    "answers": [{
                        "question": "What is your notice period?",
                        "answer": "Four weeks.",
                        "fact_ids": ["fact_9"]
                    }],
                }),
            )
            .expect_err("an unknown fact is refused");
        assert!(invented.contains("fact_9"), "{invented}");

        // A payload naming another application is refused at the handler too,
        // not only at the endpoint: the run answers for the application it was
        // started for, and for no other.
        let elsewhere = tools
            .call(
                "answer_questions",
                Some("app_1"),
                "save_answers",
                &json!({
                    "app_id": "app_2",
                    "answers": [{
                        "question": "What is your notice period?",
                        "answer": "Four weeks.",
                        "fact_ids": ["fact_1"]
                    }],
                }),
            )
            .expect_err("a run may not answer for another application");
        assert!(elsewhere.contains("app_2"), "{elsewhere}");

        // And a run bound to an application nobody holds is refused as such.
        let unknown = tools
            .call(
                "answer_questions",
                Some("app_gone"),
                "save_answers",
                &json!({
                    "app_id": "app_gone",
                    "answers": [{
                        "question": "What is your notice period?",
                        "answer": "Four weeks.",
                        "fact_ids": ["fact_1"]
                    }],
                }),
            )
            .expect_err("no application, no answer");
        assert!(unknown.contains("unknown application"), "{unknown}");

        // A run the core started for no application at all cannot answer.
        let unbound = tools
            .call(
                "answer_questions",
                None,
                "save_answers",
                &json!({
                    "app_id": "app_1",
                    "answers": [{
                        "question": "What is your notice period?",
                        "answer": "Four weeks.",
                        "fact_ids": ["fact_1"]
                    }],
                }),
            )
            .expect_err("no run application, no answer");
        assert!(unbound.contains("no application"), "{unbound}");

        assert!(
            stored_answers(&store).is_empty(),
            "nothing was written by any refusal"
        );
    }

    /// Every catalog tool must route somewhere: reaching the fallback is a bug.
    /// The tools answered here have no pending slice — `get_task_input` is the
    /// endpoint's (S2.4), `save_review` is S7.4's, `save_answers` is S7.8's, and
    /// the feature tools are S6.2's.
    #[test]
    fn no_catalog_tool_falls_through_to_the_unowned_answer() {
        for spec in tool::TOOLS.iter().filter(|spec| !served(spec.name)) {
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
                tools
                    .call("extract_postings", None, name, &json!({}))
                    .is_err(),
                "{name} must be refused"
            );
        }
    }

    /// The name this slice listens for must be the catalog's (AD-5): the tool
    /// names live in `boba-mcp-proto`, and `boba-app` cannot depend on it.
    #[test]
    fn the_feature_harvest_listens_for_the_catalog_tool() {
        let spec = tool::find(posting_features::PROPOSE_TOOL).expect("in the catalog");
        assert_eq!(spec.name, posting_features::PROPOSE_TOOL);
        assert_eq!(spec.tasks, &[tool::EXTRACT_POSTINGS]);
        assert!(spec.kind.is_write(), "a proposal, not a read");
    }

    /// A run reads its queue and is checked at the tool boundary (AD-5). The
    /// deep checks — the contract's shape, and every quote against the posting's
    /// own text — are `boba_app::posting_features`'; what is wiring's to get
    /// right is the routing, and that a proposal about no stored posting is
    /// refused rather than stored.
    #[test]
    fn the_feature_tools_are_served_and_a_proposal_is_checked() {
        let tools = repo_tools("features");

        let queue = tools
            .call(
                "extract_postings",
                None,
                "get_pending_postings",
                &json!({"limit": 5}),
            )
            .expect("the queue is served");
        assert_eq!(queue, json!({"postings": []}));

        let unknown = tools
            .call(
                "extract_postings",
                None,
                posting_features::PROPOSE_TOOL,
                &json!({"id": "pst_ghost", "features": {}}),
            )
            .expect_err("a proposal about no posting is refused");
        assert!(unknown.contains("pst_ghost"), "{unknown}");

        let no_id = tools
            .call(
                "extract_postings",
                None,
                posting_features::PROPOSE_TOOL,
                &json!({"features": {}}),
            )
            .expect_err("a proposal without an id is refused");
        assert!(no_id.contains("`id`"), "{no_id}");
    }

    /// The endpoint wraps third-party text (AD-7); a handler that pre-wrapped it
    /// would double-envelope every payload, so nothing here may return the key.
    #[test]
    fn no_handler_answer_pre_wraps_untrusted_content() {
        for spec in tool::TOOLS {
            let answer = repo_tools("wrap").call("extract_postings", None, spec.name, &json!({}));
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
        let builder = command_builder(Ok(PathBuf::from("/app/binaries/boba-mcp")));
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

    /// ADR-010's gate, first half (orquestador ruling, 2026-10-16): an untrusted
    /// task is refused on a CLI nothing probed and on a version outside its
    /// driver's range, and is allowed to **start** on a CLI this build can pin.
    /// What the run may then do is decided by its own init event, which the
    /// runtime checks before any tool call.
    #[test]
    fn the_gate_decides_whether_an_untrusted_task_may_start() {
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

        // A Full CLI with its pin in place: the run may start, and the run's own
        // init event is what verifies it from there (ADR-010 §1).
        let full = task_gate(store_with_cli(
            "full",
            CliKind::ClaudeCode,
            "2.1.301",
            claude_caps(),
        ));
        assert_eq!(full(&task("claude_code")), TaskSupport::Supported);

        // A CLI that cannot report the init event the run is verified against
        // can never be pinned, so nothing untrusted starts on it.
        let no_stream = Capabilities {
            stream_json: false,
            ..claude_caps()
        };
        let unverifiable = task_gate(store_with_cli(
            "no-stream",
            CliKind::ClaudeCode,
            "2.1.301",
            no_stream,
        ));
        assert_eq!(
            unverifiable(&task("claude_code")),
            TaskSupport::Unsupported {
                reason: Unverified::NoVerifiedConfig,
                fallback: None,
            }
        );
        // `classify_emails` keeps its deterministic fallback there.
        let mut classify = task("claude_code");
        classify.kind = boba_app::ports::agent::AgentTaskKind::ClassifyEmails;
        assert_eq!(
            unverifiable(&classify),
            TaskSupport::Unsupported {
                reason: Unverified::NoVerifiedConfig,
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

        let builder = command_builder(Ok(PathBuf::from("/app/binaries/boba-mcp")));
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
        let full = store_with_cli("pinned", CliKind::ClaudeCode, "2.1.301", claude_caps());
        let full_gate = task_gate(Arc::clone(&full));
        let builder = command_builder(Ok(PathBuf::from("/app/binaries/boba-mcp")));
        let sandbox = run_sandbox("pinned");

        let user = task_of(
            "claude_code",
            boba_app::ports::agent::AgentTaskKind::ProfileImport,
            InputTrust::User,
        );
        let spec = builder(&user, &sandbox);

        // The gate is what allows it; the argv is what pins it.
        assert!(full_gate(&user).is_supported());
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
        let builder = command_builder(Err("the boba-mcp sidecar is missing".to_string()));
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
        let builder = command_builder(Ok(PathBuf::from("/app/binaries/boba-mcp")));
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

    /// Every CLI that can reach `boba-mcp` gets a live argv; the one that cannot
    /// is refused rather than launched with no tools at all (AD-6).
    #[test]
    fn a_cli_without_mcp_is_refused_and_the_others_are_wired() {
        let builder = command_builder(Ok(PathBuf::from("/app/binaries/boba-mcp")));
        let trusted = |cli: &str| {
            task_of(
                cli,
                boba_app::ports::agent::AgentTaskKind::ProfileImport,
                InputTrust::User,
            )
        };

        // Aider has no MCP support: a run on it could neither read its input nor
        // write its output, so it never gets an invented argv.
        let spec = builder(&trusted("aider"), &run_sandbox("aider"));
        assert!(
            spec.program.to_string_lossy().contains("boba-refused-"),
            "{spec:?}"
        );
        assert!(spec.args.is_empty());

        for cli in ["claude_code", "gemini_cli", "codex", "opencode"] {
            let spec = builder(&trusted(cli), &run_sandbox(&format!("wired-{cli}")));
            assert_eq!(spec.program, PathBuf::from(cli_program(cli)), "{cli}");
            assert!(!spec.args.is_empty(), "{cli}");
        }
    }

    fn cli_program(cli: &str) -> &'static str {
        CliKind::ALL
            .into_iter()
            .find(|kind| kind.id() == cli)
            .expect("a known CLI")
            .binary_names()[0]
    }

    /// Codex takes config overrides instead of a config file, and OpenCode reads
    /// its own config from the workspace. Both name our server; neither puts the
    /// run's token on the command line (AD-6).
    #[test]
    fn the_mcp_server_is_wired_the_way_each_cli_takes_it() {
        let shim = PathBuf::from("/app/binaries/boba-mcp");
        let builder = command_builder(Ok(shim.clone()));
        let trusted = |cli: &str| {
            task_of(
                cli,
                boba_app::ports::agent::AgentTaskKind::ProfileImport,
                InputTrust::User,
            )
        };

        let sandbox = run_sandbox("codex");
        let spec = builder(&trusted("codex"), &sandbox);
        assert_eq!(
            spec.args[0..2],
            [
                "-c".to_string(),
                format!("mcp_servers.boba.command={}", shim.display())
            ],
            "{spec:?}"
        );

        // OpenCode reads the config the runtime wrote into its workspace, so its
        // argv carries no config at all.
        let spec = builder(&trusted("opencode"), &run_sandbox("opencode"));
        assert!(!spec.args.iter().any(|arg| arg.contains("mcp")), "{spec:?}");

        // And for no CLI does the run's token reach argv.
        for cli in ["claude_code", "gemini_cli", "codex", "opencode"] {
            let sandbox = run_sandbox(&format!("token-{cli}"));
            let spec = builder(&trusted(cli), &sandbox);
            assert!(
                !spec.args.iter().any(|arg| arg.contains(&sandbox.token)),
                "{cli} put the run token on the command line"
            );
        }
    }

    /// The workspace config an in-workspace CLI reads: its own file, naming the
    /// shim and carrying the endpoint and token as the shim's environment.
    #[test]
    fn a_workspace_cli_gets_its_own_config_file() {
        let config = CliKind::OpenCode
            .workspace_mcp_config(
                Path::new("/app/binaries/boba-mcp"),
                "/app/ipc/01RUN.sock",
                "a-token",
            )
            .expect("OpenCode reads a workspace config");
        assert_eq!(config.0, "opencode.json");
        assert_eq!(config.1["mcp"]["boba"]["type"], "local");
        assert_eq!(
            config.1["mcp"]["boba"]["command"][0],
            "/app/binaries/boba-mcp"
        );
        assert_eq!(
            config.1["mcp"]["boba"]["environment"]["BOBA_RUN_TOKEN"],
            "a-token"
        );
        assert_eq!(
            config.1["mcp"]["boba"]["environment"]["BOBA_IPC_ENDPOINT"],
            "/app/ipc/01RUN.sock"
        );

        // The CLIs that take their config another way have none.
        for kind in [CliKind::ClaudeCode, CliKind::GeminiCli, CliKind::Codex] {
            assert!(
                kind.workspace_mcp_config(Path::new("/bin/boba-mcp"), "e", "t")
                    .is_none(),
                "{}",
                kind.display_name()
            );
        }
    }
}
