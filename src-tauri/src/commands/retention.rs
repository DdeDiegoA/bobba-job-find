//! The `retention_sweep` job (S10.3, FR-113, AD-18).
//!
//! A thin Tauri seam: it resolves the two ports the sweep reads (the retention
//! adapter and the settings the windows come from), takes the time from the
//! clock and runs `boba_app::retention::sweep`. All the decisions — windows,
//! cutoffs, what is never deleted — live in `boba_app`; this file only wires
//! them to the app.
//!
//! A locked store never reaches this handler: the scheduler records the slot
//! `skipped_locked` and notifies (S5.1, ADR-009). A failure is diagnostic text
//! for `job_run.detail`, never user-facing prose (AD-3).

use std::sync::Arc;

use boba_app::jobs::{JobHandler, JobKind};
use boba_app::ports::clock::ClockPort;
use boba_app::ports::repo::system::SystemRepo;
use boba_app::retention::{RetentionPolicy, RetentionRepo};
use boba_app::system::SystemClock;
use tauri::{AppHandle, Runtime};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::wiring;

/// Mount the `retention_sweep` job. Called as one line from `port_list`
/// (ADR-012).
pub fn mount<R: Runtime>(app: &AppHandle<R>) {
    let registered = crate::scheduler::register_handler(
        app,
        JobKind::RetentionSweep,
        Arc::new(RetentionSweepHandler::new(app)),
    );
    if !registered {
        tracing::warn!("retention sweep handler was not registered");
    }
}

/// The scheduled retention sweep.
pub struct RetentionSweepHandler<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> RetentionSweepHandler<R> {
    /// A handler for the app it will read its ports from.
    pub fn new(app: &AppHandle<R>) -> Self {
        Self { app: app.clone() }
    }
}

impl<R: Runtime> JobHandler for RetentionSweepHandler<R> {
    fn run(&self, job: JobKind) -> Result<(), String> {
        if job != JobKind::RetentionSweep {
            return Err(format!("{} is not this handler's job", job.as_str()));
        }
        let port =
            |key: &str, error: boba_app::error::AppError| format!("{key}: {}", error.message_key);
        let store = wiring::port::<dyn RetentionRepo>(&self.app)
            .map_err(|error| port("retention repo", error))?;
        let system = wiring::port::<dyn SystemRepo>(&self.app)
            .map_err(|error| port("system repo", error))?;
        let now_text = SystemClock.now_rfc3339();
        let now = OffsetDateTime::parse(&now_text, &Rfc3339)
            .map_err(|error| format!("clock: {error}"))?;

        let policy = RetentionPolicy::from_settings(system.as_ref());
        let report = boba_app::retention::sweep(store.as_ref(), &policy, now)
            .map_err(|error| error.to_string())?;
        // The counts land in `setting` for the Privacy screen; a store that
        // cannot write them keeps the sweep's success (they are diagnostic).
        if let Err(error) = boba_app::retention::record(system.as_ref(), &report, &now_text) {
            tracing::warn!(%error, "retention: last sweep not recorded");
        }
        tracing::info!(
            postings = report.postings,
            raw_payloads = report.raw_payloads,
            emails = report.emails,
            "retention sweep finished"
        );
        Ok(())
    }
}
