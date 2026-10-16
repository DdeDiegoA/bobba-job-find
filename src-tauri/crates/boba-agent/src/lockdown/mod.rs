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
//! capabilities) and the version `--version` reported; the caller (the
//! composition root, which owns the `cli_install` row) enforces it.

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

/// The lockdown verdict for `cli` at `version`, given its probed capabilities.
pub fn tool_lockdown(cli: CliKind, version: &str, capabilities: &Capabilities) -> Lockdown {
    let Some(evidence) = verified_evidence(cli) else {
        return Lockdown::Unverified(Unverified::NoVerifiedConfig);
    };
    // A CLI whose probed help no longer carries the flags the pinned config
    // needs cannot be held to it: nothing to verify, so nothing to trust.
    if !(evidence.required)(capabilities) {
        return Lockdown::Unverified(Unverified::NoVerifiedConfig);
    }
    match version_line(version) {
        Some(line) if line == evidence.verified => Lockdown::Verified(evidence.method),
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
) -> TaskSupport {
    match (
        task.reads_untrusted(),
        tool_lockdown(cli, version, capabilities),
    ) {
        (_, Lockdown::Verified(_)) => TaskSupport::Supported,
        (false, Lockdown::Unverified(_)) => {
            TaskSupport::SupportedWithBadge(Unverified::NoVerifiedConfig)
        }
        (true, Lockdown::Unverified(reason)) => TaskSupport::Unsupported {
            reason,
            fallback: fallback_for(task),
        },
    }
}

/// The lock-down evidence a driver has, or `None` while it has none.
///
/// `verified` is the `(major, minor)` line the driver's fixtures cover: a newer
/// release keeps its lockdown only once the driver is updated and re-verified.
struct Evidence {
    method: LockdownMethod,
    /// Capabilities the config being pinned depends on.
    required: fn(&Capabilities) -> bool,
    verified: (u64, u64),
}

fn verified_evidence(cli: CliKind) -> Option<Evidence> {
    match cli {
        // The init event names the tools, and the pinned config allows only
        // `boba` ones; both need the stream-json and MCP flags.
        CliKind::ClaudeCode => Some(Evidence {
            method: LockdownMethod::InitEventToolList,
            required: |caps| caps.stream_json && caps.mcp,
            verified: (2, 1),
        }),
        // `excludeTools` is what pins Gemini's lockdown.
        CliKind::GeminiCli => Some(Evidence {
            method: LockdownMethod::PinnedLockdownConfig,
            required: |caps| caps.mcp && caps.tool_allowlist,
            verified: (0, 4),
        }),
        // Codex's `--sandbox read-only`, OpenCode's permission deny and Aider's
        // complete lack of MCP are not verified by this build yet: their argv is
        // not wired either, so they stay unsupported rather than assumed safe.
        CliKind::Codex | CliKind::OpenCode | CliKind::Aider => None,
    }
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
            tool_lockdown(CliKind::ClaudeCode, CLAUDE_VERSION, &claude_caps()),
            Lockdown::Verified(LockdownMethod::InitEventToolList)
        ));
        assert!(matches!(
            tool_lockdown(CliKind::GeminiCli, GEMINI_VERSION, &gemini_caps()),
            Lockdown::Verified(LockdownMethod::PinnedLockdownConfig)
        ));
    }

    #[test]
    fn a_version_outside_the_verified_range_loses_lockdown() {
        for version in ["1.9.0", "2.2.0", "3.0.0", "unknown", ""] {
            assert_eq!(
                tool_lockdown(CliKind::ClaudeCode, version, &claude_caps()),
                Lockdown::Unverified(Unverified::VersionOutsideVerifiedRange { verified: (2, 1) }),
                "{version:?} must not keep tool_lockdown"
            );
        }
        // A version inside the line keeps it, whatever the patch suffix.
        assert!(matches!(
            tool_lockdown(
                CliKind::ClaudeCode,
                "claude 2.1.400 (Claude Code)",
                &claude_caps()
            ),
            Lockdown::Verified(_)
        ));
    }

    #[test]
    fn a_cli_with_no_verified_lockdown_config_never_gets_lockdown() {
        for cli in [CliKind::Aider, CliKind::OpenCode, CliKind::Codex] {
            assert_eq!(
                tool_lockdown(cli, "1.0.0", &Capabilities::default()),
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
                &Capabilities::default()
            ),
            Lockdown::Unverified(Unverified::NoVerifiedConfig)
        );
        // Gemini needs its exclude-tools flag to pin the lockdown.
        let no_allowlist = Capabilities {
            mcp: true,
            ..Capabilities::default()
        };
        assert_eq!(
            tool_lockdown(CliKind::GeminiCli, GEMINI_VERSION, &no_allowlist),
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
                support(kind, CliKind::Aider, "1.0.0", &Capabilities::default()),
                TaskSupport::Unsupported {
                    reason: Unverified::NoVerifiedConfig,
                    fallback,
                },
                "{:?} must be unsupported on Aider",
                kind
            );
            // And the same on a Full CLI whose version moved out of range.
            assert_eq!(
                support(kind, CliKind::ClaudeCode, "2.2.0", &claude_caps()),
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
                support(kind, CliKind::Aider, "1.0.0", &Capabilities::default()),
                TaskSupport::SupportedWithBadge(Unverified::NoVerifiedConfig)
            );
            assert_eq!(
                support(kind, CliKind::ClaudeCode, "2.1.289", &claude_caps()),
                TaskSupport::Supported
            );
        }
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
                support(kind, CliKind::ClaudeCode, "2.1.289", &claude_caps()),
                TaskSupport::Supported,
                "{:?} must run on a Full CLI",
                kind
            );
            assert!(support(kind, CliKind::ClaudeCode, "2.1.289", &claude_caps()).is_supported());
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
}
