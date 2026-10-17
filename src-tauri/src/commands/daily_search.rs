//! The daily search run's composition and its commands (S5.2, FR-46, NFR-9,
//! NFR-14).
//!
//! The command layer is the only place that can do the three things `boba-app`
//! cannot: build the Source adapters from `boba-sources`, block on their async
//! search from the scheduler's own thread, and emit typed progress with the app
//! handle. Everything the run *decides* is
//! [`boba_app::daily_search::DailySearch`].
//!
//! Two entry points share one pipeline:
//!
//! * the scheduler's [`JobHandler`] for [`JobKind::DailySearch`] — a scheduled
//!   or "Run now" run, whose report is stored under its `job_run` id;
//! * [`run_report_retry`] — re-queries only the Sources the newest report could
//!   not reach (AC3).
//!
//! The report is one JSON row in the encrypted `setting` table (no schema
//! change), kept with the newest [`MAX_REPORTS`] runs so the screen never reads
//! an unbounded history.

use std::sync::Arc;

use boba_app::daily_search::report::{RunReport, SourceOutcome};
use boba_app::daily_search::{
    AgentPass, DailySearch, RunObserver, RunOptions, RunRepos, RunSource, RunSources, SourceReport,
};
use boba_app::error::{AppError, AppResult};
use boba_app::jobs::{JobHandler, JobKind};
use boba_app::ports::agent::AgentPort;
use boba_app::ports::clock::ClockPort;
use boba_app::ports::notifier::NotifierPort;
use boba_app::ports::repo::ingestion::IngestionRepo;
use boba_app::ports::repo::profile::ProfileRepo;
use boba_app::ports::repo::scoring::ScoringRepo;
use boba_app::ports::repo::sources::SourcesRepo;
use boba_app::ports::repo::system::SystemRepo;
use boba_app::ports::session::SessionPages;
use boba_app::ports::source::{SourceFuture, SourcePort};
use boba_app::posting_features::ExtractFeatures;
use boba_app::sources::custom;
use boba_app::system::SystemClock;
use boba_domain::source::{
    RawPosting, SearchQuery, SourceError, SourceFetch, SourceManifest, SourceRecord,
};
use boba_sources::http::HttpClient;
use boba_sources::registry::SourceRegistry;
use boba_sources::rss::CustomFeed;
use boba_sources::sdk::SourceAdapter;
use boba_sources::session::sdk::RunControl;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};
use tauri_specta::Event;

use crate::wiring;

/// The `setting` row holding the newest report's id.
pub const LATEST_KEY: &str = "run_report.latest";
/// The `setting` row holding the ids of the stored reports, newest first.
pub const INDEX_KEY: &str = "run_report.index";
/// The `setting` key prefix of one report's JSON.
pub const REPORT_PREFIX: &str = "run_report.report.";
/// How many finished reports the app keeps.
pub const MAX_REPORTS: usize = 30;

/// One Source's turn as the report screen reads it (AC2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct SourceReportView {
    pub source_id: String,
    pub name: String,
    /// `ok` | `failed`.
    pub outcome: String,
    pub fetched: u32,
    pub new: u32,
    pub duplicates: u32,
    /// Diagnostic text; the screen shows it as a detail, never as the outcome.
    pub error: Option<String>,
}

/// A finished run as the report screen reads it (AC2): the report's own fields
/// plus the totals derived from its Sources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct RunReportView {
    pub run_id: String,
    pub retry_of: Option<String>,
    pub started_at: String,
    pub finished_at: String,
    /// Whole milliseconds. Exported as a JS number (specta refuses BigInt-style
    /// types over IPC); a run's duration is far below 2^53.
    #[specta(type = specta_typescript::Number)]
    pub duration_ms: i64,
    pub sources: Vec<SourceReportView>,
    pub queried: u32,
    pub fetched: u32,
    pub new: u32,
    pub duplicates: u32,
    pub vetoed: u32,
    pub ranked: u32,
    pub agent_tasks_used: u32,
    pub errors: Vec<String>,
    /// True when at least one Source failed: the banner with Retry (AC3).
    pub partial: bool,
}

impl RunReportView {
    fn of(report: &RunReport) -> Self {
        let totals = report.totals();
        Self {
            run_id: report.run_id.clone(),
            retry_of: report.retry_of.clone(),
            started_at: report.started_at.clone(),
            finished_at: report.finished_at.clone(),
            duration_ms: report.duration_ms,
            sources: report
                .sources
                .iter()
                .map(|source| SourceReportView {
                    source_id: source.source_id.clone(),
                    name: source.name.clone(),
                    outcome: source.outcome.as_str().to_string(),
                    fetched: source.fetched,
                    new: source.new,
                    duplicates: source.duplicates,
                    error: source.error.clone(),
                })
                .collect(),
            queried: totals.queried,
            fetched: totals.fetched,
            new: totals.new,
            duplicates: totals.duplicates,
            vetoed: report.vetoed,
            ranked: report.ranked,
            agent_tasks_used: report.agent_tasks_used,
            errors: report.errors.clone(),
            partial: report.is_partial(),
        }
    }
}

