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

use boba_app::ports::agent::AgentEvent;
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
    /// adds that). `None` when this CLI cannot run unattended at all.
    ///
    /// Every flag is checked against the CLI's recorded `--help` fixture in the
    /// test below. Both CLIs list the prompt as a positional/option value after
    /// their print flag, and neither documents an end-of-options `--`, so a
    /// prompt that could be read as an option is the caller's to refuse.
    ///
    /// Claude Code additionally asks for `--output-format stream-json`: that is
    /// what makes the run's init event (and with it ADR-010 §1's lockdown
    /// check) visible to the runtime, and what lets the run report prose instead
    /// of one opaque blob. `--verbose` is required by the CLI for that format
    /// (checked against the fixture).
    pub fn non_interactive_argv(self, prompt: &str) -> Option<Vec<String>> {
        match self {
            CliKind::ClaudeCode => Some(vec![
                "--print".to_string(),
                "--verbose".to_string(),
                "--output-format".to_string(),
                "stream-json".to_string(),
                prompt.to_string(),
            ]),
            CliKind::GeminiCli => Some(vec!["--prompt".to_string(), prompt.to_string()]),
            // Codex, OpenCode and Aider: the prompt goes after the CLI's own
            // non-interactive subcommand.
            CliKind::Codex => Some(vec![
                "exec".to_string(),
                "--json".to_string(),
                "--skip-git-repo-check".to_string(),
                prompt.to_string(),
            ]),
            CliKind::OpenCode => Some(vec![
                "run".to_string(),
                "--format".to_string(),
                "json".to_string(),
                prompt.to_string(),
            ]),
            CliKind::Aider => Some(vec![
                "--message".to_string(),
                prompt.to_string(),
                "--yes-always".to_string(),
                "--no-git".to_string(),
            ]),
        }
    }

    /// How this CLI is told about the run's `boba` MCP server (AD-6). Every CLI
    /// needs one: without it a run could neither read its input nor write its
    /// output.
    pub const fn mcp_wiring(self) -> McpWiring {
        match self {
            CliKind::ClaudeCode | CliKind::GeminiCli => McpWiring::ConfigFile("--mcp-config"),
            CliKind::Codex => McpWiring::Overrides,
            CliKind::OpenCode => McpWiring::WorkspaceFile("opencode.json"),
            // Aider has no MCP support (its recorded `--help` lists none), so a
            // run on it could never reach Boba's tools.
            CliKind::Aider => McpWiring::Unsupported,
        }
    }

    /// The MCP config a `WorkspaceFile` CLI reads from its own workspace, as
    /// `(file name, contents)`. `None` for the CLIs that take their config
    /// another way.
    ///
    /// The shape is the CLI's own, so it lives with the driver: OpenCode reads
    /// `opencode.json` (`mcp.<name>` = a local server, `environment` for the
    /// shim's endpoint and token).
    pub fn workspace_mcp_config(
        self,
        shim: &std::path::Path,
        endpoint: &str,
        token: &str,
    ) -> Option<(&'static str, serde_json::Value)> {
        let McpWiring::WorkspaceFile(file) = self.mcp_wiring() else {
            return None;
        };
        Some((
            file,
            serde_json::json!({
                "$schema": "https://opencode.ai/config.json",
                "mcp": {
                    boba_mcp_proto::tool::SERVER_NAME: {
                        "type": "local",
                        "command": [shim.display().to_string()],
                        "environment": {
                            boba_mcp_proto::schema::ENV_ENDPOINT: endpoint,
                            boba_mcp_proto::schema::ENV_TOKEN: token,
                        },
                        "enabled": true,
                    }
                },
            }),
        ))
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

/// How a run's `boba` MCP server reaches a CLI (AD-6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpWiring {
    /// A `--flag <path>` pointing at the run's MCP config file, written by the
    /// runtime.
    ConfigFile(&'static str),
    /// A config file the CLI reads from the run's workspace by itself.
    WorkspaceFile(&'static str),
    /// Config overrides on the command line; the endpoint and the token travel
    /// in the process environment, never in argv (AD-6).
    Overrides,
    /// The CLI has no MCP support, so no run on it may start.
    Unsupported,
}

/// One CLI's view of an init event's tool list (ADR-010 §1), when the line was
/// one: the runtime checks it before an untrusted run may continue.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Line {
    /// Normalised events for this line (AD-4).
    pub events: Vec<AgentEvent>,
    /// The tool list the CLI reported at the start of the run, when this line
    /// was its init event. `None` for every other line, and for a CLI that
    /// reports no init event (nothing to verify, so nothing gets verified).
    pub init_tools: Option<Vec<String>>,
}

/// Reads one CLI's stdout stream into normalised events, keeping the little
/// state a stream needs (Claude Code's tool-use ids, so a tool result can name
/// its tool). One per run.
#[derive(Debug, Default)]
pub enum StreamReader {
    /// Claude Code's `stream-json`.
    Claude(claude_code::Stream),
    /// A CLI whose stdout is plain lines, or tagged JSON events already in the
    /// `AgentEvent` wire shape.
    #[default]
    Plain,
}

impl StreamReader {
    /// The reader for `kind`. Every non-Claude CLI goes through `Plain` until
    /// its own stream format is wired.
    pub fn for_cli(kind: CliKind) -> Self {
        match kind {
            CliKind::ClaudeCode => StreamReader::Claude(claude_code::Stream::default()),
            _ => StreamReader::Plain,
        }
    }

    /// Normalise one stdout line.
    pub fn line(&mut self, line: &str) -> Line {
        match self {
            StreamReader::Claude(stream) => stream.line(line),
            StreamReader::Plain => plain_line(line),
        }
    }
}

/// The CLI-agnostic parse: a tagged `AgentEvent` JSON line, or plain text.
pub fn plain_line(line: &str) -> Line {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Line::default();
    }
    if trimmed.starts_with('{')
        && let Ok(event) = serde_json::from_str::<AgentEvent>(trimmed)
    {
        return Line {
            events: vec![event],
            init_tools: None,
        };
    }
    Line {
        events: vec![AgentEvent::Text {
            text: trimmed.to_string(),
        }],
        init_tools: None,
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
        assert_eq!(
            argv,
            vec![
                "--print",
                "--verbose",
                "--output-format",
                "stream-json",
                "do the thing"
            ]
        );
        for needle in ["--print", "--verbose", "--output-format", "stream-json"] {
            assert!(claude.contains(needle), "claude does not document {needle}");
        }

        let gemini = include_str!("fixtures/gemini_help.txt");
        let argv = CliKind::GeminiCli
            .non_interactive_argv("do the thing")
            .expect("gemini argv");
        assert_eq!(argv, vec!["--prompt", "do the thing"]);
        assert!(gemini.contains("--prompt"));

        // The three CLIs that take their prompt after a subcommand, each flag
        // checked against its own recorded help.
        for (kind, fixture, argv) in [
            (
                CliKind::Codex,
                include_str!("fixtures/codex_help.txt"),
                vec!["exec", "--json", "--skip-git-repo-check", "do the thing"],
            ),
            (
                CliKind::OpenCode,
                include_str!("fixtures/opencode_help.txt"),
                vec!["run", "--format", "json", "do the thing"],
            ),
            (
                CliKind::Aider,
                include_str!("fixtures/aider_help.txt"),
                vec!["--message", "do the thing", "--yes-always", "--no-git"],
            ),
        ] {
            let built = kind.non_interactive_argv("do the thing").expect("argv");
            assert_eq!(built, argv, "{}", kind.display_name());
            for flag in argv.iter().filter(|arg| arg.starts_with('-')) {
                assert!(
                    fixture.contains(flag),
                    "{} does not document {flag}",
                    kind.display_name()
                );
            }
            if kind != CliKind::Aider {
                // The subcommand is what makes it non-interactive.
                assert!(fixture.contains(argv[0]), "{}", kind.display_name());
            }
        }
    }

    #[test]
    fn every_cli_says_how_its_mcp_server_reaches_it() {
        // AD-6: without a way to reach `boba-mcp`, a run could neither read its
        // input nor write its output, so every CLI must answer, and the answer
        // must be evidence-backed.
        for kind in CliKind::ALL {
            match kind.mcp_wiring() {
                McpWiring::ConfigFile(flag) => {
                    let fixture = match kind {
                        CliKind::ClaudeCode => include_str!("fixtures/claude_code_help.txt"),
                        CliKind::GeminiCli => include_str!("fixtures/gemini_help.txt"),
                        other => panic!("{} has no recorded help", other.display_name()),
                    };
                    assert_eq!(Some(flag), kind.mcp_config_flag());
                    assert!(fixture.contains(flag), "{}", kind.display_name());
                }
                McpWiring::Overrides => {
                    // Codex's own help names the override it takes.
                    let help = include_str!("fixtures/codex_help.txt");
                    assert!(help.contains("-c, --config <key=value>"));
                    assert!(help.contains("mcp_servers.boba.command"));
                }
                McpWiring::WorkspaceFile(file) => {
                    assert!(file.ends_with(".json"), "{file}");
                }
                McpWiring::Unsupported => {
                    // The CLI's recorded help lists no MCP support at all.
                    assert!(
                        !include_str!("fixtures/aider_help.txt").contains("mcp"),
                        "Aider's help mentions MCP"
                    );
                    assert_eq!(kind, CliKind::Aider);
                }
            }
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
