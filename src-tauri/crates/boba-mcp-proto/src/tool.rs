//! The MCP tool surface (architecture 5.5, AD-5, AD-6).
//!
//! This catalog is the whole tool vocabulary of a run. The core enforces it:
//! `tools/list` returns only the calling task's subset and `tools/call` rejects
//! anything outside it. There is no tool that submits, sends, deletes or
//! changes settings, and no tool that fetches a URL (AD-5).
//!
//! Tool arguments are validated by the handler that serves the tool (the
//! schemas here are the contract the agent sees); this slice enforces name,
//! subset, call cap and the untrusted envelope.

use serde::{Deserialize, Serialize};

/// The MCP server a run sees: the shim, and nothing else (AD-6).
pub const SERVER_NAME: &str = "boba";

/// Task ids, as carried on `AgentTaskKind::id()` (`boba-app/src/ports/agent.rs`).
pub const PROFILE_IMPORT: &str = "profile_import";
pub const PROFILE_GAPS: &str = "profile_gaps";
pub const EXTRACT_POSTINGS: &str = "extract_postings";
pub const NARRATE_SCORES: &str = "narrate_scores";
pub const TAILOR_APPLICATION: &str = "tailor_application";
pub const REVIEW_APPLICATION: &str = "review_application";
pub const ANSWER_QUESTIONS: &str = "answer_questions";
pub const CLASSIFY_EMAILS: &str = "classify_emails";
pub const TRANSLATE_PROFILE: &str = "translate_profile";
pub const IMPROVE_FACT: &str = "improve_fact";

/// Every task kind, in the order the UI lists them.
pub const ALL_TASKS: [&str; 10] = [
    PROFILE_IMPORT,
    PROFILE_GAPS,
    EXTRACT_POSTINGS,
    NARRATE_SCORES,
    TAILOR_APPLICATION,
    REVIEW_APPLICATION,
    ANSWER_QUESTIONS,
    CLASSIFY_EMAILS,
    TRANSLATE_PROFILE,
    IMPROVE_FACT,
];

/// How a tool writes, if it writes at all. Every write is a proposal or a
/// draft: the agent proposes, the core disposes (AD-5). Nothing here submits,
/// sends, deletes or edits settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Read,
    WriteProposal,
    WriteDraft,
}

impl ToolKind {
    pub const fn is_write(self) -> bool {
        !matches!(self, ToolKind::Read)
    }
}

/// One MCP tool: its name, kind, the tasks allowed to call it, and whether its
/// result carries third-party text that must arrive inside the
/// `untrusted_content` envelope (AD-7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolSpec {
    pub name: &'static str,
    pub kind: ToolKind,
    pub tasks: &'static [&'static str],
    pub untrusted: bool,
    pub about: &'static str,
    /// JSON Schema of the arguments, as the agent sees it in `tools/list`.
    pub input_schema: &'static str,
}

const NO_ARGS: &str = r#"{"type":"object","properties":{},"additionalProperties":false}"#;
const BY_ID: &str = r#"{"type":"object","properties":{"id":{"type":"string"}},"required":["id"],"additionalProperties":false}"#;
const BY_LIMIT: &str = r#"{"type":"object","properties":{"limit":{"type":"integer","minimum":1}},"required":["limit"],"additionalProperties":false}"#;

