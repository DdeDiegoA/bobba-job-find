//! `boba-mcp` — the stdio ↔ core local-IPC shim (AD-6, architecture §5.4).
//!
//! It owns no state and no policy. It connects to the run's owner-only local
//! socket, authenticates with the per-run token from `BOBA_RUN_TOKEN`, then
//! forwards every MCP JSON-RPC line from stdin to the core and writes the
//! core's reply back to stdout. The core decides the tool subset, the per-run
//! caps and the untrusted envelopes.
//!
//! stdout carries JSON-RPC only; every diagnostic goes to stderr. A failed
//! handshake exits nonzero, so the CLI reports a broken MCP server instead of
//! running with a silently empty toolset.

use std::io::{BufRead, BufReader, Write};
use std::process::ExitCode;

use boba_mcp_proto::schema as wire;
use serde_json::{Value, json};

fn main() -> ExitCode {
    match serve() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("boba-mcp: {message}");
            ExitCode::FAILURE
        }
    }
}

fn serve() -> Result<(), String> {
    let token = std::env::var(wire::ENV_TOKEN)
        .ok()
        .filter(|token| !token.is_empty())
        .ok_or_else(|| format!("{} is not set", wire::ENV_TOKEN))?;
    let endpoint = std::env::var(wire::ENV_ENDPOINT)
        .ok()
        .filter(|endpoint| !endpoint.is_empty())
        .ok_or_else(|| format!("{} is not set", wire::ENV_ENDPOINT))?;

    let stream = wire::connect(&endpoint)
        .map_err(|e| format!("cannot reach the core on {endpoint}: {e}"))?;
    let mut core = BufReader::new(stream);
    authenticate(&mut core, &token)?;
    let stdin = std::io::stdin();
    relay(&mut core, &mut stdin.lock())
}

/// Prove possession of the per-run token before relaying anything.
fn authenticate(core: &mut BufReader<wire::Stream>, token: &str) -> Result<(), String> {
    let hello = json!({
        "jsonrpc": "2.0",
        "id": 0,
        "method": wire::METHOD_AUTH,
        "params": {"token": token},
    });
    send(core, &hello.to_string())?;
    let reply: Value = serde_json::from_str(&recv(core)?)
        .map_err(|e| format!("the core's handshake reply is not JSON: {e}"))?;
    match reply.get("error") {
        Some(error) => Err(format!(
            "the core refused this run: {}",
            error["message"].as_str().unwrap_or("unauthorised")
        )),
        None => Ok(()),
    }
}

/// Forward MCP messages until stdin ends.
fn relay(core: &mut BufReader<wire::Stream>, input: &mut impl BufRead) -> Result<(), String> {
    let mut out = std::io::stdout().lock();
    while let Some(line) = read_from(input, "stdin")? {
        if line.trim().is_empty() {
            continue;
        }
        // Requests expect an answer; notifications do not.
        let expects_reply = serde_json::from_str::<Value>(&line)
            .map(|message| wire::is_request(&message))
            .unwrap_or(true);
        send(core, &line)?;
        if expects_reply {
            let reply = recv(core)?;
            writeln!(out, "{reply}")
                .and_then(|()| out.flush())
                .map_err(|e| format!("writing stdout: {e}"))?;
        }
    }
    Ok(())
}

fn send(core: &mut BufReader<wire::Stream>, line: &str) -> Result<(), String> {
    let stream = core.get_mut();
    stream
        .write_all(line.as_bytes())
        .and_then(|()| stream.write_all(b"\n"))
        .and_then(|()| stream.flush())
        .map_err(|e| format!("the core closed the connection: {e}"))
}

fn recv(core: &mut BufReader<wire::Stream>) -> Result<String, String> {
    read_from(core, "the core")
        .and_then(|line| line.ok_or_else(|| "the core closed the connection".to_string()))
}

/// One bounded line. Neither the CLI on stdin nor the core on the socket gets
/// to make this process allocate without limit (AD-6).
fn read_from(reader: &mut impl BufRead, what: &str) -> Result<Option<String>, String> {
    wire::read_message_line(reader)
        .map_err(|e| format!("{what} sent a message the shim refuses: {e}"))
}
