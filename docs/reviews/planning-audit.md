# Planning-Phase Audit — Boba Job Find

> **Reviewer:** reviewer (qa) · **Date:** 2026-10-04 · **Phase:** Planning (no code exists)
> **Scope:** `docs/research/`, `docs/product/`, `docs/design/`, `docs/architecture/` — DB schema + SQLCipher/keychain/migrations, threat model (agent CLI via `boba-mcp`, OAuth, session-board browser automation, auto-apply/kill switch/audit, supply chain/updater), and cross-doc consistency.
> **Method:** adversarial + edge-case-hunter lenses (bmad-review), repo-audit Variant B (doc/config audit) and Variant D (DB-schema reasoning). Every finding cites the source location; severity-ranked P0/P1/P2.
> **Snapshot:** the docs were being edited by `pm`/`orquestador` *during* this audit (session-board MVP decision was propagating live). Findings below are against this snapshot; re-check line numbers before acting.
> `MD5 PRD.md c587852069374e98123590b8ff10d986 · epics-and-stories.md 1148582ce765cc6de61636894b4cfec0 · ARCHITECTURE-SPINE.md 9a9ff70c323b2c932918de07298a5031 · AGENTS.md 3ed23dfc9463363e4fdc8ceb89500962`

No code exists, so nothing here is a runtime repro; each P0/P1 is a **decision that must be closed before the story that encodes it is built**, or a **documented guarantee that does not hold as written**.

---

## Executive summary

| # | Sev | Area | Finding | Owner |
|---|---|---|---|---|
| 1 | **P0** | DB / keychain / signing | macOS ad-hoc signing + Keychain-held DB key ⇒ the DB key becomes unreadable after an auto-update (lockout / data loss) | architect + Diego |
| 2 | **P0** | Threat model | "Limited"/degraded CLIs may process untrusted postings/emails while keeping native web/shell tools ⇒ prompt-injection exfiltration path | architect + Diego |
| 3 | **P1** | Cross-doc | Session-board-in-MVP decision not fully propagated: PRD FR-81/FR-83 and epic story 9.6 still say Email+ATS only | pm + architect |
| 4 | **P1** | Cross-doc | `stories.yaml` machine-readable index is corrupt: `covers` IDs lose their type prefix (AD-x/NFR-x → FR-x), `files` split on commas; phases stale (E11) | pm |
| 5 | **P1** | DB | Passphrase fallback (no Secret Service) is incompatible with unattended scheduled tray runs | architect + Diego |
| 6 | **P1** | DB | Pre-migration `VACUUM INTO` snapshot: location/encryption/retention unspecified; violates AD-2; `Delete everything` does not remove it | architect |
| 7 | **P1** | DB | Encrypted backup/restore has no schema-version or integrity validation before replacing data | architect |
| 8 | **P1** | Threat model | Agent output (narration, evidence spans, drafts) is not treated as untrusted at the UI/persistence boundary | architect |
| 9 | **P1** | Threat model | Recruiter-email content is sent to the user's LLM provider but NFR-3 only discloses "Profile and Offer content" | architect + pm |
| 10 | **P1** | Threat model | Session-browser CDP/port and cookie-protection claims are underspecified; the "OS-level encryption" claim is false on Linux without a keyring | architect |
| 11 | **P1** | Auth | OAuth loopback + deep-link fallback: no `state`/nonce specified; scheme-hijack of the fallback not in the threat model | architect + Diego |
| 12 | **P1** | Cross-doc | `research/integrations.md` still says LinkedIn/Indeed automation "Do not build" / "out of bounds", contradicting the locked MVP decision | analyst + pm |
| 13 | **P2** | DB schema | `application_answer_bank` is used but absent from the spine ERD and ownership map (AD-16) | architect |
| 14 | **P2** | Cross-doc | Stale design docs: EXPERIENCE "(v1.1 … Session Board)", preview/DESIGN "Gmail read+send" | ux |
| 15 | **P2** | Cross-doc | AGENTS.md "Stack (proposed, pending architect)" header is stale | orquestador |
| 16 | **P2** | Cross-doc | ADR-005 Context stale vs its own Decision (auto scope) | architect |
| 17 | **P2** | Threat model / audit | Audit log immutability vs retention/erasure; orphan `intent` rows; no chain anchor | architect + pm |
| 18 | **P2** | Threat model | Threat table claims OS code signing as an updater mitigation, but MVP ships unsigned (NFR-17) | architect |
| 19 | **P2** | DB | Agent workspaces survive a crash (no cleanup-on-start) | architect |
| 20 | **P2** | Cross-doc | PRD FR-33/FR-90 InfoJobs v1.1 vs §6.1 "sanctioned APIs auto-capable by default" reads as MVP | pm |

---

## P0 — must close before build

### P0-1 — macOS ad-hoc signing makes the Keychain-held DB key unreadable after an update

**Location:** `docs/architecture/architecture.md:543` (§15 table, "MVP: ad-hoc + documented Gatekeeper bypass"), `:482` (§12 key management), `docs/architecture/adr/004-storage-encryption-sqlcipher.md:11,19`, `docs/product/PRD.md:675-679` (NFR-17), `AGENTS.md` (Signing row).