/// The catalog, in the order `tools/list` returns it.
pub const TOOLS: &[ToolSpec] = &[
    ToolSpec {
        name: "get_task_input",
        kind: ToolKind::Read,
        tasks: &ALL_TASKS,
        untrusted: false,
        about: "The run's own input payload (user-provided task text).",
        input_schema: NO_ARGS,
    },
    ToolSpec {
        name: "get_profile_facts",
        kind: ToolKind::Read,
        tasks: &[
            PROFILE_IMPORT,
            PROFILE_GAPS,
            TAILOR_APPLICATION,
            REVIEW_APPLICATION,
            ANSWER_QUESTIONS,
            TRANSLATE_PROFILE,
            IMPROVE_FACT,
        ],
        untrusted: false,
        about: "The user's confirmed profile facts, with their stable fact ids.",
        input_schema: NO_ARGS,
    },
    ToolSpec {
        name: "get_preferences",
        kind: ToolKind::Read,
        tasks: &[
            PROFILE_IMPORT,
            PROFILE_GAPS,
            TAILOR_APPLICATION,
            REVIEW_APPLICATION,
            ANSWER_QUESTIONS,
            TRANSLATE_PROFILE,
        ],
        untrusted: false,
        about: "The user's search and application preferences.",
        input_schema: NO_ARGS,
    },
    ToolSpec {
        name: "get_pending_postings",
        kind: ToolKind::Read,
        tasks: &[EXTRACT_POSTINGS],
        untrusted: true,
        about: "Posting text waiting to be turned into features.",
        input_schema: BY_LIMIT,
    },
    ToolSpec {
        name: "get_posting",
        kind: ToolKind::Read,
        tasks: &[NARRATE_SCORES, TAILOR_APPLICATION, REVIEW_APPLICATION],
        untrusted: true,
        about: "One posting and its company, as the user sees it.",
        input_schema: BY_ID,
    },
    ToolSpec {
        name: "get_score_card",
        kind: ToolKind::Read,
        tasks: &[NARRATE_SCORES, TAILOR_APPLICATION, REVIEW_APPLICATION],
        untrusted: true,
        about: "The deterministic score card for a posting (the numbers are the core's).",
        input_schema: BY_ID,
    },
    ToolSpec {
        name: "get_company_research",
        kind: ToolKind::Read,
        tasks: &[TAILOR_APPLICATION, REVIEW_APPLICATION],
        untrusted: true,
        about: "Cached company research for a posting.",
        input_schema: BY_ID,
    },
    ToolSpec {
        name: "get_email_batch",
        kind: ToolKind::Read,
        tasks: &[CLASSIFY_EMAILS],
        untrusted: true,
        about: "Unread recruiter mail waiting to be classified.",
        input_schema: BY_LIMIT,
    },
    ToolSpec {
        name: "propose_profile_facts",
        kind: ToolKind::WriteProposal,
        tasks: &[PROFILE_IMPORT, PROFILE_GAPS, TRANSLATE_PROFILE],
        untrusted: false,
        about: "Propose profile facts for the user to confirm.",
        input_schema: r#"{"type":"object","properties":{"facts":{"type":"array","items":{"type":"object"}}},"required":["facts"],"additionalProperties":false}"#,
    },
    ToolSpec {
        name: "propose_gap_questions",
        kind: ToolKind::WriteProposal,
        tasks: &[PROFILE_GAPS],
        untrusted: false,
        about: "Propose the short, skippable questions that fill the Profile's gaps.",
        input_schema: r#"{"type":"object","properties":{"questions":{"type":"array","items":{"type":"object"}}},"required":["questions"],"additionalProperties":false}"#,
    },
    ToolSpec {
        name: "propose_translations",
        kind: ToolKind::WriteProposal,
        tasks: &[PROFILE_IMPORT, TRANSLATE_PROFILE],
        untrusted: false,
        about: "Propose ES/EN variants of profile facts for the user to confirm.",
        input_schema: r#"{"type":"object","properties":{"translations":{"type":"array","items":{"type":"object"}}},"required":["translations"],"additionalProperties":false}"#,
    },
    ToolSpec {
        name: "propose_fact_improvements",
        kind: ToolKind::WriteProposal,
        tasks: &[IMPROVE_FACT],
        untrusted: false,
        about: "Propose a rewritten version of one profile fact for the user to confirm.",
        input_schema: r#"{"type":"object","properties":{"improvements":{"type":"array","items":{"type":"object"}}},"required":["improvements"],"additionalProperties":false}"#,
    },
    ToolSpec {
        name: "propose_posting_features",
        kind: ToolKind::WriteProposal,
        tasks: &[EXTRACT_POSTINGS],
        untrusted: false,
        about: "Propose the extracted features of a posting.",
        input_schema: r#"{"type":"object","properties":{"id":{"type":"string"},"features":{"type":"object"}},"required":["id","features"],"additionalProperties":false}"#,
    },
    ToolSpec {
        name: "propose_status_change",
        kind: ToolKind::WriteProposal,
        tasks: &[CLASSIFY_EMAILS],
        untrusted: false,
        about: "Propose an application status change with the evidence quote.",
        input_schema: r#"{"type":"object","properties":{"email_id":{"type":"string"},"app_id":{"type":"string"},"status":{"type":"string"},"quote":{"type":"string"}},"required":["email_id","app_id","status","quote"],"additionalProperties":false}"#,
    },
    ToolSpec {
        name: "save_score_narration",
        kind: ToolKind::WriteDraft,
        tasks: &[NARRATE_SCORES],
        untrusted: false,
        about: "Save the coach narration for a score card. It never changes the numbers.",
        input_schema: r#"{"type":"object","properties":{"id":{"type":"string"},"lang":{"type":"string"},"text":{"type":"string"}},"required":["id","lang","text"],"additionalProperties":false}"#,
    },
    ToolSpec {
        name: "save_document_draft",
        kind: ToolKind::WriteDraft,
        tasks: &[TAILOR_APPLICATION],
        untrusted: false,
        about: "Save a CV or cover-letter draft for an application.",
        input_schema: r#"{"type":"object","properties":{"app_id":{"type":"string"},"doc":{"type":"object"}},"required":["app_id","doc"],"additionalProperties":false}"#,
    },
    ToolSpec {
        name: "save_answers",
        kind: ToolKind::WriteDraft,
        tasks: &[ANSWER_QUESTIONS],
        untrusted: false,
        about: "Save the drafted screening answers for an application.",
        input_schema: r#"{"type":"object","properties":{"app_id":{"type":"string"},"answers":{"type":"array","items":{"type":"object"}}},"required":["app_id","answers"],"additionalProperties":false}"#,
    },
    ToolSpec {
        name: "save_review",
        kind: ToolKind::WriteDraft,
        tasks: &[REVIEW_APPLICATION],
        untrusted: false,
        about: "Save the reviewer findings for an application's drafts.",
        input_schema: r#"{"type":"object","properties":{"app_id":{"type":"string"},"findings":{"type":"array","items":{"type":"object"}}},"required":["app_id","findings"],"additionalProperties":false}"#,
    },
];

