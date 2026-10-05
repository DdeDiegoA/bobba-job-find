//! OpenCode driver (architecture 5.3, PRD addendum A.1).

use super::{Capabilities, CliDriver, CliKind, has_all};

pub struct OpenCodeDriver;

impl CliDriver for OpenCodeDriver {
    fn kind(&self) -> CliKind {
        CliKind::OpenCode
    }

    fn capabilities(&self, help: &str) -> Capabilities {
        Capabilities {
            stream_json: has_all(help, &["--format"]),
            mcp: has_all(help, &["--mcp"]),
            tool_allowlist: has_all(help, &["--permission"]),
            budget_flag: has_all(help, &["--max-budget-usd"]),
            turn_flag: has_all(help, &["--max-turns"]),
            structured_output: has_all(help, &["--format", "json"]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELP: &str = include_str!("fixtures/opencode_help.txt");
    const VERSION: &str = include_str!("fixtures/opencode_version.txt");

    #[test]
    fn probes_capabilities_from_recorded_help() {
        assert_eq!(
            OpenCodeDriver.capabilities(HELP),
            Capabilities {
                stream_json: true,
                mcp: true,
                tool_allowlist: true,
                budget_flag: false,
                turn_flag: false,
                structured_output: true,
            }
        );
    }

    #[test]
    fn version_fixture_is_a_version_number() {
        // `opencode --version` prints just the version, verified 2026-10-04.
        assert_eq!(VERSION.trim(), "1.18.34");
    }
}
