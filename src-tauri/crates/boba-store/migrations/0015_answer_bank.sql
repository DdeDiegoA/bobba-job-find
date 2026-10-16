-- 0015_answer_bank.sql — screening answers and the bank they are reused from
-- (S7.8, FR-80, AC2/AC3). Never edit once shipped (AD-20).
--
-- Owner: apply (AD-16, spine ERD `APPLICATION ||--o{ APPLICATION_ANSWER_BANK`).
-- One row is one answer to one screening question, asked on one application.
-- The question is whatever the ATS form (or the user, pasting it in) wrote, so
-- both `question` and `answer` are third-party/agent text: they are stored as
-- data and shown as data, never parsed as markup (ADR-010).
--
-- A row is a **draft** while `approved_at` is NULL. `untrusted_origin` is the
-- ADR-010 marker and defaults to 1 — the run drafted this answer after reading a
-- form nobody in this app vouched for — so a write that forgets the column still
-- lands untrusted. Editing or confirming a row is what clears it: the user has
-- made the text theirs, and only approved rows are reused (`answer_bank_reusable`).
--
-- `needs_review` is the FR-80 flag: salary and eligibility answers come from the
-- user's Preferences and are always flagged, so they are never answered quietly.
-- `answer` is NULL when the core has no confident value (a salary question with
-- no Salary Floor set): the row waits for the user instead of the run inventing a
-- number. `fact_ids_json` names the profile facts a free-text answer stands on
-- (the no-fabrication rule); it is `[]` for the Preferences-backed classes, whose
-- value is the user's own setting rather than a claim about them.
CREATE TABLE application_answer_bank (
    id                TEXT PRIMARY KEY NOT NULL,
    application_id    TEXT NOT NULL REFERENCES application (id) ON DELETE CASCADE,
    question          TEXT NOT NULL,
    question_key      TEXT NOT NULL,
    answer            TEXT,
    class             TEXT NOT NULL CHECK (class IN ('salary', 'eligibility', 'other')),
    fact_ids_json     TEXT NOT NULL DEFAULT '[]',
    needs_review      INTEGER NOT NULL DEFAULT 0 CHECK (needs_review IN (0, 1)),
    approved_at       TEXT,
    untrusted_origin  INTEGER NOT NULL DEFAULT 1 CHECK (untrusted_origin IN (0, 1)),
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
) STRICT;

-- Reuse looks a question up by its key across every application (AC2). Partial,
-- because a draft is never reusable.
CREATE INDEX application_answer_bank_reuse_idx
    ON application_answer_bank (question_key) WHERE approved_at IS NOT NULL;

-- One application's bank, oldest first, for the studio.
CREATE INDEX application_answer_bank_application_idx
    ON application_answer_bank (application_id, created_at);
