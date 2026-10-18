//! CLI discovery and probing (FR-2, AD-4).
//!
//! Scans `PATH` plus the known install locations GUI apps miss (architecture
//! 5.2), then probes each binary: `--version` for version/health and `--help`
//! for capabilities. Every subprocess is bounded by a timeout and the CLIs are
//! probed in parallel, so detection finishes well inside the 10 s budget.

use std::env;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::drivers::{self, CliDriver, CliInstall, CliKind, HealthStatus};

/// Per-subprocess timeout. Five CLIs, two probes each, run in parallel.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// The login shell is asked once per process, and a cold rc (nvm, pyenv,
/// direnv) can take a few seconds: give it room beyond a single CLI probe.
const LOGIN_SHELL_TIMEOUT: Duration = Duration::from_secs(5);

/// Markers around the printed `PATH`, so a shell that prints a banner on
/// startup cannot be mistaken for the path itself.
const PATH_START: &str = "__BOBA_PATH__";
const PATH_END: &str = "__BOBA_END__";

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
        for entry in env::split_paths(path) {
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

/// The program the runtime spawns for `kind`: its absolute path when detection
/// found it, else the bare name (and the spawn reports its own error).
pub fn resolve_program(kind: CliKind) -> PathBuf {
    resolve_program_in(kind, &search_dirs())
}

fn resolve_program_in(kind: CliKind, dirs: &[PathBuf]) -> PathBuf {
    find_binary(kind, dirs).unwrap_or_else(|| PathBuf::from(kind.binary_names()[0]))
}

/// The `PATH` a CLI child gets: what this process inherited, plus everything
/// detection resolved (FIX1). A GUI launch inherits `/usr/bin:/bin:…` only, so a
/// CLI resolved outside it — and the `node` runtime its shim needs — is
/// reachable only through this merged path.
pub fn child_path() -> OsString {
    static CACHED: std::sync::OnceLock<OsString> = std::sync::OnceLock::new();
    CACHED
        .get_or_init(|| joined_path(&search_dirs(), env::var_os("PATH").as_deref()))
        .clone()
}

/// `inherited` first, then `dirs` that are not already on it.
fn joined_path(dirs: &[PathBuf], inherited: Option<&OsStr>) -> OsString {
    let mut entries: Vec<PathBuf> = inherited
        .map(env::split_paths)
        .into_iter()
        .flatten()
        .collect();
    for dir in dirs {
        if !entries.contains(dir) {
            entries.push(dir.clone());
        }
    }
    env::join_paths(&entries).unwrap_or_default()
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
///
/// The child gets [`child_path`]: a CLI resolved outside the GUI PATH — and the
/// `node` runtime its shim looks up by name — is reachable only through it.
fn run_capture(program: &Path, args: &[&str], timeout: Duration) -> Option<Captured> {
    run_capture_with(program, args, timeout, Some(child_path()))
}

/// [`run_capture`] on the inherited environment. The login shell probe uses
/// this: [`child_path`] itself comes from that shell, so asking for it there
/// would recurse.
fn run_capture_inherited(program: &Path, args: &[&str], timeout: Duration) -> Option<Captured> {
    run_capture_with(program, args, timeout, None)
}

fn run_capture_with(
    program: &Path,
    args: &[&str],
    timeout: Duration,
    path: Option<OsString>,
) -> Option<Captured> {
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null());
    if let Some(path) = path {
        command.env("PATH", path);
    }
    let mut child = command
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
        home.join(".opencode/bin"),
        home.join(".bun/bin"),
        home.join(".npm-global/bin"),
        home.join(".cargo/bin"),
    ];
    // nvm/volta/asdf/mise shims.
    dirs.push(home.join(".volta/bin"));
    dirs.push(home.join(".asdf/shims"));
    dirs.push(home.join(".local/share/mise/shims"));
    dirs.push(home.join(".nvm/current/bin"));
    if let Ok(entries) = std::fs::read_dir(home.join(".nvm/versions/node")) {
        for entry in entries.flatten() {
            dirs.push(entry.path().join("bin"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
        // pnpm's install location on macOS.
        dirs.push(home.join("Library/pnpm"));
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        dirs.push(PathBuf::from("/usr/local/bin"));
        dirs.push(PathBuf::from("/snap/bin"));
    }
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

/// The user's login-shell `PATH`, resolved once per process (architecture 5.2).
/// GUI apps do not inherit it, so this recovers nvm/volta/asdf/mise shims on
/// macOS/Linux.
#[cfg(unix)]
fn login_shell_path() -> Option<&'static str> {
    /// macOS users run zsh even when `SHELL` is missing from a GUI launch.
    const DEFAULT_SHELL: &str = if cfg!(target_os = "macos") {
        "/bin/zsh"
    } else {
        "/bin/bash"
    };
    static CACHED: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    CACHED
        .get_or_init(|| {
            let shell = env::var("SHELL")
                .ok()
                .filter(|shell| !shell.is_empty())
                .unwrap_or_else(|| DEFAULT_SHELL.to_string());
            login_shell_path_from(Path::new(&shell))
        })
        .as_deref()
}

#[cfg(unix)]
fn login_shell_path_from(shell: &Path) -> Option<String> {
    // Double-quoted so the shell expands `$PATH`; the markers keep a banner on
    // stdout from being read as the path.
    let command = format!("printf %s \"{PATH_START}${{PATH}}{PATH_END}\"");
    // `-i` matters: PATH additions usually live in `.zshrc`/`.bashrc`, which
    // only an interactive shell reads. `opencode` is exported there on macOS,
    // while `claude` sits in `~/.local/bin` — which is why a GUI launch found
    // one CLI and not the other (FIX1). A shell that refuses to run
    // interactively (no tty) still answers as a login shell.
    for flag in ["-ilc", "-lc"] {
        let Some(output) = run_capture_inherited(shell, &[flag, &command], LOGIN_SHELL_TIMEOUT)
        else {
            continue;
        };
        if !output.ok {
            continue;
        }
        if let Some(path) = between_markers(&output.stdout).filter(|path| !path.is_empty()) {
            return Some(path);
        }
    }
    None
}

/// The text between the markers, or the last non-empty line when a shell
/// stripped them (some print the path on a line of its own).
fn between_markers(text: &str) -> Option<String> {
    if let Some(start) = text.find(PATH_START) {
        let rest = &text[start + PATH_START.len()..];
        let path = rest.split(PATH_END).next().unwrap_or(rest).trim();
        return Some(path.to_string());
    }
    text.lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToString::to_string)
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

    /// FIX1, end to end: an OpenCode installed in `~/.opencode/bin` — where the
    /// official installer puts it — is found by the same scan the app runs.
    #[cfg(unix)]
    #[test]
    fn an_opencode_install_is_detected_where_it_lives() {
        let Some(home) = home_dir() else { return };
        let opencode = home.join(".opencode/bin/opencode");
        if !opencode.is_file() {
            return;
        }
        assert_eq!(
            find_binary(CliKind::OpenCode, &search_dirs()),
            Some(opencode)
        );
    }

    #[test]
    fn known_locations_cover_the_installs_a_gui_launch_misses() {
        let home = home_dir().expect("home");
        let dirs = known_locations();
        // FIX1: both are common npm/installer targets that are absent from the
        // minimal PATH a Finder launch hands the app.
        assert!(dirs.contains(&home.join(".opencode/bin")));
        assert!(dirs.contains(&home.join("Library/pnpm")));
        #[cfg(target_os = "macos")]
        assert!(dirs.contains(&PathBuf::from("/usr/local/bin")));
    }

    /// The real-world case: `opencode` is exported from `.zshrc`, which only an
    /// **interactive** login shell reads. `-lc` misses it, which is why a GUI
    /// launch detected Claude (in `~/.local/bin`) but not OpenCode.
    #[cfg(unix)]
    #[test]
    fn login_shell_path_reads_an_interactive_login_shell() {
        let dir = tmp_dir("loginshell");
        let shell = write_exe(
            &dir,
            "fakeshell",
            "#!/bin/sh\n\
             echo 'motd: startup noise'\n\
             if [ \"$1\" = \"-ilc\" ]; then\n\
             printf '%s' '__BOBA_PATH__/opt/from-zshrc:/usr/bin__BOBA_END__'\n\
             else\n\
             printf '%s' '__BOBA_PATH__/usr/bin:/bin__BOBA_END__'\n\
             fi\n",
        );
        assert_eq!(
            login_shell_path_from(&shell).as_deref(),
            Some("/opt/from-zshrc:/usr/bin")
        );
        fs::remove_dir_all(&dir).ok();
    }

    /// A shell that cannot run interactively still yields its login PATH.
    #[cfg(unix)]
    #[test]
    fn login_shell_path_falls_back_to_a_login_shell() {
        let dir = tmp_dir("loginshell-fallback");
        let shell = write_exe(
            &dir,
            "fakeshell",
            "#!/bin/sh\n\
             if [ \"$1\" = \"-ilc\" ]; then\n\
             echo 'zsh: no controlling terminal' >&2\n\
             exit 1\n\
             fi\n\
             printf '%s' '__BOBA_PATH__/usr/bin:/bin__BOBA_END__'\n",
        );
        assert_eq!(
            login_shell_path_from(&shell).as_deref(),
            Some("/usr/bin:/bin")
        );
        fs::remove_dir_all(&dir).ok();
    }

    /// The child PATH keeps what the process inherited and adds what detection
    /// resolved, without duplicating entries.
    #[cfg(unix)]
    #[test]
    fn joined_path_keeps_the_inherited_entries_once() {
        let merged = joined_path(
            &[PathBuf::from("/opt/from-zshrc"), PathBuf::from("/usr/bin")],
            Some(OsStr::new("/usr/bin:/bin")),
        );
        let entries: Vec<PathBuf> = env::split_paths(&merged).collect();
        assert_eq!(
            entries,
            vec![
                PathBuf::from("/usr/bin"),
                PathBuf::from("/bin"),
                PathBuf::from("/opt/from-zshrc"),
            ]
        );
    }

    /// A CLI resolves to its absolute path, so the runtime can spawn it even
    /// when the GUI PATH would not find it.
    #[cfg(unix)]
    #[test]
    fn resolve_program_returns_the_absolute_binary() {
        let dir = tmp_dir("resolve");
        write_exe(&dir, "opencode", "#!/bin/sh\necho ok\n");
        let found = resolve_program_in(CliKind::OpenCode, std::slice::from_ref(&dir));
        assert_eq!(found, dir.join("opencode"));
        // Nothing found: the bare name stays, and spawn reports its own error.
        assert_eq!(
            resolve_program_in(CliKind::Codex, &[dir]),
            PathBuf::from("codex")
        );
    }

    /// Probe children inherit the resolved PATH, so a `#!/usr/bin/env node` shim
    /// can find its runtime.
    #[cfg(unix)]
    #[test]
    fn run_capture_gives_the_child_the_resolved_path() {
        let dir = tmp_dir("childpath");
        let path = write_exe(&dir, "printpath", "#!/bin/sh\nprintf '%s' \"$PATH\"\n");
        let out = run_capture(&path, &[], Duration::from_secs(5)).expect("output");
        // Not merely "contains ~/.local/bin" (this machine may already have it
        // on the inherited path): the child gets exactly the merged path.
        assert_eq!(out.stdout, child_path().to_string_lossy());
        fs::remove_dir_all(&dir).ok();
    }
}
