//! Jobs repository (S5.1, AD-18): the single writer of `job_run`. The adapter is
//! `boba-store`; use cases depend on [`JobsRepo`], never on this module.
//!
//! Every write runs on the writer actor in one transaction, so the two decisions
//! the table enforces — one row per `(job, scheduled_for)` (AC1) and one running
//! run per job (FR-47) — cannot race a second scheduler tick or a manual run.

use rusqlite::{Connection, OptionalExtension, params};

use boba_app::jobs::{JobKind, JobRun, JobsRepo, JobsRepoError, OpenOutcome, RunStatus};

use crate::db::{Store, StoreError};

const SELECT: &str = "SELECT id, job, scheduled_for, started_at, finished_at, status, detail \
     FROM job_run";

/// A fresh run id (AD-17: ids are ULIDs, minted by the owning area).
pub fn new_run_id() -> String {
    ulid::Ulid::generate().to_string()
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<JobRun> {
    let job: String = r.get(1)?;
    let status: String = r.get(5)?;
    Ok(JobRun {
        id: r.get(0)?,
        job: JobKind::parse(&job).ok_or_else(|| bad("job", job))?,
        scheduled_for: r.get(2)?,
        started_at: r.get(3)?,
        finished_at: r.get(4)?,
        status: RunStatus::parse(&status).ok_or_else(|| bad("status", status))?,
        detail: r.get(6)?,
    })
}

fn bad(column: &'static str, value: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        format!("unknown {column} '{value}' in job_run").into(),
    )
}

fn get(conn: &Connection, job: JobKind, scheduled_for: &str) -> Result<Option<JobRun>, StoreError> {
    Ok(conn
        .query_row(
            &format!("{SELECT} WHERE job = ?1 AND scheduled_for = ?2"),
            params![job.as_str(), scheduled_for],
            row,
        )
        .optional()?)
}

fn running(conn: &Connection, job: JobKind) -> Result<Option<JobRun>, StoreError> {
    Ok(conn
        .query_row(
            &format!("{SELECT} WHERE job = ?1 AND status = 'running' ORDER BY scheduled_for DESC"),
            params![job.as_str()],
            row,
        )
        .optional()?)
}

fn last(conn: &Connection, job: JobKind) -> Result<Option<JobRun>, StoreError> {
    Ok(conn
        .query_row(
            &format!("{SELECT} WHERE job = ?1 ORDER BY scheduled_for DESC LIMIT 1"),
            params![job.as_str()],
            row,
        )
        .optional()?)
}

fn open(
    conn: &mut Connection,
    job: JobKind,
    scheduled_for: &str,
    started_at: &str,
) -> Result<OpenOutcome, StoreError> {
    let tx = conn.transaction()?;
    // The single-run lock (FR-47): when a run is executing, no second row is
    // created for this slot — it stays due for a later tick (a manual "Run now"
    // joins that run instead; see `Dispatcher::run_now`).
    if let Some(run) = running(&tx, job)? {
        return Ok(OpenOutcome::Busy(run));
    }
    let id = new_run_id();
    let inserted = tx.execute(
        "INSERT INTO job_run (id, job, scheduled_for, started_at, status) \
         VALUES (?1, ?2, ?3, ?4, 'running') \
         ON CONFLICT(job, scheduled_for) DO NOTHING",
        params![id, job.as_str(), scheduled_for, started_at],
    )?;
    let run = get(&tx, job, scheduled_for)?.ok_or_else(|| {
        StoreError::Sqlite("job_run row vanished inside its own transaction".into())
    })?;
    tx.commit()?;
    Ok(if inserted == 1 {
        OpenOutcome::Started(run)
    } else {
        OpenOutcome::Joined(run)
    })
}

