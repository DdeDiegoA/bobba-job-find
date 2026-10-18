//! CLI discovery and probing (FR-2, AD-4).
//!
//! Scans `PATH` plus the known install locations GUI apps miss (architecture
//! 5.2), then probes each binary: `--version` for version/health and `--help`
//! for capabilities. Every subprocess is bounded by a timeout and the CLIs are
//! probed in parallel, so detection finishes well inside the 10 s budget.

use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::drivers::{self, CliDriver, CliInstall, CliKind, HealthStatus};

/// Per-subprocess timeout. Five CLIs, two probes each, run in parallel.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// A detection result: an installed, probed CLI, or an install prompt (FR-2).
#[derive(Debug, Clone)]
pub enum CliStatus {
    Installed(CliInstall),
    NotInstalled { kind: CliKind },
}

/// Probe every known CLI. Always one entry per `CliKind`, in `CliKind::ALL` order.
pub fn detect_all() -> Vec<CliStatus> {
    let dirs = search_dirs();
    let drivers = drivers::registry();
    thread::scope(|scope| {
        let handles: Vec<_> = drivers
            .iter()
            .map(|driver| {
                let driver = driver.as_ref();
                let dirs = dirs.as_slice();
                scope.spawn(move || detect_one(driver, dirs))
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("detection thread"))
            .collect()
    })
}

/// Probe a user-supplied binary path (FR-2: "the user can also add a CLI by path").
pub fn probe_path(kind: CliKind, path: &Path) -> Option<CliInstall> {
    if !is_executable_file(path) {
        return None;
    }
    let driver = drivers::registry().into_iter().find(|d| d.kind() == kind)?;
    Some(probe(driver.as_ref(), path))
}

/// The directories to scan, `PATH` first (architecture 5.2).
pub fn search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = env::var_os("PATH")
        .map(|path| env::split_paths(&path).collect())
        .unwrap_or_default();
    #[cfg(unix)]
    if let Some(path) = login_shell_path() {
        for entry in env::split_paths(&path) {
            if !dirs.contains(&entry) {
                dirs.push(entry);
            }
        }
    }
    dirs.extend(known_locations());
    dirs
}

/// Find the first executable for `kind` across `dirs`.
pub fn find_binary(kind: CliKind, dirs: &[PathBuf]) -> Option<PathBuf> {
    for dir in dirs {
        for name in candidate_names(kind) {
            let candidate = dir.join(name);
            if is_executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

fn detect_one(driver: &dyn CliDriver, dirs: &[PathBuf]) -> CliStatus {
    let kind = driver.kind();
    match find_binary(kind, dirs) {
        None => CliStatus::NotInstalled { kind },
        Some(path) => CliStatus::Installed(probe(driver, &path)),
    }
}

/// Probe a resolved binary: version/health plus capabilities. Never fails; a
/// binary that cannot run is reported as `Unhealthy`.
fn probe(driver: &dyn CliDriver, path: &Path) -> CliInstall {
    let version_run = run_capture(path, &["--version"], PROBE_TIMEOUT);
    let (version, health) = match &version_run {
        Some(output) if output.ok => (
            first_line(&output.stdout),
            driver.health(true, &output.stdout),
        ),
        Some(output) => (
            String::new(),
            driver.health(false, &format!("{}{}", output.stdout, output.stderr)),
        ),
        None => (
            String::new(),
            HealthStatus::Unhealthy("version probe timed out".to_string()),
        ),
    };
    let help = run_capture(path, &["--help"], PROBE_TIMEOUT)
        .map(|output| output.stdout)
        .unwrap_or_default();
    CliInstall {
        kind: driver.kind(),
        path: path.to_path_buf(),
        version,
        health,
        capabilities: driver.capabilities(&help),
    }
}

struct Captured {
    ok: bool,
    stdout: String,
    stderr: String,
}

/// Run `program args...`, bounded by `timeout`. `None` on spawn error or timeout.
fn run_capture(program: &Path, args: &[&str], timeout: Duration) -> Option<Captured> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    // Drain both pipes on threads so a full pipe can never block the child.
    let stdout_reader = thread::spawn(move || read_to_string(stdout));
    let stderr_reader = thread::spawn(move || read_to_string(stderr));

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    Some(Captured {
        ok: status.success(),
        stdout: stdout_reader.join().unwrap_or_default(),
        stderr: stderr_reader.join().unwrap_or_default(),
    })
}

fn read_to_string(mut reader: impl std::io::Read) -> String {
    let mut text = String::new();
    let _ = reader.read_to_string(&mut text);
    text
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").trim().to_string()
}

fn candidate_names(kind: CliKind) -> Vec<String> {
    let mut names = Vec::new();
    for base in kind.binary_names() {
        names.push((*base).to_string());
        if cfg!(windows) {
            for ext in ["exe", "cmd", "bat"] {
                names.push(format!("{base}.{ext}"));
            }
        }
    }
    names
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = path.metadata() else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn known_locations() -> Vec<PathBuf> {
    let Some(home) = home_dir() else {
        return Vec::new();
    };
    let mut dirs = vec![
        home.join(".local/bin"),
        home.join(".bun/bin"),
        home.join(".npm-global/bin"),
        home.join(".cargo/bin"),
    ];
    // nvm/volta/asdf/mise shims.
    dirs.push(home.join(".volta/bin"));
    dirs.push(home.join(".asdf/shims"));
    dirs.push(home.join(".local/share/mise/shims"));
    if let Ok(entries) = std::fs::read_dir(home.join(".nvm/versions/node")) {
        for entry in entries.flatten() {
            dirs.push(entry.path().join("bin"));
        }
    }
    #[cfg(target_os = "macos")]
    dirs.push(PathBuf::from("/opt/homebrew/bin"));
    #[cfg(windows)]
    {
        if let Some(appdata) = env::var_os("APPDATA") {
            dirs.push(PathBuf::from(appdata).join("npm"));
        }
        if let Some(local) = env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join("Programs"));
        }
    }
    dirs
}

fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        env::var_os("USERPROFILE").map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        env::var_os("HOME").map(PathBuf::from)
    }
}

