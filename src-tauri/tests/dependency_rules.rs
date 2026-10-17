//! Dependency-rule enforcement (AC2, AD-16).
//!
//! `cargo deny` runs in CI (S1.8), but this test enforces the *architecture*
//! rules on every `cargo test` without extra tooling:
//!   * `boba-domain` depends only on serde, thiserror, time and ulid.
//!   * adapters never depend on each other.
//!   * `boba-app` never depends on an adapter.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const DOMAIN_ALLOWED: &[&str] = &["serde", "thiserror", "time", "ulid"];
const ADAPTERS: &[&str] = &[
    "boba-store",
    "boba-sources",
    "boba-agent",
    "boba-docs",
    "boba-mail",
    "boba-browser",
    "boba-embed",
];

/// Extract dependency keys from the `[dependencies]`-family tables of a manifest.
fn dep_keys(manifest: &str) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    let mut in_deps = false;
    for raw in manifest.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            let name = line.trim_start_matches('[').trim_end_matches(']').trim();
            in_deps = name == "dependencies"
                || name == "dev-dependencies"
                || name == "build-dependencies"
                || name.ends_with(".dependencies")
                || name.ends_with(".dev-dependencies")
                || name.ends_with(".build-dependencies");
            continue;
        }
        if !in_deps {
            continue;
        }
        if let Some((key, _)) = line.split_once('=') {
            keys.insert(key.trim().trim_matches('"').to_string());
        }
    }
    keys
}

fn load(crate_name: &str) -> BTreeSet<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("crates")
        .join(crate_name)
        .join("Cargo.toml");
    let manifest =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    dep_keys(&manifest)
}

#[test]
fn domain_depends_only_on_pure_libs() {
    let deps = load("boba-domain");
    for dep in &deps {
        assert!(
            DOMAIN_ALLOWED.contains(&dep.as_str()),
            "boba-domain must depend only on {DOMAIN_ALLOWED:?}, but depends on `{dep}`"
        );
    }
}

#[test]
fn adapters_never_depend_on_each_other() {
    for adapter in ADAPTERS {
        let deps = load(adapter);
        for other in ADAPTERS {
            if other == adapter {
                continue;
            }
            assert!(
                !deps.contains(*other),
                "adapter `{adapter}` must not depend on adapter `{other}` (AD-1)"
            );
        }
    }
}

#[test]
fn app_does_not_depend_on_adapters() {
    let deps = load("boba-app");
    for adapter in ADAPTERS {
        assert!(
            !deps.contains(*adapter),
            "boba-app must not depend on adapter `{adapter}`"
        );
    }
}