/// A run's live progress, emitted as `run_report.progressed` (AC1).
///
/// One event per phase: `started` (the run began against `total` Sources),
/// `source` (one Source's turn ended, `done` of `total` now finished) and
/// `finished` (the report is stored; the screen reads it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type, Event)]
#[tauri_specta(event_name = "run_report.progressed")]
pub struct RunReportProgressed {
    pub run_id: String,
    /// `started` | `source` | `finished`.
    pub phase: String,
    pub done: u32,
    pub total: u32,
    /// The Source whose turn just ended (`phase = source` only).
    pub source_id: Option<String>,
    pub source_name: Option<String>,
    /// `ok` | `failed` (`phase = source` only).
    pub outcome: Option<String>,
    /// Whether the finished run has failed Sources (`phase = finished` only).
    pub partial: Option<bool>,
}

/// Emit the run's progress as it works (AC1).
struct EventObserver<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> RunObserver for EventObserver<R> {
    fn run_started(&self, run_id: &str, total: u32) {
        let _ = RunReportProgressed {
            run_id: run_id.to_string(),
            phase: "started".to_string(),
            done: 0,
            total,
            source_id: None,
            source_name: None,
            outcome: None,
            partial: None,
        }
        .emit(&self.app);
    }

    fn source_finished(&self, run_id: &str, report: &SourceReport, done: u32, total: u32) {
        let _ = RunReportProgressed {
            run_id: run_id.to_string(),
            phase: "source".to_string(),
            done,
            total,
            source_id: Some(report.source_id.clone()),
            source_name: Some(report.name.clone()),
            outcome: Some(report.outcome.as_str().to_string()),
            partial: None,
        }
        .emit(&self.app);
    }

    fn run_finished(&self, report: &RunReport) {
        let _ = RunReportProgressed {
            run_id: report.run_id.clone(),
            phase: "finished".to_string(),
            done: report.sources.len() as u32,
            total: report.sources.len() as u32,
            source_id: None,
            source_name: None,
            outcome: None,
            partial: Some(report.is_partial()),
        }
        .emit(&self.app);
    }
}

/// The Sources a run queries, one turn at a time, over the configured adapters.
///
/// It is rebuilt from the store on every call: a Source the user disabled in
/// Connections is not queried on the next run, and a custom feed added since the
/// last run is (AC1, S4.8).
pub struct LiveSources {
    http: Arc<HttpClient>,
    sources: Arc<dyn SourcesRepo>,
    system: Arc<dyn SystemRepo>,
    /// The Session Board handles (S11.1b): the user's Chrome and the core
    /// gate/audit/pause. Absent in tests, and in a host where they could not
    /// be built — the run then simply has no session boards.
    session: Option<(Arc<dyn SessionPages>, Arc<dyn RunControl>)>,
}

type AdapterSet = Vec<(String, Result<Box<dyn SourceAdapter>, SourceError>)>;

/// Whether a Session Board belongs in the run (S11.1b): it must be opted in
/// and its `source` row must not be disabled.
///
/// A board the user never opted into is left out rather than reported as a
/// failed Source on every run — the registry's opt-in gate would refuse it and
/// a Retry could never succeed (dev-1, 2026-10-17).
fn session_board_runs(id: &str, opted_in: &[String], enabled: &dyn Fn(&str) -> bool) -> bool {
    opted_in.iter().any(|source| source == id) && enabled(id)
}

impl LiveSources {
    pub fn new(
        http: Arc<HttpClient>,
        sources: Arc<dyn SourcesRepo>,
        system: Arc<dyn SystemRepo>,
    ) -> Self {
        Self {
            http,
            sources,
            system,
            session: None,
        }
    }

    /// Register the Session Board adapters with their run-side handles
    /// (S11.1b): LinkedIn and Indeed then run like any other Source.
    pub fn with_session(
        mut self,
        pages: Arc<dyn SessionPages>,
        control: Arc<dyn RunControl>,
    ) -> Self {
        self.session = Some((pages, control));
        self
    }

    /// Every adapter a run may use: each built-in that is not disabled, plus the
    /// custom feeds the user saved. A feed that cannot be built keeps its id and
    /// carries its error, so it is a failed Source the banner can Retry rather
    /// than a tile that silently never runs.
    ///
    /// A row read that fails is an **error**, not "no rows": treating it as an
    /// empty table would run every Source, including the ones the user turned
    /// off (reviewer-1 F1). The run fails closed instead.
    fn adapters(&self) -> Result<AdapterSet, String> {
        let rows = self
            .sources
            .list_sources()
            .map_err(|error| error.to_string())?;
        let enabled = |id: &str| {
            rows.iter()
                .find(|row| row.id == id)
                .is_none_or(|row| row.enabled)
        };

        let mut adapters: AdapterSet = Vec::new();
        for adapter in boba_sources::feeds::builtin() {
            let id = adapter.manifest().id;
            if enabled(&id) {
                adapters.push((id, Ok(adapter)));
            }
        }
        match custom::list_custom_feeds(self.system.as_ref()) {
            Ok(feeds) => {
                for feed in feeds {
                    if !enabled(&feed.id) {
                        continue;
                    }
                    let built = CustomFeed::new(feed.id.clone(), feed.name.clone(), &feed.url)
                        .map(|feed| Box::new(feed) as Box<dyn SourceAdapter>);
                    adapters.push((feed.id, built));
                }
            }
            Err(error) => tracing::warn!(%error, "custom feeds not read for the run"),
        }
        // The Session Board Sources (S11.1b) run like any other Source, but
        // only once their own opt-in is recorded: an un-opted-in board is left
        // out rather than reported as a failed Source every run. Each page load
        // still passes SessionRun inside the adapter.
        if let Some((pages, control)) = &self.session {
            let opted_in = self.sources.opted_in_ids().unwrap_or_default();
            for adapter in boba_sources::session::builtin(Arc::clone(pages), Arc::clone(control)) {
                let id = adapter.manifest().id;
                if session_board_runs(&id, &opted_in, &enabled) {
                    adapters.push((id, Ok(adapter)));
                }
            }
        }
        Ok(adapters)
    }

