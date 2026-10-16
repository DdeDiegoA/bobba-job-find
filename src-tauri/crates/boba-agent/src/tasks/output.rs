//! Checking a task's output against its `boba-mcp-proto` schema, and what a run
//! does when the output does not match (AC2).
//!
//! A task's output is the arguments of one of its write tools (see
//! [`super::outputs`]) — the proposal or draft the core will store. Every one of
//! them is checked against the tool's `input_schema` before it is accepted, so a
//! truncated or invented payload never reaches the repos. The MCP endpoint hands
//! an invalid output back to the agent, and once the retry budget is spent the
//! run stops with `FailureReason::InvalidOutput` — visible, never a loop.
//!
//! The catalog's schemas are plain object schemas (`type`, `properties`,
//! `required`, `additionalProperties: false`, `items`, `minimum`), so the check
//! below implements exactly those keywords and the tests refuse a catalog schema
//! that leaves them.
//!
//! shortcut: a hand-rolled subset, not full JSON Schema; replace it with a
//! validator crate once a catalog tool describes itself with a keyword outside
//! the list above.

use boba_mcp_proto::tool;
use serde_json::Value;

use super::is_output;

/// How many times an invalid output is handed back to the agent before the task
/// fails (AC2: retried up to 2 times, then it fails visibly).
pub const MAX_OUTPUT_RETRIES: u32 = 2;

/// Why a run's output was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidOutput {
    /// The tool is not this task's output at all: a read tool, or another
    /// task's write tool.
    NotAnOutput { task: String, tool: String },
    /// The arguments do not match the tool's schema.
    Schema { tool: String, why: String },
}

impl std::fmt::Display for InvalidOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InvalidOutput::NotAnOutput { task, tool } => {
                write!(f, "`{tool}` is not an output of {task}")
            }
            InvalidOutput::Schema { tool, why } => write!(f, "`{tool}`: {why}"),
        }
    }
}

/// What to do after an output was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputVerdict {
    /// Send it back to the agent: it still has retries left.
    Retry,
    /// The retry budget is spent: the task fails visibly, never loops.
    Fail,
}

/// One run's invalid-output budget. The MCP endpoint owns one per run, so the
/// count is the run's, not the process's.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OutputAttempts {
    refusals: u32,
}

impl OutputAttempts {
    /// Record one invalid output and say whether another try is allowed.
    pub fn refused(&mut self) -> OutputVerdict {
        self.refusals += 1;
        if self.refusals > MAX_OUTPUT_RETRIES {
            OutputVerdict::Fail
        } else {
            OutputVerdict::Retry
        }
    }

    /// Refusals recorded so far.
    pub fn refusals(&self) -> u32 {
        self.refusals
    }
}

/// Check one output: `args` is what the agent passed to `tool` for `task`.
///
/// [`InvalidOutput::NotAnOutput`] is a wiring bug (the endpoint serves only the
/// task's subset); a schema mismatch is the agent's, and retryable.
pub fn check_output(task: &str, tool_name: &str, args: &Value) -> Result<(), InvalidOutput> {
    if !is_output(task, tool_name) {
        return Err(InvalidOutput::NotAnOutput {
            task: task.to_string(),
            tool: tool_name.to_string(),
        });
    }
    let spec = tool::find(tool_name).expect("outputs() only names catalog tools");
    let schema: Value =
        serde_json::from_str(spec.input_schema).expect("catalog schemas are valid JSON");
    fits(&schema, args).map_err(|why| InvalidOutput::Schema {
        tool: tool_name.to_string(),
        why,
    })
}

/// `fits` is the checker's core: the catalog's own schemas are exercised
/// against it in the tests below, including the one carrying `minimum`.
fn fits(schema: &Value, value: &Value) -> Result<(), String> {
    match schema.get("type").and_then(Value::as_str) {
        Some("object") => object(schema, value),
        Some("array") => array(schema, value),
        Some("string") => expect(value.is_string(), "a string"),
        Some("integer") => {
            expect(value.is_i64() || value.is_u64(), "an integer")?;
            minimum(schema, value)
        }
        Some(other) => Err(format!("unsupported schema type `{other}`")),
        // No type: nothing to check.
        None => Ok(()),
    }
}

fn object(schema: &Value, value: &Value) -> Result<(), String> {
    let Some(map) = value.as_object() else {
        return Err("an object".to_string());
    };
    let properties = schema.get("properties").and_then(Value::as_object);
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str);
    for key in required {
        if !map.contains_key(key) {
            return Err(format!("`{key}` is required"));
        }
    }
    for (key, item) in map {
        match properties.and_then(|properties| properties.get(key)) {
            Some(property) => fits(property, item)?,
            None if schema.get("additionalProperties").and_then(Value::as_bool) == Some(false) => {
                return Err(format!("`{key}` is not allowed here"));
            }
            None => {}
        }
    }
    Ok(())
}

fn array(schema: &Value, value: &Value) -> Result<(), String> {
    let Some(items) = value.as_array() else {
        return Err("an array".to_string());
    };
    if let Some(item_schema) = schema.get("items") {
        for item in items {
            fits(item_schema, item)?;
        }
    }
    Ok(())
}

