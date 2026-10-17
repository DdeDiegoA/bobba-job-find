//! Capability gating rules (FR-4, AC1–AC3).

use crate::ports::agent::AgentTaskKind;

use super::{Capability, CapabilitySnapshot, ChosenAgent, UnavailableReason, snapshot};

/// The tasks a CLI without verified `tool_lockdown` refuses (ADR-010 §2).
fn untrusted_tasks() -> Vec<AgentTaskKind> {
    AgentTaskKind::ALL
        .into_iter()
        .filter(|task| task.reads_untrusted())
        .collect()
}

fn working_agent(unsupported_tasks: Vec<AgentTaskKind>) -> Option<ChosenAgent> {
    Some(ChosenAgent {
        installed: true,
        healthy: true,
        unsupported_tasks,
    })
}

fn state(snapshot: &CapabilitySnapshot, id: &str) -> (bool, Option<UnavailableReason>) {
    let found = snapshot
        .capabilities
        .iter()
        .find(|capability| capability.id == id)
        .unwrap_or_else(|| panic!("capability {id} missing from the snapshot"));
    (found.available, found.reason)
}

#[test]
fn limited_mode_keeps_the_deterministic_work_on() {
    let limited = snapshot(None);
    assert!(limited.limited, "no agent means Limited mode (FR-4)");

    // AC1: Sources, dedup, deterministic scoring, the Tracker and manual
    // Applications keep working, and the Profile stays editable by hand.
    for id in [
        "sources",
        "dedup",
        "scoring",
        "tracker",
        "applications_manual",
        "profile_edit",
    ] {
        assert_eq!(state(&limited, id), (true, None), "{id} in Limited mode");
    }

    // AC2: every AI-only action is off, with the "Connect an agent" reason.
    for id in [
        "profile_extraction",
        "profile_gaps",
        "profile_translation",
        "posting_extraction",
        "tailoring",
        "application_review",
        "score_narration",
        "answer_questions",
        "email_classification",
    ] {
        assert_eq!(
            state(&limited, id),
            (false, Some(UnavailableReason::NoAgent)),
            "{id} in Limited mode"
        );
    }
}

#[test]
fn a_working_locked_down_engine_runs_every_capability() {
    let full = snapshot(working_agent(Vec::new()));
    assert!(!full.limited);
    assert!(
        full.capabilities
            .iter()
            .all(|c| c.available && c.reason.is_none()),
        "{full:?}"
    );
}

#[test]
fn an_unlocked_down_cli_keeps_only_the_user_trust_tasks_on() {
    // Aider: installed and healthy, but no verified tool lockdown, so ADR-010
    // refuses every task that reads untrusted content.
    let partial = snapshot(working_agent(untrusted_tasks()));
    assert!(!partial.limited, "a working CLI is not Limited mode");

    // AC3: untrusted-content tasks are disabled with the lockdown reason.
    for id in [
        "posting_extraction",
        "tailoring",
        "application_review",
        "score_narration",
        "answer_questions",
        "email_classification",
    ] {
        assert_eq!(
            state(&partial, id),
            (false, Some(UnavailableReason::CliUnsupported)),
            "{id} without tool_lockdown"
        );
    }

    // Tasks that read only the user's own input still run (ADR-010 §2).
    for id in ["profile_extraction", "profile_gaps", "profile_translation"] {
        assert_eq!(state(&partial, id), (true, None), "{id} with user input");
    }

    // Deterministic work is never gated on the agent.
    for id in [
        "sources",
        "dedup",
        "scoring",
        "tracker",
        "applications_manual",
    ] {
        assert_eq!(state(&partial, id), (true, None), "{id} is deterministic");
    }
}

#[test]
fn an_unhealthy_or_missing_install_reads_as_limited() {
    for chosen in [
        ChosenAgent {
            installed: true,
            healthy: false,
            unsupported_tasks: Vec::new(),
        },
        ChosenAgent {
            installed: false,
            healthy: true,
            unsupported_tasks: Vec::new(),
        },
    ] {
        let snapshot = snapshot(Some(chosen.clone()));
        assert!(snapshot.limited, "{chosen:?} cannot run a task");
        assert_eq!(
            state(&snapshot, "tailoring"),
            (false, Some(UnavailableReason::NoAgent))
        );
        assert_eq!(state(&snapshot, "tracker"), (true, None));
    }
}

#[test]
fn every_capability_has_a_unique_snake_case_id() {
    let ids: Vec<&str> = Capability::ALL.iter().map(|c| c.id()).collect();
    assert_eq!(ids.len(), 15, "one entry per gated capability");
    let mut seen = std::collections::HashSet::new();
    for id in ids {
        assert!(seen.insert(id), "duplicate capability id {id}");
        assert!(
            id.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
            "{id} is not snake_case"
        );
    }
}

#[test]
fn the_snapshot_lists_every_capability_in_registry_order() {
    let snapshot = snapshot(None);
    let ids: Vec<&str> = snapshot
        .capabilities
        .iter()
        .map(|capability| capability.id.as_str())
        .collect();
    assert_eq!(
        ids,
        Capability::ALL.iter().map(|c| c.id()).collect::<Vec<_>>()
    );
}
