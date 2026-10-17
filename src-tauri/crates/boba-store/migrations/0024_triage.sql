-- 0024_triage.sql — what the user decided in the Inbox (S6.6, FR-54, FR-66,
-- FR-67). Never edit once shipped (AD-20).
--
-- Owner: triage (AD-16). Three small tables, one decision each:
--
-- `triage_dismissal` is the *reason* an Offer was dismissed (FR-66/FR-67). The
-- dismissal itself stays in `posting_lifecycle.dismissed_at` (S4.4), which is
-- what hides the Offer; this row is the signal FR-67 counts, and Undo removes
-- both. One reason per posting: dismissing again replaces it.
--
-- `triage_override` is a VETO the user overruled (FR-54). The ScoreCard is
-- never rewritten (S6.3 keeps cards immutable), so the override is its own row:
-- the Gate it overruled and when. The Inbox reads it to stop hiding the Offer;
-- the record means the override is never silent.
--
-- `triage_suggestion` is the user's answer to a Preferences change Boba
-- suggested (FR-67: "accept or ignore"). The id is the change
-- (`exclude_company:<name>`, `exclude_onsite`), so the same change is never
-- suggested twice after it was answered.
--
-- Timestamps are UTC RFC 3339 text (AD-17).
CREATE TABLE triage_dismissal (
    posting_id TEXT PRIMARY KEY NOT NULL REFERENCES posting (id) ON DELETE CASCADE,
    reason     TEXT NOT NULL CHECK (reason IN ('not_interested', 'not_eligible', 'salary', 'company', 'location', 'other')),
    created_at TEXT NOT NULL
) STRICT;

-- FR-67 counts by reason and groups by company: the count query reads this.
CREATE INDEX triage_dismissal_reason_idx ON triage_dismissal (reason, created_at);

CREATE TABLE triage_override (
    posting_id TEXT PRIMARY KEY NOT NULL REFERENCES posting (id) ON DELETE CASCADE,
    gate       TEXT NOT NULL CHECK (gate IN ('eligibility', 'language', 'location', 'compensation', 'deadline')),
    created_at TEXT NOT NULL
) STRICT;

CREATE TABLE triage_suggestion (
    suggestion_id TEXT PRIMARY KEY NOT NULL,
    status        TEXT NOT NULL CHECK (status IN ('accepted', 'ignored')),
    updated_at    TEXT NOT NULL
) STRICT;
