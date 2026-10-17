-- 0022_auto_rules.sql — the Auto Rules and the ledger the caps count (S9.6,
-- FR-83/FR-87, AD-16). Never edit once shipped (AD-20).
--
-- `auto_rule` is the user's rule set: scope (sources × channels), thresholds,
-- the FLAG policy, the caps and the active hours. Core validates every bound
-- before a row lands (`RuleViolation`); the CHECKs here are the second lock, so
-- a hand-edited database still cannot hold a cap of 99.
--
-- `auto_rule_attempt` is one automatic submission, written **before** the
-- channel starts, so a send that dies mid-flight still counts against the caps.
-- The company key is the normalised employer (`acme`), which is what the
-- per-company cap groups on. Nothing here stores an email body or form answer.
CREATE TABLE auto_rule (
    id               TEXT PRIMARY KEY NOT NULL,
    name             TEXT NOT NULL,
    enabled          INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    -- JSON array of source ids; '[]' means every Source.
    source_ids       TEXT NOT NULL,
    -- JSON array of apply channels: email, ats_form, sanctioned_api, session_board.
    channels         TEXT NOT NULL,
    min_rank         REAL NOT NULL CHECK (min_rank BETWEEN 0 AND 100),
    min_fit          REAL NOT NULL CHECK (min_fit BETWEEN 0 AND 100),
    min_confidence   TEXT NOT NULL CHECK (min_confidence IN ('low', 'medium', 'high')),
    allow_flags      INTEGER NOT NULL CHECK (allow_flags IN (0, 1)),
    verdict          TEXT NOT NULL CHECK (verdict IN ('strong', 'good', 'moderate', 'weak', 'poor')),
    daily_cap        INTEGER NOT NULL CHECK (daily_cap BETWEEN 1 AND 25),
    per_company_cap  INTEGER NOT NULL CHECK (per_company_cap >= 1),
    per_company_days INTEGER NOT NULL CHECK (per_company_days BETWEEN 1 AND 365),
    active_start     INTEGER NOT NULL CHECK (active_start BETWEEN 0 AND 1439),
    active_end       INTEGER NOT NULL CHECK (active_end BETWEEN 0 AND 1439),
    created_at       TEXT NOT NULL,
    updated_at       TEXT NOT NULL
) STRICT;

CREATE TABLE auto_rule_attempt (
    id             TEXT PRIMARY KEY NOT NULL,
    rule_id        TEXT NOT NULL REFERENCES auto_rule (id) ON DELETE CASCADE,
    application_id TEXT NOT NULL,
    source_id      TEXT NOT NULL,
    channel        TEXT NOT NULL CHECK (channel IN ('email', 'ats_form', 'sanctioned_api', 'session_board')),
    company_key    TEXT NOT NULL,
    created_at     TEXT NOT NULL
) STRICT;

-- The three cap reads: all of today, one rule's day, one company's window.
CREATE INDEX auto_rule_attempt_created_idx ON auto_rule_attempt (created_at);
CREATE INDEX auto_rule_attempt_rule_idx ON auto_rule_attempt (rule_id, created_at);
CREATE INDEX auto_rule_attempt_company_idx ON auto_rule_attempt (company_key, created_at);
