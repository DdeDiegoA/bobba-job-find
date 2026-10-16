-- 0007_profile.sql — profile facts, versioning and bilingual variants (S3.4).
-- Owner: profile (AD-16). Never edit once shipped (AD-20).
--
-- The profile is a set of immutable `profile_fact` rows (AD-15). A fact **item**
-- has a stable ULID (`item_id`) that every revision keeps, so documents can cite
-- a fact across edits; each confirmed change appends a new revision row with its
-- own ULID (`id`), `supersedes` the previous revision and bumps `profile.version`.
-- An edit never mutates or deletes a row (the triggers below enforce it), so a
-- document always keeps citing the exact text it used and History can restore it.
--
-- A free-text fact carries up to two language variants (AC2, AD-17: BCP-47
-- reduced to the two shipped content languages). At least one variant must be
-- present; a missing one is flagged in the UI, never invented here.
--
-- `profile_fact_proposal` holds facts an agent proposed and the user has not
-- confirmed yet (S3.5 import, S3.6 translation drafts). Its `untrusted_origin`
-- is the producing run's `input_trust` (1 = untrusted input such as a posting;
-- 0 = user input such as the user's own CV), per ADR-010. A confirmed
-- `profile_fact` deliberately has **no** `untrusted_origin` column: it is the
-- user's own claim (AD-15). It keeps provenance instead — `provenance` says
-- whether the user typed it or confirmed an agent proposal, and `agent_run_id`
-- names the run that proposed it, for audit.
CREATE TABLE profile (
    -- One profile per app (single-user, local-only). The id is a constant, not a
    -- ULID: it never crosses IPC and every version bump must hit the same row.
    id         TEXT PRIMARY KEY NOT NULL CHECK (id = 'profile'),
    version    INTEGER NOT NULL CHECK (version >= 0),
    updated_at TEXT NOT NULL
) STRICT;

INSERT INTO profile (id, version, updated_at) VALUES ('profile', 0, '');

CREATE TABLE profile_fact (
    id              TEXT PRIMARY KEY NOT NULL,
    item_id         TEXT NOT NULL,
    kind            TEXT NOT NULL CHECK (
                        kind IN ('identity', 'experience', 'skill', 'education',
                                 'certification', 'language', 'eligibility')
                    ),
    text_en         TEXT,
    text_es         TEXT,
    evidence        TEXT,
    detail_json     TEXT NOT NULL DEFAULT '{}',
    supersedes      TEXT REFERENCES profile_fact (id),
    profile_version INTEGER NOT NULL CHECK (profile_version >= 1),
    provenance      TEXT NOT NULL CHECK (provenance IN ('user', 'agent_proposal')),
    agent_run_id    TEXT,
    created_at      TEXT NOT NULL,
    -- AC2: a fact holds at least one language variant.
    CHECK (text_en IS NOT NULL OR text_es IS NOT NULL),
    CHECK (supersedes IS NULL OR supersedes <> id),
    -- ADR-010 provenance: an agent-proposed fact always names its run.
    CHECK ((provenance = 'agent_proposal') = (agent_run_id IS NOT NULL))
) STRICT;

-- AD-15: an item has one revision per version and one child per revision, so a
-- fork or a duplicated version is rejected by the schema, not just avoided by
-- the single-writer actor.
CREATE UNIQUE INDEX profile_fact_item ON profile_fact (item_id, profile_version);
CREATE UNIQUE INDEX profile_fact_supersedes ON profile_fact (supersedes);

-- AC3 (AD-15): facts are immutable. Enforced in SQL so no code path can rewrite
-- the text a document cites; editing appends a revision instead.
CREATE TRIGGER profile_fact_no_update
BEFORE UPDATE ON profile_fact
BEGIN
    SELECT RAISE(ABORT, 'profile_fact is immutable: append a revision (AD-15)');
END;

CREATE TRIGGER profile_fact_no_delete
BEFORE DELETE ON profile_fact
BEGIN
    SELECT RAISE(ABORT, 'profile_fact is immutable: append a revision (AD-15)');
END;

CREATE TABLE profile_fact_proposal (
    id               TEXT PRIMARY KEY NOT NULL,
    -- Set when the proposal revises an existing item (translation draft, S3.6).
    item_id          TEXT,
    kind             TEXT NOT NULL CHECK (
                         kind IN ('identity', 'experience', 'skill', 'education',
                                  'certification', 'language', 'eligibility')
                     ),
    text_en          TEXT,
    text_es          TEXT,
    evidence         TEXT,
    detail_json      TEXT NOT NULL DEFAULT '{}',
    -- Excerpt the proposal was read from, shown with Confirm/Edit/Delete (S3.5).
    source_excerpt   TEXT,
    -- The producing run's `input_trust` (ADR-010): 1 untrusted, 0 user input.
    untrusted_origin INTEGER NOT NULL CHECK (untrusted_origin IN (0, 1)),
    agent_run_id     TEXT NOT NULL,
    created_at       TEXT NOT NULL,
    CHECK (text_en IS NOT NULL OR text_es IS NOT NULL)
) STRICT;
