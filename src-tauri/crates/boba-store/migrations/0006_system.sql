-- 0006_system.sql — typed settings and the append-only audit log (S1.7).
-- Owner: core/system (AD-16). Never edit once shipped (AD-20).
--
-- `setting` holds typed keys with compiled defaults (Consistency Conventions:
-- config lives in the DB, never in loose JSON files). `updated_at` is UTC
-- RFC 3339 (AD-17), written by the caller from the injected Clock.
--
-- `audit_event` is the hash-chained, append-only log (AD-12). Each outbound
-- action writes an `intent` row before it and an `outcome` row after it. Every
-- column except `hash` is covered by `hash = blake3(prev_hash ‖ JCS(row))`.
-- Columns store metadata, IDs and blake3 hashes of payloads — never payloads.
-- Zero or more rows may share a `correlation_id` (one attempt); an `intent`
-- with no `outcome` (crash) is resolved at startup with `unknown_after_crash`.
CREATE TABLE setting (
    key        TEXT PRIMARY KEY NOT NULL,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE audit_event (
    seq            INTEGER PRIMARY KEY AUTOINCREMENT,
    id             TEXT NOT NULL UNIQUE,
    created_at     TEXT NOT NULL,
    phase          TEXT NOT NULL CHECK (phase IN ('intent', 'outcome')),
    action         TEXT NOT NULL,
    outcome        TEXT CHECK (
                       (phase = 'intent'  AND outcome IS NULL) OR
                       (phase = 'outcome' AND outcome IN ('ok', 'failed', 'unknown_after_crash'))
                   ),
    correlation_id TEXT NOT NULL,
    intent_id      TEXT,
    actor          TEXT NOT NULL,
    target_kind    TEXT NOT NULL,
    target_id      TEXT,
    detail_json    TEXT NOT NULL DEFAULT '{}',
    payload_blake3 TEXT,
    prev_hash      TEXT NOT NULL,
    hash           TEXT NOT NULL,
    CHECK (
        (phase = 'intent'  AND intent_id IS NULL) OR
        (phase = 'outcome' AND intent_id IS NOT NULL)
    )
) STRICT;

CREATE INDEX audit_event_correlation ON audit_event (correlation_id);
CREATE INDEX audit_event_intent_id ON audit_event (intent_id);

-- AD-12: the log is append-only. UPDATE and DELETE are rejected in SQL, so no
-- code path — not even a buggy one — can rewrite or erase an audit row.
CREATE TRIGGER audit_event_no_update
BEFORE UPDATE ON audit_event
BEGIN
    SELECT RAISE(ABORT, 'audit_event is append-only: UPDATE rejected (AD-12)');
END;

CREATE TRIGGER audit_event_no_delete
BEFORE DELETE ON audit_event
BEGIN
    SELECT RAISE(ABORT, 'audit_event is append-only: DELETE rejected (AD-12)');
END;