    /// A registry over `only` (or every) adapter, with the recorded opt-ins
    /// applied. An adapter that could not be built is not registered: its error
    /// is the Source's report line.
    fn registry(&self, adapters: AdapterSet, only: Option<&str>) -> SourceRegistry {
        let mut registry = SourceRegistry::new(Arc::clone(&self.http));
        for (id, built) in adapters {
            if only.is_some_and(|only| only != id) {
                continue;
            }
            if let Ok(adapter) = built {
                let _ = registry.register(adapter);
            }
        }
        if let Ok(opted_in) = self.sources.opted_in_ids() {
            registry.set_opted_in(opted_in);
        }
        registry
    }

    /// The `source` row a Source's postings are stored under (the `posting`
    /// foreign key). A row the Connections screen never created is created here
    /// from the manifest, the same lazy seam Manual intake uses.
    fn ensure_row(&self, id: &str, adapter: &dyn SourceAdapter) {
        match self.sources.get_source(id) {
            // The row is there, whatever it says; never rewrite it here.
            Ok(Some(_)) => return,
            // Only a read that succeeded and found nothing creates a row: a
            // failed read must not, or a transient error would re-enable a
            // Source the user disabled (reviewer-1 F2).
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(%error, source = id, "source row not read; left alone");
                return;
            }
        }
        let manifest = adapter.manifest();
        let now = SystemClock.now_rfc3339();
        let record = SourceRecord {
            id: manifest.id.clone(),
            name: manifest.name.clone(),
            access_class: manifest.access_class,
            legal_tier: manifest.legal_tier,
            apply_capability: manifest.apply_capability,
            attribution: manifest.attribution.clone(),
            min_interval_ms: manifest.rate_limits.min_interval_ms,
            daily_cap: manifest.rate_limits.daily_cap,
            enabled: true,
            added_at: now.clone(),
            updated_at: now,
        };
        if let Err(error) = self.sources.upsert_source(&record) {
            tracing::warn!(%error, source = id, "source row not created for the run");
        }
    }
}

impl RunSources for LiveSources {
    fn sources(&self) -> Result<Vec<RunSource>, String> {
        let adapters = self.adapters()?;
        let mut sources = Vec::with_capacity(adapters.len());
        for (id, built) in &adapters {
            let name = match built {
                Ok(adapter) => {
                    let manifest = adapter.manifest();
                    self.ensure_row(id, adapter.as_ref());
                    manifest.name
                }
                // A feed that cannot be built still gets its turn in the
                // report; its search returns the build error.
                Err(_) => id.clone(),
            };
            sources.push(RunSource {
                id: id.clone(),
                name,
            });
        }
        Ok(sources)
    }

    fn search(&self, source_id: &str, query: &SearchQuery) -> Result<Vec<RawPosting>, SourceError> {
        let adapters = self
            .adapters()
            .map_err(|reason| SourceError::InvalidPayload { message: reason })?;
        match adapters
            .iter()
            .find(|(id, _)| id == source_id)
            .map(|(_, built)| built)
        {
            None => Err(SourceError::UnknownSource {
                source_id: source_id.to_string(),
            }),
            Some(Err(error)) => Err(error.clone()),
            Some(Ok(_)) => {
                let registry = self.registry(adapters, Some(source_id));
                let fetches = tauri::async_runtime::block_on(registry.search(query));
                match fetches.into_iter().next() {
                    Some(SourceFetch::Ok { postings, .. }) => Ok(postings),
                    Some(SourceFetch::Err { error, .. }) => Err(error),
                    // The registry found nothing runnable: the Source is gated
                    // (no opt-in) or gone since `sources()` read it.
                    None => Err(SourceError::OptInRequired {
                        source_id: source_id.to_string(),
                    }),
                }
            }
        }
    }
}

/// The mounted [`SourcePort`] view of the same adapters: every Source at once,
/// for callers that only want the postings (a detail fetch, a future probe).
impl SourcePort for LiveSources {
    fn manifests(&self) -> Vec<SourceManifest> {
        // A display list, not a run: an unreadable table leaves it empty rather
        // than pretending to know the Sources.
        match self.adapters() {
            Ok(adapters) => adapters
                .into_iter()
                .filter_map(|(_, built)| built.ok().map(|adapter| adapter.manifest()))
                .collect(),
            Err(error) => {
                tracing::warn!(%error, "source manifests not read");
                Vec::new()
            }
        }
    }

    fn search<'a>(&'a self, query: &'a SearchQuery) -> SourceFuture<'a, Vec<SourceFetch>> {
        Box::pin(async move {
            match self.adapters() {
                Ok(adapters) => self.registry(adapters, None).search(query).await,
                Err(reason) => vec![SourceFetch::Err {
                    source_id: "sources".to_string(),
                    error: SourceError::InvalidPayload { message: reason },
                }],
            }
        })
    }

    fn detail<'a>(
        &'a self,
        source_id: &'a str,
        external_id: &'a str,
    ) -> SourceFuture<'a, Result<RawPosting, SourceError>> {
        Box::pin(async move {
            let adapters = self
                .adapters()
                .map_err(|reason| SourceError::InvalidPayload { message: reason })?;
            self.registry(adapters, Some(source_id))
                .detail(source_id, external_id)
                .await
        })
    }
}

