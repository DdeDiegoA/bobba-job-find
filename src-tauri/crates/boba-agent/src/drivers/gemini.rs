//! Gemini CLI driver (architecture 5.3, PRD addendum A.1).

use super::{Capabilities, CliDriver, CliKind, has_all};

pub struct GeminiDriver;

impl CliDriver for GeminiDriver {
    fn kind(&self) -> CliKind {
        CliKind::GeminiCli
    }

    fn capabilities(&self, help: &str) -> Capabilities {
        Capabilities {
            stream_json: has_all(help, &["--output-format", "stream-json"]),
            mcp: has_all(help, &["--mcp-config"]),
            tool_allowlist: has_all(help, &["--allowed-tools"]),
            budget_flag: has_all(help, &["--max-budget-usd"]),
            turn_flag: has_all(help, &["--max-turns"]),
            structured_output: has_all(help, &["--json-schema"]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELP: &str = include_str!("fixtures/gemini_help.txt");
    const VERSION: &str = include_str!("fixtures/gemini_version.txt");

    #[test]
    fn probes_capabilities_from_recorded_help() {
        assert_eq!(
            GeminiDriver.capabilities(HELP),
            Capabilities {
                stream_json: true,
                mcp: true,
                tool_allowlist: true,
                budget_flag: false,
                turn_flag: false,
                structured_output: false,
            }
        );
    }

    #[test]
    fn version_fixture_names_the_cli() {
        assert!(VERSION.contains("gemini"));
    }
}
