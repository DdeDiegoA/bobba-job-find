//! Capability snapshot command (S3.3, FR-4, AC4): what the UI may offer this
//! session.
//!
//! The gate itself lives in `boba_app::capabilities`; this command only turns
//! the current engine choice and the probe's lockdown verdict into its input.
//! The probe is the same one the engine step runs, so the snapshot agrees with
//! what a run would be allowed to do, and it refreshes the `cli_install`
//! evidence the per-run gate (ADR-010) reads.

use boba_agent::detect::CliStatus;
use boba_agent::drivers::{CliInstall, HealthStatus};
use boba_agent::lockdown;
use boba_app::capabilities::{self, CapabilitySnapshot, ChosenAgent};
use boba_app::error::{AppError, AppResult, ErrorCode};
use boba_app::ports::agent::AgentTaskKind;

use crate::commands::agent::EngineChoice;
use crate::wiring;
use crate::wiring::agent::EngineProbe;

/// Everything the UI gates on, for the stored engine choice (AC1–AC3).
///
/// The probe runs bounded subprocesses, so never on the UI thread. No choice
/// stored (or a choice this build cannot read) is Limited mode, exactly what a
/// probe of nothing would produce.
#[tauri::command]
#[specta::specta]
pub async fn capabilities_snapshot(app: tauri::AppHandle) -> AppResult<CapabilitySnapshot> {
    let probe = wiring::port::<EngineProbe>(&app)?;
    let choice = crate::commands::agent::stored_choice(&app);
    let statuses = tauri::async_runtime::spawn_blocking(move || probe.detect_and_store())
        .await
        .map_err(|_| AppError::new(ErrorCode::Internal, "errors.agent.probe"))?;
    Ok(capabilities::snapshot(
        choice
            .as_ref()
            .and_then(|choice| chosen_agent(&statuses, choice)),
    ))
}

/// The chosen CLI as the gate sees it, or `None` when the probe has no entry
/// for it (nothing was ever vouched for, so nothing may run).
fn chosen_agent(statuses: &[CliStatus], choice: &EngineChoice) -> Option<ChosenAgent> {
    statuses.iter().find_map(|status| match status {
        CliStatus::Installed(install) if install.kind.id() == choice.cli => Some(ChosenAgent {
            installed: true,
            healthy: matches!(install.health, HealthStatus::Healthy),
            unsupported_tasks: refused_tasks(install),
        }),
        // A stored choice whose CLI was since uninstalled is no engine.
        CliStatus::NotInstalled { kind } if kind.id() == choice.cli => Some(ChosenAgent {
            installed: false,
            healthy: false,
            unsupported_tasks: Vec::new(),
        }),
        _ => None,
    })
}

