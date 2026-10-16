-- 0009_email_accounts.sql — connected mailboxes (S8.1, AD-13). Never edit once shipped (AD-20).
--
-- Owner: email (AD-16). One row per connected mailbox. The table holds
-- connection metadata only: the delegated scopes that were granted, the chosen
-- mode and an opaque `secret_ref` naming the keychain entry that holds the
-- tokens. Tokens, refresh tokens and app passwords live in the OS keychain and
-- never in this table (AD-13, FR-40), so a DB export carries no credential.
--
-- `client_id` is the public OAuth client id the tokens belong to (a desktop
-- public client has no secret, §11); it is kept so a later refresh uses the same
-- client. `scopes` is a space-separated list as granted by the provider; it must
-- cover `mode.required_scopes()` (NFR-5: scopes follow the mode), so switching
-- from send-only to read+send re-runs the flow when `Mail.Read` is missing.
CREATE TABLE email_account (
    id             TEXT PRIMARY KEY NOT NULL,
    provider       TEXT NOT NULL CHECK (provider IN ('microsoft_graph', 'gmail', 'imap_smtp')),
    address        TEXT NOT NULL,
    mode           TEXT NOT NULL CHECK (mode IN ('send_only', 'read_send')),
    client_id      TEXT NOT NULL,
    client_id_kind TEXT NOT NULL CHECK (client_id_kind IN ('bundled', 'byo')),
    scopes         TEXT NOT NULL,
    secret_ref     TEXT NOT NULL,
    connected_at   TEXT NOT NULL,
    updated_at     TEXT NOT NULL,
    -- One connection per mailbox: reconnecting replaces the row (AC1).
    UNIQUE (provider, address)
) STRICT;
