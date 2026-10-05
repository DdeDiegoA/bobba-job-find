//! Fake agent CLI (owned by S2.2). It replays recorded JSONL for tests and E2E
//! runs, and can emit canned scenarios. Mode is chosen by `BOBA_FAKE_MODE`.
//!
//! Modes:
//!   replay        — print `BOBA_FAKE_LINES` (newline-separated JSONL), exit 0
//!   echo          — print `one` and `two`, exit 0
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
        "spawn-child" => spawn_child(),
        "fail-auth" => fail_auth(),
        other => {
            eprintln!("unknown BOBA_FAKE_MODE: {other}");
            std::process::exit(2);
        }
    }
}

fn replay() {
    match std::env::var("BOBA_FAKE_LINES") {
        Ok(blob) => {
            for line in blob.lines() {
                println!("{line}");
            }
        }
        // Default recording: a full, well-formed run.
        Err(_) => {
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
