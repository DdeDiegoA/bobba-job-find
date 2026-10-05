# Review — CH2 chore: make `cargo deny` green (typst transitive advisories)

> **Reviewer:** reviewer (qa) · **Date:** 2026-10-05 · **Branch:** `hermad/CH2` · **Commit:** `333bfe9` (base `main` @ `4abfd61`) · **Worktree:** `.hermad/worktrees/CH2`
> **Gate:** **SECURITY** — 6 new `[advisories].ignore` entries (Diego must sign off before merge).
> **Verdict:** **APPROVED** (review) — full gate green; the ignore justifications check out; Diego's gate required.

## Diff

`deny.toml` only, +25 lines. **No `Cargo.lock` / `Cargo.toml` change** (verified `git diff --name-only` → `deny.toml`).

## Full CI gate (reviewer, in the worktree) — all exit 0

| # | Command | Result |
|---|---|---|
| 1 | `cargo metadata --locked` | ✅ 0 |
| 2 | `cargo fmt --all --check` | ✅ 0 |
| 3 | `cargo clippy --workspace --all-targets --locked -- -D warnings` | ✅ 0 |
| 4 | `cargo test --workspace --locked` | ✅ 0 — **135 passed** |
| 5 | **`cargo deny check`** | ✅ 0 — "advisories ok, bans ok, licenses ok, sources ok" |
| 6 | `pnpm install --frozen-lockfile --ignore-scripts` | ✅ 0 |
| 7 | `pnpm lint` | ✅ 0 |
| 8 | `pnpm typecheck` | ✅ 0 |
| 9 | `pnpm test` | ✅ 0 — 30 passed |
| 10 | `pnpm build` | ✅ 0 |
| 11 | `node scripts/check-i18n.mjs` | ✅ 0 |
| 12 | `node scripts/ci/check-workflow-policy.mjs` | ✅ 0 |
| 13 | `node scripts/gen-bindings.mjs --check` | ✅ 0 — bindings up to date |

## Advisory review (security gate)

I verified each ignored version against the tree (`cargo tree -i`) and each advisory's **classification from the local RUSTSEC DB** (`cargo deny --log-level info check advisories`):

| Advisory | Crate (version) | Class | Path (verified) |
|---|---|---|---|
| RUSTSEC-2025-0141 | bincode 1.3.3 | **unmaintained** | syntect → two-face → typst-library |
| RUSTSEC-2026-0206 | rustybuzz 0.20.1 | **unmaintained** | krilla / typst-layout → typst-pdf |
| RUSTSEC-2026-0192 | ttf-parser 0.25.1 | **unmaintained** | fontdb → krilla-svg / typst-library |
| RUSTSEC-2024-0320 | yaml-rust 0.4.5 | **unmaintained** | syntect → two-face → typst-library |
| RUSTSEC-2026-0194 | quick-xml 0.38.4 | **vulnerability** (quadratic duplicate-attribute check) | citationberg → hayagriva → typst-library |
| RUSTSEC-2026-0195 | quick-xml 0.38.4 | **vulnerability** (unbounded `NsReader` allocation → memory DoS) | citationberg → hayagriva → typst-library |

**Upgrade genuinely blocked:** `quick-xml@0.38.4` is pulled **only** by `citationberg 0.7.0` (via `hayagriva`); `cargo update -p citationberg` locks 0 packages (already latest) and it pins `quick-xml ^0.38.1`, so `>= 0.41` cannot be selected. The typst stack is at max stable. (A separate `quick-xml 0.42.0` exists via `plist`/`syntect` and is not affected.)

**Reachability:** the two real vulnerabilities live entirely in the citation/CSL XML path. Our templates (`render_cv`/`render_cover`) contain **no `#bibliography`/`#cite`** (grep of `templates/` is empty) and only parse our own templates, never untrusted XML — so the vulnerable parser is not exercised at MVP. The four unmaintained advisories are informational (no known vulnerability); the affected font/highlight crates process our **bundled** Lato fonts and our own templates, not untrusted input.

**Residual risk / revisit triggers** (documented in-file): citationberg allows `quick-xml >= 0.41`; typst migrates off syntect/two-face (bincode, yaml-rust), to harfrust (rustybuzz) and skrifa (ttf-parser). Re-check when the typst stack is bumped. If a later slice ever adds bibliography/citation support or parses untrusted CSL/fonts, these ignores must be re-evaluated — flag it in that slice.

## Not covered

- The advisories are dated 2026; I read their classifications from the local RUSTSEC DB rather than re-fetching rustsec.org.
- No dependency upgrade was attempted beyond the `citationberg` dry-run (the task's stated constraint).
