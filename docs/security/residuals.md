# Boba Job Find — Accepted Residual Risks

> **Status:** v1, 2026-10-17 · **Owner:** architect · Every row is a risk that was **accepted** (by Diego's gate or the standing OK to take the recommended option) and not fixed in v1. Source: `docs/reviews/*.md`, the build memory and `deny.toml`. Companion to [`security-model.md`](./security-model.md).
> Items that were fixed in their slice are not listed. Severity is the reviewer's, where given.

## 1. Advisory ignores (`deny.toml`)

| ID | Area | Risk | Why accepted | Follow-up |
|---|---|---|---|---|
| RUSTSEC-2024-0436 | `paste` via tokenizers/fastembed | Unmaintained proc-macro, compile-time only, no vuln | No alternative tokenizer for the optional embeddings feature | Revisit when tokenizers drops `paste` |
| RUSTSEC-2024-0370 | `proc-macro-error` via gtk-rs 0.18/Tauri (Linux) | Unmaintained proc-macro, compile-time only | Owned by Tauri upstream | Revisit when Tauri moves to gtk-rs ≥ 0.19 |
| RUSTSEC-2025-0141 | `bincode` via syntect/typst | Unmaintained, no vuln | Typst stack, syntax highlighting we do not use on untrusted input | Revisit when typst drops syntect/two-face |
| RUSTSEC-2026-0206 | `rustybuzz` via typst-pdf | Unmaintained, no vuln | Font shaping only | Revisit when typst moves to harfrust |
| RUSTSEC-2026-0192 | `ttf-parser` via fontdb/typst | Unmaintained, no vuln | Parses fonts we bundle | Revisit when typst moves to skrifa |
| RUSTSEC-2024-0320 | `yaml-rust` via syntect/typst | Unmaintained, no vuln | Theme YAML only | Revisit when syntect drops it |
| RUSTSEC-2026-0194, -0195 | `quick-xml` 0.38.4 via citationberg/hayagriva | DoS on crafted XML | Unreachable: our templates use no `#bibliography`/`#cite` and parse no untrusted XML. No fixed release: citationberg pins `^0.38` | Revisit when citationberg allows quick-xml ≥ 0.41 |
| RUSTSEC-2024-0449 | `unsafe-libyaml` via serde_yaml/hayagriva | Unmaintained, archived, no safe upgrade | YAML path unreachable from our four templates | Re-assess in every document slice |
| RUSTSEC-2026-0367, -0368, -0369 | `zip` 4.6.1 via tauri-plugin-updater (Windows only) | Extraction preserves SUID bits, symlink escapes | Needs an attacker-controlled archive to reach zip; the updater verifies the minisign signature **before** extraction. All zip 4.x is yanked and every updater release requires `^4` | **Remove** the three ignores when the updater accepts zip ≥ 9. They are by id, so they would cover any zip version |
| `zip@4.6.1` (yanked) | Same path | Yanked crate | Scoped crate ignore; `yanked = deny` stays on for everything else | Revisit when a non-yanked 4.x exists or the updater drops zip |
| (no id) | `zip` 8.6.0 via docx-rs 0.4.22 | The ids above are extraction-side advisories hidden by the by-id ignore | `boba-docs` only **writes** DOCX; extraction is not reachable | Upgrade docx-rs when it moves to zip ≥ 9 |
| RUSTSEC-2026-0370, -0371, -0372 | `zvariant`, `zbus_names`, `zvariant_derive` via zbus 5 (Linux D-Bus deps) | Unmaintained, no vuln | Folded into zbus 6; no dependent accepts zbus 6 yet. Not built on macOS | Revisit when Tauri plugins accept zbus 6 |

## 2. Key custody, locked state and audit integrity

| ID | Area | Risk | Why accepted | Follow-up |
|---|---|---|---|---|
| R-S1.7-1 (ADR-011) | Audit chain | Rows after a lagging anchor can be truncated undetected, only while the anchor already lags after a crash between insert and keychain write | A lagging anchor is what lets the app recover from a crash | None; documented in ADR-011 |
| R-S1.2b-1 | Unlock | The open `Store` and its pool keep the derived key until quit; idle expiry drops only the session copy | An in-process attacker is out of scope (ADR-013) | Doc wording was corrected to match |
| R-S1.2b-2 | Unlock | The idle gate covers port lookups only; an `Arc<P>` already held keeps working | Same | Scheduler and agent runs must not cache a port across the idle gate |
| R-S1.2b-3 | Unlock | KDF parameters read from the record are not bounded; a local writer could force a memory DoS on unlock. It cannot weaken the store | Local-write attacker is out of scope | Optional hardening: clamp to the OWASP ceiling |
| R-S1.2b-4 | Unlock | The KDF record exists briefly at the default umask before `0600`; it holds only salt and parameters | Low impact | Optional hardening: create with mode `0600` |
| R-S1.2b-5 | Unlock | `PRAGMA key` strings are plain `String`s, not zeroized (pre-existing in `db::apply_key`) | Low risk | None |
| R-S1.2b-6 | Unlock | The passphrase sits in React state and crosses IPC as JSON; the JS copy cannot be zeroized | Local desktop app | Clear the input as soon as possible |
| R-S1.2b-7 / R-S1.2a | Windows | `owner_only` is a no-op on Windows; we rely on the per-user AppData ACL | Per-user directory | Verify in the S10.6 QA pass |
| R-S1.2b-8 | macOS | Manual key-survives-upgrade test (`docs/qa/macos-upgrade-test.md`) not run | Needs a real signed build | Run it before the first release |
| R-S10.2-D1 | Reset | On Windows, if the DB file cannot be unlinked, its content is erased but the file and key stay; the app still starts on the empty store | File locking on Windows | Surfaced via `remaining`; re-check at S10.6 |
| R-CH6-1 | Store tests | `cfg(test)` is per crate; a test outside `boba-store` that calls `Store::open` would compile the platform path | No such test exists | Re-check if one is added |

## 3. Agent sandbox and untrusted content

| ID | Area | Risk | Why accepted | Follow-up |
|---|---|---|---|---|
| R-S2.3a-1 | Agent socket (macOS) | The socket file mode cannot be set; only the `0700` directory protects it | Same-user processes are inside the threat model | None |
| R-S2.3a-2 | Agent socket | Per-run endpoint uses the last 16 ULID chars (80 bits); not a boundary | The per-run token is the boundary | None |
| R-S2.3b-1 | Agent sandbox (Windows) | The owner-only DACL is covered by a compile check and a non-null test, not by an ACE-list test on a Windows runner | CI has no Windows test job | Add a Windows job |
| R-S2.3b-2 | Lockdown scope | Only Claude Code is verified for untrusted tasks; Codex, OpenCode and Aider are unsupported for them | Safe direction (ADR-014) | Add drivers with recorded init-event fixtures |
| R-S2.4-1 | Lockdown gate | The version-only pass for Claude relies on the pinned config being applied for that version | ADR-010 re-verification on each CLI update; the per-run init check backs it | None |
| R-S11.1b-3 | Session boards | The untrusted tag is documented and `ammonia`-normalised, not enforced by a type | Relies on the S2.4 gate for any LLM step | Add a taint type |
| R-S3.5-1 | CV import | PDF inflation cap is checked after `lopdf` loads; many streams just under the per-stream cap cause a transient memory spike | The input is a file the user chose | Pre-scan the PDF |
| R-S3.5-2 | CV import | The import command accepts any path | Input is the user's own file; UI uses the dialog | Restrict to dialog-granted paths |

## 4. Network, sources and email

| ID | Area | Risk | Why accepted | Follow-up |
|---|---|---|---|---|
| R-S4.8-1 | Custom sources | An unresolvable host shows the `private_host` copy | Refusal is correct; wording misleads | Copy polish |
| R-S4.8-2 | Custom sources | Resolve-then-connect is not IP-pinned, so DNS rebinding between check and connect is theoretical | Redirects are off | Pin the resolved IP |
| R-S8.1-1 | Graph OAuth | `offline_access` is requested beyond `Mail.Read`/`Mail.Send` | Needed for the background daily run; shown in plain language | None |
| R-S8.1-2 | Graph OAuth | The bundled client id comes from build-time `BOBA_GRAPH_CLIENT_ID`, which is not registered yet | BYO client id works | Register the public client before release |
| R-S8.1-3 | OAuth | `state` is compared with `!=`; stray loopback connections are served one at a time with a 5 s timeout | Local, single-use 256-bit state; bounded by the 10-minute TTL | None |
| R-S8.2-1 | Gmail send | The bundled Google client secret ships in every binary | Google documents the installed-app secret as non-confidential | Register the client; set `BOBA_GMAIL_CLIENT_ID/SECRET` at build |
| R-S8.2-2 | Gmail send | Live OAuth, SMTP and IMAP paths are tested only with mocks | No real registered client yet | Verify once against a real client (S10.6) |
| R-S8.2-3 | Gmail | The client secret sits in the keychain entry next to the tokens | Same exposure as the tokens (AD-13) | None |
| R-S8.5-1 | Gmail read | BYO projects in Google's "Testing" status issue refresh tokens that expire after 7 days | Google policy | Documented in the BYO setup text |
| R-S8.5-2 | Gmail read | `gmail.readonly` is restricted; public distribution of BYO reading needs verification, so the bundled client stays send-only | CASA deferred | Revisit with CASA |
| R-S8.5-3 | Gmail read | One GET per message, up to 500 per page on first sync | Slow, not unsafe | Concurrency later |
| R-S9.3-1 | Email apply | A crash after `Sent` is recorded but before the `ok` outcome logs `unknown_after_crash` for a sent message | Conservative; send stays blocked by `check_evidence` | Revisit if the audit UI needs the distinction |

## 5. Apply automation and session boards

| ID | Area | Risk | Why accepted | Follow-up |
|---|---|---|---|---|
| R-S11.1a-4 | Session store | Before unlock, `keyring_available()` reads as `linux_weak` | Conservative: asks for the extra acknowledgement | None |
| R-S11.1a-5 | Challenge detection | `classify_challenge` matches substrings and can false-positive | Fails safe (pauses a source) | Tune with real page text |
| R-S11.1a-6 | Session opt-in | `enable` does not re-check the Linux keyring at confirm time | Low | Re-check at confirm |
| R-S11.1b-1 | Session sign-out (Windows) | The `SingletonLock` format is unverified on Windows, so the live-Chrome check may not fire | Verify on a Windows box | S10.6 QA |
| R-S11.1b-2 | Session pacing | Effective minimum delay is 18 s, not 30 s (30 s with 40% jitter) | Documented | None |
| R-S11.1b-4 | Session boards | If `opted_in_ids()` fails the boards are left out of the run, not reported | Fails closed | None |
| R-S11.1b-5 | Session audit | The page-load audit is best-effort per source; a failed audit read/write refuses that page load | Fails closed per source | None |
| R-S11.2-1 | Session PDF | A plaintext PDF can survive a crash until a startup sweep exists | Local file | Add a startup sweep |
| R-S11.3-1 | Session Auto | The browser port never presses a board's submit; Auto parks as `channel_unavailable`. Policy is enforced but nothing can send yet | Deliberate for v1 | Parked feature |
| R-S11.3-2 | Outbox | `OutboxService::enqueue` has no production caller; enforcement goes through `candidate_for`/`decide()` | Same decision seam | Carry to the wave that wires the Outbox |
| R-S11.3-3 | Session policy | The 60-minute hold constant is duplicated (`MIN_HOLD_MINUTES`, `SESSION_BOARD_MIN_HOLD_MINUTES`) | Equal today | Merge into one constant |
| R-S11.3-4 | Session policy | An apply-side challenge does not disable the reading Source; the run side does | Out of scope; the user acts, Boba does not | Close in a later pass |
| R-S6.8-1 | Fraud rules | The salary 3×-p50 scam rule is inert until a salary benchmark source ships | No benchmark data | Post-v1 |

## 6. Data egress and logs

| ID | Area | Risk | Why accepted | Follow-up |
|---|---|---|---|---|
| R-S10.1-1 | Export | A token inside a URL **path** segment is not stripped | Rare on supported boards | None |
| R-S10.1-2 | Export | `audit.csv` keeps `prev_hash` and `hash`; profile facts and notes ship as written | Chain evidence, not keys; the user's own data and a warning covers it | None |
| R-S9.9-1 | Audit viewer | `detail_json` is shown verbatim; wrapped error strings could carry sensitive text | The payload builders read carry no cookies or tokens | Redact errors at the source |
| R-S9.9-2 | Audit export | `audit_log_export_csv` writes to any path it receives | The UI passes the save-dialog path | Restrict to dialog paths |
| R-S8.1-4 | Tokens | `TokenSet` and `TokenResponse` derive `Debug`, which prints tokens if ever logged | Nothing logs them today | Replace with a redacting `Debug` |

## 7. Release pipeline

| ID | Area | Risk | Why accepted | Follow-up |
|---|---|---|---|---|
| R-S10.5-F2 | CI | Any `v*` tag push runs the signing release | Needs repository write access | Restrict `v*` tags with a ruleset, protect the `release` environment, document in `RELEASING.md`; first release as `-beta` |
| R-S10.5-F3 | CI | The `.p12` password is on argv for milliseconds on the runner | Ephemeral, single-tenant runner | None |
| R-S10.5-F4 | CI | The publish gate checks that `latest.json` has signatures, not that they verify | The updater verifies the signature before install | None |
| R-S10.5-F5 | Updater | The updater endpoint slug is hard-coded and was not confirmed against the published repo | No remote in the worktree | Confirm before the first `v*` tag |
| R-S10.5-keys | Signing keys | The updater minisign key and the macOS identity sit in `.hermad/keys/` (gitignored) | Interim | Move to CI secrets plus an offline backup; **never delete the directory** first |
| R-S10.5-win | Windows | Login-launch window flash (S5.4 F3) | Cosmetic | Set `visible=false` at launch when autostarted |
| R-S10.5-sig | Signing | Windows signing depends on SignPath Foundation approval; macOS needs the documented Gatekeeper bypass | Free OSS path (ADR-009) | Apple Developer ID later |
