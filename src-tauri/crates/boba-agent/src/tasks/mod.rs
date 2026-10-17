//! The task registry (AC1, AC2): every agent task Boba runs, and what counts as
//! that task's output.
//!
//! The registry is keyed on the task id — the same string that keys
//! `agent_run.task_kind`, the MCP endpoint's tool subset and
//! `agent-pack/skills/<id>/SKILL.md`. The task's **output tools are derived**
//! from the `boba-mcp-proto` catalog (AD-5): a task cannot claim an output the
//! catalog does not give it, and adding a write tool to a task's list in the
//! catalog adds it here. Nothing restates the catalog.
//!
//! What a task *writes* is checked against the same catalog's schemas — see
//! [`output`].

use boba_app::ports::agent::AgentTaskKind;
use boba_mcp_proto::tool;

pub mod output;

pub use output::{InvalidOutput, MAX_OUTPUT_RETRIES, OutputAttempts, OutputVerdict, check_output};

/// Every agent task, as the catalog lists them. A task is only real when it
/// appears here, in `AgentTaskKind` and as `agent-pack/skills/<id>/SKILL.md`.
pub const TASKS: [&str; 9] = tool::ALL_TASKS;

/// The tools whose arguments are `task`'s output: the write tools the catalog
/// gives it, in catalog order (AD-5). Unknown ids get nothing.
pub fn outputs(task: &str) -> Vec<&'static str> {
    tool::tools_for_task(task)
        .into_iter()
        .filter(|spec| spec.kind.is_write())
        .map(|spec| spec.name)
        .collect()
}

/// Whether `tool` is one of `task`'s outputs.
pub fn is_output(task: &str, tool_name: &str) -> bool {
    outputs(task).contains(&tool_name)
}

/// The registry and the task kinds must agree, in both directions: a kind
/// outside the registry would run with no skill and no checked output.
pub fn covers(kind: AgentTaskKind) -> bool {
    TASKS.contains(&kind.id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registry_and_the_task_kinds_agree() {
        assert_eq!(TASKS.len(), AgentTaskKind::ALL.len());
        for kind in AgentTaskKind::ALL {
            assert!(covers(kind), "{} is not in the registry", kind.id());
        }
        // Every write tool in the catalog belongs to a task in the registry,
        // and every task has at least one way to produce its output.
        for spec in tool::TOOLS {
            if !spec.kind.is_write() {
                continue;
            }
            for owner in spec.tasks {
                assert!(
                    TASKS.contains(owner),
                    "`{}` is a write tool for {owner}, which is not in the registry",
                    spec.name
                );
            }
        }
        for task in TASKS {
            assert!(!outputs(task).is_empty(), "{task} has no output tool");
        }
    }

    #[test]
    fn a_tasks_outputs_are_the_write_tools_the_catalog_gives_it() {
        // Derived, not restated: the registry must agree with the catalog.
        for task in TASKS {
            let expected: Vec<&str> = tool::tools_for_task(task)
                .into_iter()
                .filter(|spec| spec.kind.is_write())
                .map(|spec| spec.name)
                .collect();
            assert_eq!(outputs(task), expected, "{task}");
        }
        assert_eq!(
            outputs(tool::PROFILE_IMPORT),
            vec!["propose_profile_facts", "propose_translations"]
        );
        assert_eq!(
            outputs(tool::EXTRACT_POSTINGS),
            vec!["propose_posting_features"]
        );
        assert_eq!(outputs(tool::REVIEW_APPLICATION), vec!["save_review"]);
    }

    #[test]
    fn a_read_tool_is_never_a_tasks_output() {
        assert!(is_output(
            tool::EXTRACT_POSTINGS,
            "propose_posting_features"
        ));
        assert!(!is_output(tool::EXTRACT_POSTINGS, "get_pending_postings"));
        assert!(!is_output(tool::EXTRACT_POSTINGS, "get_task_input"));
        assert!(!is_output(tool::EXTRACT_POSTINGS, "save_document_draft"));
        assert!(!is_output(tool::CLASSIFY_EMAILS, "made_up_tool"));
        assert!(!is_output("made_up_task", "get_task_input"));
    }
}
