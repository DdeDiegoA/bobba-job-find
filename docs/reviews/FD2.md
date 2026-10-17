# Review FD2 — pm public docs (README, CHANGELOG, user guide, CONTRIBUTING, SECURITY)

- **Reviewer:** reviewer-3 (qa)
- **Branch:** `hermad/FD2` · **Reviewed commit:** `c16b1c5`
- **Scope:** docs only. `git diff main c16b1c5 --stat`: 12 files, all docs/markdown. No code changed.
- **Verdict:** CHANGES REQUESTED (3 must-fix)

## Gate checks

| Check | Result |
|---|---|
| No code changed | Pass |
| `node scripts/ci/check-workflow-policy.mjs` | Pass |
| Commands in CONTRIBUTING exist | Pass (`pnpm lint/typecheck/test`, `scripts/ci/check-i18n.mjs`, `scripts/gen-bindings.mjs`, `scripts/gen_stories.py --check`, `src-tauri/tests/dependency_rules.rs`, Rust 1.99.0 pin, Node 24 in CI) |
| Auto defaults (README, applying.md) | Pass (`DEFAULT_DAILY_CAP=5`, `DEFAULT_GLOBAL_DAILY_CAP=10`, global max 25, per-company 1 / 30 d, gap 5–20 min, hold 5 min–24 h, session hold ≥ 60) |
| Verdict bands (ranking.md) | Pass (75 / 60 / 45 / 30 / below 30) |
| Tracker statuses | Pass (`AppStatus` in `boba-domain/src/application`) |
| Fraud and ghost badges | Pass (`SignalKind` in `boba-domain/src/scoring/fraud`) |
| Offers per run default 30 | Pass (`DEFAULT_OFFERS_PER_RUN`) |
| Locked idle timeout 8 h | Pass (`DEFAULT_IDLE`) |
| Source failing after 3 runs | Pass (`sources/health.rs`, FR-31) |
| Session board Auto: type board name, 3/day, 10 max, 10 min gap, 60 min hold, 7 d pause, 30 d reconfirm | Pass |
| UI labels named in docs (Activate rule, Mark applied, Send application, I submitted it, Override veto, Reset to defaults, Retry reading the key, Test fetch, Save feed, Disclosures, Network activity, AI engine) | Pass (in `locales/en`) |
| Platforms and bundle targets (macOS 13+, dmg/msi/nsis/deb/appimage/rpm) | Pass (`tauri.conf.json`) |
| Relative links | 7 broken, all to targets that do not exist yet (see must-fix 2 and 3) |

## Must-fix

1. **Privacy claim is incomplete (`README.md` "Privacy", `docs/user-guide/privacy-and-backup.md` "Network").** The docs say Boba only contacts "the Sources you enabled, your mailbox provider, and (if you turn it on) GitHub for update checks". Two other opt-in contacts exist in code:
   - Optional embeddings model download from Hugging Face (`src-tauri/crates/boba-embed/src/encoder.rs`, `hf-hub`), when the user opts in.
   - Optional salary FX rates from `api.frankfurter.dev` (`src-tauri/crates/boba-app/src/network_log/fx.rs`, locale `network.json` "Salary conversion", off by default).
   Fix: list both as opt-in contacts, or write "…and the optional extras you switch on (model download, exchange rates)". The privacy page must match the Network activity view.

2. **CHANGELOG claims a released 1.0.0 that the code does not yet carry.** `CHANGELOG.md` has `## [1.0.0] - 2026-10` and a release link `github.com/diegoarenas/boba-job-find/releases/tag/v1.0.0`. The workspace is at `0.1.0` (`package.json`, `src-tauri/tauri.conf.json`), no tag exists, and the updater endpoint slug is unconfirmed (residual `R-S10.5-F5`). Fix: label the section `## [Unreleased]` until the version bump and first `-beta` tag, or get the version bump and release decision from Diego first. Remove or mark the release link as a placeholder.

3. **Broken links to pages that do not exist on any branch.**
   - `docs/contributing/` (README, CONTRIBUTING, docs/README): not present on `main`, FD1 or FD2. CH13 has no commits yet. Merge CH13 first, or drop these links.
   - `docs/security/` (README, SECURITY, docs/README): resolves only once FD1 merges. Merge FD1 before FD2.
   Re-run the link check after both merge.

## Nits (optional)

1. `docs/user-guide/troubleshooting.md` says "Boba is signed with the project's own certificate". Accurate for macOS, but Windows is unsigned (SignPath pending). Consider stating the per-OS status in one line.
2. `docs/user-guide/getting-started.md` step 6: "Safe Sources are already on for your markets". Consider naming the default-on list or linking to Connections, since the set varies by market.

## Checked and fine

- No invented features found: InfoJobs is correctly listed as v1.1 (`prd-addendum.md`), session boards are described with their opt-in gates, Limited mode wording matches `limitedBody`.
- No secrets, no real personal data, no private emails in the docs (the only address in code, `hiring@boba.dev`, is fixture data and is not quoted).
- README and SECURITY agree with `docs/security/security-model.md` on Kill Switch, caps, OAuth loopback and session boards.

## Re-review at `142db51`

- **Changed since `0ebf94e`:** `README.md`, `CHANGELOG.md`, `CONTRIBUTING.md`, `docs/README.md`, `docs/user-guide/{getting-started,privacy-and-backup,troubleshooting}.md`. No code changed (no `.rs`, `.ts`, `.tsx`, `.json` in the diff).
- **Must-fix 1 (privacy list):** closed. Privacy page and README now name the optional extras: GitHub update checks, the Hugging Face on-device model download, and api.frankfurter.dev salary exchange rates. Matches `boba-embed/src/encoder.rs` and `boba-app/src/network_log/fx.rs`.
- **Must-fix 2 (1.0.0 claim):** closed. CHANGELOG section is now "Unreleased, targeting 1.0.0" with the "not yet tagged" line, and the `v1.0.0` release link is gone.
- **Must-fix 3 (broken links):** closed for `docs/contributing/`. The links are dropped from README, CONTRIBUTING and docs/README.
- **Remaining, expected:** `docs/security/` links in README, SECURITY and docs/README resolve only after FD1 merges. Merge FD1 first (FD1 approved at `093d8d2`, content `a28ceec`).
- **Nits:** closed for the Windows/Linux Gatekeeper wording and the Sources sentence. Two line-length nits remain (getting-started step 6, privacy "Network" bullet): reflow when convenient.
- **Checked:** workflow policy pass; no emails, keys or tokens added; relative links pass except the FD1-dependent `docs/security/` set.

**Verdict: APPROVED** at `142db51` (content), pending FD1 merge for the `docs/security/` links.
