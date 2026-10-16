//! CLI drivers (AD-4): one per supported agent CLI.
//!
//! A driver recognises its binary and reads the CLI's `--help` text to probe
//! the capabilities the runtime cares about. Executing the CLI, streaming its
//! events and building argv live in S2.2 (`runtime`/`supervisor`).

pub mod aider;
pub mod claude_code;
pub mod codex;
pub mod gemini;
pub mod opencode;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The agent CLIs Boba knows how to drive (FR-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CliKind {
    ClaudeCode,
    Codex,
    OpenCode,
    GeminiCli,
    Aider,
}

impl CliKind {
    /// Every kind, in the order the UI lists them.
    pub const ALL: [CliKind; 5] = [
        CliKind::ClaudeCode,
        CliKind::Codex,
        CliKind::OpenCode,
        CliKind::GeminiCli,
        CliKind::Aider,
    ];

    /// Stable id used in the DB (`cli_install.kind`) and IPC.
    pub const fn id(self) -> &'static str {
        match self {
            CliKind::ClaudeCode => "claude_code",
            CliKind::Codex => "codex",
            CliKind::OpenCode => "opencode",
            CliKind::GeminiCli => "gemini_cli",
            CliKind::Aider => "aider",
        }
    }

    /// Human-readable name.
    pub const fn display_name(self) -> &'static str {
        match self {
            CliKind::ClaudeCode => "Claude Code",
            CliKind::Codex => "Codex CLI",
            CliKind::OpenCode => "OpenCode",
            CliKind::GeminiCli => "Gemini CLI",
            CliKind::Aider => "Aider",
        }
    }

    /// Executable file names to look for, without a platform extension.
    pub const fn binary_names(self) -> &'static [&'static str] {
        match self {
            CliKind::ClaudeCode => &["claude"],
            CliKind::Codex => &["codex"],
            CliKind::OpenCode => &["opencode"],
            CliKind::GeminiCli => &["gemini"],
            CliKind::Aider => &["aider"],
        }
    }

    /// The flag that points this CLI at a run's MCP config **file**, when that
    /// flag takes the path as its value.
    ///
    /// Flags live with the driver because that is what AD-4 makes the driver's
    /// business, and each value here is checked against the CLI's recorded
    /// `--help` fixture in the test below: nothing is guessed. `None` means the
    /// flag is not a simple `--flag <path>` (Codex takes a config override
    /// string, Aider has no MCP support), so the argv for those CLIs stays with
    /// the driver work rather than being invented in the composition root.
    pub const fn mcp_config_flag(self) -> Option<&'static str> {
        match self {
            CliKind::ClaudeCode | CliKind::GeminiCli => Some("--mcp-config"),
            CliKind::Codex | CliKind::OpenCode | CliKind::Aider => None,
        }
    }

    /// The flags that start **one non-interactive run** with `prompt`, in the
    /// order the CLI documents them, without the MCP-config flag (the caller
    /// adds that). `None` when this CLI's argv is not wired yet.
    ///
    /// Every flag is checked against the CLI's recorded `--help` fixture in the
    /// test below. Both CLIs list the prompt as a positional/option value after
    /// their print flag, and neither documents an end-of-options `--`, so a
    /// prompt that could be read as an option is the caller's to refuse.
    pub fn non_interactive_argv(self, prompt: &str) -> Option<Vec<String>> {
        match self {
            CliKind::ClaudeCode => Some(vec!["--print".to_string(), prompt.to_string()]),
            CliKind::GeminiCli => Some(vec!["--prompt".to_string(), prompt.to_string()]),
            CliKind::Codex | CliKind::OpenCode | CliKind::Aider => None,
        }
    }

    /// Install link shown for a "Not installed" CLI (FR-2).
    pub const fn install_url(self) -> &'static str {
        match self {
            CliKind::ClaudeCode => "https://docs.anthropic.com/en/docs/claude-code/setup",
            CliKind::Codex => "https://github.com/openai/codex",
            CliKind::OpenCode => "https://opencode.ai/docs/",
            CliKind::GeminiCli => "https://github.com/google-gemini/gemini-cli",
            CliKind::Aider => "https://aider.chat/docs/install.html",
        }
    }
}

/// Capabilities probed at detection, never hardcoded (AD-4, architecture 5.3).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub stream_json: bool,
    pub mcp: bool,
    pub tool_allowlist: bool,
    pub budget_flag: bool,
    pub turn_flag: bool,
    pub structured_output: bool,
}

impl Capabilities {
    /// Canonical JSON, stored in `cli_install.capabilities_json` (AD-4).
    pub fn to_json(self) -> String {
        serde_json::to_string(&self).expect("Capabilities is plain data")
    }
}

/// Auth/health from the non-interactive version probe (FR-2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthStatus {
    Healthy,
    /// The CLI ran but reported it needs a login or an API key.
    AuthRequired,
    /// The CLI ran but failed for another reason.
    Unhealthy(String),
}

