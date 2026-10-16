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
use process_wrap::std::{ChildWrapper, CommandWrap, ProcessGroup};
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

/// How long `reap` waits for the stderr reader once the child is gone. Long
/// enough to drain a closed pipe, short enough that a grandchild still holding
/// stderr open cannot stall the run.
const STDERR_JOIN_GRACE: Duration = Duration::from_millis(500);

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
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        // Join the stderr reader too: `stderr_so_far()` must never race it, or
        // an auth failure can be classified from an empty string. The join is
        // bounded: a grandchild that inherits stderr can hold the pipe open for
        // good, and the run must not hang on it. Past the grace the reader is
        // detached and stderr is whatever it had collected.
        if let Some(reader) = self.stderr_reader.take() {
            let deadline = Instant::now() + STDERR_JOIN_GRACE;
            while !reader.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(1));
            }
            if reader.is_finished() {
                let _ = reader.join();
            }
        }
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
    use std::fs;
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

    #[cfg(unix)]
    #[test]
    fn kill_tears_down_the_whole_process_tree() {
        let dir = std::env::temp_dir().join(format!("boba-tree-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");
        let marker = dir.join("grandchild-ran");

        let mut proc = SupervisedProcess::spawn(&spec(&[
            ("BOBA_FAKE_MODE", "spawn-child"),
            ("BOBA_FAKE_MARKER", marker.to_str().unwrap()),
            ("BOBA_FAKE_DELAY_MS", "400"),
        ]))
        .expect("spawn");

        // Let the child start, then kill the group while the grandchild sleeps.
        std::thread::sleep(Duration::from_millis(100));
        proc.kill();

        // The grandchild would touch the marker at 400ms; it must never run.
        std::thread::sleep(Duration::from_millis(600));
        assert!(!marker.exists(), "grandchild survived the group kill");
        fs::remove_dir_all(&dir).ok();
    }

    // A grandchild that inherits stderr must not hang the run: the stderr join
    // is bounded, so `wait` returns even while the pipe is still held.
    #[cfg(unix)]
    #[test]
    fn a_grandchild_holding_stderr_does_not_hang_wait() {
        let mut proc = SupervisedProcess::spawn(&spec(&[
            ("BOBA_FAKE_MODE", "fail-auth"),
            ("BOBA_FAKE_STDERR_HOLD_MS", "5000"),
        ]))
        .expect("spawn");

        // The CLI itself is done at once; only its stderr is still open.
        while !matches!(proc.recv_line(Duration::from_secs(5)), Stream::Eof) {}

        let started = Instant::now();
        proc.wait();
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "wait blocked on a grandchild holding stderr for {:?}",
            started.elapsed()
        );
    }
}
