# ADR-004 — Storage: SQLCipher-encrypted SQLite + OS keychain

- **Status:** Accepted · Approved by Diego 2026-10-04 (§17 Q6), amended by ADR-009 · 2026-10-04
- **Spine:** AD-2, AD-13, AD-16, AD-20

## Context
Locked: local-only, encrypted SQLite, OS keychain for secrets, manual export and backup. The data includes PII (work history, salary, contact details), third-party recruiter emails, generated documents and OAuth tokens.

## Decision
- **rusqlite 0.40** with feature `bundled-sqlcipher-vendored-openssl`: SQLCipher 4, statically linked, so the build is identical on all three OSes. WAL, `foreign_keys=ON`, `secure_delete=ON`.
- **Key.** 32 random bytes generated at first run, stored in the OS keychain via **keyring-core 1.0** plus the platform stores (macOS Keychain, Windows Credential Manager, Linux Secret Service), under service `app.bobajobfind`, entry `db-key`. **Fallback** when no keychain is available: the user sets a passphrase, and the key is derived with Argon2id. That key is never stored.
- **Other secrets** (OAuth refresh tokens, API keys) live only in the keychain. The DB stores `secret_ref` ids.
- **Blobs** (PDF, DOCX, `.eml`) are stored inside the DB, so there is no plaintext file sprawl. Agent workspaces are temporary and deleted after each run.
- **Single owner.** Only the core opens the DB: one writer actor plus a read pool (r2d2).
- **Migrations.** Forward-only embedded SQL (rusqlite_migration 2.6), with a `VACUUM INTO` snapshot before migrating.
- **Backup.** An encrypted export re-keyed with a user passphrase (`sqlcipher_export`), plus a readable, upstream-compatible export.

## Consequences
- On macOS, Keychain access is bound to the app's signature. An ad-hoc-signed update would lose the key, so releases use a stable identity, and a missing key leads to the Locked state, never a new key (ADR-009).
- Losing the keychain entry (OS reinstall, a wiped keychain) means losing the data. Onboarding therefore pushes the user to make a passphrase-protected recovery backup.
- Vendored OpenSSL increases build time. rust-cache in CI absorbs it.
- No ORM. Repositories are hand-written SQL in `boba-store`, which is boring and fully inspectable.
- On Linux without Secret Service (some minimal window managers) the user types a passphrase once per login session (key in zeroized memory with an idle timeout). Until they do, scheduled jobs are recorded as `skipped_locked` and the user is notified (ADR-009).

## Alternatives
- **sqlx (SQLite).** SQLCipher is not supported cleanly, and compile-time checks need a live DB. Rejected.
- **libSQL/Turso encryption.** Ties us to a fork and a different toolchain. Rejected.
- **App-level field encryption on plain SQLite.** Easy to miss a column, indexes leak data, and it is more code. Rejected.
- **tauri-plugin-stronghold.** A separate vault format, and Tauri plans to remove it in v3. Rejected.