/// A resolved, probed CLI installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliInstall {
    pub kind: CliKind,
    pub path: PathBuf,
    pub version: String,
    pub health: HealthStatus,
    pub capabilities: Capabilities,
}

/// The detection half of the `CliDriver` contract (architecture 5.3).
pub trait CliDriver: Send + Sync {
    fn kind(&self) -> CliKind;

    /// Capabilities read from the CLI's `--help` text.
    fn capabilities(&self, help: &str) -> Capabilities;

    /// Health from the version probe. Default: a successful probe is healthy;
    /// otherwise login/API-key words in the output mean `AuthRequired`.
    fn health(&self, version_ok: bool, output: &str) -> HealthStatus {
        if version_ok {
            return HealthStatus::Healthy;
        }
        const AUTH_HINTS: [&str; 6] = [
            "log in",
            "login",
            "not authenticated",
            "api key",
            "api_key",
            "unauthorized",
        ];
        let lower = output.to_ascii_lowercase();
        if AUTH_HINTS.iter().any(|hint| lower.contains(hint)) {
            HealthStatus::AuthRequired
        } else {
            HealthStatus::Unhealthy(output.trim().to_string())
        }
    }
}

/// The drivers Boba ships, one per `CliKind`, in `CliKind::ALL` order.
pub fn registry() -> Vec<Box<dyn CliDriver>> {
    vec![
        Box::new(claude_code::ClaudeCodeDriver),
        Box::new(codex::CodexDriver),
        Box::new(opencode::OpenCodeDriver),
        Box::new(gemini::GeminiDriver),
        Box::new(aider::AiderDriver),
    ]
}

/// True when every needle appears in `help`.
pub(crate) fn has_all(help: &str, needles: &[&str]) -> bool {
    needles.iter().all(|needle| help.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_covers_every_kind_in_order() {
        let kinds: Vec<CliKind> = registry().iter().map(|driver| driver.kind()).collect();
        assert_eq!(kinds, CliKind::ALL.to_vec());
    }

    #[test]
    fn capabilities_serialize_to_json_with_every_flag() {
        let json = Capabilities {
            stream_json: true,
            ..Capabilities::default()
        }
        .to_json();
        for key in [
            "stream_json",
            "mcp",
            "tool_allowlist",
            "budget_flag",
            "turn_flag",
            "structured_output",
        ] {
            assert!(json.contains(key), "missing {key} in {json}");
        }
    }

    #[test]
    fn health_classifies_auth_errors() {
        let driver = aider::AiderDriver;
        assert_eq!(driver.health(true, ""), HealthStatus::Healthy);
        assert_eq!(
            driver.health(false, "error: not authenticated, run log in"),
            HealthStatus::AuthRequired
        );
        assert_eq!(
            driver.health(false, " segfault "),
            HealthStatus::Unhealthy("segfault".to_string())
        );
    }

    #[test]
    fn the_mcp_config_flag_is_the_one_the_cli_documents() {
        // Evidence, not invention: the flag must appear in the recorded help.
        for (kind, fixture) in [
            (
                CliKind::ClaudeCode,
                include_str!("fixtures/claude_code_help.txt"),
            ),
            (CliKind::GeminiCli, include_str!("fixtures/gemini_help.txt")),
        ] {
            let flag = kind.mcp_config_flag().expect("a path-taking MCP flag");
            assert!(
                fixture.contains(flag),
                "{} does not document {flag}",
                kind.display_name()
            );
            assert!(
                fixture.contains(&format!("{flag} <")),
                "{flag} for {} does not take a value",
                kind.display_name()
            );
        }
        for kind in [CliKind::Codex, CliKind::OpenCode, CliKind::Aider] {
            assert_eq!(kind.mcp_config_flag(), None, "{}", kind.display_name());
        }
    }

    #[test]
    fn the_non_interactive_argv_is_the_one_the_cli_documents() {
        // Evidence, not invention: a run must be non-interactive or it hangs
        // until the wall-clock limit, and each flag must be in the recorded help.
        let claude = include_str!("fixtures/claude_code_help.txt");
        let argv = CliKind::ClaudeCode
            .non_interactive_argv("do the thing")
            .expect("claude argv");
        assert_eq!(argv, vec!["--print", "do the thing"]);
        assert!(claude.contains("--print"));
        assert!(claude.contains("[prompt]"), "the prompt is positional");

        let gemini = include_str!("fixtures/gemini_help.txt");
        let argv = CliKind::GeminiCli
            .non_interactive_argv("do the thing")
            .expect("gemini argv");
        assert_eq!(argv, vec!["--prompt", "do the thing"]);
        assert!(gemini.contains("--prompt"));

        for kind in [CliKind::Codex, CliKind::OpenCode, CliKind::Aider] {
            assert_eq!(kind.non_interactive_argv("x"), None, "{}", kind.id());
        }
    }

    #[test]
    fn install_urls_are_present() {
        for kind in CliKind::ALL {
            assert!(kind.install_url().starts_with("https://"));
            assert!(!kind.display_name().is_empty());
            assert!(!kind.binary_names().is_empty());
        }
    }
}
