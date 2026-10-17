-- 0027_source_auto.sql — the Session Board Auto opt-in and the challenge pause
-- (S11.3, FR-91, AD-11). Never edit once shipped (AD-20).
--
-- Auto on a Prohibited-automation Source is a **separate** opt-in from the
-- reading one (S4.1/AD-9): the user types the Source name after the ToS/ban
-- warning, and the row records both the name and the warning version, so a
-- changed warning invalidates every stored acceptance. A lapsed acceptance
-- (30 days) falls back to Review in the core.
--
-- `source.paused_until` is the deadline a CAPTCHA, login wall, "unusual
-- activity" warning or rate limit sets: seven days. It lives on the Source row
-- the policy reads, so closing the UI cannot shorten the pause. NULL means no
-- pause has ever been set.
CREATE TABLE source_auto_optin (
    source_id       TEXT PRIMARY KEY NOT NULL REFERENCES source (id) ON DELETE CASCADE,
    confirmed_name  TEXT NOT NULL,
    warning_version TEXT NOT NULL,
    accepted_at     TEXT NOT NULL
) STRICT;

ALTER TABLE source ADD COLUMN paused_until TEXT;