fn skip_locked(
    conn: &Connection,
    job: JobKind,
    scheduled_for: &str,
    at: &str,
) -> Result<Option<JobRun>, StoreError> {
    let inserted = conn.execute(
        "INSERT INTO job_run (id, job, scheduled_for, started_at, finished_at, status) \
         VALUES (?1, ?2, ?3, ?4, ?4, 'skipped_locked') \
         ON CONFLICT(job, scheduled_for) DO NOTHING",
        params![new_run_id(), job.as_str(), scheduled_for, at],
    )?;
    if inserted == 0 {
        return Ok(None);
    }
    get(conn, job, scheduled_for)
}

fn finish(
    conn: &Connection,
    id: &str,
    finished_at: &str,
    status: RunStatus,
    detail: Option<&str>,
) -> Result<JobRun, StoreError> {
    if status == RunStatus::Running {
        return Err(StoreError::Sqlite("a run cannot finish as running".into()));
    }
    let updated = conn.execute(
        "UPDATE job_run SET status = ?2, finished_at = ?3, detail = ?4 \
         WHERE id = ?1 AND status = 'running'",
        params![id, status.as_str(), finished_at, detail],
    )?;
    if updated == 0 {
        return Err(StoreError::Sqlite(format!(
            "no running job_run row '{id}' to finish"
        )));
    }
    conn.query_row(&format!("{SELECT} WHERE id = ?1"), params![id], row)
        .map_err(StoreError::from)
}

fn interrupt(conn: &Connection, at: &str, detail: &str) -> Result<u64, StoreError> {
    let updated = conn.execute(
        "UPDATE job_run SET status = 'failed', finished_at = ?1, detail = ?2 \
         WHERE status = 'running'",
        params![at, detail],
    )?;
    Ok(updated as u64)
}

fn map_err(error: StoreError) -> JobsRepoError {
    JobsRepoError::new(error.to_string())
}

impl JobsRepo for Store {
    fn open_run(
        &self,
        job: JobKind,
        scheduled_for: &str,
        started_at: &str,
    ) -> Result<OpenOutcome, JobsRepoError> {
        let scheduled_for = scheduled_for.to_string();
        let started_at = started_at.to_string();
        self.execute(move |conn| open(conn, job, &scheduled_for, &started_at))
            .map_err(map_err)
    }

    fn record_skipped_locked(
        &self,
        job: JobKind,
        scheduled_for: &str,
        at: &str,
    ) -> Result<Option<JobRun>, JobsRepoError> {
        let scheduled_for = scheduled_for.to_string();
        let at = at.to_string();
        self.execute(move |conn| skip_locked(conn, job, &scheduled_for, &at))
            .map_err(map_err)
    }

    fn finish_run(
        &self,
        id: &str,
        finished_at: &str,
        status: RunStatus,
        detail: Option<&str>,
    ) -> Result<JobRun, JobsRepoError> {
        let id = id.to_string();
        let finished_at = finished_at.to_string();
        let detail = detail.map(str::to_string);
        self.execute(move |conn| finish(conn, &id, &finished_at, status, detail.as_deref()))
            .map_err(map_err)
    }

    fn get_run(&self, job: JobKind, scheduled_for: &str) -> Result<Option<JobRun>, JobsRepoError> {
        let scheduled_for = scheduled_for.to_string();
        self.read(move |conn| get(conn, job, &scheduled_for))
            .map_err(map_err)
    }

    fn running_run(&self, job: JobKind) -> Result<Option<JobRun>, JobsRepoError> {
        self.read(move |conn| running(conn, job)).map_err(map_err)
    }

    fn last_run(&self, job: JobKind) -> Result<Option<JobRun>, JobsRepoError> {
        self.read(move |conn| last(conn, job)).map_err(map_err)
    }

