//! Claude Code driver (architecture 5.3, PRD addendum A.1).

use std::collections::HashMap;

use serde_json::Value;

use super::{Capabilities, CliDriver, CliKind, Line, has_all, plain_line};
use boba_app::ports::agent::AgentEvent;

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

/// Reads Claude Code's `--output-format stream-json` (verified against the CLI
/// on 2026-10-16, 2.1.301). The stream is one JSON object per line:
///
/// ```text
/// {"type":"system","subtype":"init","tools":["mcp__boba__get_task_input",…],…}
/// {"type":"system","subtype":"hook_started"|"hook_response"|…}      (the user's own hooks)
/// {"type":"assistant","message":{"content":[{"type":"text","text":"…"}]}}
/// {"type":"assistant","message":{"content":[{"type":"tool_use","id":…,"name":…,"input":…}]}}
/// {"type":"assistant","message":{"content":[{"type":"server_tool_use","id":…,"name":…,"input":…}]}}
/// {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":…,"content":…}]}}
/// {"type":"result","subtype":"success","result":"…","total_cost_usd":…,"usage":{…}}
/// {"type":"rate_limit_event","rate_limit_info":{…}}
/// ```
///
/// Only the agent's own work becomes events: prose, tool calls and their
/// results. The init event is where ADR-010 §1's tool list comes from, and the
/// CLI's own hook and rate-limit traffic is dropped rather than shown as the
/// agent's output.
///
/// Both tool-invoking block types are tool calls. `server_tool_use` is the CLI's
/// other one — a native tool it runs itself — so dropping it would hide a native
/// call from the untrusted-run gate; a new tool-invoking block type must be
/// added here (CH8).
#[derive(Debug, Default)]
pub struct Stream {
    /// `tool_use` id → tool name, so a later `tool_result` can name its tool.
    tool_names: HashMap<String, String>,
}

impl Stream {
    pub fn line(&mut self, line: &str) -> Line {
        let trimmed = line.trim();
        let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
            // Not the documented stream: keep it visible as text rather than
            // dropping it silently.
            return plain_line(trimmed);
        };
        match value.get("type").and_then(Value::as_str) {
            Some("system") if value.get("subtype").and_then(Value::as_str) == Some("init") => {
                Line {
                    events: Vec::new(),
                    init_tools: Some(tool_list(&value)),
                }
            }
            Some("assistant") => Line {
                events: self.message_events(&value),
                init_tools: None,
            },
            Some("user") => Line {
                events: self.tool_result_events(&value),
                init_tools: None,
            },
            Some("result") => Line {
                events: usage_event(&value),
                init_tools: None,
            },
            // The CLI's own traffic — hooks it runs for the user, status and
            // rate-limit notices, partial-message frames: not the agent's work,
            // so it never becomes the run's output.
            Some("system" | "rate_limit_event" | "stream_event" | "status") => Line::default(),
            // Not the documented stream at all (another CLI's fixture, an older
            // shape, or a plain error line): the CLI-agnostic parse keeps it
            // visible instead of dropping it.
            _ => plain_line(trimmed),
        }
    }

    /// The prose and tool calls of one assistant message.
    fn message_events(&mut self, value: &Value) -> Vec<AgentEvent> {
        let mut events = Vec::new();
        for block in content_blocks(value) {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(text) = block.get("text").and_then(Value::as_str)
                        && !text.is_empty()
                    {
                        events.push(AgentEvent::Text {
                            text: text.to_string(),
                        });
                    }
                }
                // Both tool-invoking block types, so the untrusted-run gate sees
                // a native tool whichever block the CLI used to call it (CH8).
                Some("tool_use" | "server_tool_use") => {
                    let name = block
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    if let Some(id) = block.get("id").and_then(Value::as_str) {
                        self.tool_names.insert(id.to_string(), name.clone());
                    }
                    events.push(AgentEvent::ToolCall {
                        name,
                        input: block.get("input").cloned().unwrap_or(Value::Null),
                    });
                }
                // Thinking blocks stay out of the run report.
                _ => {}
            }
        }
        events
    }

    /// The tool results of one user (i.e. tool-return) message, each named from
    /// the `tool_use` it answers.
    fn tool_result_events(&mut self, value: &Value) -> Vec<AgentEvent> {
        let mut events = Vec::new();
        for block in content_blocks(value) {
            if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                continue;
            }
            let name = block
                .get("tool_use_id")
                .and_then(Value::as_str)
                .and_then(|id| self.tool_names.get(id))
                .cloned()
                .unwrap_or_default();
            events.push(AgentEvent::ToolResult {
                name,
                output: text_of(block.get("content")),
            });
        }
        events
    }
}

