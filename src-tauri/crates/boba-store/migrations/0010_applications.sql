-- 0010_applications.sql — Applications and their status history (S9.1, FR-93/94,
-- AD-16, AD-17). Never edit once shipped (AD-20).
--
-- Owner: apply (AD-16). One `application` is the user's pursuit of one Offer;
-- the Tracker board is a projection of `status`. The stored vocabulary is the
-- upstream one plus `saved` (the CSV export leaves `saved` rows out, the JSON
-- export includes them), and the CHECK below is what keeps it in one place: the
-- UI derives its columns from the status, never the other way round (AC1).
--
-- `offer_snapshot_json` freezes the Offer as it was when the application was
-- created, so a card keeps showing what was applied to after the posting is
-- refreshed or expires (FR-94). `notes` and `next_action` are the user's
-- annotations on that card. Timestamps are UTC RFC 3339 text (AD-17).
CREATE TABLE application (
    id                  TEXT PRIMARY KEY NOT NULL,
    posting_id          TEXT NOT NULL REFERENCES posting (id) ON DELETE CASCADE,
    status              TEXT NOT NULL CHECK (status IN
                          ('saved', 'drafted', 'applied', 'interview', 'offer',
                           'hired', 'rejected', 'no_response', 'offer_declined',
                           'withdrawn')),
    offer_snapshot_json TEXT NOT NULL,
    notes               TEXT,
    next_action         TEXT,
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL
) STRICT;

CREATE INDEX application_status_idx ON application (status, created_at);

-- One row per status change (AC2/AC4): who moved the card, from where to where,
-- and why. `from_status` is NULL only for the row that records how the
-- application was born. The table is append-only: a correction is another move,
-- which is what makes Undo and the timeline honest (AC3).
CREATE TABLE status_event (
    id             TEXT PRIMARY KEY NOT NULL,
    application_id TEXT NOT NULL REFERENCES application (id) ON DELETE CASCADE,
    from_status    TEXT CHECK (from_status IS NULL OR from_status IN
                       ('saved', 'drafted', 'applied', 'interview', 'offer',
                        'hired', 'rejected', 'no_response', 'offer_declined',
                        'withdrawn')),
    to_status      TEXT NOT NULL CHECK (to_status IN
                       ('saved', 'drafted', 'applied', 'interview', 'offer',
                        'hired', 'rejected', 'no_response', 'offer_declined',
                        'withdrawn')),
    source         TEXT NOT NULL CHECK (source IN
                       ('user', 'status_proposal', 'apply_outcome', 'import')),
    reason         TEXT,
    created_at     TEXT NOT NULL,
    CHECK (from_status IS NULL OR from_status <> to_status)
) STRICT;

CREATE INDEX status_event_application_idx ON status_event (application_id, created_at, id);