**Trigger condition.** MVP ships **ad-hoc-signed** macOS builds. The DB encryption key is stored as a generic-password item in the login Keychain (`app.bobajobfind` / `db-key`). macOS gates access to a Keychain item by the *designated requirement* of the creating binary. An ad-hoc signature has no Team ID; its designated requirement is effectively the **cdhash**, which changes on every build. After the signed updater replaces the `.app`, the new binary no longer matches the item's ACL → the user gets an access prompt (scheduled/headless tray runs cannot answer it), or the app creates a **new** `db-key` item → **the existing SQLCipher DB is unreadable**.

**Expected vs actual.** Expected: data stored today is readable after an app update. Actual: the key material and the binary that is allowed to read it are coupled to a signature that the chosen signing path deliberately keeps unstable.

**Concrete fix.**
- Treat stable macOS signing as a **release blocker for the first version that persists the key**, or
- if unsigned/ad-hoc must ship, **do not store the DB key in the Keychain**: use the Argon2id passphrase path as the primary on macOS for unsigned builds, and store no key item. Alternatively sign with a stable self-signed certificate whose designated requirement is pinned.
- Add an explicit acceptance test to story 1.2: "upgrade the app build (different binary hash) and confirm the store still opens without user interaction"; and require a completed recovery backup before the first run when the build is unsigned.
- Note the interaction in ADR-004 Consequences, where the trade-off currently reads only as "OS reinstall / wiped keychain".

**Potential consequence.** Users lose their entire encrypted DB after an auto-update; the failure is silent for background runs.

---

### P0-2 — "Limited"/degraded agent CLIs can process untrusted content with their native web/shell tools intact

**Location:** `docs/architecture/architecture.md:205` ("Limited: … All tasks are available, with a degraded-security badge and auto-apply disabled for runs that read untrusted text"), `:507` (threat row "Exfiltration via the agent": "Degraded-security CLIs are flagged and blocked from auto paths"), `docs/product/PRD.md:657` (NFR-7), `docs/architecture/ARCHITECTURE-SPINE.md:83` (AD-6). Compare `:196` — Codex `--sandbox read-only` still lets its shell read the workspace (and, for a "Limited" mode, the binary may keep network tools).

**Trigger condition.** A job posting or recruiter email contains injected instructions. The run is dispatched to a CLI that Boba has classified as **Limited** (MCP present, full tool lockdown absent). The untrusted text arrives inside the `{"untrusted_content": …}` envelope — but the *agent itself* is a tool-capable process that still has its own web-fetch/shell tools. The envelope is a prompt-level guard; it does not remove the capability. The documented mitigation only blocks **auto-apply** for these runs, not the exfiltration itself.

**Expected vs actual.** Expected (threat table): injected text cannot make the agent leak the profile. Actual: for every CLI except "Full", the capability to fetch an attacker URL remains, and the model can be induced to use it.

**Concrete fix.**
- Make *reading untrusted content* a capability-gated task class: a CLI must advertise `tool_lockdown` (strict MCP / allowlist that removes native shell+web) to run `extract_postings`, `classify_emails`, `tailor_application`, `review_application`, `narrate_scores`.
- CLIs lacking it are **unavailable for those tasks** (UI shows why), not merely barred from auto paths. Text-only CLIs already follow this pattern (input must fit the prompt); apply the same gate.
- Update AD-6 and §5.3 so the rule is "no untrusted-content task on a CLI without verified lockdown", and update the threat-table mitigation to match.
- Add an acceptance criterion to story 2.3: a Limited CLI attempting an untrusted-content task is refused, not merely denied auto-apply.

**Potential consequence.** A single malicious job posting exfiltrates the user's profile/PII through the user's own CLI provider.

---

## P1 — resolve during planning

### P1-3 — Session-board-in-MVP decision is not fully propagated (binding spec conflict)

**Location (current):**
- `docs/product/PRD.md:472` (FR-81 channel list) — "Session Board **[v1.1]**".
- `docs/product/PRD.md:481` (FR-83) — Auto Rules "scoped to Sources × Apply Channels **(Email Apply, ATS Form in MVP)**".
- `docs/product/epics-and-stories.md:733` (story 9.6) — same "{Email Apply, ATS Form}" scope.
- vs `docs/product/PRD.md:519` (FR-91 **[MVP]**), `:742`, `:749` (§10.1 MVP includes Session Board + Auto on Session Boards), `epics-and-stories.md:83,100,827` (E11 **[MVP]**), `architecture.md:434,572` (§17 Q5 "In MVP"), `adr/005:29`.

**Trigger condition.** A dev reading FR-81/FR-83 or story 9.6 builds the Auto Rules editor and channel detection for **Email + ATS only**; the MVP test plan (from FR-91/§10.1) then fails, or session-board auto is left out of the rules engine it must plug into. The channel enum, the FR-83 rule scope and the `session_form` policy branch are three different places that must agree.

**Concrete fix.**
- FR-81: change "- Session Board [v1.1]" → "- Session Board [MVP: LinkedIn, Indeed; v1.1: LATAM boards]".
- FR-83 first sentence: "scoped to Sources × Apply Channels (**Email Apply, ATS Form, and Session Board**)" and note the extra per-source opt-in gate.
- Mirror the same scope into `epics-and-stories.md:733` (9.6).
- Sweep for any remaining `[v1.1]` attached to FR-30/FR-91 or session boards.

**Potential consequence.** MVP ships without the auto-apply channel the user explicitly decided to include; or the rules engine is built too narrowly and needs a schema change.

