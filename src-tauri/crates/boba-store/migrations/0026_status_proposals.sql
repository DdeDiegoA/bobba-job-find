-- 0026_status_proposals.sql — what Boba read in a recruiter reply and what it
-- proposes to do about it (S8.4, FR-100..FR-103, ADR-010). Never edit once
-- shipped (AD-20).
--
-- Owner: status proposals (AD-16). Two tables:
--
-- `email_classification` is the verdict on one stored `email_message` (AC1):
-- the class, the confidence, the line it was read from and which classifier
-- produced it. `untrusted_origin` follows ADR-010: deterministic rules quote
-- the message's own words and carry the message's taint (0); an LLM read the
-- text and its quote can never be trusted further than plain text (1). The
-- CHECK is the invariant, not a caller's habit: an LLM row is always tainted.
--
-- `status_proposal` is the offer to move an Application (AC2). `application_id`
-- is NULL when the message did not name one unambiguously: the user picks, and
-- approval writes the selected id. `from_status` is filled when the proposal is
-- approved (the status the card had then); `to_status` is fixed at proposal
-- time. `state` records what happened, so an automatically applied change
-- (AC5) can be undone later. `UNIQUE (message_id)` keeps one message from
-- producing a second proposal: dismissing one is an answer, not a request for
-- another.
--
-- AC6: `untrusted_origin` is here from the start, NOT NULL with a default of 1,
-- so a writer that forgets to set it taints the row rather than trusting it,
-- and the quoted line is only ever rendered as plain text (ADR-010).
--
-- Timestamps are UTC RFC 3339 text (AD-17).
CREATE TABLE email_classification (
    message_id       TEXT    PRIMARY KEY NOT NULL
                             REFERENCES email_message (id) ON DELETE CASCADE,
    class            TEXT    NOT NULL CHECK (class IN
                         ('acknowledgement', 'rejection', 'interview_invite',
                          'assessment', 'offer', 'request_for_info', 'other')),
    confidence       REAL    NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
    quoted_line      TEXT    NOT NULL DEFAULT '',
    method           TEXT    NOT NULL CHECK (method IN ('deterministic', 'llm')),
    untrusted_origin INTEGER NOT NULL DEFAULT 1 CHECK (untrusted_origin IN (0, 1)),
    classified_at    TEXT    NOT NULL,
    -- ADR-010: an LLM classification is always untrusted text. A deterministic
    -- rule quotes the message itself and is stored untainted.
    CHECK (method != 'llm' OR untrusted_origin = 1),
    CHECK (method != 'deterministic' OR untrusted_origin = 0)
) STRICT;

CREATE TABLE status_proposal (
    id               TEXT    PRIMARY KEY NOT NULL,
    message_id       TEXT    NOT NULL UNIQUE
                             REFERENCES email_message (id) ON DELETE CASCADE,
    application_id   TEXT    REFERENCES application (id) ON DELETE SET NULL,
    from_status      TEXT    CHECK (from_status IS NULL OR from_status IN
                         ('saved', 'drafted', 'applied', 'interview', 'offer',
                          'hired', 'rejected', 'no_response', 'offer_declined',
                          'withdrawn')),
    to_status        TEXT    NOT NULL CHECK (to_status IN
                         ('saved', 'drafted', 'applied', 'interview', 'offer',
                          'hired', 'rejected', 'no_response', 'offer_declined',
                          'withdrawn')),
    class            TEXT    NOT NULL CHECK (class IN
                         ('acknowledgement', 'rejection', 'interview_invite',
                          'assessment', 'offer', 'request_for_info', 'other')),
    confidence       REAL    NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
    quoted_line      TEXT    NOT NULL DEFAULT '',
    untrusted_origin INTEGER NOT NULL DEFAULT 1 CHECK (untrusted_origin IN (0, 1)),
    state            TEXT    NOT NULL CHECK (state IN
                         ('pending', 'approved', 'dismissed', 'auto_applied',
                          'undone')),
    created_at       TEXT    NOT NULL,
    decided_at       TEXT
) STRICT;

-- Today's needs-you queue reads the pending rows, newest first.
CREATE INDEX status_proposal_state_idx ON status_proposal (state, created_at);

-- A card's history and an undo look up what was proposed for one application.
CREATE INDEX status_proposal_application_idx ON status_proposal (application_id, created_at);