/// The tool called `name`, if the catalog has one.
pub fn find(name: &str) -> Option<&'static ToolSpec> {
    TOOLS.iter().find(|tool| tool.name == name)
}

/// Whether `task_id` may call `tool`. Unknown tasks and unknown tools get
/// nothing: the subset is the catalog's, never the agent's.
pub fn allows(task_id: &str, tool: &str) -> bool {
    find(tool).is_some_and(|spec| spec.tasks.contains(&task_id))
}

/// The tool subset `task_id` may see, in catalog order.
pub fn tools_for_task(task_id: &str) -> Vec<&'static ToolSpec> {
    TOOLS
        .iter()
        .filter(|tool| tool.tasks.contains(&task_id))
        .collect()
}

/// A `tools/list` entry for one tool.
pub fn list_entry(spec: &ToolSpec) -> serde_json::Value {
    serde_json::json!({
        "name": spec.name,
        "description": spec.about,
        "inputSchema": serde_json::from_str::<serde_json::Value>(spec.input_schema)
            .expect("tool schemas are valid JSON"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_task_sees_only_its_own_subset() {
        let profile = tools_for_task(PROFILE_IMPORT);
        let names: Vec<&str> = profile.iter().map(|tool| tool.name).collect();
        assert_eq!(
            names,
            vec![
                "get_task_input",
                "get_profile_facts",
                "get_preferences",
                "propose_profile_facts",
                "propose_translations",
            ]
        );
        assert!(!allows(PROFILE_IMPORT, "get_email_batch"));
        assert!(!allows(PROFILE_IMPORT, "save_document_draft"));
        // S3.6: the interview asks questions and may propose what the context
        // it was given states — never a translation, never a posting feature.
        let gaps: Vec<&str> = tools_for_task(PROFILE_GAPS)
            .iter()
            .map(|tool| tool.name)
            .collect();
        assert_eq!(
            gaps,
            vec![
                "get_task_input",
                "get_profile_facts",
                "get_preferences",
                "propose_profile_facts",
                "propose_gap_questions",
            ]
        );
        assert!(!allows(PROFILE_GAPS, "propose_translations"));
        assert!(!allows(PROFILE_GAPS, "get_email_batch"));
        assert!(allows(CLASSIFY_EMAILS, "propose_status_change"));
        assert!(allows(TAILOR_APPLICATION, "save_document_draft"));
    }

    #[test]
    fn every_task_gets_a_nonempty_subset_and_task_input() {
        for task in ALL_TASKS {
            let subset = tools_for_task(task);
            assert!(!subset.is_empty(), "{task} has no tools");
            assert!(
                subset.iter().any(|tool| tool.name == "get_task_input"),
                "{task} cannot read its own input"
            );
        }
    }

    #[test]
    fn no_tool_can_send_submit_delete_or_touch_settings() {
        // AD-5: the agent never sends, submits, deletes or changes settings.
        const FORBIDDEN: [&str; 9] = [
            "send", "submit", "apply", "delete", "remove", "drop", "setting", "config", "fetch",
        ];
        for tool in TOOLS {
            for word in FORBIDDEN {
                assert!(
                    !tool.name.contains(word),
                    "`{}` looks like a tool that does `{word}`",
                    tool.name
                );
                assert!(
                    !tool.about.to_ascii_lowercase().contains(word),
                    "`{}` documents `{word}`",
                    tool.name
                );
            }
        }
    }

    #[test]
    fn write_tools_are_proposals_or_drafts_only() {
        for tool in TOOLS {
            if !tool.kind.is_write() {
                continue;
            }
            assert!(
                tool.name.starts_with("propose_") || tool.name.starts_with("save_"),
                "`{}` is a write tool outside the propose_/save_ vocabulary",
                tool.name
            );
            assert_ne!(tool.kind, ToolKind::Read);
        }
        let writes = TOOLS.iter().filter(|tool| tool.kind.is_write()).count();
        let reads = TOOLS.len() - writes;
        // S3.6 added `propose_gap_questions` and FX10a `propose_fact_improvements`,
        // so the catalog holds ten writes.
        assert_eq!((reads, writes), (8, 10));
    }

    #[test]
    fn every_untrusted_reader_is_marked_and_none_is_a_writer() {
        for tool in TOOLS {
            if tool.untrusted {
                assert_eq!(tool.kind, ToolKind::Read, "{}", tool.name);
            }
        }
        let untrusted: Vec<&str> = TOOLS
            .iter()
            .filter(|tool| tool.untrusted)
            .map(|tool| tool.name)
            .collect();
        assert_eq!(
            untrusted,
            vec![
                "get_pending_postings",
                "get_posting",
                "get_score_card",
                "get_company_research",
                "get_email_batch",
            ]
        );
    }

    #[test]
    fn tool_names_and_schemas_are_unique_and_valid() {
        let mut seen = Vec::new();
        for tool in TOOLS {
            assert!(!seen.contains(&tool.name), "duplicate tool {}", tool.name);
            seen.push(tool.name);
            let entry = list_entry(tool);
            assert_eq!(entry["name"], tool.name);
            assert!(entry["inputSchema"].is_object(), "{}", tool.name);
        }
    }

    #[test]
    fn unknown_names_are_denied_everywhere() {
        assert!(find("shell").is_none());
        for task in ALL_TASKS {
            assert!(!allows(task, "shell"));
            assert!(!allows(task, "web_fetch"));
            assert!(!allows(task, ""));
        }
    }
}