fn minimum(schema: &Value, value: &Value) -> Result<(), String> {
    let Some(min) = schema.get("minimum").and_then(Value::as_i64) else {
        return Ok(());
    };
    let Some(number) = value.as_i64() else {
        return Ok(());
    };
    expect(number >= min, &format!("an integer >= {min}"))
}

fn expect(ok: bool, what: &str) -> Result<(), String> {
    if ok {
        Ok(())
    } else {
        Err(format!("expected {what}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_valid_output_passes() {
        assert_eq!(
            check_output(
                tool::EXTRACT_POSTINGS,
                "propose_posting_features",
                &json!({"id": "pst_1", "features": {"skills": ["rust"]}}),
            ),
            Ok(())
        );
        assert_eq!(
            check_output(
                tool::PROFILE_IMPORT,
                "propose_profile_facts",
                &json!({"facts": [{"kind": "skill"}]}),
            ),
            Ok(())
        );
    }

    #[test]
    fn a_missing_a_mistyped_or_extra_field_is_refused() {
        let cases = [
            // Missing a required field.
            json!({"features": {}}),
            // Wrong type.
            json!({"id": 7, "features": {}}),
            // An extra field the schema does not allow.
            json!({"id": "pst_1", "features": {}, "note": "hi"}),
            // Not an object at all.
            json!("pst_1"),
        ];
        for args in cases {
            let refused = check_output(tool::EXTRACT_POSTINGS, "propose_posting_features", &args)
                .expect_err(&format!("{args} must be refused"));
            assert!(
                matches!(refused, InvalidOutput::Schema { .. }),
                "{args}: {refused:?}"
            );
            assert!(refused.to_string().contains("propose_posting_features"));
        }
    }

    #[test]
    fn a_read_tool_or_another_tasks_tool_is_not_an_output() {
        for (task, tool_name) in [
            (tool::EXTRACT_POSTINGS, "get_pending_postings"),
            (tool::EXTRACT_POSTINGS, "get_task_input"),
            (tool::EXTRACT_POSTINGS, "save_document_draft"),
            (tool::NARRATE_SCORES, "save_review"),
            (tool::PROFILE_IMPORT, "made_up_tool"),
        ] {
            let refused = check_output(task, tool_name, &json!({}))
                .expect_err(&format!("{tool_name} must not be an output of {task}"));
            assert!(
                matches!(refused, InvalidOutput::NotAnOutput { .. }),
                "{refused:?}"
            );
        }
    }

    #[test]
    fn a_numeric_bound_in_a_schema_is_enforced() {
        let schema: Value = serde_json::from_str(
            tool::find("get_pending_postings")
                .expect("catalog tool")
                .input_schema,
        )
        .expect("JSON");
        assert_eq!(fits(&schema, &json!({"limit": 1})), Ok(()));
        assert!(fits(&schema, &json!({"limit": 0})).is_err());
        assert!(fits(&schema, &json!({"limit": "5"})).is_err());
        assert!(fits(&schema, &json!({})).is_err());
    }

    #[test]
    fn an_invalid_output_is_re_offered_twice_and_then_the_task_fails() {
        let mut attempts = OutputAttempts::default();
        assert_eq!(attempts.refused(), OutputVerdict::Retry);
        assert_eq!(attempts.refused(), OutputVerdict::Retry);
        assert_eq!(attempts.refused(), OutputVerdict::Fail);
        assert_eq!(attempts.refusals(), 3);
        // And it stays failed however many more arrive.
        assert_eq!(attempts.refused(), OutputVerdict::Fail);
        assert_eq!(MAX_OUTPUT_RETRIES, 2);
    }

    #[test]
    fn every_catalog_schema_is_within_the_supported_subset() {
        // A catalog schema that leaves these keywords must fail this test, not
        // silently pass validation on the day a task starts writing.
        const KEYWORDS: [&str; 6] = [
            "type",
            "properties",
            "required",
            "additionalProperties",
            "items",
            "minimum",
        ];
        for spec in tool::TOOLS {
            let schema: Value = serde_json::from_str(spec.input_schema).expect("JSON schema");
            let mut seen = Vec::new();
            collect_keywords(&schema, &mut seen);
            for keyword in seen {
                assert!(
                    KEYWORDS.contains(&keyword.as_str()),
                    "`{}` uses `{keyword}`, outside the checker subset",
                    spec.name
                );
            }
        }
    }

    /// Every write tool's schema must reject the empty object, so a task cannot
    /// "succeed" by writing nothing at all.
    #[test]
    fn no_output_schema_accepts_an_empty_write() {
        for spec in tool::TOOLS {
            if !spec.kind.is_write() {
                continue;
            }
            let schema: Value = serde_json::from_str(spec.input_schema).expect("JSON schema");
            assert!(
                fits(&schema, &json!({})).is_err(),
                "`{}` accepts `{{}}`",
                spec.name
            );
        }
    }

    fn collect_keywords(schema: &Value, out: &mut Vec<String>) {
        let Some(map) = schema.as_object() else {
            return;
        };
        for (key, value) in map {
            out.push(key.clone());
            match key.as_str() {
                "properties" => {
                    for property in value.as_object().into_iter().flatten().map(|(_, v)| v) {
                        collect_keywords(property, out);
                    }
                }
                "items" => collect_keywords(value, out),
                _ => {}
            }
        }
    }
}
