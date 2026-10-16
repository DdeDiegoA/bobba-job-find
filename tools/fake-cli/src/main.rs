//! Fake agent CLI (owned by S2.2). It replays recorded JSONL for tests and E2E
//! runs, and can emit canned scenarios. Mode is chosen by `BOBA_FAKE_MODE`.
//!
//! Modes:
//!   replay        — print `BOBA_FAKE_LINES` (newline-separated JSONL), exit 0
//!   echo          — print `one` and `two`, exit 0
//!   claude-stream — print a Claude Code `stream-json` run (init event, one
//!                   assistant message, result); the init event's tool list is
//!                   `BOBA_FAKE_TOOLS` (comma-separated, default one boba tool),
//!                   which is how ADR-010's run-time observation is tested both
//!                   ways
//!   spawn-child   — spawn a background grandchild that touches
//!                   `BOBA_FAKE_MARKER` after `BOBA_FAKE_DELAY_MS`, then exit 0
//!   emit-lines    — alias of replay, kept for readability in fixtures
//!   fail-auth     — write an auth error to stderr, exit 1
//!
//! ponytail: modes are tiny on purpose; add one only when a test needs it.

use std::process::{Command, Stdio};
use std::time::Duration;

fn main() {
    let mode = std::env::var("BOBA_FAKE_MODE").unwrap_or_else(|_| "echo".to_string());
    match mode.as_str() {
        "echo" => println!("one\ntwo"),
        "replay" | "emit-lines" => replay(),
        "claude-stream" => claude_stream(),
        "spawn-child" => spawn_child(),
        "fail-auth" => fail_auth(),
        other => {
            eprintln!("unknown BOBA_FAKE_MODE: {other}");
            std::process::exit(2);
        }
    }
}

/// A Claude Code `stream-json` run, in the recorded shape (2.1.301).
fn claude_stream() {
    let tools = std::env::var("BOBA_FAKE_TOOLS")
        .unwrap_or_else(|_| "mcp__boba__get_task_input".to_string());
    let tools: Vec<String> = tools
        .split(',')
        .filter(|tool| !tool.is_empty())
        .map(|tool| format!("\"{tool}\""))
        .collect();
    println!(
        r#"{{"type":"system","subtype":"init","cwd":".","tools":[{}],"mcp_servers":[{{"name":"boba","status":"connected"}}],"claude_code_version":"2.1.301"}}"#,
        tools.join(",")
    );
    println!(r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"done"}}]}}}}"#);
    println!(
        r#"{{"type":"result","subtype":"success","result":"done","num_turns":1,"total_cost_usd":0.01,"usage":{{"input_tokens":10,"output_tokens":2}}}}"#
    );
}

fn replay() {
    match std::env::var("BOBA_FAKE_LINES") {
        Ok(blob) => {
            for line in blob.lines() {
                println!("{line}");
            }
        }
        // Default recording: a full, well-formed run, opened the way a Claude
        // Code `stream-json` run is — with its init event, whose tool list is
        // where ADR-010 §1's observation comes from.
        Err(_) => {
            println!(
                r#"{{"type":"system","subtype":"init","tools":["mcp__boba__get_task_input","mcp__boba__propose_posting_features"]}}"#
            );
            println!(r#"{{"type":"started","cli":"claude_code","model":"sonnet"}}"#);
            println!(r#"{{"type":"text","text":"Working on it"}}"#);
            println!(r#"{{"type":"tool_call","name":"get_posting","input":{{"id":"01H"}}}}"#);
            println!(r#"{{"type":"tool_result","name":"get_posting","output":"ok"}}"#);
            println!(r#"{{"type":"usage","tokens_in":120,"tokens_out":40,"cost_usd":0.002}}"#);
            println!(r#"{{"type":"finished","exit_code":0}}"#);
        }
    }
}

fn spawn_child() {
    let marker = std::env::var("BOBA_FAKE_MARKER").unwrap_or_else(|_| "/tmp/boba-marker".into());
    let delay_ms: u64 = std::env::var("BOBA_FAKE_DELAY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(400);

    // A grandchild that keeps running until after the parent is killed. Its
    // process group is the fake CLI's, so a group kill must take it down too.
    let script = format!("sleep {}; : > {}", delay_ms as f64 / 1000.0, marker);
    let _ = Command::new("sh")
        .arg("-c")
        .arg(&script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();

    // Keep the parent alive long enough for the test to kill the group.
    std::thread::sleep(Duration::from_millis(delay_ms + 2_000));
    println!(r#"{{"type":"finished","exit_code":0}}"#);
}

fn fail_auth() {
    eprintln!("error: not authenticated, please log in");
    std::process::exit(1);
}
