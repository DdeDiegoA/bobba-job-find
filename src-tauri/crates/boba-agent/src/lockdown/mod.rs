//! The `tool_lockdown` capability and the untrusted-task hard gate (ADR-010).
//!
//! ADR-010 made `tool_lockdown` a **probed** capability: an MCP config can add
//! tools, it can never take a CLI's own native web, shell or file tools away. A
//! task that reads third-party content (`extract_postings`, `narrate_scores`,
//! `tailor_application`, `review_application`, `classify_emails`) therefore runs
//! only on a CLI whose lockdown this build can verify, for the version it was
//! verified against. Everything else is refused — a hard gate, no user override
//! — while the two tasks that only read the user's own input keep running with a
//! security badge.
//!
//! The gate is a pure decision over evidence the driver probes (`--help`
//! capabilities), the version `--version` reported, and whether the runtime has
//! **observed** the locked-down CLI agree to the pinning. The caller (the
//! composition root, which owns the `cli_install` row) enforces it.
//!
//! Ruling (c), orquestador 2026-10-16: no CLI is verified until the run-time
//! observation ADR-010 §1 asks for exists (the CLI's init event listing only
//! boba tools, wired in S2.4), so **no untrusted task is lifted yet** — the
//! verdict takes [`LockdownConfirmation`] as an explicit input and the
//! composition root passes [`LockdownConfirmation::NotObserved`] until S2.4
//! supplies [`LockdownConfirmation::Observed`].

use crate::drivers::{Capabilities, CliKind};
use boba_app::ports::agent::AgentTaskKind;

/// How a CLI's lockdown is verified (ADR-010 §1, in order of preference).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockdownMethod {
    /// The init event lists the available tools and they are all `boba` tools
    /// (Claude Code `stream-json`), checked on the run's own first event.
    InitEventToolList,
    /// A per-version lockdown config is pinned in the driver and covered by
    /// recorded fixtures (Gemini `excludeTools`, OpenCode permission deny).
    PinnedLockdownConfig,
    /// A sandbox mode blocks network and writes outside the run workspace
    /// (Codex `--sandbox read-only`).
    SandboxMode,
}

/// Why a CLI install has no verified `tool_lockdown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unverified {
    /// This driver has no verified lockdown config for this CLI at all.
    NoVerifiedConfig,
    /// The version is outside the range the driver's fixtures cover, so
    /// `tool_lockdown` is lost until the driver is updated (ADR-010 §1). The
    /// payload is the verified `(major, minor)` line.
    VersionOutsideVerifiedRange { verified: (u64, u64) },
    /// The pinned config exists for this version, but nothing has watched the
    /// CLI agree to it: the init-event observation is S2.4's (ruling (c),
    /// 2026-10-16), and until it lands no untrusted task runs.
    Unobserved,
}

/// Whether the run-time observation ADR-010 §1 asks for has confirmed this
/// CLI's lockdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockdownConfirmation {
    /// The runtime watched this CLI's init event and it listed only boba tools.
    Observed,
    /// No observer exists yet (S2.4): the pinned config is in place, but nothing
    /// has seen the CLI agree to it, so nothing untrusted runs (ruling (c)).
    NotObserved,
}

/// The lockdown verdict for one CLI install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lockdown {
    Verified(LockdownMethod),
    Unverified(Unverified),
}

/// Whether a task may run on a CLI install (ADR-010 §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskSupport {
    /// Runs.
    Supported,
    /// Runs, but this CLI cannot be trusted with third-party content, so the UI
    /// shows it with a security badge.
    SupportedWithBadge(Unverified),
    /// Refused: the hard gate. `fallback` is what still runs instead.
    Unsupported {
        reason: Unverified,
        fallback: Option<Fallback>,
    },
}

impl TaskSupport {
    /// Whether the task may be dispatched at all.
    pub const fn is_supported(self) -> bool {
        !matches!(self, TaskSupport::Unsupported { .. })
    }
}

/// What runs when an untrusted task is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fallback {
    /// The deterministic email rules rank and label, without the LLM
    /// (ADR-010 §2).
    DeterministicRules,
}

