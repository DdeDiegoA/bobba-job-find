# ADR-013 — Passphrase key derivation (Argon2id), its KDF record and the login-session key

- **Status:** Accepted · Approved by Diego 2026-10-16 (standing OK, via the orquestador, S1.2b) · 2026-10-16
- **Spine:** AD-13, AD-18, AD-20, ADR-004, ADR-009
- **Source:** story 1.2 (AC1/AC2), story 1.2b, ADR-009 §2/§3

## Context

ADR-004 and ADR-009 put the SQLCipher key in the OS keychain, with an Argon2id
passphrase as the fallback when no keychain is usable (headless Linux without
Secret Service). Story 1.2b implements that fallback, and two questions had no
recorded answer:

1. **Argon2id needs a salt**, and the salt must survive restarts or the same
   passphrase derives a different key. Where does a non-secret salt live, bearing
   in mind that "the app data directory holds only the encrypted DB, redacted logs,
   temporary agent workspaces and the browser profile" (prd-addendum A.4)?
2. **What does "the derived key is held in zeroized memory for the login session,
   dropped on idle timeout (default 8 h), quit or logout" mean in code**, given that
   the store — not the UI — owns the key it was opened with, and that a mounted
   port cannot be unmounted in Tauri?

## Decision

1. **The KDF record is a file next to the database**: `boba.db.kdf` (same path plus
   `.kdf`), created owner-only `0600` from its first byte (no chmod window; Windows:
   the app-data directory is already user-scoped, as accepted in S1.2a) and never
   over an existing file. It is JSON, versioned, and holds **only** the
   salt (16 random bytes, hex), the algorithm (`argon2id`), the parameters
   (`m_cost = 19456` KiB, `t_cost = 2`, `p_cost = 1`, the OWASP-recommended
   Argon2id profile) and the output length (32 bytes). **No key material is ever
   written to it**, so the file is safe to copy or back up — and the app data
   directory gains exactly one non-secret file.
2. **Parameters are read from the record, never assumed.** Deriving with the
   current defaults instead of the recorded ones would silently derive a different
   key after any parameter change and lock the user out of their data. An
   unreadable or unknown-version record is **refused**, never guessed.
3. **Never mint a KDF record over existing data.** If `boba.db` exists without a
   record, its key came from the keychain, and the passphrase path refuses it
   (`errors.unlock.no_passphrase`) instead of writing a salt that would derive a key
   that can never open it (ADR-009 §2). A record is only created when no `boba.db`
   exists.
4. **Before anything is unlocked, the derived key is proven**: the flow opens the
   database read-only with the key and reads `sqlite_master`, the same proof the
   store itself makes (a wrong key fails with `NotADatabase`). Only then does it
   mount the store and arm the session.
5. **The session is one per process** (`boba_store::unlock::KeySession`), managed by
   the composition root: closed by default when the passphrase path is in play,
   *idle* (transparent) when the keychain opened the store, and holding the derived
   key zeroized while unlocked. The idle timeout is **lazy**: every port lookup
   counts as activity, reads of the status do not, and a lookup after the window
   drops the session's key, flips the store state to `Locked` and returns the typed
   Locked error. Quit and logout drop it through the same path (`lock`). So no timer
   thread is needed, and the key is gone before the next command can see it.
   Re-unlocking does **not** re-open the store: the ports are already mounted, so
   the composition root only re-arms the session and reports the store `Ready`
   again (`mount_with_key` decides from the mounted ports, never from the status,
   because a second mount panics).
6. **Restoring from a recovery backup stays where it belongs** (S10.2): the unlock
   screen offers the keychain retry and the passphrase; the third option
   (ADR-009 §3) lands with the backup file.

## Consequences

- One extra file next to the database, documented here and in the QA doc. It is not
  an exportable secret and it is not in the DB (it cannot be: it is needed to open
  the DB).
- Losing the KDF record loses the passphrase path for that store: the key cannot be
  re-derived. The recovery backup (FR-111, S10.2) is the answer, which is why
  onboarding forces one on builds without Apple Developer ID or Authenticode.
- The idle timeout is enforced at the port boundary, so it holds for every command
  and for the MCP path. It does not close the already-open SQLite connections, and a
  caller that already holds an `Arc<P>` keeps using it: an attacker with code
  execution inside the process is out of scope (they can read memory anyway). Long
  running work must therefore not cache port handles across an idle window (S5.1).
- The KDF parameters are bounded on read (1 GiB, 16 passes, 8 lanes): a tampered
  record is refused instead of making unlock allocate until the machine dies. They
  are never clamped — clamping would derive a different key.
- The pool manager and the writer hold the key hex for the process lifetime; they
  are `zeroize`d buffers (S1.2a) and are only dropped at quit. So "dropped on idle
  timeout" means **the session's copy**: the open store keeps its own until quit. A
  future slice that wants "lock the store and truly close the connections" needs a
  mount/unmount redesign (ADR-012 §4), not a bigger session.

## Alternatives considered

- **Salt in a plain sidecar as raw bytes** — rejected: a JSON record can carry the
  parameters too, so the derivation is reproducible and auditable.
- **No salt / a fixed salt** — rejected: it makes one precomputed table valid for
  every install.
- **Salt in the Keychain** — rejected: the fallback exists exactly for machines
  without a usable keychain.
- **Store the passphrase (or the key) encrypted with a passphrase** — rejected: two
  secrets where one is enough, and the key would be at rest twice.
- **A background timer that locks on idle** — rejected: the lazy check at the port
  boundary is the same guarantee without a thread, and it cannot fire while the app
  is busy doing legitimate work.
- **Re-keying the DB when the keychain appears later** — rejected: it contradicts
  ADR-009 §2 and would rewrite the user's data to fix a startup path.