/// The scheduler's handler for the daily search (S5.1's seam, S5.2's use case).
pub struct DailySearchHandler<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> DailySearchHandler<R> {
    pub fn new(app: &AppHandle<R>) -> Self {
        Self { app: app.clone() }
    }
}

impl<R: Runtime> JobHandler for DailySearchHandler<R> {
    fn run(&self, job: JobKind) -> Result<(), String> {
        if job != JobKind::DailySearch {
            return Err(format!("{} is not this handler's job", job.as_str()));
        }
        // The row the scheduler opened and is running: it is the report's id,
        // so "Run now", a scheduled run and its retry line up under one id. The
        // row is read without the idle gate, the way the scheduler reads it
        // (S5.1): being locked is why this run exists, not a reason to refuse.
        let jobs = self
            .app
            .try_state::<Arc<dyn boba_app::jobs::JobsRepo>>()
            .map(|state| Arc::clone(state.inner()))
            .ok_or_else(|| "no job_run owner is mounted".to_string())?;
        let run_id = jobs
            .running_run(JobKind::DailySearch)
            .map_err(|error| error.to_string())?
            .map(|run| run.id)
            .ok_or_else(|| "no running daily_search row".to_string())?;

        // The run itself succeeded even when a Source did not: a failed Source
        // is the report's story and the banner's Retry, never a failed job row.
        let execution = execute(&self.app, RunOptions::for_run(run_id), None)
            .map_err(|error| error.message_key.to_string())?;
        store_report(execution.system.as_ref(), &execution.report)
            .map_err(|error| error.message_key.to_string())?;
        Ok(())
    }
}

/// What one run produced, and the store it was produced against, so the caller
/// decides what is stored (a retry folds its lines into the report it retried
/// instead of replacing it — reviewer-1 F4).
struct Execution {
    report: RunReport,
    system: Arc<dyn SystemRepo>,
}

/// Resolve the ports and run the use case. `only_sources` makes the run a
/// retry (AC3); `retry_of` names the report it follows.
fn execute<R: Runtime>(
    app: &AppHandle<R>,
    options: RunOptions,
    only_sources: Option<Vec<String>>,
) -> AppResult<Execution> {
    let http = wiring::port::<HttpClient>(app)?;
    let sources = wiring::port::<dyn SourcesRepo>(app)?;
    let system = wiring::port::<dyn SystemRepo>(app)?;
    let ingestion = wiring::port::<dyn IngestionRepo>(app)?;
    let scoring = wiring::port::<dyn ScoringRepo>(app)?;
    let profile = wiring::port::<dyn ProfileRepo>(app)?;
    let agents = wiring::port::<dyn AgentPort>(app)?;
    let clock: Arc<dyn ClockPort> = Arc::new(SystemClock);

    let features = Arc::new(ExtractFeatures::new(
        Arc::clone(&ingestion),
        Arc::clone(&profile),
        agents,
        Arc::clone(&clock),
    ));
    let notifier: Arc<dyn NotifierPort> =
        Arc::new(crate::scheduler::notify::TauriNotifier::new(app.clone()));
    // S11.1b: the Session Board handles; an unavailable browser/port leaves
    // them failing per Source, never aborting the run.
    let (session_pages, session_control) = wiring::session::controls(app);
    let live = Arc::new(
        LiveSources::new(Arc::clone(&http), Arc::clone(&sources), Arc::clone(&system))
            .with_session(session_pages, session_control),
    );

    let mut options = options;
    options.only_sources = only_sources;
    options.agent = stored_engine(system.as_ref()).map(|(cli, model)| AgentPass::new(cli, model));
    if options.agent.is_none() {
        tracing::info!("daily search: no engine chosen, scoring from deterministic features");
    }

    let service = DailySearch::new(
        live,
        RunRepos {
            sources,
            ingestion,
            scoring,
            profile,
            system: Arc::clone(&system),
        },
        features,
        clock,
        Some(notifier),
        Some(Arc::new(EventObserver { app: app.clone() })),
    );

    let report = service
        .run(options)
        .map_err(|error| AppError::internal(error.message_key()))?;
    Ok(Execution { report, system })
}

/// The engine the user chose (S3.2), read through the port so this stays
/// runtime-generic: the same `ai_engine` row the engine step writes.
fn stored_engine(system: &dyn SystemRepo) -> Option<(String, Option<String>)> {
    #[derive(Deserialize)]
    struct Stored {
        cli: String,
        #[serde(default)]
        model: Option<String>,
    }
    let raw = system
        .setting_get(crate::commands::agent::ENGINE_KEY)
        .ok()
        .flatten()?;
    let stored: Stored = serde_json::from_str(&raw).ok()?;
    let cli = stored.cli.trim().to_string();
    if cli.is_empty() {
        return None;
    }
    let model = stored
        .model
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty());
    Some((cli, model))
}

/// Serializes the report store: the index is a read-modify-write and a
/// scheduled run and a Retry run on their own threads (reviewer-1 F3).
static REPORT_STORE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Keep one finished report, and the newest [`MAX_REPORTS`] of them.
///
/// The report key is written before the index, and the index before
/// [`LATEST_KEY`], so a reader that follows `LATEST_KEY` always finds the whole
/// report. Writers are serialized, so two runs cannot drop each other's index
/// entries or leave `LATEST_KEY` pointing at the older run.
fn store_report(system: &dyn SystemRepo, report: &RunReport) -> AppResult<()> {
    let _guard = REPORT_STORE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    write_report(system, report)
}

