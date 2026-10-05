# Review — CH1 chore: trim dev-profile debuginfo

> **Reviewer:** reviewer (qa) · **Date:** 2026-10-05 · **Branch:** `hermad/CH1` · **Commit:** `b7f2c6a` · **Worktree:** `.hermad/worktrees/CH1`
> **Verdict:** **APPROVED** — no blocker. Not a story slice; a build-hygiene chore (disk was the incident).

## Diff

Root `Cargo.toml` only, +13 lines:
```toml
[profile.dev]
debug = "line-tables-only"

[profile.dev.package."*"]
debug = false
```
No `[profile.release]` section — release defaults untouched (confirmed: `grep '\[profile'` → only the two dev entries). Verified `split-debuginfo` is not set by this slice.

## Reproduced checks (reviewer, in the worktree)

| Command | Result |
|---|---|
| `cargo clean` then `cargo test --workspace --locked` | ✅ exit 0 (25 suites) |
| `cargo fmt --all --check` / `cargo clippy --workspace --all-targets --locked -- -D warnings` | ✅ exit 0 |

Note: the first `cargo test` run hit a transient target-dir fingerprint error (partial `target/`, not disk — 8 GB free). A `cargo clean` + rebuild passed cleanly; not attributable to the change.

## Assessment

- `debug = "line-tables-only"` keeps file:line in backtraces for our crates while dropping variable/type debuginfo; `[profile.dev.package."*"] debug = false` drops dependency debuginfo entirely. This is the standard, minimal way to shrink `target/` (dev reports 3.6 GB → 2.1 GB clean build).
- Release and CI are unaffected (release profile untouched; CI uses its own profile).
- Scope is exactly one file; no ownership conflict.

## Not covered

- Absolute `target/` size on other platforms, and a live `RUST_BACKTRACE=1` panic trace (dev showed file:line; the config guarantees line tables).
