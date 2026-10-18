//! End-to-end test of the shim: the real `boba-mcp` binary, a real owner-only
//! local socket, and a fake core that scripts its replies. Proves the shim
//! authenticates first, relays request/notification framing correctly and never
//! invents output (AD-6).

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};
use std::thread::JoinHandle;

use boba_mcp_proto::schema as wire;
use interprocess::local_socket::traits::Listener as _;
use serde_json::{Value, json};

/// A core stand-in: it accepts one connection and answers with `reply`.
struct FakeCore {
    dir: tempfile::TempDir,
    endpoint: String,
    received: JoinHandle<Vec<Value>>,
}

impl FakeCore {
    /// `reply` returns `None` to drop the connection without answering.
    fn start(reply: impl Fn(&Value) -> Option<Value> + Send + 'static) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let endpoint = wire::endpoint(dir.path(), "01RUN");
        let listener = wire::listener(&endpoint).expect("bind the run's endpoint");

        let received = std::thread::spawn(move || {
            let mut seen = Vec::new();
            let Ok(stream) = listener.accept() else {
                return seen;
            };
            let mut core = BufReader::new(stream);
            while let Ok(Some(message)) = read_message(&mut core) {
                let answer = reply(&message);
                seen.push(message);
                match answer {
                    Some(answer) => {
                        let stream = core.get_mut();
                        let _ = writeln!(stream, "{answer}");
                        let _ = stream.flush();
                    }
                    None => break,
                }
            }
            seen
        });
        Self {
            dir,
            endpoint,
            received,
        }
    }

    fn messages(self) -> Vec<Value> {
        self.received.join().expect("fake core thread")
    }
}

fn read_message(core: &mut BufReader<wire::Stream>) -> std::io::Result<Option<Value>> {
    let mut line = String::new();
    if core.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    Ok(serde_json::from_str(line.trim()).ok())
}

/// The shim cargo built for these tests. `CARGO_BIN_EXE_*` is the path cargo
/// guarantees and builds before the tests run; it is never a guess at
/// `target/debug`.
///
/// The target is `boba-mcp-shim`, not `boba-mcp`, because `tauri-build` copies
/// `src-tauri/binaries/boba-mcp-<triple>` — a 0-byte placeholder until a
/// release build fills it — to `target/<profile>/boba-mcp` whenever the
/// desktop crate builds. A bin target of that name races that copy and loses:
/// the shim tests then spawn an empty, non-executable file and fail with
/// `PermissionDenied` (FX20). The shipped sidecar is still named `boba-mcp`
/// (AD-6).
fn shim_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_boba-mcp-shim"))
}

/// FX20's regression: the binary these tests spawn is the one cargo built, not
/// a sidecar placeholder copied over it.
#[test]
fn the_spawned_shim_is_the_binary_cargo_built() {
    let path = shim_path();
    let metadata = std::fs::metadata(&path)
        .unwrap_or_else(|error| panic!("shim at {} is not runnable: {error}", path.display()));
    assert!(
        metadata.is_file(),
        "shim at {} is not a file",
        path.display()
    );
    assert!(
        metadata.len() > 0,
        "shim at {} is empty: a sidecar placeholder was copied over it",
        path.display()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_ne!(
            metadata.permissions().mode() & 0o111,
            0,
            "shim at {} has no execute bit",
            path.display()
        );
    }
}