/// The user's login-shell `PATH`, resolved once (architecture 5.2). GUI apps do
/// not inherit it, so this recovers nvm/volta/asdf/mise shims on macOS/Linux.
#[cfg(unix)]
fn login_shell_path() -> Option<String> {
    let shell = env::var("SHELL").ok()?;
    run_capture(Path::new(&shell), &["-lc", "echo $PATH"], PROBE_TIMEOUT)
        .filter(|output| output.ok)
        .map(|output| output.stdout.trim().to_string())
        .filter(|path| !path.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::fs;

    #[cfg(unix)]
    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("boba-detect-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[cfg(unix)]
    fn write_exe(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let path = dir.join(name);
        fs::write(&path, body).expect("write script");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
        path
    }

    #[cfg(unix)]
    #[test]
    fn find_binary_finds_an_executable_in_the_search_dirs() {
        let dir = tmp_dir("find");
        write_exe(&dir, "claude", "#!/bin/sh\necho ok\n");
        let dirs = vec![dir.clone()];
        let found = find_binary(CliKind::ClaudeCode, &dirs);
        assert_eq!(found, Some(dir.join("claude")));
        assert_eq!(find_binary(CliKind::Codex, &[dir]), None);
    }

    #[cfg(unix)]
    #[test]
    fn probe_reports_version_health_and_capabilities() {
        let dir = tmp_dir("probe");
        let help = include_str!("../drivers/fixtures/claude_code_help.txt");
        let version = include_str!("../drivers/fixtures/claude_code_version.txt");
        let script = format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\nprintf '%s' '{}'\nelse\nprintf '%s' '{}'\nfi\n",
            version.trim(),
            help.replace('\'', "'\\''")
        );
        let path = write_exe(&dir, "claude", &script);
        let install = probe_path(CliKind::ClaudeCode, &path).expect("install");
        assert_eq!(install.version, "2.1.301 (Claude Code)");
        assert_eq!(install.health, HealthStatus::Healthy);
        assert!(install.capabilities.stream_json);
        assert!(install.capabilities.mcp);
        assert!(install.capabilities.tool_allowlist);
        fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn probe_path_rejects_a_missing_binary() {
        let dir = tmp_dir("missing");
        assert!(probe_path(CliKind::Aider, &dir.join("nope")).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn run_capture_times_out_on_a_hanging_process() {
        let dir = tmp_dir("hang");
        let path = write_exe(&dir, "hang", "#!/bin/sh\nsleep 30\n");
        let started = Instant::now();
        let out = run_capture(&path, &[], Duration::from_millis(100));
        assert!(out.is_none());
        assert!(started.elapsed() < Duration::from_secs(5));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn search_dirs_includes_path_and_known_locations() {
        let dirs = search_dirs();
        let home = home_dir().expect("home");
        assert!(dirs.contains(&home.join(".local/bin")));
        assert!(dirs.contains(&home.join(".cargo/bin")));
    }
}
