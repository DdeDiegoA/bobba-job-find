-- 0019_jobs.sql — one row per job execution (S5.1, FR-41, FR-43, FR-47, AD-18).
-- Never edit once shipped (AD-20).
--
-- `UNIQUE (job, scheduled_for)` is the whole of AC1: the scheduler may ask for
-- the same slot on every tick, and the insert can only ever create one row.
-- `scheduled_for` is the slot the run belongs to (UTC RFC 3339, AD-17), not the
-- moment it started: a run that catches up after sleep keeps its original slot,
-- which is what makes catch-up idempotent too (AC2, FR-43).
--
-- `status` is `running` while the job executes, and terminal after:
-- `succeeded`, `failed`, or `skipped_locked`. The last one is ADR-009 §3: the
-- store was locked when the slot was due, so the run never happened and is never
-- recorded as a success. `detail` is diagnostic text (an error, the skip
-- reason), never user-facing prose.
--
-- Timestamps are UTC RFC 3339 text (AD-17).
CREATE TABLE job_run (
    id            TEXT PRIMARY KEY NOT NULL,
    job           TEXT NOT NULL CHECK (job IN ('daily_search', 'email_sync', 'rescore', 'retention_sweep')),
    scheduled_for TEXT NOT NULL,
    started_at    TEXT NOT NULL,
    finished_at   TEXT,
    status        TEXT NOT NULL CHECK (status IN ('running', 'succeeded', 'failed', 'skipped_locked')),
    detail        TEXT,
    UNIQUE (job, scheduled_for)
) STRICT;

-- The single-run lock (FR-47) looks for the one running row of a job; a partial
-- index keeps that lookup off the whole history.
CREATE INDEX job_run_running_idx ON job_run (job) WHERE status = 'running';

-- The scheduler plans each tick from the newest slot a job has a row for.
CREATE INDEX job_run_recent_idx ON job_run (job, scheduled_for DESC);
