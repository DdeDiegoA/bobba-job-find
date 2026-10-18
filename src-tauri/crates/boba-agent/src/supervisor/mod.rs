//! Process supervision (AD-1): every child gets its own process group on Unix
//! or Job Object on Windows, so cancelling kills the whole tree. stdout is
//! streamed line by line; stderr is collected for error classification. The
//! wall-clock limit is enforced by the caller's loop, not here.

use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[cfg(windows)]
use process_wrap::std::JobObject;
#[cfg(unix)]
use process_wrap::std::ProcessGroup;
use process_wrap::std::{ChildWrapper, CommandWrap};
use thiserror::Error;

/// How to launch a child. `env` carries per-run values (MCP token, workspace).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Error)]
pub enum SupervisorError {
    #[error("failed to spawn `{program}`: {message}")]
    Spawn { program: String, message: String },
    #[error("the child has no captured stdout")]
    NoStdout,
}

/// How long `reap` waits for a pipe reader once the child is gone. Long enough
/// to drain a closed pipe — including one a grandchild closes a moment later,
/// which is what `run_lifecycle`'s late-stderr test relies on — and short
/// enough that a grandchild holding either end open cannot stall the run
/// (CH12: stdout is bounded like stderr).
const READER_JOIN_GRACE: Duration = Duration::from_secs(2);

/// One item from the child's stdout stream.
#[derive(Debug)]
pub enum Stream {
    Line(String),
    /// No line arrived within the timeout; re-check cancellation/limits.
    Idle,
    /// stdout closed; the process is finishing.
    Eof,
}

/// A spawned child in its own process group / Job Object.
pub struct SupervisedProcess {
    child: Box<dyn ChildWrapper>,
    lines: Receiver<String>,
    reader: Option<JoinHandle<()>>,
    stderr: Arc<Mutex<String>>,
    stderr_reader: Option<JoinHandle<()>>,
    reaped: bool,
}

impl SupervisedProcess {
    pub fn spawn(spec: &CommandSpec) -> Result<Self, SupervisorError> {
        let program = spec.program.clone();
        let display = program.display().to_string();
        let mut wrap = CommandWrap::with_new(program, |command| {
            command.args(&spec.args);
            command.stdin(Stdio::null());
            command.stdout(Stdio::piped());
            command.stderr(Stdio::piped());
            if let Some(cwd) = &spec.cwd {
                command.current_dir(cwd);
            }
            for (key, value) in &spec.env {
                command.env(key, value);
            }
        });
        #[cfg(unix)]
        wrap.wrap(ProcessGroup::leader());
        #[cfg(windows)]
        wrap.wrap(JobObject);

        let mut child = wrap.spawn().map_err(|e| SupervisorError::Spawn {
            program: display,
            message: e.to_string(),
        })?;

        let stdout = child.stdout().take().ok_or(SupervisorError::NoStdout)?;
        let stderr_pipe = child.stderr().take();
        let (tx, lines) = channel();
        let reader = thread::Builder::new()
            .name("boba-agent-stdout".to_string())
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                let mut buf = String::new();
                loop {
                    buf.clear();
                    match reader.read_line(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            let line = buf.trim_end_matches(['\n', '\r']).to_string();
                            if tx.send(line).is_err() {
                                break;
                            }
                        }
                    }
                }
            })
            .map_err(|e| SupervisorError::Spawn {
                program: "boba-agent-stdout".to_string(),
                message: e.to_string(),
            })?;

        let stderr = Arc::new(Mutex::new(String::new()));
        let mut stderr_reader = None;
        if let Some(pipe) = stderr_pipe {
            let sink = Arc::clone(&stderr);
            stderr_reader = thread::Builder::new()
                .name("boba-agent-stderr".to_string())
                .spawn(move || {
                    let mut text = String::new();
                    let mut reader = BufReader::new(pipe);
                    let _ = reader.read_to_string(&mut text);
                    if let Ok(mut guard) = sink.lock() {
                        guard.push_str(&text);
                    }
                })
                .ok();
        }

        Ok(Self {
            child,
            lines,
            reader: Some(reader),
            stderr,
            stderr_reader,
            reaped: false,
        })
    }

    /// Wait up to `timeout` for the next stdout line.
    pub fn recv_line(&self, timeout: Duration) -> Stream {
        match self.lines.recv_timeout(timeout) {
            Ok(line) => Stream::Line(line),
            Err(RecvTimeoutError::Timeout) => Stream::Idle,
            Err(RecvTimeoutError::Disconnected) => Stream::Eof,
        }
    }

    /// Everything the child has written to stderr so far.
    pub fn stderr_so_far(&self) -> String {
        self.stderr.lock().map(|g| g.clone()).unwrap_or_default()
    }

    /// Kill the whole process tree and reap it.
    pub fn kill(&mut self) {
        let _ = self.child.start_kill();
        self.reap();
    }

    /// Wait for the child to exit (and reap any remaining group members).
    pub fn wait(&mut self) -> Option<ExitStatus> {
        let status = self.child.wait().ok();
        self.reap();
        status
    }

    fn reap(&mut self) {
        if self.reaped {
            return;
        }
        self.reaped = true;
        let _ = self.child.wait();
        // Both joins are bounded: a grandchild that inherits either pipe can
        // hold it open for good, and the run must not hang on it. Past the
        // grace the reader is detached and the buffer is whatever it collected.
        if let Some(reader) = self.reader.take() {
            join_bounded(reader);
        }
        // `stderr_so_far()` must never race its reader, or an auth failure can
        // be classified from an empty string.
        if let Some(reader) = self.stderr_reader.take() {
            join_bounded(reader);
        }
    }
}

