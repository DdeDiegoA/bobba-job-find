-- 0018_posting_lifecycle.sql — a posting's seen, repost, staleness and expiry
-- state (S4.4, FR-50, FR-51, FR-52). Never edit once shipped (AD-20).
--
-- Owner: ingestion (AD-16). One row per posting, opened in the same transaction
-- that inserts the posting (so the staleness check runs on the first
-- observation) and refreshed whenever a source sees the posting again.
--
-- `content_hash` + `description_simhash` are the fingerprint of the version the
-- user was last told about: they move when the user sees or dismisses the
-- posting, not when a board merely re-sends it (FR-50). `missing_runs` counts
-- consecutive search runs that did not see the posting and is what makes the
-- second miss *Possibly closed*; `disappeared_at` is when that streak began,
-- and a sighting inside 30 days of it records `repost_at` (FR-51). `stale_at`
-- records a posting over 90 days old. `possibly_closed_at` is set when a
-- posting is missing for two runs or past its deadline (FR-52); the posting is
-- never deleted and Applications saved on it stay readable.
--
-- Timestamps are UTC RFC 3339 text (AD-17).
CREATE TABLE posting_lifecycle (
    posting_id         TEXT    PRIMARY KEY NOT NULL REFERENCES posting (id) ON DELETE CASCADE,
    content_hash       TEXT    NOT NULL DEFAULT '',
    description_simhash INTEGER,
    seen_at            TEXT,
    dismissed_at       TEXT,
    missing_runs       INTEGER NOT NULL DEFAULT 0 CHECK (missing_runs >= 0),
    disappeared_at     TEXT,
    repost_at          TEXT,
    stale_at           TEXT,
    possibly_closed_at TEXT,
    updated_at         TEXT    NOT NULL
) STRICT;

-- The tracker asks for the *Possibly closed* postings with saved Applications
-- (FR-52); that is a small set, but it is read on every inbox load.
CREATE INDEX posting_lifecycle_closed_idx ON posting_lifecycle (possibly_closed_at)
    WHERE possibly_closed_at IS NOT NULL;

-- Postings stored before this migration get a row now: an empty fingerprint
-- (the user has not been shown this version under this rule yet) and the
-- simhash 0008 already computed, so the next notification is judged fairly.
INSERT INTO posting_lifecycle (posting_id, content_hash, description_simhash, updated_at)
SELECT id, '', simhash, last_seen FROM posting;
