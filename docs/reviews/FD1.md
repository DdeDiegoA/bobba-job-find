# Review FD1 — architect docs sync and security docs

- **Reviewer:** reviewer-3 (qa)
- **Branch:** `hermad/FD1` · **Reviewed commit:** `bbb61c3`
- **Scope:** docs only. `git diff main bbb61c3 --stat`: 10 files, all under `docs/`. No code changed.
- **Verdict:** CHANGES REQUESTED (1 must-fix)

## Gate checks

| Check | Result |
|---|---|
| No code changed | Pass (docs/ only) |
| `node scripts/ci/check-workflow-policy.mjs` | Pass |
| Relative links in changed files | Pass (0 broken) |
| Spine AD-3 event names vs code | Pass (`task:progressed`, `task:finished`, `run_report:progressed`, `kill_switch:changed`, `notification:opened` exist in `src-tauri/src/events`, `ipc::tests::every_registered_event_name_is_legal_for_tauri` exists) |
| tech-stack licence list vs `deny.toml` | Pass (same allow-list, plus the `webpki-*` CDLA exception) |
| ADR-014..017 facts vs code | Pass (`0700` dir, `D:P(A;;GA;;;OW)`, migrations 0011+ renumbered, `locales/{en,es}/errors/<area>.json`, `ERROR_FRAGMENTS`) |
| security-model §6 caps and holds | Pass (`DEFAULT_SESSION_DAILY_CAP=3`, `MAX=10`, gap 10 min, hold 60, challenge 7 d, reconfirm 30 d; pacing 30 s with 40 % jitter, floor 18 s) |
| security-model §7 kill switch | Pass (Cmd/Ctrl+Shift+K in `src-tauri/src/shortcuts`) |
| security-model §8 OAuth | Pass (`STATE_TTL` = 600 s; Gmail send-only scope; `gmail.readonly` only for BYO read) |
| security-model §10 release | Pass (`.github/workflows/ci.yml` gates secrets on PR; `release.yml` tag `v*`) |
| residuals.md vs `deny.toml` and `docs/reviews/` | Pass for security gates. Non-security S5.2 notes (`R-S5.2-1..5`) are not listed (see nit 1) |

## Must-fix

1. **`docs/security/security-model.md` §9 is inaccurate.** It says "All outbound HTTP goes through one hardened `HttpClient`" with private-address rejection and redirects off. The optional embeddings model download does not: it uses `hf-hub` (fastembed's own client) in `src-tauri/crates/boba-embed/src/encoder.rs:181`, behind the `embeddings` feature. The network log records it, but the HttpClient guards do not apply. Fix: state the exception in §9 and add a residual row in `residuals.md` (for example `R-S6.9-1`: hf-hub client is outside HttpClient, opt-in, checksum before load, recorded in the network log).

## Nits (optional)

1. `residuals.md` says "every row is a risk accepted by Diego's gate". The S5.2 notes are low-severity performance/quality items, not security gate items. Either list them in a short "non-security notes" section or drop the sentence's "every row" claim.
2. `security-model.md` §2 T8 says private addresses are "rejected at connect time". That is accurate, but `R-S4.8-2` (resolve-then-connect is not IP-pinned) should be linked from there so the two read consistently.

## Notes for the orquestador

- FD2 links to `docs/security/` resolve only after FD1 merges. Merge order: FD1 before FD2.
- FD2 links to `docs/contributing/` do not resolve on any branch yet (CH13 has no commits). Merge CH13 before FD2, or remove those links.
