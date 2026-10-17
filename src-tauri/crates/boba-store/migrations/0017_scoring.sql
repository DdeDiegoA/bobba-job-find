-- 0017_scoring.sql — ScoreCards (S6.3, FR-55..57, FR-61, FR-14, AD-8).
-- Owner: scoring (AD-16). Never edit once shipped (AD-20).
--
-- One row is one Offer scored against one set of inputs:
-- `(posting, profile version, prefs version, model version)` is the identity
-- (architecture §8), so a change to the Profile (FR-13), to the Preferences
-- (FR-14) or to the scoring model does not rewrite what the user was shown — it
-- adds a row. The UNIQUE constraint below **is** that rule: the writer inserts
-- and never updates, and a second card for the same key is an error rather than
-- an overwrite (AC3).
--
-- `rank`, `fit`, `quality` are the two axes and the number they combine into
-- (0–100). `fit` and `quality` are nullable on purpose: a factor's input can be
-- missing (no embeddings, no salary benchmark), and a missing axis is stored as
-- NULL — never as a zero, which would read as "terrible" instead of "unknown"
-- (AC2, AD-8).
--
-- `factors_json` is the eleven factors as `boba_domain::scoring::factors::
-- FactorScore` serialises them: `{factor, score, weight, effective_weight,
-- matched, missing, evidence, confidence}` per factor, in F1..Q5 order. It is
-- written once with the row, so a card reads back byte-identical (FR-61).
-- `gates_json` is the GateReport that preceded the score (FR-53), so the Offer
-- detail shows one consistent picture.
--
-- `score_narration` and `market_benchmark` (this area's other tables, spine §12)
-- land with the slices that write them: S6.5's narration run and the benchmark
-- importer. Neither needs a migration from here.
CREATE TABLE score_card (
    id              TEXT    PRIMARY KEY,
    posting_id      TEXT    NOT NULL REFERENCES posting (id) ON DELETE CASCADE,
    profile_version INTEGER NOT NULL,
    prefs_version   INTEGER NOT NULL,
    model_version   INTEGER NOT NULL,
    rank            REAL    NOT NULL,
    fit             REAL,
    quality         REAL,
    verdict         TEXT    NOT NULL CHECK (verdict IN
                        ('strong', 'good', 'moderate', 'weak', 'poor')),
    confidence      TEXT    NOT NULL CHECK (confidence IN ('low', 'medium', 'high')),
    factors_json    TEXT    NOT NULL,
    gates_json      TEXT    NOT NULL,
    created_at      TEXT    NOT NULL,
    -- AC3: one card per input triple, and nothing ever rewrites one.
    UNIQUE (posting_id, profile_version, prefs_version, model_version)
) STRICT;

-- The immutability rule, enforced by the database rather than trusted to the
-- writer: no UPDATE reaches a stored card. Deleting an Offer still cascades.
CREATE TRIGGER score_card_immutable BEFORE UPDATE ON score_card
BEGIN
    SELECT RAISE(ABORT, 'score_card rows are immutable');
END;

-- `latest_card`: the newest card for one Offer.
CREATE INDEX score_card_latest_idx ON score_card (posting_id, created_at DESC, id DESC);