/// Store a retry's outcome, **reading the stored report at merge time**
/// (reviewer-1 F6): the retry runs for minutes, and a scheduled run may have
/// stored a newer report meanwhile.
///
/// The report being retried is read again under the write lock. When it is
/// still the newest, the retry folds into it (F4): the retried Sources replace
/// their failed lines and the Sources the first run reached keep theirs. When a
/// newer run took over, the report being retried is no longer the one to touch
/// — the retry is stored as **its own** report (`retry_of` names the run it
/// followed) and the newer run's report is left byte for byte alone.
fn store_retry(
    system: &dyn SystemRepo,
    retried_run_id: &str,
    retry: &RunReport,
) -> AppResult<RunReport> {
    let _guard = REPORT_STORE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let stored = match latest_report(system) {
        Some(latest) if latest.run_id == retried_run_id => merge_retry(&latest, retry),
        _ => retry.clone(),
    };
    write_report(system, &stored)?;
    Ok(stored)
}

/// The write itself; callers hold [`REPORT_STORE`].
fn write_report(system: &dyn SystemRepo, report: &RunReport) -> AppResult<()> {
    let now = SystemClock.now_rfc3339();
    let json = serde_json::to_string(report).map_err(|error| {
        AppError::internal("errors.run_report.store").with_detail("reason", error.to_string())
    })?;
    let key = format!("{REPORT_PREFIX}{}", report.run_id);
    system.setting_put(&key, &json, &now).map_err(store_error)?;

    let mut index = report_index(system);
    index.retain(|id| id != &report.run_id);
    index.insert(0, report.run_id.clone());
    let pruned: Vec<String> = index.split_off(index.len().min(MAX_REPORTS));
    for id in pruned {
        let _ = system.setting_delete(&format!("{REPORT_PREFIX}{id}"));
    }
    system
        .setting_put(
            INDEX_KEY,
            &serde_json::to_string(&index).unwrap_or_else(|_| "[]".to_string()),
            &now,
        )
        .map_err(store_error)?;
    system
        .setting_put(LATEST_KEY, &report.run_id, &now)
        .map_err(store_error)?;
    Ok(())
}

/// Fold a retry's lines into the report it retried (reviewer-1 F4).
///
/// A retried Source replaces its old line and its old error; a Source the first
/// run reached keeps its line. The scoring counters and the agent tasks add up,
/// the errors concatenate, and the merged report keeps the original run's id and
/// start, because it is the same run the user is looking at, now complete.
fn merge_retry(previous: &RunReport, retry: &RunReport) -> RunReport {
    let mut merged = previous.clone();
    for line in &retry.sources {
        match merged
            .sources
            .iter_mut()
            .find(|source| source.source_id == line.source_id)
        {
            Some(existing) => *existing = line.clone(),
            None => merged.sources.push(line.clone()),
        }
    }
    merged.vetoed += retry.vetoed;
    merged.ranked += retry.ranked;
    merged.agent_tasks_used += retry.agent_tasks_used;
    merged.errors.extend(retry.errors.iter().cloned());
    merged.finished_at = retry.finished_at.clone();
    merged.duration_ms += retry.duration_ms;
    merged
}

