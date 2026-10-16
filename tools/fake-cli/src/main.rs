//! Fake agent CLI (owned by S2.2). It replays recorded JSONL for tests and E2E
//! runs, and can emit canned scenarios. Mode is chosen by `BOBA_FAKE_MODE`.
//!
//! Modes:
//!   replay        — print `BOBA_FAKE_LINES` (newline-separated JSONL), exit 0
//!   echo          — print `one` and `two`, exit 0
//!   silent        — print nothing and exit 0: a CLI that never showed the init
//!                   event an untrusted run is verified against
//!   silent-fail   — the same, exiting non-zero
//!   claude-stream — print a Claude Code `stream-json` run (init event, one
//!                   assistant message, result); the init event's tool list is
//!                   `BOBA_FAKE_TOOLS` (comma-separated, default one boba tool),
//!                   which is how ADR-010's run-time observation is tested both
//!                   ways
//!   spawn-child   — spawn a background grandchild that touches
//!                   `BOBA_FAKE_MARKER` after `BOBA_FAKE_DELAY_MS`, then exit 0
//!   emit-lines    — alias of replay, kept for readability in fixtures
//!   fail-auth     — write an auth error to stderr, exit 1. `BOBA_FAKE_STDERR_HOLD_MS`
//!                   also leaves a grandchild holding stderr open that long,
//!                   so a test can prove a run waits for stderr to close
//!   mcp-run       — open with the init event, then speak the run's MCP endpoint
//!                   (`BOBA_IPC_ENDPOINT`/`BOBA_RUN_TOKEN`): authenticate and make
//!                   one `tools/call` per entry in `BOBA_FAKE_CALLS` (JSON array
//!                   of `{"name":…,"arguments":…}`), reporting each reply as a
//!                   text line. It then holds so the **runtime's** own limit —
//!                   output failure or wall clock — is what ends the run.
//!
//! ponytail: modes are tiny on purpose; add one only when a test needs it.

use std::io::{BufReader, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

use boba_mcp_proto::schema as wire;

fn main() {
    let mode = std::env::var("BOBA_FAKE_MODE").unwrap_or_else(|_| "echo".to_string());
    match mode.as_str() {
        "echo" => println!("one\ntwo"),
        "silent" => {}
        "silent-fail" => std::process::exit(3),
        "replay" | "emit-lines" => replay(),
        "claude-stream" => claude_stream(),
        "spawn-child" => spawn_child(),
        "fail-auth" => fail_auth(),
        "mcp-run" => mcp_run(),
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
            println!(
                r#"{{"type":"tool_call","name":"mcp__boba__get_posting","input":{{"id":"01H"}}}}"#
            );
            println!(r#"{{"type":"tool_result","name":"mcp__boba__get_posting","output":"ok"}}"#);
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
    // A grandchild keeps the stderr pipe open past this process (its stdio is
    // inherited, unlike `spawn-child`). The reader only sees stderr at EOF, so
    // this is how a test forces a run to wait for the pipe to close.
    if let Ok(ms) = std::env::var("BOBA_FAKE_STDERR_HOLD_MS")
        && let Ok(ms) = ms.parse::<u64>()
    {
        let _ = Command::new("sh")
            .arg("-c")
            .arg(format!("sleep {}", ms as f64 / 1000.0))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn();
    }
    std::process::exit(1);
}

/// A fake run that talks to its own MCP endpoint, so a test can drive the whole
/// chain — workspace, endpoint, init-event check, tool policy and output
/// validation — end to end.
fn mcp_run() {
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

    // What the runtime put in this run's workspace: the agent pack it furnished
    // and the MCP config it wrote.
    let cwd = std::env::current_dir().unwrap_or_default();
    println!(
        "{}",
        serde_json::json!({
            "type": "text",
            "text": format!(
                "cwd={} pack={} mcp={}",
                cwd.display(),
                std::path::Path::new("CLAUDE.md").exists(),
                std::path::Path::new("mcp.json").exists(),
            ),
        })
    );

    let calls: Vec<serde_json::Value> = std::env::var("BOBA_FAKE_CALLS")
        .ok()
        .and_then(|blob| serde_json::from_str(&blob).ok())
        .unwrap_or_default();
    match mcp_calls(&calls) {
        Ok(replies) => {
            for reply in replies {
                println!("{}", serde_json::json!({"type": "text", "text": reply}));
            }
        }
        Err(error) => println!(
            "{}",
            serde_json::json!({"type": "text", "text": format!("mcp-error: {error}")})
        ),
    }

    // Hold: the runtime's own limit (output failure, wall clock) must be what
    // ends this run, so a passing test proves that limit fired rather than an
    // orderly exit. `BOBA_FAKE_HOLD_MS=0` lets a clean run exit instead.
    let hold_ms: u64 = std::env::var("BOBA_FAKE_HOLD_MS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(10_000);
    if hold_ms > 0 {
        std::thread::sleep(Duration::from_millis(hold_ms));
    }
    println!(
        r#"{{"type":"result","subtype":"success","result":"done","num_turns":1,"total_cost_usd":0.0,"usage":{{"input_tokens":0,"output_tokens":0}}}}"#
    );
}

/// Authenticate with the run's endpoint, then one `tools/call` per entry.
fn mcp_calls(calls: &[serde_json::Value]) -> std::io::Result<Vec<String>> {
    let endpoint = std::env::var(wire::ENV_ENDPOINT).unwrap_or_default();
    let token = std::env::var(wire::ENV_TOKEN).unwrap_or_default();
    let mut core = BufReader::new(wire::connect(&endpoint)?);

    let mut replies = Vec::new();
    let auth = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": wire::METHOD_AUTH,
        "params": {"token": token},
    });
    replies.push(format!("auth -> {}", outcome(&request(&mut core, &auth)?)));
    for (index, call) in calls.iter().enumerate() {
        let message = serde_json::json!({
            "jsonrpc": "2.0",
            "id": index + 2,
            "method": wire::METHOD_TOOLS_CALL,
            "params": {
                "name": call.get("name").cloned().unwrap_or(serde_json::Value::Null),
                "arguments": call.get("arguments").cloned().unwrap_or_else(|| serde_json::json!({})),
            },
        });
        let reply = request(&mut core, &message)?;
        replies.push(format!(
            "{} -> {}",
            call["name"].as_str().unwrap_or("?"),
            outcome(&reply)
        ));
    }
    Ok(replies)
}

/// Send one request and read its reply.
fn request(
    core: &mut BufReader<wire::Stream>,
    message: &serde_json::Value,
) -> std::io::Result<serde_json::Value> {
    let stream = core.get_mut();
    stream.write_all(message.to_string().as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    let line = wire::read_message_line(core)?.unwrap_or_default();
    Ok(serde_json::from_str(&line).unwrap_or(serde_json::Value::Null))
}

/// A reply's error code, or `ok` when it succeeded.
fn outcome(reply: &serde_json::Value) -> String {
    match reply["error"]["code"].as_i64() {
        Some(code) => code.to_string(),
        None => "ok".to_string(),
    }
}
