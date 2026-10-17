-- 0021_narration.sql — Explanation narration (S6.5, FR-59, AD-5, ADR-010).
-- Owner: scoring (AD-16). Never edit once shipped (AD-20).
--
-- One row is one agent-written explanation of one ScoreCard in one UI language
-- (architecture §12). It is bound to `score_card_id`, not to the Offer alone: a
-- re-score writes a new card, and what the user was told stays attached to the
-- numbers it was checked against (AD-8, FR-61). A re-run in the same language
-- writes a new row; the newest one is what the detail screen shows, and the
-- older ones stay readable.
--
-- `untrusted_origin` is `1` for every row this app writes: the narration is
-- derived from third-party posting text, so the UI renders it as plain text and
-- never as markup (ADR-010). The column starts here rather than being added
-- later, so no narration can ever exist without the rule that governs it.
CREATE TABLE score_narration (
    id               TEXT    PRIMARY KEY,
    posting_id       TEXT    NOT NULL REFERENCES posting (id) ON DELETE CASCADE,
    score_card_id    TEXT    NOT NULL REFERENCES score_card (id) ON DELETE CASCADE,
    lang             TEXT    NOT NULL CHECK (length(lang) > 0),
    text             TEXT    NOT NULL,
    untrusted_origin INTEGER NOT NULL DEFAULT 1
                         CHECK (untrusted_origin IN (0, 1)),
    created_at       TEXT    NOT NULL
) STRICT;

-- `latest_narration`: the newest text for one Offer in one language.
CREATE INDEX score_narration_latest_idx
    ON score_narration (posting_id, lang, created_at DESC, id DESC);
