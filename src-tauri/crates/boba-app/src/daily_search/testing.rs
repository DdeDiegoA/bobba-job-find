//! Test doubles for the daily search use case (test-only).
//!
//! The real adapters live outside `boba-app` (`boba-sources` for the network,
//! `boba-store` for the rows); these fakes mirror the contracts the use case
//! reads, so a test is about the run's decisions, not about SQLite or HTTP.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use boba_domain::source::{
    OptInRecord, RawPosting, RunStatus as SourceRunStatus, SearchQuery, SourceError, SourceRecord,
    SourceRunRecord,
};

use crate::ports::clock::ClockPort;
use crate::ports::notifier::{Notification, NotifierError, NotifierPort};
use crate::ports::repo::sources::{SourcesRepo, SourcesRepoError};

use super::{RunObserver, RunSource, RunSources, SourceReport};

/// One Source and the answer its turn returns.
type Scripted = (RunSource, Result<Vec<RawPosting>, SourceError>);

/// A `RunSources` that answers from a script, per source id.
#[derive(Default)]
pub struct ScriptedSources {
    script: Mutex<VecDeque<Scripted>>,
    queries: Mutex<Vec<(String, SearchQuery)>>,
}

impl ScriptedSources {
    /// One source per item, answered in that order.
    pub fn new(items: impl IntoIterator<Item = Scripted>) -> Self {
        Self {
            script: Mutex::new(items.into_iter().collect()),
            queries: Mutex::new(Vec::new()),
        }
    }

    /// The `(source id, query)` pairs the run asked for, in order.
    pub fn queries(&self) -> Vec<(String, SearchQuery)> {
        self.queries.lock().unwrap().clone()
    }
}

impl RunSources for ScriptedSources {
    fn sources(&self) -> Result<Vec<RunSource>, String> {
        Ok(self
            .script
            .lock()
            .unwrap()
            .iter()
            .map(|(source, _)| source.clone())
            .collect())
    }

    fn search(&self, source_id: &str, query: &SearchQuery) -> Result<Vec<RawPosting>, SourceError> {
        self.queries
            .lock()
            .unwrap()
            .push((source_id.to_string(), query.clone()));
        let script = self.script.lock().unwrap();
        let position = script
            .iter()
            .position(|(source, _)| source.id == source_id)
            .expect("the run queried a source it was not told about");
        script[position].1.clone()
    }
}

/// One source a test wants the run to see.
pub fn source(id: &str) -> RunSource {
    RunSource {
        id: id.to_string(),
        name: id.to_string(),
    }
}

/// A `SourcesRepo` that keeps the run rows it was handed, so a test can assert
/// that every Source got its `source_run` history.
#[derive(Default)]
pub struct RecordingSourcesRepo {
    sources: Mutex<Vec<SourceRecord>>,
    runs: Mutex<Vec<SourceRunRecord>>,
}

impl RecordingSourcesRepo {
    /// Every run row, started and finished, in the order the repo saw them.
    pub fn runs(&self) -> Vec<SourceRunRecord> {
        self.runs.lock().unwrap().clone()
    }

    fn apply_finish(
        &self,
        id: &str,
        finished_at: &str,
        status: SourceRunStatus,
        postings_found: i64,
        error: Option<&str>,
    ) -> Result<(), SourcesRepoError> {
        let mut runs = self.runs.lock().unwrap();
        let row = runs
            .iter_mut()
            .find(|row| row.id == id)
            .ok_or_else(|| SourcesRepoError::Invalid(id.to_string()))?;
        row.finished_at = Some(finished_at.to_string());
        row.status = status;
        row.postings_found = postings_found;
        row.error = error.map(str::to_string);
        Ok(())
    }
}

impl SourcesRepo for RecordingSourcesRepo {
    fn upsert_source(&self, record: &SourceRecord) -> Result<(), SourcesRepoError> {
        let mut sources = self.sources.lock().unwrap();
        match sources.iter_mut().find(|row| row.id == record.id) {
            Some(row) => *row = record.clone(),
            None => sources.push(record.clone()),
        }
        Ok(())
    }

    fn get_source(&self, id: &str) -> Result<Option<SourceRecord>, SourcesRepoError> {
        Ok(self
            .sources
            .lock()
            .unwrap()
            .iter()
            .find(|row| row.id == id)
            .cloned())
    }

    fn list_sources(&self) -> Result<Vec<SourceRecord>, SourcesRepoError> {
        Ok(self.sources.lock().unwrap().clone())
    }

    fn set_source_enabled(
        &self,
        id: &str,
        enabled: bool,
        updated_at: &str,
    ) -> Result<(), SourcesRepoError> {
        let mut sources = self.sources.lock().unwrap();
        let row = sources
            .iter_mut()
            .find(|row| row.id == id)
            .ok_or_else(|| SourcesRepoError::Invalid(id.to_string()))?;
        row.enabled = enabled;
        row.updated_at = updated_at.to_string();
        Ok(())
    }

    fn record_optin(&self, _optin: &OptInRecord) -> Result<(), SourcesRepoError> {
        Ok(())
    }

    fn get_optin(&self, _source_id: &str) -> Result<Option<OptInRecord>, SourcesRepoError> {
        Ok(None)
    }

    fn remove_optin(&self, _source_id: &str) -> Result<(), SourcesRepoError> {
        Ok(())
    }