/// The stored report ids, newest first.
fn report_index(system: &dyn SystemRepo) -> Vec<String> {
    system
        .setting_get(INDEX_KEY)
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// The newest stored report, if any.
fn latest_report(system: &dyn SystemRepo) -> Option<RunReport> {
    let id = system.setting_get(LATEST_KEY).ok().flatten()?;
    let raw = system
        .setting_get(&format!("{REPORT_PREFIX}{id}"))
        .ok()
        .flatten()?;
    serde_json::from_str(&raw).ok()
}

fn store_error(error: boba_app::ports::repo::system::SystemRepoError) -> AppError {
    AppError::internal("errors.run_report.store").with_detail("reason", error.to_string())
}

/// The newest run's report (AC2), `None` before the first run.
#[tauri::command]
#[specta::specta]
pub async fn run_report_latest(app: tauri::AppHandle) -> AppResult<Option<RunReportView>> {
    let system = wiring::port::<dyn SystemRepo>(&app)?;
    Ok(latest_report(system.as_ref()).map(|report| RunReportView::of(&report)))
}

/// Retry the Sources the newest report could not reach (AC3): re-queries only
/// those, on its own thread, and emits `run_report.progressed` as it works.
/// Returns as soon as the retry starts; the screen follows the events.
#[tauri::command]
#[specta::specta]
pub async fn run_report_retry(app: tauri::AppHandle) -> AppResult<()> {
    let system = wiring::port::<dyn SystemRepo>(&app)?;
    let report = latest_report(system.as_ref())
        .ok_or_else(|| AppError::invalid_input("errors.run_report.no_report"))?;
    let failed: Vec<String> = report
        .sources
        .iter()
        .filter(|source| source.outcome == SourceOutcome::Failed)
        .map(|source| source.source_id.clone())
        .collect();
    if failed.is_empty() {
        return Err(AppError::invalid_input(
            "errors.run_report.nothing_to_retry",
        ));
    }

    let run_id = report.run_id.clone();
    let retried_run_id = report.run_id.clone();
    let thread_app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("boba-daily-search-retry".to_string())
        .spawn(move || {
            let options = RunOptions {
                run_id: format!("retry_{}", ulid_run_id()),
                retry_of: Some(run_id),
                only_sources: None,
                agent: None,
            };
            match execute(&thread_app, options, Some(failed)) {
                // The retried Sources replace their failed lines; the Sources
                // the first run reached keep theirs, so the newest view still
                // shows the whole run (reviewer-1 F4).
                Ok(execution) => {
                    if let Err(error) = store_retry(
                        execution.system.as_ref(),
                        &retried_run_id,
                        &execution.report,
                    ) {
                        tracing::warn!(key = error.message_key, "retried report not stored");
                    }
                }
                Err(error) => {
                    tracing::warn!(key = error.message_key, "daily search retry failed");
                }
            }
        });
    if let Err(error) = spawned {
        return Err(AppError::internal("errors.run_report.retry_not_started")
            .with_detail("reason", error.to_string()));
    }
    Ok(())
}

/// A fresh ULID for a retry that has no `job_run` row of its own. AD-17's id
/// helper lives in `boba-domain` so the app layer needs no id crate.
fn ulid_run_id() -> String {
    boba_domain::profile::new_ulid()
}

/// Mount the run's ports and register its job handler (one call from
/// `port_list`, so AD-012's append-only list stays one line per slice).
pub fn mount<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let http = wiring::port::<HttpClient>(app).map_err(|error| error.message_key.to_string())?;
    let sources =
        wiring::port::<dyn SourcesRepo>(app).map_err(|error| error.message_key.to_string())?;
    let system =
        wiring::port::<dyn SystemRepo>(app).map_err(|error| error.message_key.to_string())?;
    let (session_pages, session_control) = wiring::session::controls(app);
    let live = Arc::new(
        LiveSources::new(http, sources, system).with_session(session_pages, session_control),
    );
    wiring::mount_port::<dyn SourcePort>(app, live);
    crate::scheduler::register_handler(
        app,
        JobKind::DailySearch,
        Arc::new(DailySearchHandler::new(app)),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use boba_app::ports::repo::system::{NewIntent, SettingRecord, SystemRepo, SystemRepoError};
    use boba_domain::audit::{AuditOutcome, AuditRow, ChainStatus};

    use super::*;

    /// The settings half of [`SystemRepo`], which is all this module reads and
    /// writes; the audit methods belong to S1.7 and are never reached here.
    #[derive(Default)]
    struct MemorySettings {
        rows: Mutex<BTreeMap<String, SettingRecord>>,
    }

    impl SystemRepo for MemorySettings {
        fn setting_get(&self, key: &str) -> Result<Option<String>, SystemRepoError> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .get(key)
                .map(|row| row.value.clone()))
        }

        fn setting_put(
            &self,
            key: &str,
            value: &str,
            updated_at: &str,
        ) -> Result<(), SystemRepoError> {
            self.rows.lock().unwrap().insert(
                key.to_string(),
                SettingRecord {
                    key: key.to_string(),
                    value: value.to_string(),
                    updated_at: updated_at.to_string(),
                },
            );
            Ok(())
        }

        fn setting_list(&self) -> Result<Vec<SettingRecord>, SystemRepoError> {
            Ok(self.rows.lock().unwrap().values().cloned().collect())
        }

        fn setting_delete(&self, key: &str) -> Result<bool, SystemRepoError> {
            Ok(self.rows.lock().unwrap().remove(key).is_some())
        }

        fn audit_append_intent(&self, _: &NewIntent) -> Result<AuditRow, SystemRepoError> {
            unreachable!("the run report does not write the audit log")
        }

        fn audit_append_outcome(
            &self,
            _: &AuditRow,
            _: AuditOutcome,
            _: &str,
            _: &str,
        ) -> Result<AuditRow, SystemRepoError> {
            unreachable!("the run report does not write the audit log")
        }

        fn audit_head_hash(&self) -> Result<Option<String>, SystemRepoError> {
            unreachable!("the run report does not read the audit log")
        }

        fn audit_verify_chain(&self) -> Result<ChainStatus, SystemRepoError> {
            unreachable!("the run report does not read the audit log")
        }

        fn audit_unresolved_intents(&self) -> Result<Vec<AuditRow>, SystemRepoError> {
            unreachable!("the run report does not read the audit log")
        }

        fn audit_anchor_get(&self) -> Result<Option<String>, SystemRepoError> {
            unreachable!("the run report does not read the audit log")
        }

        fn audit_anchor_is_known(&self, _: &str) -> Result<bool, SystemRepoError> {
            unreachable!("the run report does not read the audit log")
        }

        fn audit_anchor_set(&self, _: &str) -> Result<(), SystemRepoError> {
            unreachable!("the run report does not write the audit log")
        }
    }

    fn report(run_id: &str, failed: bool) -> RunReport {
        RunReport {
            run_id: run_id.to_string(),
            retry_of: None,
            started_at: "2026-10-17T08:00:00Z".to_string(),
            finished_at: "2026-10-17T08:01:00Z".to_string(),
            duration_ms: 60_000,
            sources: vec![SourceReport {
                source_id: "remotive".to_string(),
                name: "Remotive".to_string(),
                outcome: if failed {
                    SourceOutcome::Failed
                } else {
                    SourceOutcome::Ok
                },
                fetched: 0,
                new: 0,
                duplicates: 0,
                error: failed.then(|| "HTTP 503".to_string()),
            }],
            vetoed: 0,
            ranked: 0,
            agent_tasks_used: 0,
            errors: Vec::new(),
        }
    }

    /// A `SourcesRepo` whose reads can be made to fail, so the fail-closed
    /// rules (reviewer-1 F1/F2) are exercised without SQLite.
    struct CheckedSources {
        list: Result<(), ()>,
        get: Result<(), ()>,
        upserts: std::sync::atomic::AtomicUsize,
    }

    impl CheckedSources {
        fn failing_get() -> Self {
            Self {
                list: Ok(()),
                get: Err(()),
                upserts: std::sync::atomic::AtomicUsize::new(0),
            }
        }

        fn upserts(&self) -> usize {
            self.upserts.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    impl boba_app::ports::repo::sources::SourcesRepo for CheckedSources {
        fn upsert_source(
            &self,
            _record: &SourceRecord,
        ) -> Result<(), boba_app::ports::repo::sources::SourcesRepoError> {
            self.upserts
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }

        fn get_source(
            &self,
            _id: &str,
        ) -> Result<Option<SourceRecord>, boba_app::ports::repo::sources::SourcesRepoError>
        {
            match self.get {
                Ok(()) => Ok(None),
                Err(()) => Err(boba_app::ports::repo::sources::SourcesRepoError::Store(
                    "disk".to_string(),
                )),
            }
        }

        fn list_sources(
            &self,
        ) -> Result<Vec<SourceRecord>, boba_app::ports::repo::sources::SourcesRepoError> {
            match self.list {
                Ok(()) => Ok(Vec::new()),
                Err(()) => Err(boba_app::ports::repo::sources::SourcesRepoError::Store(
                    "disk".to_string(),
                )),
            }
        }

        fn set_source_enabled(
            &self,
            _id: &str,
            _enabled: bool,
            _updated_at: &str,
        ) -> Result<(), boba_app::ports::repo::sources::SourcesRepoError> {
            unreachable!("the run does not toggle Sources")
        }

        fn record_optin(
            &self,
            _optin: &boba_domain::source::OptInRecord,
        ) -> Result<(), boba_app::ports::repo::sources::SourcesRepoError> {
            unreachable!("the run does not record opt-ins")
        }

        fn get_optin(
            &self,
            _source_id: &str,
        ) -> Result<
            Option<boba_domain::source::OptInRecord>,
            boba_app::ports::repo::sources::SourcesRepoError,
        > {
            Ok(None)
        }

        fn remove_optin(
            &self,
            _source_id: &str,
        ) -> Result<(), boba_app::ports::repo::sources::SourcesRepoError> {
            unreachable!("the run does not remove opt-ins")
        }

        fn opted_in_ids(
            &self,
        ) -> Result<Vec<String>, boba_app::ports::repo::sources::SourcesRepoError> {
            Ok(Vec::new())
        }

        fn start_run(
            &self,
            _run: &boba_domain::source::SourceRunRecord,
        ) -> Result<(), boba_app::ports::repo::sources::SourcesRepoError> {
            unreachable!("the run history is the use case's, not this test's")
        }

        fn recent_runs(
            &self,
            _source_id: &str,
            _limit: usize,
        ) -> Result<
            Vec<boba_domain::source::SourceRunRecord>,
            boba_app::ports::repo::sources::SourcesRepoError,
        > {
            Ok(Vec::new())
        }

        fn finish_run(
            &self,
            _id: &str,
            _finished_at: &str,
            _status: boba_domain::source::RunStatus,
            _postings_found: i64,
            _error: Option<&str>,
        ) -> Result<(), boba_app::ports::repo::sources::SourcesRepoError> {
            unreachable!("the run history is the use case's, not this test's")
        }
    }

    fn live(sources: Arc<CheckedSources>) -> LiveSources {
        let http = HttpClient::new(boba_sources::http::HttpClientConfig {
            min_interval: std::time::Duration::from_millis(1),
            retry_base_delay: std::time::Duration::from_millis(1),
            ..boba_sources::http::HttpClientConfig::default()
        })
        .expect("client");
        LiveSources::new(Arc::new(http), sources, Arc::new(MemorySettings::default()))
    }

    /// S11.1b: a session board joins the run only when opted in and enabled;
    /// otherwise it is left out, never reported as an impossible failed Source.
    #[test]
    fn a_session_board_runs_only_when_opted_in_and_enabled() {
        let opted = vec!["linkedin".to_string()];
        let all_enabled = |_id: &str| true;
        let none_enabled = |_id: &str| false;
        assert!(session_board_runs("linkedin", &opted, &all_enabled));
        assert!(
            !session_board_runs("indeed", &opted, &all_enabled),
            "never opted in: left out, not failed"
        );
        assert!(
            !session_board_runs("linkedin", &opted, &none_enabled),
            "disabled by the user"
        );
        assert!(!session_board_runs("linkedin", &[], &all_enabled));
    }

    /// F1: a failed read of the source rows fails the run closed instead of
    /// running every Source, disabled ones included.
    #[test]
    fn a_source_row_read_error_fails_the_run_closed() {
        let sources = Arc::new(CheckedSources {
            list: Err(()),
            get: Ok(()),
            upserts: std::sync::atomic::AtomicUsize::new(0),
        });
        let live = live(sources);

        let error = live.sources().expect_err("a failed read is an error");
        assert!(error.contains("disk"), "{error}");
    }

    /// F2: an unreadable `source` row is left alone — never recreated, which
    /// would re-enable a Source the user disabled.
    #[test]
    fn an_unreadable_source_row_is_never_recreated() {
        let sources = Arc::new(CheckedSources::failing_get());
        let live = live(Arc::clone(&sources));

        let listed = live.sources().expect("the sources still run");
        assert!(!listed.is_empty(), "the built-in feeds are still listed");
        assert_eq!(sources.upserts(), 0, "no row is created from a failed read");
    }

    /// F4: a retry replaces its failed lines and keeps the Sources the first
    /// run reached, under the original run's id.
    #[test]
    fn a_retry_folds_into_the_report_it_retried() {
        let previous = RunReport {
            sources: vec![
                SourceReport {
                    source_id: "remotive".to_string(),
                    name: "Remotive".to_string(),
                    outcome: SourceOutcome::Ok,
                    fetched: 4,
                    new: 3,
                    duplicates: 1,
                    error: None,
                },
                SourceReport {
                    source_id: "hn".to_string(),
                    name: "Hacker News".to_string(),
                    outcome: SourceOutcome::Failed,
                    fetched: 0,
                    new: 0,
                    duplicates: 0,
                    error: Some("HTTP 503".to_string()),
                },
            ],
            ..report("run_1", false)
        };
        let retry = RunReport {
            run_id: "retry_1".to_string(),
            retry_of: Some("run_1".to_string()),
            sources: vec![SourceReport {
                source_id: "hn".to_string(),
                name: "Hacker News".to_string(),
                outcome: SourceOutcome::Ok,
                fetched: 2,
                new: 1,
                duplicates: 1,
                error: None,
            }],
            ranked: 1,
            duration_ms: 5_000,
            ..report("retry_1", false)
        };

        let merged = merge_retry(&previous, &retry);

        assert_eq!(merged.run_id, "run_1", "one run, still the same report");
        assert_eq!(merged.sources.len(), 2, "nothing is dropped");
        assert_eq!(merged.sources[0].source_id, "remotive");
        assert_eq!(merged.sources[0].fetched, 4);
        assert_eq!(merged.sources[1].outcome, SourceOutcome::Ok);
        assert_eq!(merged.sources[1].error, None);
        assert_eq!(merged.ranked, previous.ranked + 1);
        assert_eq!(merged.finished_at, retry.finished_at);
        assert!(!merged.is_partial(), "the retry cleared the banner");
    }

    /// F6: a newer run stored while the retry was in flight is never
    /// overwritten — the retry lands as its own report instead.
    #[test]
    fn a_retry_does_not_overwrite_a_newer_run() {
        let system = MemorySettings::default();
        store_report(&system, &report("run_1", true)).expect("stored");
        store_report(&system, &report("run_2", false)).expect("stored");

        let retry = RunReport {
            run_id: "retry_1".to_string(),
            retry_of: Some("run_1".to_string()),
            ..report("retry_1", false)
        };
        let stored = store_retry(&system, "run_1", &retry).expect("stored");

        assert_eq!(stored.run_id, "retry_1", "the retry is its own report");
        assert_eq!(
            latest_report(&system).expect("latest").run_id,
            "retry_1",
            "the retry finished last, so it is the newest view"
        );
        let untouched = system
            .setting_get("run_report.report.run_2")
            .expect("read")
            .expect("the newer run is still stored");
        let unchanged: RunReport = serde_json::from_str(&untouched).expect("json");
        assert_eq!(
            unchanged,
            report("run_2", false),
            "left byte for byte alone"
        );
    }

    /// F6's happy path: the report being retried is still the newest, so the
    /// retried line replaces its failed one and the run keeps its id.
    #[test]
    fn a_retry_merges_into_the_still_newest_report() {
        let system = MemorySettings::default();
        let mut failed = report("run_1", true);
        failed.sources[0].source_id = "hn".to_string();
        store_report(&system, &failed).expect("stored");

        let ok_line = SourceReport {
            source_id: "hn".to_string(),
            name: "Hacker News".to_string(),
            outcome: SourceOutcome::Ok,
            fetched: 3,
            new: 2,
            duplicates: 1,
            error: None,
        };
        let retry = RunReport {
            run_id: "retry_1".to_string(),
            retry_of: Some("run_1".to_string()),
            sources: vec![ok_line],
            ..report("retry_1", false)
        };

        let stored = store_retry(&system, "run_1", &retry).expect("stored");
        assert_eq!(stored.run_id, "run_1");
        assert_eq!(stored.sources[0].outcome, SourceOutcome::Ok);
        assert_eq!(latest_report(&system).expect("latest"), stored);
    }

    /// AC2: a stored report is what the screen reads back.
    #[test]
    fn a_stored_report_reads_back_as_the_latest() {
        let system = MemorySettings::default();
        store_report(&system, &report("run_1", true)).expect("stored");

        let latest = latest_report(&system).expect("the report is there");
        assert_eq!(latest, report("run_1", true));
        let view = RunReportView::of(&latest);
        assert!(view.partial);
        assert_eq!(view.sources[0].outcome, "failed");
    }

    /// The history is bounded: the oldest reports are deleted, not accumulated.
    #[test]
    fn only_the_newest_reports_are_kept() {
        let system = MemorySettings::default();
        for n in 0..(MAX_REPORTS + 5) {
            store_report(&system, &report(&format!("run_{n}"), false)).expect("stored");
        }

        let index = report_index(&system);
        assert_eq!(index.len(), MAX_REPORTS);
        assert_eq!(index[0], format!("run_{}", MAX_REPORTS + 4));
        assert!(
            system
                .setting_get("run_report.report.run_0")
                .unwrap()
                .is_none()
        );
    }
}