---

### P1-4 — `stories.yaml` is a corrupt machine-readable index

**Location:** `docs/product/stories.yaml` (`# Generated from docs/product/epics-and-stories.md`).

Two systematic defects:

1. **`covers` loses the ID type prefix, so ADs and NFRs become fake FRs.** The generator strips `AD-`/`NFR-` and keeps the number, then labels everything `FR-`. Examples against `epics-and-stories.md`:
   - Story **1.2** epics covers `FR-109, AD-2, AD-13, AD-20, NFR-2, NFR-12` → yaml `["FR-2","FR-12","FR-109"]` (`stories.yaml:21`). `AD-2`/`NFR-12` became `FR-2`/`FR-12`.
   - Story **1.4** epics `FR-1, FR-114, AD-21, NFR-15` → `["FR-1","FR-15","FR-114"]` (`:44`). `NFR-15` → `FR-15`, which is a *Later* requirement (profile enrichment).
   - Story **1.6** epics `FR-42, AD-18, NFR-18` → `["FR-18","FR-42"]` (`:66`).
   - Story **1.8** epics `NFR-6, NFR-22` → `["FR-6","FR-22"]` (`:88`).
   - Story **10.5** epics `NFR-16…NFR-20, AD-22` → `["FR-16","FR-17","FR-19","FR-20"]` (`:719`).

2. **`files` are split on every comma**, including commas inside `{}` and backticks. Examples: `["locales/{en", "es}/", "src/app/"]` (`:43`), `["src-tauri/src/` (tray)"]` (`:65`), `["boba-agent/` (CliDriver)", "cli_install` table"]` (`:97`).

3. **Phases are stale**: E11 stories `11.1`/`11.2`/`11.3` still say `phase: "v1.1"` (`:737` and following) while `epics-and-stories.md` now marks E11 **[MVP]**.

**Trigger condition.** Any tooling that uses `stories.yaml` (dev spawn/plan, coverage checks, `hermad plan-devs`) routes or validates against wrong requirements — e.g. treats story 1.2 as covering `FR-2`/`FR-12` and not `AD-2`/`AD-13`/`NFR-2`, and receives malformed file paths.