    fn opted_in_ids(&self) -> Result<Vec<String>, SourcesRepoError> {
        Ok(Vec::new())
    }

    fn auto_opt_in(
        &self,

        _source_id: &str,
    ) -> Result<Option<boba_domain::apply::session_policy::SessionAutoOptIn>, SourcesRepoError>
    {
        Ok(None)
    }

    fn record_auto_opt_in(
        &self,

        _optin: &boba_domain::apply::session_policy::SessionAutoOptIn,
    ) -> Result<(), SourcesRepoError> {
        Ok(())
    }

    fn remove_auto_opt_in(&self, _source_id: &str) -> Result<(), SourcesRepoError> {
        Ok(())
    }

    fn auto_paused_until(&self, _source_id: &str) -> Result<Option<String>, SourcesRepoError> {
        Ok(None)
    }

    fn set_auto_paused_until(
        &self,

        _source_id: &str,

        _until: Option<&str>,

        _updated_at: &str,
    ) -> Result<(), SourcesRepoError> {
        Ok(())
    }

    fn start_run(&self, run: &SourceRunRecord) -> Result<(), SourcesRepoError> {
        self.runs.lock().unwrap().push(run.clone());
        Ok(())
    }

    fn recent_runs(
        &self,
        source_id: &str,
        limit: usize,
    ) -> Result<Vec<SourceRunRecord>, SourcesRepoError> {
        let runs = self.runs.lock().unwrap();
        Ok(runs
            .iter()
            .rev()
            .filter(|row| row.source_id == source_id)
            .take(limit)
            .cloned()
            .collect())
    }

    fn finish_run(
        &self,
        id: &str,
        finished_at: &str,
        status: SourceRunStatus,
        postings_found: i64,
        error: Option<&str>,
    ) -> Result<(), SourcesRepoError> {
        self.apply_finish(id, finished_at, status, postings_found, error)
    }
}

/// A `RunObserver` that keeps what the run reported, in order.
#[derive(Default)]
pub struct RecordingObserver {
    started: Mutex<Vec<(String, u32)>>,
    finished: Mutex<Vec<(SourceReport, u32, u32)>>,
    runs: Mutex<Vec<String>>,
}

impl RecordingObserver {
    /// `(run id, total sources)` for every run that started.
    pub fn started(&self) -> Vec<(String, u32)> {
        self.started.lock().unwrap().clone()
    }

    /// Every Source the run finished, with its position.
    pub fn sources(&self) -> Vec<(SourceReport, u32, u32)> {
        self.finished.lock().unwrap().clone()
    }

    /// The ids of the runs that finished.
    pub fn finished_runs(&self) -> Vec<String> {
        self.runs.lock().unwrap().clone()
    }
}

impl RunObserver for RecordingObserver {
    fn run_started(&self, run_id: &str, total: u32) {
        self.started
            .lock()
            .unwrap()
            .push((run_id.to_string(), total));
    }

    fn source_finished(&self, _run_id: &str, report: &SourceReport, done: u32, total: u32) {
        self.finished
            .lock()
            .unwrap()
            .push((report.clone(), done, total));
    }

    fn run_finished(&self, report: &super::report::RunReport) {
        self.runs.lock().unwrap().push(report.run_id.clone());
    }
}

/// A `NotifierPort` that keeps what the run asked to show.
#[derive(Default)]
pub struct RecordingNotifier {
    shown: Mutex<Vec<Notification>>,
}

impl RecordingNotifier {
    /// Every notification, in the order it was asked for.
    pub fn shown(&self) -> Vec<Notification> {
        self.shown.lock().unwrap().clone()
    }
}

impl NotifierPort for RecordingNotifier {
    fn notify(&self, notification: Notification) -> Result<(), NotifierError> {
        self.shown.lock().unwrap().push(notification);
        Ok(())
    }
}

/// A clock that advances one second per read, so start and end never collapse.
pub struct StepClock {
    reads: AtomicUsize,
}

impl StepClock {
    pub fn new() -> Self {
        Self {
            reads: AtomicUsize::new(0),
        }
    }
}

impl Default for StepClock {
    fn default() -> Self {
        Self::new()
    }
}

impl ClockPort for StepClock {
    fn now_rfc3339(&self) -> String {
        let n = self.reads.fetch_add(1, Ordering::SeqCst) as u64;
        let seconds = n % 60;
        format!("2026-10-17T08:{:02}:00Z", seconds)
    }
}

/// A posting as a Source returns it: the one shape every test starts from.
pub fn raw(external_id: &str, url: &str, title: &str) -> RawPosting {
    RawPosting {
        external_id: external_id.to_string(),
        title: title.to_string(),
        company: "Boba Labs".to_string(),
        url: url.to_string(),
        locations: vec!["Remote".to_string()],
        remote: Some(true),
        description_html: Some("<p>We build APIs in Rust.</p>".to_string()),
        posted_at: Some("2026-10-10T00:00:00Z".to_string()),
        salary_text: None,
        raw: format!(r#"{{"id":"{external_id}"}}"#),
    }
}

/// A posting whose posted work mode is on-site, so the Location Gate can see it.
pub fn onsite(external_id: &str, url: &str, title: &str) -> RawPosting {
    RawPosting {
        remote: Some(false),
        locations: vec!["Madrid, Spain".to_string()],
        ..raw(external_id, url, title)
    }
}