/// The lockdown verdict for `cli` at `version`, given its probed capabilities
/// and whether the run-time observation has confirmed it.
pub fn tool_lockdown(
    cli: CliKind,
    version: &str,
    capabilities: &Capabilities,
    confirmation: LockdownConfirmation,
) -> Lockdown {
    let Some(evidence) = lockdown_evidence(cli) else {
        return Lockdown::Unverified(Unverified::NoVerifiedConfig);
    };
    // A CLI whose probed help no longer carries the flags the pinned config
    // needs cannot be held to it: nothing to verify, so nothing to trust.
    if !(evidence.required)(capabilities) {
        return Lockdown::Unverified(Unverified::NoVerifiedConfig);
    }
    match version_line(version) {
        Some(line) if line == evidence.verified => match confirmation {
            LockdownConfirmation::Observed => Lockdown::Verified(evidence.method),
            LockdownConfirmation::NotObserved => Lockdown::Unverified(Unverified::Unobserved),
        },
        _ => Lockdown::Unverified(Unverified::VersionOutsideVerifiedRange {
            verified: evidence.verified,
        }),
    }
}

/// Whether `task` may run on this CLI install, and if not, why.
pub fn support(
    task: AgentTaskKind,
    cli: CliKind,
    version: &str,
    capabilities: &Capabilities,
    confirmation: LockdownConfirmation,
) -> TaskSupport {
    match (
        task.reads_untrusted(),
        tool_lockdown(cli, version, capabilities, confirmation),
    ) {
        (_, Lockdown::Verified(_)) => TaskSupport::Supported,
        (false, Lockdown::Unverified(reason)) => TaskSupport::SupportedWithBadge(reason),
        (true, Lockdown::Unverified(reason)) => TaskSupport::Unsupported {
            reason,
            fallback: fallback_for(task),
        },
    }
}

/// What a driver can pin to lock its CLI down, or `None` while it has nothing.
///
/// `verified` is the `(major, minor)` line the driver's fixtures cover: a newer
/// release keeps its lockdown only once the driver is updated and re-verified.
/// Having this evidence is necessary but not sufficient: the run-time
/// observation ([`LockdownConfirmation`]) decides.
struct Evidence {
    method: LockdownMethod,
    /// Capabilities the config being pinned depends on.
    required: fn(&Capabilities) -> bool,
    verified: (u64, u64),
}

fn lockdown_evidence(cli: CliKind) -> Option<Evidence> {
    match cli {
        // The most a CLI's own flags can say, both halves documented in the
        // recorded `--help` and checked in the test below: `--strict-mcp-config`
        // (only the servers in our config, i.e. `boba` alone) and `--allowedTools`
        // listing the run's own boba tools. That is *not* proof that the CLI's
        // native tools are gone: an allowlist pre-approves, it does not remove,
        // so this evidence alone never verifies a run — the run-time observation
        // is what says the tool set really is ours (reviewer round 2; ruling (c):
        // that observation is S2.4's).
        CliKind::ClaudeCode => Some(Evidence {
            method: LockdownMethod::PinnedLockdownConfig,
            required: |caps| caps.mcp && caps.tool_allowlist,
            verified: (2, 1),
        }),
        // Codex's `--sandbox read-only`, OpenCode's permission deny and Aider's
        // complete lack of MCP are not verified by this build yet: their argv is
        // not wired either, so they stay unsupported rather than assumed safe.
        // Gemini probes `--exclude-tools`, but the tool list that would pin its
        // lockdown is not covered by a recorded fixture yet, and an incomplete
        // exclusion list is worse than none.
        CliKind::GeminiCli | CliKind::Codex | CliKind::OpenCode | CliKind::Aider => None,
    }
}

/// The flags that narrow this CLI's tool set for one run's task, or `None` when
/// this build has no documented way to narrow it.
///
/// Every flag here is checked against the driver's recorded `--help` fixture in
/// the test below: nothing is guessed (AD-4). They are necessary but **not
/// sufficient** for a verified lockdown — an allowlist is not a removal — so the
/// caller adds them to every run's argv (a narrower tool set is never worse) and
/// the verdict still waits for the run-time observation (S2.4).
pub fn pinned_args(cli: CliKind, task_id: &str) -> Option<Vec<String>> {
    match cli {
        CliKind::ClaudeCode => {
            // Comma-separated, as its `--help` documents.
            let allowed = boba_mcp_proto::tool::tools_for_task(task_id)
                .into_iter()
                .map(|spec| boba_tool_name(spec.name))
                .collect::<Vec<_>>()
                .join(",");
            Some(vec![
                "--strict-mcp-config".to_string(),
                // `--tools ""` **removes** the CLI's own built-in tools (its
                // `--help`: "Use \"\" to disable all tools"). This is the
                // removal the run-time observation can verify: an allowlist
                // pre-approves, it does not take anything away, so on its own it
                // would leave every native tool in the init event.
                "--tools".to_string(),
                String::new(),
                "--allowedTools".to_string(),
                allowed,
            ])
        }
        CliKind::GeminiCli | CliKind::Codex | CliKind::OpenCode | CliKind::Aider => None,
    }
}