**Concrete fix.**
- Regenerate `stories.yaml` from the markdown with a parser that (a) preserves the full ID (`FR-*`, `NFR-*`, `AD-*`) and (b) splits `files` on `, ` only outside `` ` ``/`{}`. Add a CI check that re-derives the yaml and fails on diff.
- Re-sync phases (E11 = MVP, story 13.4 = MVP — see P2-20 note).
- While fixing: the epics source itself mixes phase inside a v1.1 epic (story 13.4 Gmail read is **MVP** inside "Epic 13: Coaching extras [v1.1]"); give 13.4 its own home or fix the epic phase.

**Potential consequence.** Silent mis-routing of work and wrong coverage validation; the "dependencies checked in stories.yaml" claim is only as good as a corrupt file.

---

### P1-5 — Passphrase fallback (`no Secret Service`) cannot run unattended

**Location:** `docs/architecture/architecture.md:482` (§12), `docs/architecture/adr/004-storage-encryption-sqlcipher.md:11,19` ("on Linux without Secret Service the user must type a passphrase at launch"), `docs/architecture/ARCHITECTURE-SPINE.md:136`, story 1.2 AC (`docs/product/stories.yaml:24`).

**Trigger condition.** The app starts at login / is resumed from the tray to run `daily_search` (`architecture.md:128`) but the DB key is only derivable from a passphrase the user has not typed. The scheduler, the email sync and the retention sweep all need the DB; each fails or silently skips.

**Expected vs actual.** AD-18/FR-43 promise a run that "just happens" within 5 min of wake; the fallback design has no unattended path.

**Concrete fix.** Define the contract for the passphrase fallback: (a) prompt once per login session and cache the key in `zeroize`d memory with an idle timeout; (b) if not unlocked, the scheduler records the run as skipped and notifies "Boba is locked — enter your passphrase to run"; (c) never silently mark a run as successful. Document it in ADR-004 Consequences and story 1.2.

**Potential consequence.** On headless/minimal Linux the "daily search" never runs and the user is never told.

---

### P1-6 — Pre-migration `VACUUM INTO` snapshot: unspecified, and defeats "Delete everything"

**Location:** `docs/architecture/ARCHITECTURE-SPINE.md:182` (AD-20), `docs/architecture/architecture.md:489`, `docs/architecture/adr/004:15`; conflict with AD-2 (`ARCHITECTURE-SPINE.md:63`, "No component writes user data to disk outside the DB") and with FR-112 (`PRD.md:619-620`, Reset "wipes the DB and keychain entries").

**Trigger condition.** Any migration runs → a full copy of the encrypted DB (every posting, email, document blob, the audit log) is written somewhere. Where, with what mode, under what key, and for how long is **never stated**. If it lingers, "Delete everything" removes the DB and the keychain entry but leaves a complete second copy on disk; if it is created with the same key and the key is deleted, it becomes unreadable but still occupies disk.

**Expected vs actual.** Expected: "All app data lives inside an encrypted local database" and Reset permanently deletes all data. Actual: a second full copy exists outside the DB lifecycle and outside Reset.

**Concrete fix.** Specify in AD-20/§12: snapshot path under app data with `0600`/owner-only DACL; created only when disk space allows (`≥ 2 × DB size`), **deleted on successful migration** and after a fixed safety window (e.g. N days / N successful launches); included in the Reset deletion list; listed in Settings → Privacy as a local artifact.

**Potential consequence.** Unbounded disk growth; a complete PII copy that survives "Delete everything".

---

### P1-7 — Encrypted backup/restore has no validation before replacing data

**Location:** `docs/architecture/architecture.md:490` (§12 export/backup), `docs/product/PRD.md:616-617` (FR-111), `docs/product/epics-and-stories.md:784-787` (story 10.2), `docs/architecture/adr/004:18`.

**Trigger condition.** The user restores a backup produced by an older (or newer) app schema, or a corrupted/hostile SQLCipher file. Nothing in the design validates `user_version`/schema, migration state, app version or an integrity checksum before "replaces the current data".

**Concrete fix.** On restore: open the backup read-only with its passphrase, verify a stored `schema_version` + app version range and (if present) a header checksum/format magic; refuse and explain on mismatch; take a safety snapshot of the current DB first; only then swap. Add to story 10.2 AC.

**Potential consequence.** Silent data loss or an app that cannot open its own DB after a cross-version restore.

---

### P1-8 — Agent output is not treated as untrusted at the UI/persistence boundary

**Location:** `docs/architecture/architecture.md:274-282` (write tools), `:88` (AD-7 sanitizes only "text from postings, email bodies and company pages"), `docs/architecture/ARCHITECTURE-SPINE.md:204` (`save_score_narration`), story 2.4/6.5. The write tools (`save_score_narration`, `save_document_draft`, `propose_posting_features` evidence spans) persist text derived from untrusted input.

**Trigger condition.** A malicious posting induces the agent to emit narration/evidence/draft text containing hostile markup (or an instruction that a *later* agent run reads as trusted "profile"/"narration" context). The UI renders narration and evidence; the next run reads it back. AD-7 covers the inbound envelope but not the round-trip.

**Concrete fix.** (a) Treat every LLM-produced string as untrusted on render (plain text or ammonia-sanitized; never `dangerouslySetInnerHTML`); neutralize Typst markup in drafts before rendering. (b) Preserve provenance: text persisted from an untrusted run keeps an `untrusted_origin` flag and is re-enveloped when fed to a later run. (c) Extend the threat table with "injection persistence / stored prompt injection".

**Potential consequence.** Stored XSS in the webview, or a self-reinforcing injection that escalates across runs.

---

### P1-9 — Recruiter-email content sent to the LLM provider is undisclosed

**Location:** `docs/product/PRD.md:653` (NFR-3: "Profile and Offer content is sent to the provider…"), `docs/architecture/architecture.md:507` (threat table disclosure wording), `:474` (classification). `classify_emails` is an agent task whose input is email bodies.

**Trigger condition.** An uncertain recruiter reply is classified by the user's CLI → the email body (third-party personal data) is sent to Anthropic/OpenAI/etc. The onboarding disclosure names only Profile and Offer content.

**Concrete fix.** Update NFR-3 and the onboarding copy to name **email content** explicitly; add an option to keep email classification deterministic-only (no LLM) or on a local model; add this data flow to Settings → Privacy → Network activity (already lists purposes).

**Potential consequence.** Users are told data flow is limited to Profile+Offer while their inbox is also processed by a third party; a privacy/GDPR expectation gap.

---

### P1-10 — Session-browser CDP/cookie security claims are underspecified and partly false

**Location:** `docs/architecture/architecture.md:343-351` (§6.4), `docs/product/PRD.md:237` (FR-30, "cookies … protected by the browser's own OS-level encryption"), `docs/product/prd-addendum.md:71-77` (A.4), `docs/architecture/adr/005:19`.

**Trigger condition / gaps.**
- The CDP endpoint's binding is never specified. `chromiumoxide` launches Chrome with `--user-data-dir`; if a fixed `--remote-debugging-port` (or `--remote-debugging-address=0.0.0.0`) is used, another local process (or a page) can drive the user's authenticated session. Fix: bind to `127.0.0.1`, ephemeral port, and/or use a pipe; never expose beyond the user.
- "Protected by the browser's own OS-level encryption" is **false on Linux without a Secret Service/keyring**: Chromium falls back to a hardcoded key, so cookies are effectively plaintext for any same-user process. This is exactly the configuration ADR-004 already treats as a first-class case (no keychain). Fix: qualify the warning per platform; on no-keyring Linux, state the cookies are not meaningfully encrypted and require an explicit acknowledgement.
- The dedicated profile persists login cookies indefinitely with no rotation/expiry story. Fix: state retention and offer "sign out / clear session" per source.

**Potential consequence.** A user is driven into believing the session cookies are better protected than they are; session hijack by same-user malware; CDP port misuse.

---

### P1-11 — OAuth fallback (deep-link) and `state` handling absent from the threat model

**Location:** `docs/architecture/architecture.md:472` (§11 OAuth, "RFC 8252 loopback … with PKCE"), `:132` and `tech-stack.md:29` (deep-link plugin as OAuth fallback), `adr/002`. No `state`/nonce, no deep-link hijack analysis.

**Trigger condition.** Loopback is blocked → the app falls back to a custom-scheme deep link. Any same-user app can register the same scheme and race for the redirect; PKCE protects the code exchange, but `state` (CSRF) and redirect validation are not specified, and the fallback is not covered by the threat table.

**Concrete fix.** Specify: mandatory `state` (cryptographically random, single-use, bound to the running run), PKCE `S256` in both loopback and deep-link paths, validate the issuer/code against the stored verifier+state, and document the deep-link residual risk (scheme squatting) with a fallback to loopback/BYO client. Add a threat-table row for OAuth redirect interception. This is auth → escalate to Diego.

**Potential consequence.** Authorization-code interception in the fallback path; account/token compromise.

---

### P1-12 — Research doc contradicts the locked MVP decision on LinkedIn/Indeed automation

**Location:** `docs/research/integrations.md:231-232` ("LinkedIn automation … **Do not build**", "Indeed/Glassdoor/ZipRecruiter automation … **Do not build**"), `:180-182` ("Out of bounds for a shipping product: silent auto-submit on LinkedIn/Indeed…"), `:110`; vs `docs/product/PRD.md:519,742,749` and `docs/architecture/architecture.md:572` (session-board auto **in MVP**).

**Trigger condition.** Research is an input others cite (`prd-addendum.md:33`, `PRD` inputs). Shipping with a research doc that says "do not build" the exact MVP feature creates a compliance/review contradiction and confuses new contributors.

**Concrete fix.** Add an explicit deviation note at the top of `integrations.md` §4/§6/§7: "Product decision 2026-10-04: LinkedIn/Indeed session-board auto-apply ships in MVP, opt-in per source, core-executed with caps/kill-switch/audit; this overrides the earlier 'do not build' recommendation; the residual account-ban risk is user-accepted and re-confirmed every 30 days." Keep the underlying risk facts. Update the risk matrix row.

**Potential consequence.** A future maintainer (or a security/legal reviewer) cites the research against the shipped feature.

---

## P2 — fix when convenient / during the owning story

### P2-13 — `application_answer_bank` missing from the spine ownership map
**Location:** `docs/architecture/ARCHITECTURE-SPINE.md:271-302` (ERD + owner table), used at `docs/architecture/architecture.md:458`, `docs/product/epics-and-stories.md:620,714`. AD-16 says every table has exactly one owner; this table has none and no ERD edge. **Fix:** add it (owner: apply) to the ERD and ownership table, with its relation to `application`.

### P2-14 — Stale design docs after the MVP decision
**Location:** `docs/design/EXPERIENCE.md:82` ("(v1.1 generic page / SDK / **Session Board**)"), `docs/design/preview.html:563` ("Gmail OAuth · **read+send** · Connected"), `docs/design/DESIGN.md:617-620` ("Gmail OAuth … read+send"). Session boards are MVP (P1-3); Gmail read is send-only by default with BYO/IMAP (PRD FR-34). **Fix:** reclassify Session Board as MVP in the custom-source line; change the Gmail prototype tile to "send-only (read via IMAP/BYO)".

### P2-15 — AGENTS.md stale stack header
**Location:** `AGENTS.md:36` "## Stack (proposed, pending architect)" while the same file's pipeline says architecture ✅ and the architecture docs are final. **Fix:** retitle to "Stack (adopted — source of truth: `docs/architecture/tech-stack.md`)".

### P2-16 — ADR-005 internal contradiction
**Location:** `docs/architecture/adr/005-source-access-and-scraping-policy.md:7` (Context: "auto-apply is ON-capable only for public ATS forms and sanctioned APIs") vs `:29` (Decision: session-board auto ships in MVP). **Fix:** mark line 7 as the superseded earlier policy, or rewrite Context to narrate the change.

### P2-17 — Audit log: immutability vs retention, crash gaps, and no anchor
**Location:** `docs/architecture/ARCHITECTURE-SPINE.md:127-130` (AD-12), `docs/architecture/architecture.md:459`, `docs/product/PRD.md:514,623`, story 9.9. Gaps: (a) the log retains "the exact Package" indefinitely while email/raw retention is 90/180 days → retention controls are not authoritative; no redaction/tombstone path for third-party data; (b) a crash between `intent` and `outcome` leaves an orphan intent with no reconciliation rule; (c) `blake3(prev_hash ‖ canonical_json(row))` — `canonical_json` must exclude `hash` and be deterministic, and the chain has no external anchor, so a full rewrite is undetectable. **Fix:** define retention/redaction for audit payloads (store hash + reference instead of full PII), an orphan-intent startup pass, and `canonical_json` invariants (+ optionally anchor the latest hash in the OS keychain).

### P2-18 — Threat table overstates updater mitigation for MVP
**Location:** `docs/architecture/architecture.md:514` ("OS code signing (Apple notarization, Windows Authenticode)") vs `docs/product/PRD.md:675-679` (NFR-17: MVP unsigned/ad-hoc). **Fix:** make the row match MVP (minisign updater signature only; OS signing optional/deferred) — overlaps P0-1.

### P2-19 — Agent workspaces survive a crash
**Location:** `docs/architecture/architecture.md:235` and `:68` ("deleted when the run ends"), AD-2/AD-6. Workspaces contain rendered prompts plus task inputs (profile/offer text) and are deleted only on clean completion. **Fix:** delete/overwrite any orphaned run workspace on startup; document it in AD-6.

### P2-20 — InfoJobs v1.1 vs §6.1 "auto-capable by default"
**Location:** `docs/product/PRD.md:693` ("Auto-capable by default … Sanctioned APIs (InfoJobs)") vs `:516` (FR-90 [v1.1]), `:760` (§10.2 v1.1). Reads as if InfoJobs auto is MVP. **Fix:** tag it "[v1.1]" in §6.1 or rephrase to "in v1.1". (Also: story 13.4 Gmail read is **MVP** living inside the v1.1 epic — see P1-4.)

---

## Cross-doc consistency matrix (post-live-edit snapshot)

| Decision | AGENTS.md | PRD | addendum | epics | design | architecture/ADR | research | Verdict |
|---|---|---|---|---|---|---|---|---|
| Session-board auto in **MVP** | ✅ In MVP | ⚠ FR-81/FR-83 still old; FR-91/§10.1 ✅ | ✅ A.3 MVP | ⚠ E11 ✅, story 9.6 old, yaml stale | ⚠ EXPERIENCE:82 stale | ✅ §17 Q5, ADR-005:29 | ❌ says "do not build" | **P1-3, P1-12, P2-14** |
| Gmail **send-only** MVP | ✅ | ✅ | ✅ | ✅ (13.4 BYO read) | ⚠ preview/DESIGN read+send | ✅ | ✅ | **P2-14** |
| Optional embeddings (opt-in download) | ✅ | ✅ FR-55/story 6.9 | ✅ A.5 | ✅ | n/a | ✅ Q4 | ✅ | consistent |
| OSS signing path (unsigned/ad-hoc + SignPath) | ✅ | ✅ NFR-17 | ✅ | ✅ 10.5 | n/a | ✅ §15, Q1 | n/a | consistent, but see **P0-1/P2-18** |
| Local-only, no backend, no telemetry | ✅ | ✅ NFR-1 | ✅ | ✅ | ✅ | ✅ | ✅ | consistent |

---

## Peer routing

- **architect (tech):** P0-1, P0-2, P1-5…P1-11, P2-13, P2-16, P2-17, P2-18, P2-19.
- **pm (scope/consistency):** P1-3, P1-4, P1-9 (copy), P1-12, P2-14, P2-20; plus the story 13.4 epic-phase glitch in P1-4.
- **escalate to Diego (approval policy — auth/DB/security/money):** P0-1 (signing↔keychain), P0-2 (threat-model sign-off), P1-5/P1-7/P1-11 (DB key + OAuth), P1-10 (session cookies).

## What this audit did **not** cover
- Runtime behaviour: no code exists. Every item is a design/decision finding, not a reproduced defect.
- Dependency CVE/licence facts: versions in `tech-stack.md` were not independently re-verified against registries during this pass (cargo-deny/`npm` checks are specified for CI).
- UX visual/a11y quality beyond scope references: DESIGN/EXPERIENCE were checked for scope consistency, not for craft (that is a separate review).

---

# Round 2 — Re-audit after Diego's rulings (2026-10-05)

> **Reviewer:** reviewer (qa) · **Trigger:** orquestador, round 2. Architect, pm and ux applied Diego's rulings (`architecture.md` §17 approvals, ADR-009/ADR-010) plus every "approved" audit fix.
> **Method:** same lenses. **Every finding re-verified against the current file text, not against the change summaries.** `stories.yaml` re-derived and validated: `python3 scripts/gen_stories.py --check` → **exit 0**.
> **Round-2 snapshot hashes:** PRD `e0875e693f72bc0ea49f8cf8a20f5f7f` · epics `979a3878ed5eee7fe73793a9641ac001` · stories.yaml `6c10cf770665b54ce3d03bc203589c52` · ARCHITECTURE-SPINE `b94f841deab29950ec1a1e1cec59ed57` · architecture `49cd8fe61069f8250d3ac2764ac2b8de` · DESIGN `0929bb5d00e5da4e9a69306f7180be38` · EXPERIENCE `3d56680911fce60c1f5b72f32fdc0a35` · voice-and-tone `9dca501c85898aeb6d12531c32f39d0f` · integrations `b3b0e6bf47267f4958d61b460bf90d79` · AGENTS `69535b5027deb68e259a88c69732ba00`

## Verdict

**All 20 findings resolved. No P0, P1 or P2 open.** P0-1 residual (`architecture.md:587`) fixed by `architect` and re-verified; P2-14 copy fixed by `ux`, and the rendering regression that fix introduced (P2-14c) was fixed and re-verified. The binding spec conflicts (P1-3) and the corrupt machine index (P1-4) are genuinely fixed, not papered over.

| # | Sev | Status | Evidence (current text) | Owner |
|---|---|---|---|---|
| P0-1 | P0 | ✅ Resolved | ADR-009 items 1–6; §15 signing row "never ad-hoc" (`architecture.md:561`) + signing guard (`:566`); story 1.2 upgrade AC + `VACUUM INTO` AC (`epics:130-131`); forced recovery backup in ADR-009 Consequences, story 3.8 (`epics:335`) and 10.2 (`epics:807-808`); PRD NFR-17 (`:678`); AGENTS signing row. | architect |
| P0-2 | P0 | ✅ Resolved | ADR-010 items 1–2; AD-4/AD-6/AD-7 (`SPINE:73,83,88`); NFR-7 (`PRD:659`); threat row `Exfiltration via the agent`; story 2.3 AC hard gate; UX `DESIGN §10.3` (TaskUnsupportedNotice) + `EXPERIENCE §9.15`. | architect |
| P1-3 | P1 | ✅ Resolved | FR-30 `[MVP: LinkedIn, Indeed]` (`PRD:233`); FR-81 `:473`; FR-83 `:483` includes Session Board under FR-91; story 9.6 AC `epics:755`; E11 `[MVP]`; story 11.4 v1.1. | pm |
| P1-4 | P1 | ✅ Resolved | `scripts/gen_stories.py` preserves `FR/NFR/AD/ADR` prefixes, backtick/brace-aware `files`, validates phase/deps/cycle/coverage; `--check` exit 0; story 1.8 adds the CI diff check (`epics:194`); E11 stories `phase: MVP`; story 13.4 → 8.5. | pm |
| P1-5 | P1 | ✅ Resolved | ADR-009 item 3; §12 (`architecture.md:496`); story 1.2 AC `skipped_locked` + 8 h idle timeout; `EXPERIENCE §9.15` + `DESIGN §10.3`. | architect |
| P1-6 | P1 | ✅ Resolved | ADR-009 item 6; §12 snapshot line (`architecture.md:503`); Reset deletes snapshots (`:504`); Settings → Privacy lists it. | architect |
| P1-7 | P1 | ✅ Resolved | ADR-009 item 7; §12 restore line (`architecture.md:504`) read-only + `boba_backup_meta` + `integrity_check` + schema guard + pre-swap snapshot; story 10.2 AC. | architect |
| P1-8 | P1 | ✅ Resolved | ADR-010 item 3; AD-7 (`SPINE:88`); threat row `Stored prompt injection / XSS via agent output` (`architecture.md:522`); story 2.3 AC. | architect |
| P1-9 | P1 | ✅ Resolved | NFR-3 names recruiter email + deterministic-only option (`PRD:655`); story 8.4 AC (`epics:682-683`); threat row disclosure wording. | pm |
| P1-10 | P1 | ✅ Resolved | §6.4 CDP pipe/`--remote-debugging-port=0` on `127.0.0.1` + per-platform cookie warning + sign-out (`architecture.md:360-362`); FR-30 (`PRD:237-238`); ADR-005 item 5; threat row `:524`. | architect |
| P1-11 | P1 | ✅ Resolved | §11 loopback-only, PKCE S256, single-use 256-bit `state`, no deep-link fallback (`architecture.md:485-486`); threat row `:523`; `tech-stack` deep-link struck; ADR-001; FR-34; agent AC `epics:652`. | architect |
| P1-12 | P1 | ✅ Resolved | `integrations.md` deviation notes (`:105,:171,:198`), risk matrix rows (`:237-238`); PRD §7 deviation (`:703`); ADR-005 item on the supersede. | pm |
| P2-13 | P2 | ✅ Resolved | `APPLICATION_ANSWER_BANK` ERD edge (`SPINE:289`) + owner `apply` (`SPINE:304`). | architect |
| P2-14 | P2 | ✅ Resolved | Sitemap/flow/tiles fixed (`EXPERIENCE:81,247,507-525`; `DESIGN:619-628,646-648`; `preview.html:563`). Copy conditional EN+ES (`voice-and-tone:104,128-129,190-191`). P2-14c (table render) fixed — note above the table (`:86-89`), 1 `<table>`/25 `<tr>`, standalone `<blockquote>`. | ux |
| P2-15 | P2 | ✅ Resolved | `AGENTS.md` now `## Stack (architect-approved)`. | orquestador |
| P2-16 | P2 | ✅ Resolved | ADR-005 Context now narrates the same-day supersede. | architect |
| P2-17 | P2 | ✅ Resolved | AD-12 (`SPINE:130`): hashes not payloads, JCS canonical form, orphan-intent pass, keychain head anchor, no UPDATE/DELETE triggers. | architect |
| P2-18 | P2 | ✅ Resolved | Threat row `Update hijack` matches MVP: minisign only, macOS self-signed not Gatekeeper-trusted, Windows unsigned (`architecture.md:532`). | architect |
| P2-19 | P2 | ✅ Resolved | ADR-010 item 4; AD-6; story 2.3 AC orphan sweep; §5.4 startup sweep. | architect |
| P2-20 | P2 | ✅ Resolved | §6.1 "Sanctioned APIs (InfoJobs, available from v1.1)" (`PRD:696`); FR-33/FR-90 `[v1.1]`. | pm |