/// The tasks ADR-010's gate refuses on this install: same verdict the per-run
/// gate reaches from the stored evidence (engine step's `unsupported_tasks`).
fn refused_tasks(install: &CliInstall) -> Vec<AgentTaskKind> {
    AgentTaskKind::ALL
        .into_iter()
        .filter(|task| {
            !lockdown::support(*task, install.kind, &install.version, &install.capabilities)
                .is_supported()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use boba_agent::detect::CliStatus;
    use boba_agent::drivers::{Capabilities, CliInstall, CliKind, HealthStatus};
    use boba_agent::lockdown;
    use boba_app::capabilities::{
        Capability, CapabilitySnapshot, ChosenAgent, UnavailableReason, snapshot,
    };
    use boba_app::ports::agent::AgentTaskKind;

    use crate::commands::agent::EngineChoice;

    use super::chosen_agent;

    fn claude_caps() -> Capabilities {
        Capabilities {
            stream_json: true,
            mcp: true,
            tool_allowlist: true,
            ..Capabilities::default()
        }
    }

    fn installed(kind: CliKind, version: &str, capabilities: Capabilities) -> CliStatus {
        CliStatus::Installed(CliInstall {
            kind,
            path: format!("/usr/local/bin/{}", kind.binary_names()[0]).into(),
            version: version.to_string(),
            health: HealthStatus::Healthy,
            capabilities,
        })
    }

    fn statuses() -> Vec<CliStatus> {
        vec![
            installed(CliKind::ClaudeCode, "2.1.301 (Claude Code)", claude_caps()),
            installed(CliKind::Aider, "0.86.0", Capabilities::default()),
            CliStatus::NotInstalled {
                kind: CliKind::Codex,
            },
        ]
    }

    fn choice(cli: &str) -> EngineChoice {
        EngineChoice {
            cli: cli.to_string(),
            model: None,
        }
    }

    fn state(snapshot: &CapabilitySnapshot, id: &str) -> (bool, Option<UnavailableReason>) {
        let found = snapshot
            .capabilities
            .iter()
            .find(|capability| capability.id == id)
            .unwrap_or_else(|| panic!("capability {id} missing from the snapshot"));
        (found.available, found.reason)
    }

    fn snapshot_for(cli: &str, statuses: &[CliStatus]) -> CapabilitySnapshot {
        snapshot(chosen_agent(statuses, &choice(cli)).as_ref().cloned())
    }

    #[test]
    fn a_locked_down_chosen_cli_runs_every_capability() {
        let snapshot = snapshot_for("claude_code", &statuses());
        assert!(!snapshot.limited);
        assert!(
            snapshot
                .capabilities
                .iter()
                .all(|c| c.available && c.reason.is_none()),
            "{snapshot:?}"
        );
    }

    #[test]
    fn an_unlocked_down_chosen_cli_refuses_the_untrusted_tasks() {
        // AC3: the probe's lockdown verdict reaches the gate as the reason.
        let snapshot = snapshot_for("aider", &statuses());
        assert!(!snapshot.limited);
        assert_eq!(
            state(&snapshot, "tailoring"),
            (false, Some(UnavailableReason::CliUnsupported))
        );
        assert_eq!(
            state(&snapshot, "posting_extraction"),
            (false, Some(UnavailableReason::CliUnsupported))
        );
        assert_eq!(
            state(&snapshot, "profile_extraction"),
            (true, None),
            "user-trust tasks still run"
        );
    }

    #[test]
    fn a_chosen_cli_that_is_not_installed_reads_as_no_agent() {
        // The wizard stored a choice, then the CLI was uninstalled.
        let snapshot = snapshot_for("codex", &statuses());
        assert!(snapshot.limited);
        assert_eq!(
            state(&snapshot, "tailoring"),
            (false, Some(UnavailableReason::NoAgent))
        );
        assert_eq!(state(&snapshot, "tracker"), (true, None));
    }

    #[test]
    fn an_auth_required_cli_is_not_a_working_engine() {
        let statuses = vec![CliStatus::Installed(CliInstall {
            kind: CliKind::ClaudeCode,
            path: "/usr/local/bin/claude".into(),
            version: "2.1.301 (Claude Code)".to_string(),
            health: HealthStatus::AuthRequired,
            capabilities: claude_caps(),
        })];
        let snapshot = snapshot_for("claude_code", &statuses);
        assert!(snapshot.limited);
        assert_eq!(
            state(&snapshot, "email_classification"),
            (false, Some(UnavailableReason::NoAgent))
        );
    }

    #[test]
    fn no_stored_choice_is_limited_mode() {
        // S3.8 stores no choice when the user picks Limited mode.
        let snapshot = snapshot(None);
        assert!(snapshot.limited);
        assert_eq!(
            state(&snapshot, "profile_extraction"),
            (false, Some(UnavailableReason::NoAgent))
        );
    }

    #[test]
    fn the_gate_reads_every_untrusted_task_off_the_probe() {
        let chosen = chosen_agent(&statuses(), &choice("aider")).expect("aider is probed");
        let mut refused: Vec<&str> = chosen
            .unsupported_tasks
            .iter()
            .map(|task| task.id())
            .collect();
        refused.sort_unstable();
        let mut expected: Vec<&str> = AgentTaskKind::ALL
            .into_iter()
            .filter(|task| task.reads_untrusted())
            .map(|task| task.id())
            .collect();
        expected.sort_unstable();
        assert_eq!(refused, expected);
    }

    /// The install behind `cli`, as the per-run gate (S2.4) reads it.
    fn install_of(statuses: &[CliStatus], cli: &str) -> (CliKind, String, Capabilities) {
        match statuses
            .iter()
            .find(|status| match status {
                CliStatus::Installed(install) => install.kind.id() == cli,
                CliStatus::NotInstalled { kind } => kind.id() == cli,
            })
            .expect("every test CLI is probed")
        {
            CliStatus::Installed(install) => {
                (install.kind, install.version.clone(), install.capabilities)
            }
            CliStatus::NotInstalled { kind } => (*kind, String::new(), Capabilities::default()),
        }
    }

    #[test]
    fn capability_availability_never_disagrees_with_the_per_run_gate() {
        // The reviewer's focus: the UI model is not a second opinion. For every
        // task-bearing capability, availability must be exactly the verdict the
        // per-run gate (`lockdown::support`, S2.4) reaches for that install —
        // never more permissive (no bypass) and never less (no over-promise).
        let statuses = statuses();
        for cli in ["claude_code", "aider", "codex"] {
            let chosen = chosen_agent(&statuses, &choice(cli)).expect("CLI is probed");
            let gate = snapshot(Some(chosen.clone()));
            let (kind, version, capabilities) = install_of(&statuses, cli);
            for capability in Capability::ALL {
                let Some(task) = capability.task() else {
                    continue;
                };
                let expected = if !chosen.installed || !chosen.healthy {
                    (false, Some(UnavailableReason::NoAgent))
                } else if lockdown::support(task, kind, &version, &capabilities).is_supported() {
                    (true, None)
                } else {
                    (false, Some(UnavailableReason::CliUnsupported))
                };
                assert_eq!(
                    state(&gate, capability.id()),
                    expected,
                    "{} on {cli}",
                    capability.id()
                );
            }
        }
    }

    #[test]
    fn limited_mode_blocks_every_task_a_run_would_otherwise_dispatch() {
        // No stored choice is Limited mode (FR-4): every AI capability is off
        // with the agent reason, so the UI never offers a run the untrusted
        // gate (S2.4) would refuse. Deterministic work stays on.
        let no_choice = snapshot(None);
        assert!(no_choice.limited);
        for capability in Capability::ALL {
            match capability.task() {
                Some(_) => assert_eq!(
                    state(&no_choice, capability.id()),
                    (false, Some(UnavailableReason::NoAgent)),
                    "{} in Limited mode",
                    capability.id()
                ),
                None => assert_eq!(
                    state(&no_choice, capability.id()),
                    (true, None),
                    "{} is deterministic",
                    capability.id()
                ),
            }
        }
    }

    #[test]
    fn a_healthy_install_maps_to_a_working_agent() {
        let chosen = chosen_agent(&statuses(), &choice("claude_code")).expect("claude is probed");
        assert_eq!(
            chosen,
            ChosenAgent {
                installed: true,
                healthy: true,
                unsupported_tasks: Vec::new(),
            }
        );
    }
}
