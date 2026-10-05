//! Aider driver (architecture 5.3, PRD addendum A.1).
//!
//! Aider has no MCP and no tool surface, so it probes as text-only: every
//! capability is false (its task tier is decided by the runtime, AD-4).

use super::{Capabilities, CliDriver, CliKind};

pub struct AiderDriver;

impl CliDriver for AiderDriver {
    fn kind(&self) -> CliKind {
        CliKind::Aider
    }

    fn capabilities(&self, _help: &str) -> Capabilities {
        Capabilities::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELP: &str = include_str!("fixtures/aider_help.txt");
    const VERSION: &str = include_str!("fixtures/aider_version.txt");

    #[test]
    fn probes_as_text_only() {
        assert_eq!(AiderDriver.capabilities(HELP), Capabilities::default());
    }

    #[test]
    fn version_fixture_names_the_cli() {
        assert!(VERSION.contains("aider"));
    }
}