/// ADR-010's check for a CLI verified by its init event: the tool list it
/// reports contains only this app's MCP tools (`mcp__boba__*`, Claude Code's
/// naming for a tool of the server in `boba-mcp-proto`).
///
/// Fail-closed by construction: an empty list, a renamed tool or a single native
/// tool fails the check, so a run is refused rather than started with tools the
/// `untrusted_content` envelope does not protect. If a CLI ever renames its MCP
/// tools, runs are refused (loudly) instead of silently unprotected.
pub fn init_event_is_locked_down(tools: &[String]) -> bool {
    !tools.is_empty() && tools.iter().all(|tool| is_boba_tool(tool))
}

/// How this app's MCP tool is named in a CLI's argv and init event.
fn boba_tool_name(tool: &str) -> String {
    format!("mcp__{}__{tool}", boba_mcp_proto::tool::SERVER_NAME)
}

/// Whether the CLI reported one of this app's MCP tools.
fn is_boba_tool(tool: &str) -> bool {
    tool.starts_with(&format!("mcp__{}__", boba_mcp_proto::tool::SERVER_NAME))
}

/// The `(major, minor)` a version string reports, e.g. `2.1.289 (Claude Code)`.
fn version_line(version: &str) -> Option<(u64, u64)> {
    let mut numbers = version
        .split(|ch: char| !ch.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<u64>().ok());
    Some((numbers.next()?, numbers.next()?))
}