/// Run the shim with `env`, feed it `input`, return status, stdout and stderr.
fn run_shim(env: &[(&str, &str)], input: &str) -> (ExitStatus, String, String) {
    let shim = shim_path();
    let mut child = Command::new(&shim)
        .env_clear()
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("spawn the shim {}: {error}", shim.display()));

    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("shim output");
    (
        output.status,
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn authenticates_then_relays_requests_and_notifications() {
    let core = FakeCore::start(|message| match message["method"].as_str() {
        Some(wire::METHOD_AUTH) => Some(json!({"jsonrpc": "2.0", "id": 0, "result": {"ok": true}})),
        Some(wire::METHOD_TOOLS_LIST) => {
            Some(json!({"jsonrpc": "2.0", "id": message["id"], "result": {"tools": []}}))
        }
        // A notification gets no reply.
        _ => None,
    });
    let token = "run-token";

    let (status, stdout, stderr) = run_shim(
        &[
            (wire::ENV_TOKEN, token),
            (wire::ENV_ENDPOINT, core.endpoint.as_str()),
        ],
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n\
         {\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
    );

    let replies: Vec<Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("stdout is JSON-RPC only"))
        .collect();
    assert_eq!(replies.len(), 1, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(replies[0]["id"], 1);
    assert!(status.success(), "{status} stderr: {stderr}");

    let seen = core.messages();
    assert_eq!(seen.len(), 3, "{seen:?}");
    assert_eq!(seen[0]["method"], wire::METHOD_AUTH);
    assert_eq!(seen[0]["params"]["token"], token);
    assert_eq!(seen[1]["method"], wire::METHOD_TOOLS_LIST);
    assert_eq!(seen[2]["method"], "notifications/initialized");
    // The token never reaches stdout.
    assert!(!stdout.contains(token));
}

#[test]
fn a_missing_token_refuses_to_start() {
    let (status, stdout, stderr) = run_shim(&[], "");
    assert!(!status.success());
    assert!(stdout.is_empty(), "stdout: {stdout:?}");
    assert!(stderr.contains(wire::ENV_TOKEN), "stderr: {stderr}");
}

#[test]
fn a_rejected_token_exits_nonzero_and_prints_no_jsonrpc() {
    let core = FakeCore::start(|_| {
        Some(wire::error(
            json!(0),
            wire::CODE_UNAUTHORIZED,
            "unknown run token",
        ))
    });
    let (status, stdout, stderr) = run_shim(
        &[
            (wire::ENV_TOKEN, "stale"),
            (wire::ENV_ENDPOINT, core.endpoint.as_str()),
        ],
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n",
    );

    assert!(!status.success());
    assert!(stdout.is_empty(), "stdout: {stdout:?}");
    assert!(stderr.contains("refused"), "stderr: {stderr}");
    // The refused message was never relayed to the core.
    assert_eq!(core.messages().len(), 1);
}

#[test]
fn an_oversized_stdin_frame_is_refused_and_never_relayed() {
    let core = FakeCore::start(|message| {
        (message["method"] == wire::METHOD_AUTH)
            .then(|| json!({"jsonrpc": "2.0", "id": 0, "result": {"ok": true}}))
    });
    let huge = format!("{}\n", "x".repeat(wire::MAX_LINE_BYTES + 1));

    let (status, stdout, stderr) = run_shim(
        &[
            (wire::ENV_TOKEN, "token"),
            (wire::ENV_ENDPOINT, core.endpoint.as_str()),
        ],
        &huge,
    );

    assert!(!status.success());
    assert!(stdout.is_empty(), "stdout: {stdout:?}");
    assert!(stderr.contains("stdin"), "stderr: {stderr}");
    // Only the handshake reached the core.
    assert_eq!(core.messages().len(), 1);
}

#[test]
fn a_core_that_disappears_is_an_error_not_an_empty_toolset() {
    let core = FakeCore::start(|message| {
        (message["method"] == wire::METHOD_AUTH)
            .then(|| json!({"jsonrpc": "2.0", "id": 0, "result": {"ok": true}}))
    });
    let (status, stdout, stderr) = run_shim(
        &[
            (wire::ENV_TOKEN, "token"),
            (wire::ENV_ENDPOINT, core.endpoint.as_str()),
        ],
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n",
    );

    assert!(!status.success());
    assert!(stdout.is_empty(), "stdout: {stdout:?}");
    assert!(stderr.contains("closed the connection"), "stderr: {stderr}");
    drop(core.dir);
}
