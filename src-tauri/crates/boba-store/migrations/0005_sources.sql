-- 0005_sources.sql — configured sources, run history and opt-ins (S4.1, AD-9). Never edit once shipped (AD-20).
--
-- Owner: sources (AD-16). `access_class` / `legal_tier` / `apply_capability` are
-- the manifest enums; their CHECK sets mirror `boba_domain::source`. A gated
-- source may run only when it has a row in `source_optin`; the registry enforces
-- the gate, this table records the user's acceptance and warning version.
CREATE TABLE source (
    id                TEXT    PRIMARY KEY NOT NULL,
    name              TEXT    NOT NULL,
    access_class      TEXT    NOT NULL CHECK (access_class IN
                        ('official_api', 'public_json', 'rss', 'html_scrape', 'session_browser', 'external_process')),
    legal_tier        TEXT    NOT NULL CHECK (legal_tier IN
                        ('safe', 'caution', 'prohibited_automation')),
    apply_capability  TEXT    NOT NULL CHECK (apply_capability IN
                        ('none', 'link_out', 'ats_form', 'sanctioned_api', 'email', 'session_form')),
    attribution       TEXT,
    min_interval_ms   INTEGER NOT NULL CHECK (min_interval_ms > 0),
    daily_cap         INTEGER CHECK (daily_cap IS NULL OR daily_cap > 0),
    enabled           INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    added_at          TEXT    NOT NULL,
    updated_at        TEXT    NOT NULL
) STRICT;

-- One opt-in per source (AD-9). Re-confirmation (30 days for prohibited_automation)
-- is enforced by a later slice; `accepted_at` is the latest acceptance.
CREATE TABLE source_optin (
    source_id       TEXT PRIMARY KEY NOT NULL REFERENCES source (id) ON DELETE CASCADE,
    warning_version TEXT NOT NULL,
    accepted_at     TEXT NOT NULL
) STRICT;

CREATE TABLE source_run (
    id             TEXT    PRIMARY KEY NOT NULL,
    source_id      TEXT    NOT NULL REFERENCES source (id) ON DELETE CASCADE,
    status         TEXT    NOT NULL CHECK (status IN ('running', 'ok', 'failed')),
    started_at     TEXT    NOT NULL,
    finished_at    TEXT,
    postings_found INTEGER NOT NULL DEFAULT 0,
    error          TEXT
) STRICT;

CREATE INDEX source_run_source_idx ON source_run (source_id, started_at);
