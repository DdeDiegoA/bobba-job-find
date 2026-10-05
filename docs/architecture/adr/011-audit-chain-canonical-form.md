# ADR-011 — Audit chain: frozen canonical form and anchor semantics

- **Status:** Accepted · 2026-10-05 (S1.7 merged; review `docs/reviews/S1.7.md`)
- **Spine:** AD-12

## Context
AD-12 hashes each `audit_event` row as `blake3(prev_hash ‖ canonical_json(row))`. Once the first row ships, those bytes become an on-disk contract: any change to key order, escaping or the set of fields breaks verification of every existing chain. S1.7 also fixed how the keychain `audit-head` anchor tracks the head of the chain, including after a crash.

## Decision
1. **Frozen canonical form (v1).** `boba_domain::audit::AuditRow::canonical_json` is RFC 8785 (JCS), restricted to TEXT and INTEGER values:
   - **Keys** are emitted in this exact sorted order: `action, actor, correlation_id, created_at, detail_json, id, intent_id, outcome, payload_blake3, phase, prev_hash, seq, target_id, target_kind`. `hash` is excluded.
   - **Optional columns** (`intent_id`, `outcome`, `payload_blake3`, `target_id`) are present as JSON `null` when empty. They are never omitted.
   - **Strings** use the shortest JSON escapes, lowercase `\uXXXX` for control characters, no escaping of `/`, and raw UTF-8 for everything else. `seq` is a plain integer. `detail_json` is embedded as a **string**: it is not re-canonicalized as nested JSON.
   - **Hash input** is `prev_hash` (ASCII hex) **immediately followed by** the canonical JSON. `prev_hash` therefore appears twice, once as the prefix and once as a column, which is intended. The hash is blake3, stored as 64 lowercase hex characters. The genesis `prev_hash` is 64 zeros.
   - The golden test `canonical_json_is_sorted_and_excludes_hash` is the contract. **Changing this form needs a new ADR and a versioned chain**: a `chain_version` column plus a migration that starts a new segment anchored to the old head. Existing rows are never re-hashed.
2. **Anchor semantics.** The store writes the row and then mirrors its hash to the keychain (`app.bobajobfind / audit-head`), both inside the single-writer actor. At startup the chain is verified first. Then:
   - **Anchor = head:** clean.
   - **Anchor names an earlier known row:** this is a crash between the insert and the keychain write. The chain is accepted and the anchor is re-synced to the head, with no warning.
   - **Anchor names no row** (tail truncated, or the log re-created): an `AnchorMismatch` tamper warning is shown to the user.
   - **No anchor** (first run, or after Reset): the anchor is written.

## Accepted residual risk
- **Rows past the anchor are accepted.** Accepting a lagging anchor is what lets the app recover from a crash. The cost is that rows appended after the anchored row can be truncated undetected, but only while the anchor already lags, which happens after a crash between the insert and the keychain write (a sub-millisecond window). In normal operation the anchor equals the head, so any tail truncation removes the anchored row and is detected.
- **Local tamper evidence, not tamper proof.** A process running as the same user can rewrite both the DB and the keychain item consistently. AD-12 defends against accidental corruption and casual edits, and makes every outbound action explainable. It does not stop malware that already has the user's privileges, which is outside the threat model (architecture §13).

## Consequences
- Refactors of `AuditRow` must not touch `canonical_json` byte output. CI enforces this through the golden test.
- Adding a column to `audit_event` is a canonical-form change (point 1), not a free schema edit.

## Alternatives
- **serde_json with a canonicalizing crate:** a dependency on crate behaviour we don't control for a frozen contract. Rejected in favour of the 40-line explicit writer.
- **Strict anchor (any lag is a tamper signal):** a false alarm after every crash. Rejected.
