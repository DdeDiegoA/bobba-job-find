-- 0023_apply_outbox.sql — the Outbox hold and the dry run (S9.7, FR-84/FR-85,
-- AD-16). Never edit once shipped (AD-20).
--
-- `apply_outbox` is one queued automatic send. It is written when the policy
-- says Auto and the rule is past its dry run (`held`), or when the rule is still
-- in its dry run (`would_send`, reported and never sent). During the hold the
-- user may cancel or edit the item; `subject_override`/`body_override` hold only
-- what the user changed, never the whole payload.
--
-- One application holds at most one open item at a time: a second queue of the
-- same application is a bug, not a second send.
--
-- `apply_rule_dry_run` is one rule's dry-run progress (FR-84): how many matches
-- it reports and whether the user activated it. A rule with no row never ran
-- and reports the compiled default of 3.
CREATE TABLE apply_outbox (
    id               TEXT PRIMARY KEY NOT NULL,
    application_id   TEXT NOT NULL,
    rule_id          TEXT NOT NULL,
    source_id        TEXT NOT NULL,
    channel          TEXT NOT NULL CHECK (channel IN ('email', 'ats_form', 'sanctioned_api', 'session_board')),
    company_key      TEXT NOT NULL,
    title            TEXT NOT NULL,
    company          TEXT NOT NULL,
    state            TEXT NOT NULL CHECK (state IN ('would_send', 'held', 'releasing', 'sent', 'cancelled', 'needs_review')),
    -- The configured hold, with the channel floor already applied (FR-91).
    hold_minutes     INTEGER NOT NULL CHECK (hold_minutes BETWEEN 5 AND 1440),
    -- When the hold ends (RFC 3339, UTC).
    release_at       TEXT NOT NULL,
    subject_override TEXT,
    body_override    TEXT,
    note             TEXT,
    created_at       TEXT NOT NULL,
    updated_at       TEXT NOT NULL
) STRICT;

CREATE UNIQUE INDEX apply_outbox_live_idx ON apply_outbox (application_id)
    WHERE state IN ('would_send', 'held', 'releasing');

-- The release sweep reads the held rows whose time has come.
CREATE INDEX apply_outbox_state_idx ON apply_outbox (state, release_at);

CREATE TABLE apply_rule_dry_run (
    rule_id      TEXT PRIMARY KEY NOT NULL REFERENCES auto_rule (id) ON DELETE CASCADE,
    required     INTEGER NOT NULL CHECK (required BETWEEN 1 AND 10),
    seen         INTEGER NOT NULL CHECK (seen >= 0),
    activated_at TEXT,
    updated_at   TEXT NOT NULL
) STRICT;
