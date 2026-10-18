-- 0028_job_run_cancelled.sql — a cancelled run is its own terminal status
-- (FX15). Never edit once shipped (AD-20).
--
-- `cancelled` is not `succeeded`: the run stopped between Sources because the
-- user asked it to, and its report keeps the Sources that did finish. SQLite
-- cannot alter a CHECK constraint, so the table is rebuilt with every row
-- copied and both indexes recreated (the old ones drop with the old table).
CREATE TABLE job_run_rebuilt (
    id            TEXT PRIMARY KEY NOT NULL,
    job           TEXT NOT NULL CHECK (job IN ('daily_search', 'email_sync', 'rescore', 'retention_sweep')),
    scheduled_for TEXT NOT NULL,
    started_at    TEXT NOT NULL,
    finished_at   TEXT,
    status        TEXT NOT NULL CHECK (status IN ('running', 'succeeded', 'failed', 'skipped_locked', 'cancelled')),
    detail        TEXT,
    UNIQUE (job, scheduled_for)
) STRICT;

INSERT INTO job_run_rebuilt (id, job, scheduled_for, started_at, finished_at, status, detail)
SELECT id, job, scheduled_for, started_at, finished_at, status, detail FROM job_run;

DROP TABLE job_run;
ALTER TABLE job_run_rebuilt RENAME TO job_run;

CREATE INDEX job_run_running_idx ON job_run (job) WHERE status = 'running';
CREATE INDEX job_run_recent_idx ON job_run (job, scheduled_for DESC);
