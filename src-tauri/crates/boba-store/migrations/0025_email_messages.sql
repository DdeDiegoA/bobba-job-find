-- 0025_email_messages.sql — the recruiter mail Boba is allowed to keep (S8.3,
-- FR-38, FR-104, FR-113). Never edit once shipped (AD-20).
--
-- Owner: email (AD-16). In read mode only messages that matched the scope are
-- stored: a sender on an applied-company domain, a known ATS sender, or a reply
-- in a sent thread (FR-38). Everything else is discarded after matching and
-- never reaches disk. A manual `.eml` import (FR-104) enters the same pipeline,
-- so its row differs only in `origin` (and in having no `account_id`: it was
-- never fetched from a mailbox).
--
-- `provider_message_id` is the id the source assigned (a Graph/IMAP id, or the
-- Message-ID), and for a manual import the Message-ID or a hash of the file, so
-- importing the same `.eml` twice is idempotent. Disconnecting a mailbox deletes
-- its mail (FR-39); a local import has no connection to cascade from.
--
-- `body_text` holds text/plain only, never HTML (AD-7: untrusted mail text is
-- stored as plain text). `received_at` is the message's Date header as UTC
-- RFC 3339 text (AD-17); the retention sweep (FR-113, default 180 days) reads
-- it, and `in_reply_to`/`references_header` are what a later slice matches a
-- reply to its thread with.
CREATE TABLE email_message (
    id                  TEXT PRIMARY KEY NOT NULL,
    account_id          TEXT REFERENCES email_account (id) ON DELETE CASCADE,
    provider_message_id TEXT NOT NULL,
    message_id          TEXT,
    in_reply_to         TEXT NOT NULL DEFAULT '',
    references_header   TEXT NOT NULL DEFAULT '',
    from_address        TEXT NOT NULL,
    from_name           TEXT,
    to_addresses        TEXT NOT NULL DEFAULT '',
    subject             TEXT NOT NULL DEFAULT '',
    received_at         TEXT NOT NULL,
    body_text           TEXT NOT NULL,
    has_attachments     INTEGER NOT NULL DEFAULT 0 CHECK (has_attachments IN (0, 1)),
    matched_by          TEXT NOT NULL CHECK (matched_by IN ('company_domain', 'ats_sender', 'sent_thread')),
    origin              TEXT NOT NULL CHECK (origin IN ('sync', 'eml')),
    imported_at         TEXT NOT NULL,
    -- Only a fetched message has a connection; only an import may have none.
    CHECK ((origin = 'sync') = (account_id IS NOT NULL)),
    -- One row per message per connection: an overlapping incremental window
    -- re-delivers the same message and must not store it twice.
    UNIQUE (account_id, provider_message_id)
) STRICT;

-- The same message imported on its own twice stays one row. (`UNIQUE` treats
-- NULLs as distinct, so the connection-less case needs its own index.)
CREATE UNIQUE INDEX email_message_local_idx ON email_message (provider_message_id)
    WHERE account_id IS NULL;

-- The retention sweep (FR-113) counts and purges by age, across connections.
CREATE INDEX email_message_received_idx ON email_message (received_at);

-- Purging one connection (FR-39) and reading its thread history.
CREATE INDEX email_message_account_idx ON email_message (account_id, received_at);
