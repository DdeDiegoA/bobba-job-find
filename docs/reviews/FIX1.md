# FIX1 review (reviewer-2)

Commit reviewed: `31d04c1` (branch `hermad/FIX1`), on top of `db1f22f`.

**Verdict: CHANGES REQUESTED** (one blocking bug, see B1).

## Gate (run in the worktree, `CARGO_INCREMENTAL=0`)

| Check | Result |
|---|---|
| `pnpm lint && pnpm typecheck && pnpm test` | pass; 79 files, 602 tests; lint reports 3 warnings, no errors |
| `node scripts/check-i18n.mjs` | pass |
| `node scripts/ci/check-conventions.mjs` | pass |
| `node scripts/ci/check-workflow-policy.mjs` | pass |
| `node scripts/gen-bindings.mjs --check` | pass |
| `cargo fmt --all --check` | pass |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | pass |
| `cargo test --workspace --locked` | pass; all suites green, 0 failed |

## Acceptance points checked

- **Login-shell PATH, used for detection and spawn.** Detection (`search_dirs`) and
  spawn (`child_path`, `sandboxed`, `resolve_program`, `commands/agent.rs`
  listing) all use the resolved PATH. The `-ilc` then `-lc` fallback and the
  markers are covered by fake-shell tests. Partly failing: the timeout does not
  cover the reader join (B1).
- **Windows `.cmd` shims.** `candidate_names` adds `.cmd`/`.bat`/`.exe` on
  Windows and `resolve_program` returns the absolute path. Not tested on Windows
  here (unverified). See R1.
- **Test connection, en/es, never dead-ends.** `test.failureOn` names the CLI in
  en and es. Limited mode (`limited.*`) lets Next proceed with or without a
  passing test, and `testedOk` stays false, so no fake pass is recorded.
- **Lockdown gate not weakened.** Pinned flags are the same values in the same
  set; only their position moved, and only for Claude (`pinned_args` returns
  `None` for every other CLI). The init-event check is unchanged. The argv test
  asserts the prompt precedes `--tools` and `--allowedTools`, and still asserts
  `--strict-mcp-config` and the `--tools ""` removal.
- **Tests per case.** Covered: `-ilc` vs `-lc`, banner noise, no-tty fallback,
  merged PATH without duplicates, absolute resolution, child PATH, OpenCode
  auth-on-stdout, known locations, prompt-before-pin ordering, CLI named in the
  failure, Limited mode from the step and with no CLI installed.

## Findings

### B1 — blocking: login-shell probe can hang past its timeout (correctness, medium-high)

- **Where:** `src-tauri/crates/boba-agent/src/detect/mod.rs:233`
  (`stdout: stdout_reader.join()`) in `run_capture_with`, reached from
  `login_shell_path_from` (`:359`–`:375`) with `LOGIN_SHELL_TIMEOUT` (5 s).
- **Scenario:** the login shell exits 0 but a background child it started
  (a daemon from `.zshrc`, `sleep 20 &`) still holds the stdout pipe. `try_wait`
  returns success, so the timeout never fires. The code then joins the reader,
  which blocks until that child exits.
- **Repro:** a scratch program (outside the repo) with `/bin/sh -c 'sleep 20 & printf ok'`
  took 20.05 s to return from the join. Same shape as the shell probe.
- **Impact:** `detect_all` and `search_dirs` (and the wizard's engine snapshot)
  stall for as long as the background process runs. The orquestador asked for
  "login-shell PATH resolution with a timeout", and this path has none.
  `-ilc` reads more rc files than `-lc`, so the exposure grows with this fix.
- **Expected:** a bounded read on both the success and timeout paths. For
  example, read with a deadline and drop the reader thread's result if it has
  not finished, or reuse the existing timeout for the join.
- **Test to add:** a fake shell that backgrounds `sleep` holding stdout and
  exits 0; assert `login_shell_path_from` returns within about 2 s (not 20 s)
  and still returns the path if it printed one.

## Residual risks (not blocking)

- **R1 — Windows `.cmd` spawn.** Rust runs `.cmd` shims through `cmd.exe`, which
  rejects some argument characters (e.g. `"`, `%`, newlines) with an error. The
  prompt travels in argv, so a profile prompt with those characters may fail to
  spawn on Windows. Not verified here; needs a Windows run before release.
- **R2 — weak test.** `an_opencode_install_is_detected_where_it_lives` returns
  early when `~/.opencode/bin/opencode` is absent, so it passes vacuously on CI.
  The fake-shell and `resolve_program_in` tests carry the coverage.
- **R3 — stdout classification.** `classify_exit` scans the CLI's `Text` events,
  which are agent prose, not only errors. It runs only on nonzero exit with a
  Crashed stderr, and it matches strong phrases only. Low risk; keep the hint
  list tight.
- **R4 — no retry on a failed login shell.** `login_shell_path` is cached per
  process, including `None`. A transient failure on first call stays until
  restart.

## Requested before approval

1. Fix B1 and add the bounded-join test.
2. Re-run the full CI gate in the worktree and re-emit `HERMAD:DONE`/review with a
   new commit SHA.