/// What runs when this task is refused (ADR-010 §2).
fn fallback_for(task: AgentTaskKind) -> Option<Fallback> {
    match task {
        AgentTaskKind::ClassifyEmails => Some(Fallback::DeterministicRules),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drivers::Capabilities;
    use boba_app::ports::agent::AgentTaskKind;

    /// The capabilities Claude Code probes from its recorded `--help`.
    fn claude_caps() -> Capabilities {
        Capabilities {
            stream_json: true,
            mcp: true,
            tool_allowlist: true,
            ..Capabilities::default()
        }
    }

    fn gemini_caps() -> Capabilities {
        Capabilities {
            mcp: true,
            tool_allowlist: true,
            ..Capabilities::default()
        }
    }

    const CLAUDE_VERSION: &str = include_str!("../drivers/fixtures/claude_code_version.txt");
    const GEMINI_VERSION: &str = include_str!("../drivers/fixtures/gemini_version.txt");

    #[test]
    fn the_recorded_cli_versions_are_inside_their_verified_range() {
        assert!(matches!(
            tool_lockdown(
                CliKind::ClaudeCode,
                CLAUDE_VERSION,
                &claude_caps(),
                LockdownConfirmation::Observed
            ),
            Lockdown::Verified(LockdownMethod::PinnedLockdownConfig)
        ));
        // Gemini's `--exclude-tools` values are not pinned by a recorded fixture
        // yet, so its lockdown is not verified however good its version is.
        assert_eq!(
            tool_lockdown(
                CliKind::GeminiCli,
                GEMINI_VERSION,
                &gemini_caps(),
                LockdownConfirmation::Observed
            ),
            Lockdown::Unverified(Unverified::NoVerifiedConfig)
        );
    }

    /// Ruling (c), 2026-10-16: nothing is verified until the runtime observation
    /// exists (S2.4), however well the CLI's version and pinned config check out.
    #[test]
    fn nothing_is_verified_until_the_run_time_observation_exists() {
        assert_eq!(
            tool_lockdown(
                CliKind::ClaudeCode,
                CLAUDE_VERSION,
                &claude_caps(),
                LockdownConfirmation::NotObserved
            ),
            Lockdown::Unverified(Unverified::Unobserved)
        );
        for kind in [
            AgentTaskKind::ExtractPostings,
            AgentTaskKind::NarrateScores,
            AgentTaskKind::TailorApplication,
            AgentTaskKind::ReviewApplication,
            AgentTaskKind::ClassifyEmails,
        ] {
            assert!(matches!(
                support(
                    kind,
                    CliKind::ClaudeCode,
                    CLAUDE_VERSION,
                    &claude_caps(),
                    LockdownConfirmation::NotObserved
                ),
                TaskSupport::Unsupported {
                    reason: Unverified::Unobserved,
                    ..
                }
            ));
        }
    }

    #[test]
    fn the_pinned_lockdown_flags_are_the_ones_the_cli_documents() {
        // Evidence, not invention: a flag may be pinned only if the CLI's
        // recorded help lists it, and every tool named must be one of our own.
        let claude_help = include_str!("../drivers/fixtures/claude_code_help.txt");
        for flag in ["--strict-mcp-config", "--tools", "--allowedTools"] {
            assert!(
                claude_help.contains(flag),
                "claude does not document {flag}"
            );
        }
        assert!(
            claude_help.contains("Comma-separated list of allowed tools"),
            "the allowlist separator is documented"
        );
        assert!(
            claude_help.contains(r#"Use "" to disable all tools"#),
            "the removal of the built-in tool set is documented"
        );

        let pinned = pinned_args(CliKind::ClaudeCode, "extract_postings").expect("pinned");
        assert_eq!(pinned[0], "--strict-mcp-config");
        // The removal comes before the allowlist, and takes an empty value.
        assert_eq!(pinned[1], "--tools");
        assert_eq!(pinned[2], "");
        assert_eq!(pinned[3], "--allowedTools");
        let allowed: Vec<&str> = pinned[4].split(',').collect();
        let expected: Vec<String> = boba_mcp_proto::tool::tools_for_task("extract_postings")
            .into_iter()
            .map(|spec| format!("mcp__{}__{}", boba_mcp_proto::tool::SERVER_NAME, spec.name))
            .collect();
        assert_eq!(allowed, expected, "the allowlist is this task's own tools");
        assert_eq!(pinned.len(), 5, "{pinned:?}");

        for cli in [
            CliKind::GeminiCli,
            CliKind::Codex,
            CliKind::OpenCode,
            CliKind::Aider,
        ] {
            assert_eq!(
                pinned_args(cli, "extract_postings"),
                None,
                "{}",
                cli.display_name()
            );
        }
    }

    #[test]
    fn the_init_event_is_what_confirms_claude_is_locked_down() {
        // ADR-010 §1: the init event lists the available tools, and only boba's.
        let only_boba = [
            "mcp__boba__get_task_input".to_string(),
            "mcp__boba__save_document_draft".to_string(),
        ];
        assert!(init_event_is_locked_down(&only_boba));
        for native in ["Bash", "WebFetch", "Write", "Read", "boba"] {
            let mut tools = only_boba.to_vec();
            tools.push(native.to_string());
            assert!(
                !init_event_is_locked_down(&tools),
                "{native} must fail the check"
            );
        }
        // A CLI that reports no tools is not the configuration we verified.
        assert!(!init_event_is_locked_down(&[]));
    }

    #[test]
    fn a_version_outside_the_verified_range_loses_lockdown() {
        for version in ["1.9.0", "2.2.0", "3.0.0", "unknown", ""] {
            assert_eq!(
                tool_lockdown(
                    CliKind::ClaudeCode,
                    version,
                    &claude_caps(),
                    LockdownConfirmation::Observed
                ),
                Lockdown::Unverified(Unverified::VersionOutsideVerifiedRange { verified: (2, 1) }),
                "{version:?} must not keep tool_lockdown"
            );
        }
        // A version inside the line keeps it, whatever the patch suffix.
        assert!(matches!(
            tool_lockdown(
                CliKind::ClaudeCode,
                "claude 2.1.400 (Claude Code)",
                &claude_caps(),
                LockdownConfirmation::Observed
            ),
            Lockdown::Verified(_)
        ));
    }

    #[test]
    fn a_cli_with_no_verified_lockdown_config_never_gets_lockdown() {
        for cli in [CliKind::Aider, CliKind::OpenCode, CliKind::Codex] {
            assert_eq!(
                tool_lockdown(
                    cli,
                    "1.0.0",
                    &Capabilities::default(),
                    LockdownConfirmation::Observed
                ),
                Lockdown::Unverified(Unverified::NoVerifiedConfig),
                "{}",
                cli.display_name()
            );
        }
    }

    #[test]
    fn a_cli_whose_probed_capabilities_no_longer_fit_the_pinned_config_loses_lockdown() {
        // No `--mcp-config`, no stream-json init event: nothing to pin.
        assert_eq!(
            tool_lockdown(
                CliKind::ClaudeCode,
                CLAUDE_VERSION,
                &Capabilities::default(),
                LockdownConfirmation::Observed
            ),
            Lockdown::Unverified(Unverified::NoVerifiedConfig)
        );
        // Without the documented allowlist flag there is nothing to pin.
        let no_allowlist = Capabilities {
            mcp: true,
            ..Capabilities::default()
        };
        assert_eq!(
            tool_lockdown(
                CliKind::ClaudeCode,
                CLAUDE_VERSION,
                &no_allowlist,
                LockdownConfirmation::Observed
            ),
            Lockdown::Unverified(Unverified::NoVerifiedConfig)
        );
    }

    #[test]
    fn every_untrusted_task_is_refused_on_a_cli_without_lockdown() {
        // ADR-010 §2, the whole untrusted class.
        for (kind, fallback) in [
            (AgentTaskKind::ExtractPostings, None),
            (AgentTaskKind::NarrateScores, None),
            (AgentTaskKind::TailorApplication, None),
            (AgentTaskKind::ReviewApplication, None),
            (
                AgentTaskKind::ClassifyEmails,
                Some(Fallback::DeterministicRules),
            ),
        ] {
            assert_eq!(
                support(
                    kind,
                    CliKind::Aider,
                    "1.0.0",
                    &Capabilities::default(),
                    LockdownConfirmation::Observed
                ),
                TaskSupport::Unsupported {
                    reason: Unverified::NoVerifiedConfig,
                    fallback,
                },
                "{:?} must be unsupported on Aider",
                kind
            );
            // And the same on a Full CLI whose version moved out of range.
            assert_eq!(
                support(
                    kind,
                    CliKind::ClaudeCode,
                    "2.2.0",
                    &claude_caps(),
                    LockdownConfirmation::Observed
                ),
                TaskSupport::Unsupported {
                    reason: Unverified::VersionOutsideVerifiedRange { verified: (2, 1) },
                    fallback,
                },
                "{:?} must be unsupported on an unverified version",
                kind
            );
        }
    }

    #[test]
    fn the_user_input_tasks_still_run_with_a_security_badge() {
        for kind in [
            AgentTaskKind::ProfileImport,
            AgentTaskKind::TranslateProfile,
        ] {
            assert_eq!(
                support(
                    kind,
                    CliKind::Aider,
                    "1.0.0",
                    &Capabilities::default(),
                    LockdownConfirmation::Observed
                ),
                TaskSupport::SupportedWithBadge(Unverified::NoVerifiedConfig)
            );
            assert_eq!(
                support(
                    kind,
                    CliKind::ClaudeCode,
                    "2.1.289",
                    &claude_caps(),
                    LockdownConfirmation::Observed
                ),
                TaskSupport::Supported
            );
        }
    }

    /// The badge carries the same reason the gate would refuse the task with, so
    /// the UI can explain this CLI's own gap (ADR-010 §2).
    #[test]
    fn the_badge_keeps_why_the_cli_is_not_verified() {
        assert_eq!(
            support(
                AgentTaskKind::ProfileImport,
                CliKind::ClaudeCode,
                "2.2.0",
                &claude_caps(),
                LockdownConfirmation::Observed
            ),
            TaskSupport::SupportedWithBadge(Unverified::VersionOutsideVerifiedRange {
                verified: (2, 1)
            })
        );
    }

    #[test]
    fn every_task_runs_on_a_verified_full_cli() {
        for kind in [
            AgentTaskKind::ProfileImport,
            AgentTaskKind::ExtractPostings,
            AgentTaskKind::NarrateScores,
            AgentTaskKind::TailorApplication,
            AgentTaskKind::ReviewApplication,
            AgentTaskKind::ClassifyEmails,
            AgentTaskKind::TranslateProfile,
        ] {
            assert_eq!(
                support(
                    kind,
                    CliKind::ClaudeCode,
                    "2.1.289",
                    &claude_caps(),
                    LockdownConfirmation::Observed
                ),
                TaskSupport::Supported,
                "{:?} must run on a Full CLI",
                kind
            );
        }
    }

    #[test]
    fn only_unsupported_tasks_are_held_back() {
        assert!(
            !TaskSupport::Unsupported {
                reason: Unverified::NoVerifiedConfig,
                fallback: None,
            }
            .is_supported()
        );
        assert!(TaskSupport::Supported.is_supported());
        assert!(TaskSupport::SupportedWithBadge(Unverified::NoVerifiedConfig).is_supported());
    }

    /// The two tables that decide a run's safety must agree: a CLI can be
    /// verified only if there is a pinned config to verify it against, so no run
    /// is ever "verified" with nothing to pin.
    #[test]
    fn every_cli_that_could_be_verified_has_a_pinned_config() {
        for cli in CliKind::ALL {
            if lockdown_evidence(cli).is_some() {
                assert!(
                    pinned_args(cli, "extract_postings").is_some(),
                    "{}: verifiable but nothing to pin",
                    cli.display_name()
                );
            }
        }
        // And today the pinned config alone verifies nothing (ruling (c)).
        assert!(matches!(
            tool_lockdown(
                CliKind::ClaudeCode,
                CLAUDE_VERSION,
                &claude_caps(),
                LockdownConfirmation::NotObserved
            ),
            Lockdown::Unverified(_)
        ));
    }
}
