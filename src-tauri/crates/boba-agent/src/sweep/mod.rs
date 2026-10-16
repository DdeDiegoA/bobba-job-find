//! Startup sweep of orphaned run workspaces and endpoints (ADR-010 §4).
//!
//! At startup nothing is running, so every `<app data>/runs/*` directory and
//! every `<app data>/ipc/*` endpoint is a leftover from a run that died without
//! cleaning up (a panic-abort, a SIGKILL, a power loss) and is deleted. This is
//! disk hygiene, not security: a workspace is empty by construction and the
//! endpoints are per run (S2.3a).
//!
//! Windows names its pipes in a namespace, so there is no endpoint file to
//! sweep there; the endpoint directory simply does not exist.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::mcp::config::SandboxConfig;

/// What one startup sweep removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SweepReport {
    /// Leftover run workspaces (and anything else under `runs/`).
    pub workspaces: usize,
    /// Leftover MCP endpoints under `ipc/`.
    pub endpoints: usize,
}

/// Delete every leftover run workspace and endpoint.
///
/// Best effort per entry: a file another process still holds is left for the
/// next start rather than failing the launch. Traversal errors do propagate, so
/// a misconfigured app-data path is visible instead of silently sweeping nothing.
pub fn sweep_orphans(cfg: &SandboxConfig) -> io::Result<SweepReport> {
    Ok(SweepReport {
        workspaces: clear_dir(&cfg.runs_root)?,
        endpoints: clear_dir(&cfg.app_data.join("ipc"))?,
    })
}

/// Remove every entry of `dir`; a missing directory is nothing to sweep.
fn clear_dir(dir: &Path) -> io::Result<usize> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let mut removed = 0;
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let path: PathBuf = entry.path();
        let deleted = if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        if deleted.is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::config::test_support::TempRoot;
    use std::path::Path;

    /// A sandbox config whose runs root and ipc directory are already populated
    /// with what a crashed run leaves behind.
    fn leftovers() -> TempRoot {
        let root = TempRoot::new();
        for run_id in ["01AAA", "01BBB"] {
            let workspace = root.path().join("runs").join(run_id);
            std::fs::create_dir_all(&workspace).unwrap();
            std::fs::write(workspace.join("mcp.json"), b"{}").unwrap();
        }
        std::fs::write(root.path().join("runs").join("stray-file"), b"x").unwrap();
        if !root.path().join("ipc").exists() {
            std::fs::create_dir_all(root.path().join("ipc")).unwrap();
        }
        for run_id in ["01AAA", "01BBB"] {
            std::fs::write(root.path().join("ipc").join(format!("{run_id}.sock")), b"").unwrap();
        }
        root
    }

    #[test]
    fn every_leftover_workspace_and_endpoint_is_deleted() {
        let root = leftovers();
        let cfg = root.sandbox_config(Path::new("/app/boba-mcp"));

        let report = sweep_orphans(&cfg).expect("sweep");

        assert_eq!(report.workspaces, 3, "two workspaces and a stray file");
        assert_eq!(report.endpoints, 2);
        assert!(
            std::fs::read_dir(root.path().join("runs"))
                .unwrap()
                .next()
                .is_none(),
            "a workspace survived the sweep"
        );
        assert!(
            std::fs::read_dir(root.path().join("ipc"))
                .unwrap()
                .next()
                .is_none(),
            "an endpoint survived the sweep"
        );
    }

    #[test]
    fn a_first_launch_with_nothing_to_sweep_is_not_an_error() {
        let root = TempRoot::new();
        assert!(!root.path().exists(), "nothing has been created yet");

        let report = sweep_orphans(&root.sandbox_config(Path::new("/app/boba-mcp")))
            .expect("a missing app data dir is simply nothing to sweep");

        assert_eq!(report, SweepReport::default());
    }

    #[test]
    fn the_swept_endpoint_directory_is_where_the_wire_puts_endpoints() {
        let root = TempRoot::new();
        let endpoint = boba_mcp_proto::schema::endpoint(root.path(), "01AAA");
        assert_eq!(
            Path::new(&endpoint).parent(),
            Some(root.path().join("ipc").as_path()),
            "the sweep would leave a real endpoint behind"
        );
    }

    #[test]
    fn the_sweep_is_idempotent() {
        let root = leftovers();
        let cfg = root.sandbox_config(Path::new("/app/boba-mcp"));

        sweep_orphans(&cfg).expect("first sweep");
        assert_eq!(
            sweep_orphans(&cfg).expect("second sweep"),
            SweepReport::default()
        );
    }
}
