-- 0008_postings.sql — Offers and their provenance (S4.2, FR-48, AD-17, AD-19).
-- Owner: ingestion (AD-16). Never edit once shipped (AD-20).
--
-- One `posting` is one Offer in the normalised shape every later stage reads
-- (ranking, tailoring, the tracker). Every source observation that produced or
-- re-saw it is a row in `posting_source`, so dedup merges Offers and never
-- deletes provenance (AD-19).
--
-- Conventions from AD-17: ids are ULIDs (`pst_…`), timestamps are UTC RFC 3339
-- text, money is `{amount_minor, currency, period}` (the three columns are all
-- set or all NULL — they are one value), and `lang` is a BCP-47 tag.
--
-- `company` is stored as text: the MVP slice plan has no company table. The
-- description is kept twice on purpose: `description_html` is the ammonia
-- output the UI may render (AD-7), `description_text` is what the scorer and
-- the S4.3 simhash read.
--
-- `dedup_key` / `simhash` are written by S4.3, which owns no migration of its
-- own; the columns and their index therefore live here (erD: architecture.md
-- §12). `requirements_json` is filled by S6.2's deterministic extraction.
CREATE TABLE posting (
    id                TEXT    PRIMARY KEY NOT NULL,
    source_id         TEXT    NOT NULL REFERENCES source (id) ON DELETE CASCADE,
    external_id       TEXT    NOT NULL,
    title             TEXT    NOT NULL,
    company           TEXT    NOT NULL,
    location_json     TEXT    NOT NULL DEFAULT '[]',
    work_mode         TEXT    NOT NULL CHECK (work_mode IN
                        ('remote', 'hybrid', 'onsite', 'unknown')),
    amount_minor      INTEGER,
    currency          TEXT,
    pay_period        TEXT CHECK (pay_period IS NULL OR pay_period IN
                        ('year', 'month', 'hour')),
    posted_at         TEXT,
    deadline          TEXT,
    first_seen        TEXT    NOT NULL,
    last_seen         TEXT    NOT NULL,
    lang              TEXT    NOT NULL,
    description_html  TEXT    NOT NULL DEFAULT '',
    description_text  TEXT    NOT NULL DEFAULT '',
    requirements_json TEXT    NOT NULL DEFAULT '[]',
    apply_url         TEXT    NOT NULL,
    apply_email       TEXT,
    dedup_key         TEXT    NOT NULL DEFAULT '',
    simhash           INTEGER,
    CHECK ((amount_minor IS NULL) = (currency IS NULL)
           AND (currency IS NULL) = (pay_period IS NULL))
) STRICT;

CREATE INDEX posting_dedup_key_idx ON posting (dedup_key);

-- One row per (posting, source, source id): the raw payload as the source sent
-- it, kept for auditing and for re-ranking (AD-19). Untrusted text (AD-7).
CREATE TABLE posting_source (
    posting_id  TEXT NOT NULL REFERENCES posting (id) ON DELETE CASCADE,
    source_id   TEXT NOT NULL REFERENCES source (id) ON DELETE CASCADE,
    external_id TEXT NOT NULL,
    url         TEXT NOT NULL,
    raw         TEXT NOT NULL,
    fetched_at  TEXT NOT NULL,
    PRIMARY KEY (posting_id, source_id, external_id)
) STRICT;

CREATE INDEX posting_source_url_idx ON posting_source (url);
