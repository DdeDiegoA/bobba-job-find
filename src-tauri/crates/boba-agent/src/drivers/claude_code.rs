//! Claude Code driver (architecture 5.3, PRD addendum A.1).

use super::{Capabilities, CliDriver, CliKind, has_all};

pub struct ClaudeCodeDriver;

impl CliDriver for ClaudeCodeDriver {
    fn kind(&self) -> CliKind {
        CliKind::ClaudeCode
    }

    fn capabilities(&self, help: &str) -> Capabilities {
        Capabilities {
            stream_json: has_all(help, &["--output-format", "stream-json"]),
            mcp: has_all(help, &["--mcp-config"]),
            tool_allowlist: has_all(help, &["--allowedTools"]),
            budget_flag: has_all(help, &["--max-budget-usd"]),
            turn_flag: has_all(help, &["--max-turns"]),
            structured_output: has_all(help, &["--json-schema"]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Recorded probe fixtures. Source flags verified on 2026-10-04 (A.1).
    const HELP: &str = include_str!("fixtures/claude_code_help.txt");
    const VERSION: &str = include_str!("fixtures/claude_code_version.txt");

    #[test]
    fn probes_every_capability_from_recorded_help() {
        assert_eq!(
            ClaudeCodeDriver.capabilities(HELP),
            Capabilities {
                stream_json: true,
                mcp: true,
                tool_allowlist: true,
                budget_flag: true,
                turn_flag: true,
                structured_output: true,
            }
        );
    }

    #[test]
    fn version_fixture_names_the_cli() {
        assert!(VERSION.contains("Claude Code"));
        assert!(VERSION.contains("2.1.289"));
    }

    #[test]
    fn empty_help_probes_nothing() {
        assert_eq!(ClaudeCodeDriver.capabilities(""), Capabilities::default());
    }
}
