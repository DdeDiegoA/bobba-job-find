# ADR-009 — DB key custody across unsigned builds, and the Locked state

- **Status:** Accepted · Approved by Diego 2026-10-05 (planning audit P0-1, P1-5…P1-7) · 2026-10-04
- **Spine:** AD-13, AD-18, AD-20, AD-22
- **Source:** planning audit P0-1, P1-5, P1-6, P1-7 (`docs/reviews/planning-audit.md`)

## Context
Diego chose the free OSS signing path (§17 Q1): no Apple Developer ID and no Authenticode at MVP. The DB key lives in the OS keychain (ADR-004). On macOS, Keychain access is bound to the creating binary's *designated requirement*. An **ad-hoc** signature's requirement is its cdhash, which changes on every build. After an auto-update, the new binary can't read `db-key`: the user sees a prompt that a background run can't answer, or, worse, the app creates a new key and the existing DB becomes unreadable. Windows Credential Manager and Linux Secret Service do not bind items to the signature, so only macOS is affected. The audit also found three gaps nearby: the passphrase fallback has no unattended behaviour, the pre-migration snapshot has no lifecycle, and restore does no validation.

## Decision
1. **Every macOS build is signed with ONE persistent self-signed certificate** (no ad-hoc builds), with a pinned designated requirement. Releases, betas and CI artifacts all use the **stable, project-owned self-signed code-signing identity** (generated once, private key stored as a CI secret plus an offline maintainer backup). Its designated requirement (`identifier "app.bobajobfind" and certificate leaf = H"…"`) stays the same across builds, so updates keep Keychain access. Gatekeeper still warns on first install, and the documented bypass still applies. `release.yml` and the CI build fail if `codesign -dv` reports `Signature=adhoc` or a designated requirement other than the pinned one. Local `tauri dev` builds, which don't hold the private key, use a separate keychain service (`app.bobajobfind.dev`) and a separate data dir, so they never touch a user's real store.
2. **Never mint a key over existing data.** At startup the core generates `db-key` only if **no** `boba.db` exists. If the DB exists and the key can't be read (ACL change, wiped keychain, missing Secret Service), the app enters the **Locked state**. It never creates a new key or a new DB.
3. **Locked state** (also the normal state for the passphrase fallback):
   - The UI shows an unlock screen with these options: retry keychain access (foreground, so the OS prompt can be answered), enter the passphrase (fallback path), or restore from the recovery backup.
   - The scheduler records each due job as `skipped_locked` in `job_run`, never as success, and sends one notification per slot: "Boba is locked. Open it to run today's search."
   - **Passphrase path:** the user unlocks once per login session. The derived key stays in `zeroize`d memory with an **idle timeout** (default 8 h, configurable). Idle timeout, quitting or logging out drops the key and returns the app to Locked. A scheduled run while locked is skipped and notified, never silently counted as a success.
4. **Developer ID migration** (when Apple signing arrives): the first launch after the switch has a new designated requirement. It goes through the Locked state once, the user approves "Always Allow" in the foreground, and the next update is stable again. Release notes say so.
5. **Upgrade test.** Story 1.2 gains this acceptance check: install build N, store data, install build N+1 (different binary hash, same identity), and the store opens with no user interaction. It runs on macOS release candidates (manual QA, because there is no macOS e2e).
6. **Pre-migration snapshot** (AD-20): `<appdata>/snapshots/pre-<from>-<to>-<ts>.db`, owner-only (0600 or an owner-only DACL), same key as the DB. It is taken only if free disk space is at least 2× the DB size; otherwise the migration is refused with a clear message. It is deleted after 3 successful launches on the new schema or 14 days, whichever comes later. Reset deletes it. Settings → Privacy lists it.
7. **Restore validation** (FR-111 and story 10.2): open the backup **read-only** with its passphrase; check the format marker table (`boba_backup_meta{format, app_version, schema_version, created_at}`) and run `PRAGMA integrity_check`; refuse a `schema_version` newer than this app supports and explain why. Before swapping in the backup, snapshot the current DB (same lifecycle as item 6), then migrate the restored copy forward with the normal migration path.

## Consequences
- Free and boring. It costs one secret and one CI check. Updates keep their data on all three OSes.
- A lost self-signed private key means one forced re-approval for every macOS user. The offline backup of the key covers this.
- Users are never silently cut off from their data. The worst case is a visible Locked state with a recovery path.
- **Forced recovery backup.** On builds without Apple Developer ID or Authenticode (every MVP build), onboarding cannot finish, and no first run happens, until the user has created a passphrase-protected recovery backup. Story 1.2 / onboarding AC.

## Alternatives
- **Passphrase as the primary macOS key path.** Safe, but every login needs typing and background runs depend on it. Rejected as the default; it stays the fallback.
- **Pay for Apple Developer ID now.** The clean fix, but it contradicts Q1. It remains the planned upgrade.
- **Store the key in a file next to the DB.** That defeats the encryption. Rejected.
