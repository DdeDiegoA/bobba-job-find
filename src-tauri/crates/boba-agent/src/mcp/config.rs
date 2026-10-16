//! Per-run sandbox setup (AD-6, architecture §5.4): a fresh workspace under app
//! data, a per-run token, and an MCP config that points the CLI at `boba-mcp`
//! and at nothing else.
//!
//! Nothing here reads the DB or the keychain (AD-2): the core passes the
//! endpoint and the token, the run only sees its own workspace.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use boba_mcp_proto::schema as wire;
use boba_mcp_proto::tool;
use serde_json::{Value, json};

/// The MCP config file inside a run's workspace.
pub const MCP_CONFIG_FILE: &str = "mcp.json";
/// Token strength: 32 bytes from the OS CSPRNG.
const TOKEN_BYTES: usize = 32;

/// A run's fresh workspace: owner-only, empty, and deleted when the run ends.
#[derive(Debug)]
pub struct Workspace {
    dir: PathBuf,
}

impl Workspace {
    /// Create `runs_root/<run_id>`, empty and owner-only. `run_id` is the run's
    /// ULID; a directory already there is a leftover from a crashed run and is
    /// discarded, never reused (AD-6).
    pub fn create(runs_root: &Path, run_id: &str) -> io::Result<Self> {
        let dir = runs_root.join(run_id);
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
        }
        fs::create_dir_all(&dir)?;
        owner_only(&dir)?;
        Ok(Self { dir })
    }

    pub fn path(&self) -> &Path {
        &self.dir
    }

    /// Write the run's MCP config and return its path.
    pub fn write_mcp_config(
        &self,
        shim: &Path,
        endpoint: &str,
        token: &str,
    ) -> io::Result<PathBuf> {
        let file = self.dir.join(MCP_CONFIG_FILE);
        fs::write(&file, mcp_config_json(shim, endpoint, token).to_string())?;
        Ok(file)
    }

    /// Delete the workspace at the end of the run.
    pub fn delete(self) {
        fs::remove_dir_all(&self.dir).ok();
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        // The run is over either way: a leftover workspace must not survive it.
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// A per-run token: 32 CSPRNG bytes, hex-encoded. It travels in an environment
/// variable, never on a command line.
pub fn new_token() -> io::Result<String> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(io::Error::other)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// The MCP config the CLI is started with: exactly one server, `boba-mcp`.
pub fn mcp_config_json(shim: &Path, endpoint: &str, token: &str) -> Value {
    json!({
        "mcpServers": {
            tool::SERVER_NAME: {
                "command": shim.display().to_string(),
                "args": [],
                "env": {
                    wire::ENV_ENDPOINT: endpoint,
                    wire::ENV_TOKEN: token,
                },
            }
        }
    })
}

/// The environment the CLI needs, so the `boba-mcp` it spawns can reach this
/// run's endpoint and prove it belongs to this run.
pub fn run_env(endpoint: &str, token: &str) -> Vec<(String, String)> {
    vec![
        (wire::ENV_ENDPOINT.to_string(), endpoint.to_string()),
        (wire::ENV_TOKEN.to_string(), token.to_string()),
    ]
}

/// A prepared run sandbox: the workspace, the endpoint and the token the run is
/// allowed to use.
#[derive(Debug)]
pub struct RunSandbox {
    pub workspace: Workspace,
    pub endpoint: String,
    pub token: String,
    pub config: PathBuf,
}

/// Prepare one run: create the workspace, mint the token and write the config.
/// `runs_root` is the app data `runs` directory; `shim` is the packaged
/// `boba-mcp` binary.
pub fn prepare(
    runs_root: &Path,
    run_id: &str,
    shim: &Path,
    endpoint: &str,
) -> io::Result<RunSandbox> {
    let workspace = Workspace::create(runs_root, run_id)?;
    let token = new_token()?;
    let config = workspace.write_mcp_config(shim, endpoint, &token)?;
    Ok(RunSandbox {
        workspace,
        endpoint: endpoint.to_string(),
        token,
        config,
    })
}

#[cfg(unix)]
fn owner_only(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn owner_only(_dir: &Path) -> io::Result<()> {
    Ok(())
}

/// Scratch directories for the MCP tests, removed on drop (S2.2 did the same
/// in `runtime`; the suite runs tests in parallel, so names are per-test).
#[cfg(test)]
pub(crate) mod test_support {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    pub(crate) struct TempRoot(PathBuf);

    impl TempRoot {
        pub(crate) fn new() -> Self {
            static SEQ: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "boba-mcp-{}-{}",
                std::process::id(),
                SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            Self(dir)
        }

        pub(crate) fn path(&self) -> &Path {
            &self.0
        }

        /// This root's MCP endpoint (a socket path under `ipc/`).
        pub(crate) fn endpoint(&self) -> String {
            super::wire::endpoint(&self.0)
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::TempRoot;
    use super::*;

    #[test]
    fn workspace_is_fresh_and_deleted_when_the_run_ends() {
        let root = TempRoot::new();
        // A leftover workspace from a crashed run must not be reused.
        fs::create_dir_all(root.path().join("01RUN")).unwrap();
        fs::write(root.path().join("01RUN").join("stale.txt"), "old").unwrap();

        let workspace = Workspace::create(root.path(), "01RUN").expect("create");
        assert!(workspace.path().is_dir());
        assert!(
            !workspace.path().join("stale.txt").exists(),
            "the workspace was not fresh"
        );
        assert!(fs::read_dir(workspace.path()).unwrap().next().is_none());

        let path = workspace.path().to_path_buf();
        workspace.delete();
        assert!(!path.exists(), "the workspace survived the run");
    }

    #[test]
    fn a_dropped_workspace_is_deleted() {
        let root = TempRoot::new();
        {
            let _workspace = Workspace::create(root.path(), "01RUN").expect("create");
        }
        assert!(!root.path().join("01RUN").exists());
    }

    #[cfg(unix)]
    #[test]
    fn workspace_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let root = TempRoot::new();
        let workspace = Workspace::create(root.path(), "01RUN").expect("create");
        let mode = fs::metadata(workspace.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "workspace mode {mode:o}");
    }

    #[test]
    fn the_mcp_config_lists_only_the_shim() {
        let shim = Path::new("/app/binaries/boba-mcp");
        let config = mcp_config_json(shim, "/app/ipc/boba-mcp.sock", "abc123");

        let servers = config["mcpServers"].as_object().expect("mcpServers");
        assert_eq!(servers.len(), 1, "a run sees exactly one MCP server");
        let server = &servers[tool::SERVER_NAME];
        assert_eq!(server["command"], shim.display().to_string());
        assert_eq!(server["args"], json!([]));
        assert_eq!(server["env"][wire::ENV_TOKEN], "abc123");
        assert_eq!(server["env"][wire::ENV_ENDPOINT], "/app/ipc/boba-mcp.sock");
        // The token is never an argument: process listings are world-readable.
        assert!(!config.to_string().contains("/app/binaries/boba-mcp abc123"));
    }

    #[test]
    fn tokens_are_long_hex_and_unique() {
        let first = new_token().expect("token");
        let second = new_token().expect("token");
        assert_eq!(first.len(), TOKEN_BYTES * 2);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }

    #[test]
    fn prepare_creates_the_workspace_config_and_env() {
        let root = TempRoot::new();
        let shim = Path::new("/app/binaries/boba-mcp");
        let sandbox =
            prepare(root.path(), "01RUN", shim, "/app/ipc/boba-mcp.sock").expect("prepare");

        assert!(sandbox.config.is_file());
        assert_eq!(
            sandbox.config,
            sandbox.workspace.path().join(MCP_CONFIG_FILE)
        );
        let env = run_env(&sandbox.endpoint, &sandbox.token);
        assert!(env.contains(&(wire::ENV_TOKEN.to_string(), sandbox.token.clone())));
        assert!(env.contains(&(wire::ENV_ENDPOINT.to_string(), sandbox.endpoint.clone())));

        let written: Value =
            serde_json::from_str(&fs::read_to_string(&sandbox.config).unwrap()).unwrap();
        assert_eq!(
            written["mcpServers"][tool::SERVER_NAME]["env"][wire::ENV_TOKEN],
            sandbox.token
        );
    }
}