/// The init event's tool list. A missing or unreadable list is an **empty** one,
/// never `None`: an init event this build cannot read is not a locked-down CLI,
/// so the run is refused rather than trusted (ADR-010 §1).
fn tool_list(value: &Value) -> Vec<String> {
    value
        .get("tools")
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| tool.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn content_blocks(value: &Value) -> impl Iterator<Item = &Value> {
    value
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

/// The final usage the CLI reported, if any (the core never guesses).
fn usage_event(value: &Value) -> Vec<AgentEvent> {
    let usage = value.get("usage");
    let number = |key: &str| {
        usage
            .and_then(|usage| usage.get(key))
            .and_then(Value::as_i64)
    };
    let event = AgentEvent::Usage {
        tokens_in: number("input_tokens"),
        tokens_out: number("output_tokens"),
        cost_usd: value.get("total_cost_usd").and_then(Value::as_f64),
    };
    vec![event]
}

/// A tool result's content as text: Claude sends a string or a list of blocks.
fn text_of(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .map(|block| {
                block
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Recorded probe fixtures. Source flags verified on 2026-10-04 (A.1).
    const HELP: &str = include_str!("fixtures/claude_code_help.txt");
    const VERSION: &str = include_str!("fixtures/claude_code_version.txt");
    const STREAM: &str = include_str!("fixtures/claude_code_stream.jsonl");

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
        assert!(VERSION.contains("2.1.301"));
    }

    #[test]
    fn empty_help_probes_nothing() {
        assert_eq!(ClaudeCodeDriver.capabilities(""), Capabilities::default());
    }

    /// The recorded stream (2.1.301) must come out as the agent's own work:
    /// prose, tool calls named on both sides, and the reported usage.
    #[test]
    fn a_recorded_stream_becomes_events() {
        let mut stream = Stream::default();
        let mut events = Vec::new();
        let mut init = None;
        for line in STREAM.lines().filter(|line| !line.trim().is_empty()) {
            let parsed = stream.line(line);
            init = init.or(parsed.init_tools);
            events.extend(parsed.events);
        }

        // ADR-010 §1: the init event's tool list, all of it ours.
        assert_eq!(
            init.as_deref(),
            Some(
                [
                    "mcp__boba__get_task_input".to_string(),
                    "mcp__boba__propose_posting_features".to_string()
                ]
                .as_slice()
            )
        );
        // The CLI's own hook and rate-limit traffic never becomes events.
        assert!(
            events.iter().all(|event| !matches!(
                event,
                AgentEvent::ToolCall { name, .. } if name.contains("hook")
            )),
            "{events:?}"
        );
        assert_eq!(
            events,
            vec![
                AgentEvent::Text {
                    text: "I'll read the postings first.".to_string()
                },
                AgentEvent::ToolCall {
                    name: "mcp__boba__get_task_input".to_string(),
                    input: serde_json::json!({}),
                },
                // The tool result is named from the call it answers.
                AgentEvent::ToolResult {
                    name: "mcp__boba__get_task_input".to_string(),
                    output: r#"{"input":"extract the offers"}"#.to_string(),
                },
                AgentEvent::Text {
                    text: "Three postings extracted.".to_string()
                },
                AgentEvent::Usage {
                    tokens_in: Some(1200),
                    tokens_out: Some(40),
                    cost_usd: Some(0.035308),
                },
            ]
        );
    }

    /// CH8: a stream block that invokes a tool is a tool call whatever its type
    /// — `server_tool_use` is the CLI's other tool-invoking block, and dropping
    /// it would hide a native tool from the untrusted-run gate.
    #[test]
    fn a_server_tool_use_block_is_a_tool_call_too() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"server_tool_use","id":"srv1","name":"WebSearch","input":{"query":"x"}}]}}"#;
        let parsed = Stream::default().line(line);
        assert_eq!(
            parsed.events,
            vec![AgentEvent::ToolCall {
                name: "WebSearch".to_string(),
                input: serde_json::json!({"query": "x"}),
            }]
        );
        // And its result resolves to the same name, from the id the call recorded.
        let mut stream = Stream::default();
        stream.line(line);
        let result = stream.line(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"srv1","content":"ok"}]}}"#,
        );
        assert_eq!(
            result.events,
            vec![AgentEvent::ToolResult {
                name: "WebSearch".to_string(),
                output: "ok".to_string(),
            }]
        );
    }

    /// An init event listing a native tool is never "locked down", and a stream
    /// line that is not the documented JSON stays visible as text.
    #[test]
    fn a_native_tool_in_the_init_event_is_reported_as_such() {
        let hostile = r#"{"type":"system","subtype":"init","tools":["mcp__boba__get_task_input","Bash","WebFetch"]}"#;
        let parsed = Stream::default().line(hostile);
        let tools = parsed.init_tools.expect("an init event");
        assert!(!crate::lockdown::init_event_is_locked_down(&tools));
        assert!(parsed.events.is_empty());

        // An init event whose tool list cannot be read is an empty one, never a
        // pass: fail closed.
        let unreadable = Stream::default().line(r#"{"type":"system","subtype":"init"}"#);
        assert_eq!(unreadable.init_tools, Some(Vec::new()));

        // Nothing but the init event carries a tool list.
        let other = Stream::default().line(r#"{"type":"assistant","message":{"content":[]}}"#);
        assert_eq!(other.init_tools, None);

        let prose = Stream::default().line("Error: not authenticated");
        assert_eq!(
            prose.events,
            vec![AgentEvent::Text {
                text: "Error: not authenticated".to_string()
            }]
        );
    }
}