## Open items (Round 2)

### ~~P2-14b — `voice-and-tone.md` promised recruiter-reply reading regardless of provider~~ — **RESOLVED** (ux, 2026-10-05)

**Severity:** P2 (user-facing copy; contradicted the Gmail send-only default).
**Original location:** `docs/design/voice-and-tone.md:99/123/184`.
**Fix applied & re-verified:** copy is now conditional on the connection mode, EN+ES — `:104` onboarding Email ("Gmail sends only by default…"), `:128-129` empty state split (read-on vs send-only), `:190-191` Send (Review) split (read-on vs send-only) — plus a binding rule at `:94` ("Reply-reading copy is always conditional (FR-34)"). ✅ Content correct. (The rule's placement caused **P2-14c** below.)

### ~~P2-14c — the P2-14b binding note sat inside the §5.1 table and broke its rendering~~ — **RESOLVED** (ux, 2026-10-05)

**Severity:** P2 (doc rendering regression; hid the just-fixed strings).
**Original location:** `docs/design/voice-and-tone.md:94-97` (blockquote between table rows `:92` and `:98`).
**Problem (original):** GFM terminated the §5.1 table at the blank line; the rendered table had 6 `<tr>` (header + 5 rows, stopping at "Agent CLI — none") and every following row — Email step, Apply Mode, all Recovery strings — was swallowed into a `<blockquote><p>` as raw `| … |` text.
**Fix applied & re-verified:** the binding note now sits **above** the table (`:86-89`, after "Order follows PRD FR-1…FR-10.", blank line each side); the note renders as a standalone `<blockquote>`. Re-render of §5.1 (`markdown_it`, GFM): **1 `<table>` / 25 `<tr>` (header + 24 rows)**, table contiguous through `:116`; Email step `:104`, Apply Mode and all Recovery rows present as `<td>` cells. ✅

### ~~P0-1 residual — `architecture.md` §17 Q1 still says macOS ships "unsigned/ad-hoc"~~ — **RESOLVED** (architect, 2026-10-05)

**Severity:** P3 (decision-log row contradicted the resolved decision; low risk of being read as current).
**Location:** `docs/architecture/architecture.md:587`.
**Problem (original):** the Q1 row read "Free OSS path first: **unsigned/ad-hoc builds** with a documented Gatekeeper/SmartScreen bypass". ADR-009 item 1, §15 (`:561`) and the signing guard (`:566`) all say macOS uses **one persistent self-signed certificate and never ad-hoc**; only Windows is unsigned.
**Fix applied & re-verified:** §17 Q1 now reads "**macOS:** every build is signed with one persistent project self-signed certificate (never ad-hoc, ADR-009), with a documented Gatekeeper bypass… **Windows:** unsigned … until SignPath Foundation"; §17.1 P0-1 row aligned to "one persistent self-signed certificate for every macOS build". Matches §15 / ADR-009. ✅

### P3 note (low confidence, no action required before build)

`EXPERIENCE §9.15` / `DESIGN §10.3` show a task × CLI support matrix where one CLI (Codex) is supported for `extract_postings`/`classify_emails` but unsupported for `tailor/review/narrate`. ADR-010 models `tool_lockdown` as a single probed capability, so under that model the support column should be uniform per CLI; per-task differences may legitimately come from other capabilities (`structured_output`), but the `TaskUnsupportedNotice` reason text currently always says "tool_lockdown". Optional: have the reason name the actual missing capability. Not a spec conflict.

## Cross-doc consistency matrix (Round 2)

| Decision | AGENTS.md | PRD | addendum | epics | design | architecture/ADR | research | Verdict |
|---|---|---|---|---|---|---|---|---|
| Session-board auto in **MVP** | ✅ | ✅ FR-30/81/83/91/§10.1 | ✅ A.3 | ✅ E11, story 9.6 | ✅ | ✅ §17 Q5, ADR-005 | ✅ deviation notes | **consistent** |
| Gmail **send-only** MVP | ✅ | ✅ FR-34 | ✅ | ✅ (8.5 BYO read) | ✅ copy + render fixed | ✅ | ✅ | consistent |
| Optional embeddings (opt-in download) | ✅ | ✅ FR-55/story 6.9 | ✅ A.5 | ✅ | n/a | ✅ Q4 | ✅ | consistent |
| OSS signing path (never ad-hoc mac; SignPath win) | ✅ | ✅ NFR-17 | ✅ | ✅ 10.5 | n/a | ✅ §15, Q1 row ✅ (`:587`, fixed) | n/a | consistent |
| Local-only, no backend, no telemetry | ✅ | ✅ NFR-1 | ✅ | ✅ | ✅ | ✅ | ✅ | consistent |
| Untrusted-content hard gate (P0-2) | ✅ | ✅ NFR-7 | ✅ A.6 | ✅ story 2.3 | ✅ §10.3/§9.15 | ✅ ADR-010 | ✅ | consistent |
| stories.yaml machine index | n/a | source | n/a | mirror | n/a | n/a | n/a | consistent (`--check` exit 0) |

## Peer routing (Round 2)

- **ux:** ~~**P2-14b**~~ and ~~**P2-14c**~~ — **both RESOLVED** (re-verified 2026-10-05). Also: the P0-1 forced prep-run recovery backup *is* already specced — DESIGN onboarding step 9, EXPERIENCE Flow A step 8, story 3.8 + 10.2 AC, and voice-and-tone §5.1 recovery strings — so no new first-run surface is needed.
- **architect:** ~~**P0-1 residual** (`architecture.md:587`)~~ — **RESOLVED** (fixed & re-verified 2026-10-05, §17 Q1 + §17.1).
- No P0/P1 routed. Round-1 escalations (P0-1/P0-2 sign-off, DB key, OAuth, session cookies) were already ruled on by Diego on 2026-10-05 and are reflected in ADR-009/ADR-010.

## What Round 2 did **not** cover
- Same as Round 1: no runtime repro (no code), no independent CVE/licence re-verification, no visual/a11y craft review.
- Warning **copy** for Prohibited-automation sources (FR-30/FR-91 typed confirmation) is referenced and versioned but not yet reviewed as copy — it is a candidate for the first build-phase review.

