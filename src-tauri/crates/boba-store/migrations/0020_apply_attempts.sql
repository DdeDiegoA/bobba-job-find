-- 0020_apply_attempts.sql — one row per send attempt (S9.3, AC2, AD-12).
-- Never edit once shipped (AD-20).
--
-- Owner: apply (AD-16). The row is written **before** the message leaves and the
-- next send reads it, which is what keeps a network failure from mailing the
-- same application twice: `sent` refuses a resend, `pending` (the process died
-- mid-send) asks the user to check, `uncertain` (transport, no answer) resends
-- only with the user's word, and `failed` (provider refused) is free to retry.
--
-- The message body and the attachment bytes are never stored (AD-12): the row
-- keeps the recipient and the subject as metadata, plus the payload's blake3,
-- which is also what the audit `intent` row names. `message_id` is Boba's id
-- for the message: Microsoft Graph's `sendMail` answers 202 with no body, so
-- Boba assigns one when the attempt is recorded and the audit `outcome` names
-- it.
CREATE TABLE apply_attempt (
    id             TEXT PRIMARY KEY NOT NULL,
    application_id TEXT NOT NULL REFERENCES application (id) ON DELETE CASCADE,
    channel        TEXT NOT NULL CHECK (channel IN ('email')),
    account_id     TEXT NOT NULL,
    recipient      TEXT NOT NULL,
    subject        TEXT NOT NULL,
    payload_blake3 TEXT NOT NULL,
    message_id     TEXT,
    state          TEXT NOT NULL CHECK (state IN ('pending', 'sent', 'failed', 'uncertain')),
    -- Provider class only (`unreachable`, `rejected`, …): never a body or a token.
    error          TEXT,
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL,
    sent_at        TEXT
) STRICT;

-- The evidence read is always "every attempt of this application".
CREATE INDEX apply_attempt_application_idx ON apply_attempt (application_id, created_at);

-- AC2 at the database: one application never has two live attempts, so two
-- racing clicks cannot both reach the provider — the second insert fails before
-- anything is sent. `failed` and `uncertain` rows stay out of the index, which
-- is what lets a retry (or a user-confirmed resend) write a new attempt.
CREATE UNIQUE INDEX apply_attempt_one_live_idx ON apply_attempt (application_id)
    WHERE state IN ('pending', 'sent');
