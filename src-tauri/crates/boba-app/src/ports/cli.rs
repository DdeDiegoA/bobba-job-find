//! `CliInstallsPort` — the agent CLIs this machine has, as AD-4's probe found
//! them (S7.7 needs it to pick the one a generation run starts on).
//! Adapter: `boba-store` (the `cli_install` rows).
//!
//! The port only reports what was detected. **Choosing** an engine is the
//! engine step's job (S3.2); until it lands, a caller may proceed only when
//! there is exactly one CLI to choose from.

/// A failure reading the detected installations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("cli install store error: {0}")]
pub struct CliInstallsError(pub String);

impl CliInstallsError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// The agent CLIs detected on this machine (AD-4).
pub trait CliInstallsPort: Send + Sync {
    /// One entry per installed CLI **kind** (`claude_code`, `codex`, …), each
    /// once, sorted, so the caller never has to dedupe a machine that has two
    /// paths to the same CLI.
    fn installed(&self) -> Result<Vec<String>, CliInstallsError>;
}