/// Join a pipe reader, but only if it is done within [`READER_JOIN_GRACE`].
fn join_bounded(reader: JoinHandle<()>) {
    let deadline = Instant::now() + READER_JOIN_GRACE;
    while !reader.is_finished() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(1));
    }
    if reader.is_finished() {
        let _ = reader.join();
    }
}

impl Drop for SupervisedProcess {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.child.start_kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(any(unix, windows))]
    use std::fs;
    #[cfg(any(unix, windows))]
    use std::time::Instant;

    /// The fake CLI, built on demand so a cold target still works (see
    /// `runtime::fake_cli_path`).
    fn fake_cli() -> PathBuf {
        crate::runtime::fake_cli_path()
    }

    fn spec(env: &[(&str, &str)]) -> CommandSpec {
        CommandSpec {
            program: fake_cli(),
            args: vec![],
            cwd: None,
            env: env
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
        }
    }

    #[test]
    fn streams_stdout_lines_then_eof() {
        let mut proc =
            SupervisedProcess::spawn(&spec(&[("BOBA_FAKE_MODE", "echo")])).expect("spawn");
        let mut lines = Vec::new();
        loop {
            match proc.recv_line(Duration::from_secs(5)) {
                Stream::Line(line) => lines.push(line),
                Stream::Eof => break,
                Stream::Idle => continue,
            }
        }
        assert_eq!(lines, vec!["one".to_string(), "two".to_string()]);
        assert!(proc.wait().expect("exit").success());
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn kill_tears_down_the_whole_process_tree() {
        let dir = std::env::temp_dir().join(format!("boba-tree-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");
        let marker = dir.join("grandchild-ran");
        let ready = dir.join("grandchild-started");
        let pid_file = dir.join("grandchild-pid");

        let mut proc = SupervisedProcess::spawn(&spec(&[
            ("BOBA_FAKE_MODE", "spawn-child"),
            ("BOBA_FAKE_MARKER", marker.to_str().unwrap()),
            ("BOBA_FAKE_READY", ready.to_str().unwrap()),
            ("BOBA_FAKE_GRANDCHILD_PID", pid_file.to_str().unwrap()),
            // Long enough that only the kill can stop it.
            ("BOBA_FAKE_DELAY_MS", "60000"),
        ]))
        .expect("spawn");

        // Deterministic sync (CH12): wait for the grandchild to say it is up,
        // then kill the group. Nothing here sleeps "long enough and hopes".
        let pid = wait_for_pid(&pid_file);
        proc.kill();

        // The grandchild is gone now — proven by its pid, not by waiting out
        // its sleep — and it never touched the marker.
        assert!(
            until(|| !alive(pid)),
            "grandchild {pid} survived the group kill"
        );
        assert!(!marker.exists(), "grandchild ran after the group kill");
        fs::remove_dir_all(&dir).ok();
    }

    // A grandchild that inherits stderr must not hang the run: the stderr join
    // is bounded, so `wait` returns even while the pipe is still held.
    #[cfg(unix)]
    #[test]
    fn a_grandchild_holding_stderr_does_not_hang_wait() {
        let mut proc = SupervisedProcess::spawn(&spec(&[
            ("BOBA_FAKE_MODE", "fail-auth"),
            // Far longer than any assertion below, so a pass can only mean the
            // join was bounded.
            ("BOBA_FAKE_STDERR_HOLD_MS", "60000"),
        ]))
        .expect("spawn");

        // The CLI itself is done at once; only its stderr is still open.
        while !matches!(proc.recv_line(Duration::from_secs(5)), Stream::Eof) {}

        let started = Instant::now();
        proc.wait();
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "wait blocked on a grandchild holding stderr for {:?}",
            started.elapsed()
        );
    }

    // CH12: the stdout join is bounded too. The CLI exits at once while a
    // grandchild keeps stdout open, so `wait` must not block on it.
    #[cfg(unix)]
    #[test]
    fn a_grandchild_holding_stdout_does_not_hang_wait() {
        let mut proc = SupervisedProcess::spawn(&spec(&[
            ("BOBA_FAKE_MODE", "fail-auth"),
            ("BOBA_FAKE_STDOUT_HOLD_MS", "60000"),
        ]))
        .expect("spawn");

        let started = Instant::now();
        // `wait` reaps the (already exited) CLI, then joins both readers.
        proc.wait();
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "wait blocked on a grandchild holding stdout for {:?}",
            started.elapsed()
        );
    }

    /// Wait (bounded) for the grandchild to write its pid.
    #[cfg(any(unix, windows))]
    fn wait_for_pid(path: &std::path::Path) -> u32 {
        let mut pid = None;
        assert!(
            until(|| {
                pid = fs::read_to_string(path)
                    .ok()
                    .and_then(|text| text.trim().parse().ok());
                pid.is_some()
            }),
            "the grandchild never announced itself: {path:?}"
        );
        pid.expect("checked above")
    }

    /// Whether `pid` still exists (`kill -0`).
    #[cfg(unix)]
    fn alive(pid: u32) -> bool {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    /// Whether `pid` still exists (Windows has no `kill -0`; `tasklist` with
    /// `PID eq` is the liveness probe `boba-browser` already uses). An unknown
    /// answer counts as alive, so a failed probe cannot pass the test.
    #[cfg(windows)]
    fn alive(pid: u32) -> bool {
        std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).contains(&pid.to_string()))
            .unwrap_or(true)
    }

    /// Poll `condition` until it holds, with a deadline. Used for conditions
    /// another process produces (never as a substitute for a signal).
    #[cfg(any(unix, windows))]
    fn until(mut condition: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if condition() {
                return true;
            }
            thread::sleep(Duration::from_millis(5));
        }
        condition()
    }
}