    fn mark_interrupted(&self, at: &str, detail: &str) -> Result<u64, JobsRepoError> {
        let at = at.to_string();
        let detail = detail.to_string();
        self.execute(move |conn| interrupt(conn, &at, &detail))
            .map_err(map_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate;

    fn connection() -> Connection {
        let mut conn = Connection::open_in_memory().expect("in-memory sqlite");
        migrate::embedded()
            .to_latest(&mut conn)
            .expect("apply migrations");
        conn
    }

    const SLOT: &str = "2026-10-16T08:00:00Z";
    const NOW: &str = "2026-10-16T08:00:01Z";

    #[test]
    fn migration_creates_the_job_run_table() {
        let conn = connection();
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name = 'job_run'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn opening_the_same_slot_twice_leaves_one_row() {
        let mut conn = connection();
        let OpenOutcome::Started(first) = open(&mut conn, JobKind::DailySearch, SLOT, NOW).unwrap()
        else {
            panic!("the first run starts");
        };
        finish(&conn, &first.id, NOW, RunStatus::Succeeded, None).unwrap();

        let second = open(&mut conn, JobKind::DailySearch, SLOT, NOW).unwrap();

        assert!(
            matches!(second, OpenOutcome::Joined(_)),
            "the slot already has its row (AC1): {second:?}"
        );
        assert_eq!(one_row_count(&conn), 1);
    }

    #[test]
    fn different_jobs_get_their_own_row_for_the_same_slot() {
        let mut conn = connection();
        open(&mut conn, JobKind::DailySearch, SLOT, NOW).unwrap();
        open(&mut conn, JobKind::EmailSync, SLOT, NOW).unwrap();

        assert_eq!(one_row_count(&conn), 2);
        assert!(get(&conn, JobKind::DailySearch, SLOT).unwrap().is_some());
        assert!(get(&conn, JobKind::EmailSync, SLOT).unwrap().is_some());
    }

    #[test]
    fn a_second_run_while_one_is_executing_gets_busy() {
        let mut conn = connection();
        let OpenOutcome::Started(first) = open(&mut conn, JobKind::DailySearch, SLOT, NOW).unwrap()
        else {
            panic!("the first run starts");
        };

        // Another slot arrives while the 08:00 run is still executing (FR-47).
        let busy = open(
            &mut conn,
            JobKind::DailySearch,
            "2026-10-16T08:01:00Z",
            "2026-10-16T08:01:00Z",
        )
        .unwrap();

        match busy {
            OpenOutcome::Busy(run) => assert_eq!(run.id, first.id),
            other => panic!("must wait, never start a second run: {other:?}"),
        }
        assert_eq!(one_row_count(&conn), 1);
    }

    #[test]
    fn a_locked_slot_is_recorded_once_and_never_as_a_success() {
        let conn = connection();

        let first = skip_locked(&conn, JobKind::DailySearch, SLOT, NOW).unwrap();
        let second = skip_locked(&conn, JobKind::DailySearch, SLOT, NOW).unwrap();

        let run = first.expect("the first look at the slot records it");
        assert_eq!(run.status, RunStatus::SkippedLocked);
        assert_eq!(run.finished_at.as_deref(), Some(NOW));
        assert!(second.is_none(), "a second tick stays quiet (AC4)");
        assert_eq!(one_row_count(&conn), 1);

        // Once unlocked, the same slot still never becomes a success: the row
        // keeps its skipped_locked status and a run cannot start over it.
        let mut conn = conn;
        let reopened = open(&mut conn, JobKind::DailySearch, SLOT, NOW).unwrap();
        assert!(matches!(reopened, OpenOutcome::Joined(_)), "{reopened:?}");
        assert_eq!(
            get(&conn, JobKind::DailySearch, SLOT)
                .unwrap()
                .unwrap()
                .status,
            RunStatus::SkippedLocked
        );
    }

    #[test]
    fn finishing_a_run_writes_its_terminal_status() {
        let mut conn = connection();
        let OpenOutcome::Started(run) = open(&mut conn, JobKind::DailySearch, SLOT, NOW).unwrap()
        else {
            panic!("started");
        };

        let done = finish(
            &conn,
            &run.id,
            "2026-10-16T08:05:00Z",
            RunStatus::Succeeded,
            None,
        )
        .unwrap();

        assert_eq!(done.status, RunStatus::Succeeded);
        assert_eq!(done.finished_at.as_deref(), Some("2026-10-16T08:05:00Z"));
        assert!(running(&conn, JobKind::DailySearch).unwrap().is_none());
    }

    /// FX15: a cancelled run has its own terminal status, so it never reads as
    /// a success.
    #[test]
    fn a_cancelled_run_is_stored_as_cancelled() {
        let mut conn = connection();
        let OpenOutcome::Started(run) = open(&mut conn, JobKind::DailySearch, SLOT, NOW).unwrap()
        else {
            panic!("started");
        };

        let done = finish(
            &conn,
            &run.id,
            "2026-10-16T08:05:00Z",
            RunStatus::Cancelled,
            Some("cancelled by user"),
        )
        .unwrap();

        assert_eq!(done.status, RunStatus::Cancelled);
        assert_ne!(done.status, RunStatus::Succeeded, "never a success");
        let stored: String = conn
            .query_row(
                "SELECT status FROM job_run WHERE id = ?1",
                params![run.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, "cancelled");
    }

    #[test]
    fn finishing_a_run_twice_is_an_error() {
        let mut conn = connection();
        let OpenOutcome::Started(run) = open(&mut conn, JobKind::DailySearch, SLOT, NOW).unwrap()
        else {
            panic!("started");
        };
        finish(&conn, &run.id, NOW, RunStatus::Failed, Some("boom")).unwrap();

        assert!(
            finish(&conn, &run.id, NOW, RunStatus::Succeeded, None).is_err(),
            "a closed run is immutable"
        );
    }

    #[test]
    fn a_run_cannot_finish_as_running() {
        let mut conn = connection();
        let OpenOutcome::Started(run) = open(&mut conn, JobKind::DailySearch, SLOT, NOW).unwrap()
        else {
            panic!("started");
        };

        assert!(finish(&conn, &run.id, NOW, RunStatus::Running, None).is_err());
    }

    #[test]
    fn last_run_is_the_newest_slot_of_that_job() {
        let mut conn = connection();
        open(&mut conn, JobKind::DailySearch, "2026-10-15T08:00:00Z", NOW).unwrap();
        finish(
            &conn,
            &get(&conn, JobKind::DailySearch, "2026-10-15T08:00:00Z")
                .unwrap()
                .unwrap()
                .id,
            NOW,
            RunStatus::Succeeded,
            None,
        )
        .unwrap();
        open(&mut conn, JobKind::DailySearch, SLOT, NOW).unwrap();

        assert_eq!(
            last(&conn, JobKind::DailySearch)
                .unwrap()
                .map(|r| r.scheduled_for),
            Some(SLOT.to_string())
        );
        assert_eq!(last(&conn, JobKind::EmailSync).unwrap(), None);
    }

    #[test]
    fn mark_interrupted_closes_only_running_rows() {
        let mut conn = connection();
        open(&mut conn, JobKind::DailySearch, SLOT, NOW).unwrap();
        open(&mut conn, JobKind::EmailSync, SLOT, NOW).unwrap();
        let email = get(&conn, JobKind::EmailSync, SLOT).unwrap().unwrap();
        finish(&conn, &email.id, NOW, RunStatus::Succeeded, None).unwrap();

        let closed = interrupt(&conn, NOW, "interrupted by shutdown").unwrap();

        assert_eq!(closed, 1);
        assert!(running(&conn, JobKind::DailySearch).unwrap().is_none());
        assert_eq!(
            get(&conn, JobKind::EmailSync, SLOT)
                .unwrap()
                .unwrap()
                .status,
            RunStatus::Succeeded,
            "a finished row is untouched"
        );
    }

    fn one_row_count(conn: &Connection) -> i64 {
        conn.query_row("SELECT count(*) FROM job_run", [], |r| r.get(0))
            .unwrap()
    }
}
