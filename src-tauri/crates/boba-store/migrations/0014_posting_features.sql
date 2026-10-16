-- 0014_posting_features.sql — an Offer's features, per Profile version (S6.2,
-- FR-55/FR-56 inputs, FR-61, AD-5, ADR-010). Owner: ingestion (AD-16).
-- Never edit once shipped (AD-20).
--
-- The scorer reads one `PostingFeatures` set per (posting, profile.version)
-- (FR-61): the deterministic lexicon pass always runs, and an `extract_postings`
-- agent run may add claims of its own. Every row therefore carries its
-- `source` (`det` | `llm`) and how sure the core is of it (`confidence`), so the
-- UI can say "refined by AI" and a later re-score can tell the two apart.
--
-- `value` is stored as the claim reads: a skill as the posting wrote it, a
-- seniority / language tag / money value in its canonical form. The primary key
-- is the claim's identity — `(posting, profile version, kind, value, source)` —
-- so re-running an extraction upserts instead of duplicating (FR-61: the same
-- inputs give the same features).
--
-- Evidence is one of two shapes, and both are stored:
--   * `quoted` — the sentence of `posting.description_text` the claim was read
--     from, with byte offsets into that text (the same `QuotedSpan` the Gates
--     use, FR-53);
--   * `field` — a structured Offer field name (`money`, `title`), for values
--     that have no span in the prose.
--
-- `untrusted_origin` is ADR-010's flag and defaults to 1 (untrusted): a quoted
-- span is third-party text verbatim, and a claim an agent proposed is that run's
-- `input_trust` (a posting run is untrusted). The UI renders `evidence_text` and
-- every `llm` value as plain or sanitised data only — never as markup.
--
-- `posting_features_run` is the cache marker, and it is what makes an *empty*
-- feature set a real answer: a posting whose extraction found nothing still has
-- a row here, so it is neither "pending" nor re-read on every run. `agent_done`
-- says whether an agent pass has been served for this version (a passed run with
-- no proposal counts; a failed run leaves it at 0 so a later run retries).
CREATE TABLE posting_features (
    posting_id       TEXT    NOT NULL REFERENCES posting (id) ON DELETE CASCADE,
    profile_version  INTEGER NOT NULL,
    kind             TEXT    NOT NULL CHECK (kind IN
                        ('skill', 'seniority', 'education', 'language',
                         'compensation', 'red_flag', 'note')),
    value            TEXT    NOT NULL,
    source           TEXT    NOT NULL CHECK (source IN ('det', 'llm')),
    confidence       TEXT    NOT NULL CHECK (confidence IN
                        ('low', 'medium', 'high')),
    evidence_kind    TEXT    NOT NULL CHECK (evidence_kind IN ('quoted', 'field')),
    evidence_text    TEXT    NOT NULL DEFAULT '',
    evidence_start   INTEGER,
    evidence_end     INTEGER,
    untrusted_origin INTEGER NOT NULL DEFAULT 1
                        CHECK (untrusted_origin IN (0, 1)),
    PRIMARY KEY (posting_id, profile_version, kind, value, source),
    -- A quote has offsets into the posting text; a field name has none.
    CHECK ((evidence_kind = 'quoted') = (evidence_start IS NOT NULL)),
    CHECK ((evidence_kind = 'quoted') = (evidence_end IS NOT NULL))
) STRICT;

-- The cache marker and the run queue: one row per (posting, profile version)
-- whose deterministic pass has run.
CREATE TABLE posting_features_run (
    posting_id      TEXT    NOT NULL REFERENCES posting (id) ON DELETE CASCADE,
    profile_version INTEGER NOT NULL,
    agent_done      INTEGER NOT NULL DEFAULT 0 CHECK (agent_done IN (0, 1)),
    extracted_at    TEXT    NOT NULL,
    PRIMARY KEY (posting_id, profile_version)
) STRICT;

-- `get_pending_postings`: what still deserves an agent pass, newest first.
CREATE INDEX posting_features_pending_idx
    ON posting_features_run (profile_version, agent_done, posting_id);
