//! The daily search run (S5.2, FR-46, NFR-9, NFR-14): query every enabled
//! Source, ingest and deduplicate what they return, score what is new, and
//! report what happened.
//!
//! The run is the one place the pieces of the pipeline meet. Each piece keeps
//! its own decision and its own tests:
//!
//! * the Sources answer through [`RunSources`] — a use-case-local port, because
//!   live progress needs a turn per Source (the [`crate::ports::source::SourcePort`]
//!   search returns every Source at once and cannot report between them);
//! * ingestion and dedup are S4.3's [`crate::ingest::dedup::ingest_deduped`];
//! * feature extraction is S6.2's [`ExtractFeatures`], and scoring is S6.3's
//!   [`ScoringService`] — this module only decides *which* Offers and *with what*
//!   inputs;
//! * the end-of-run sweep is S4.4's [`crate::ingest::lifecycle::finish_run`];
//! * the `source_run` history is FR-31's, and the health fold reads it (S4.8).
//!
//! A Source that fails is one line in the report, never the end of the run
//! (AC1). The run budget bounds the agent pass (FR-20, S2.5); the postings past
//! it are still ranked from deterministic features.

pub mod inputs;
pub mod report;

use std::sync::Arc;
use std::time::Duration;

use boba_domain::scoring::gates::{self, Verdict as GateVerdict};
use boba_domain::scoring::rank::{ScoreCardRow, Verdict as BandVerdict};
use boba_domain::source::{
    RawPosting, RunStatus as SourceRunStatus, SearchQuery, SourceError, SourceRunRecord,
};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::ingest::dedup::{Outcome, ingest_deduped};
use crate::ingest::lifecycle::{self, RunWindow};
use crate::ports::agent::RunBudget;
use crate::ports::clock::ClockPort;
use crate::ports::notifier::{Notification, NotifierPort};
use crate::ports::repo::ingestion::IngestionRepo;
use crate::ports::repo::profile::ProfileRepo;
use crate::ports::repo::scoring::ScoringRepo;
use crate::ports::repo::sources::SourcesRepo;
use crate::ports::repo::system::SystemRepo;
use crate::posting_features::{ExtractFeatures, ExtractRequest};
use crate::scoring::{ScoreRequest, ScoringService};
use crate::sources::health;

pub use inputs::ScoringInputs;
pub use report::{RunReport, RunTotals, SourceOutcome, SourceReport};

/// How long one agent feature task inside a run may take.
const AGENT_TASK_WALL_CLOCK: Duration = Duration::from_secs(300);

/// How many finished `source_run` rows the health fold reads (S4.8).
const HEALTH_WINDOW: usize = 5;

/// The default number of agent feature tasks one run may spend (FR-20).
pub const DEFAULT_AGENT_TASKS: u32 = 30;

/// A Source the run may query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSource {
    pub id: String,
    pub name: String,
}

/// The Source side of a run, one turn at a time.
///
/// Each call is one Source: the run reports progress as each returns (AC1), and
/// a failure of one never touches the others. A posting that a Source cannot
/// fetch is that Source's error, not the run's.
pub trait RunSources: Send + Sync {
    /// The Sources this run should query, in the order they run.
    fn sources(&self) -> Result<Vec<RunSource>, String>;

    /// Fetch one Source for this query.
    fn search(&self, source_id: &str, query: &SearchQuery) -> Result<Vec<RawPosting>, SourceError>;
}

/// What a run tells the outside world while it works (AC1).
pub trait RunObserver: Send + Sync {
    /// The run began with `total` Sources.
    fn run_started(&self, run_id: &str, total: u32);

    /// One Source's turn ended; `done` of `total` are now finished.
    fn source_finished(&self, run_id: &str, report: &SourceReport, done: u32, total: u32);

    /// The run ended and its report is ready.
    fn run_finished(&self, report: &RunReport);
}

/// The agent pass one run may take (FR-20, S2.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentPass {
    /// A `CliKind` id, e.g. `claude_code`.
    pub cli: String,
    pub model: Option<String>,
    /// How many Offers this run may hand to the model.
    pub max_tasks: u32,
}

impl AgentPass {
    /// The default: the user's chosen engine, the PRD's per-run budget.
    pub fn new(cli: impl Into<String>, model: Option<String>) -> Self {
        Self {
            cli: cli.into(),
            model,
            max_tasks: DEFAULT_AGENT_TASKS,
        }
    }
}

/// What one run is asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOptions {
    /// The id the report and its progress events carry (the `job_run` id for a
    /// scheduled run, a fresh ULID for a retry).
    pub run_id: String,
    /// The run whose failed Sources this one retries (AC3).
    pub retry_of: Option<String>,
    /// Query only these Sources (a retry); every runnable Source otherwise.
    pub only_sources: Option<Vec<String>>,
    /// Run the agent feature pass, within its budget. `None` scores from the
    /// deterministic features only (no engine chosen, or a test run).
    pub agent: Option<AgentPass>,
}

impl RunOptions {
    /// A scheduled run over every Source.
    pub fn for_run(run_id: impl Into<String>) -> Self {
        Self {
            run_id: run_id.into(),
            retry_of: None,
            only_sources: None,
            agent: None,
        }
    }
}

/// The repos a run reads and writes, grouped so the constructor reads as one
/// thing (ADR-16: each repo has one owner area).
pub struct RunRepos {
    /// `source` and `source_run` (AD-9).
    pub sources: Arc<dyn SourcesRepo>,
    /// `posting` and `posting_source` (AD-19).
    pub ingestion: Arc<dyn IngestionRepo>,
    /// `score_card` (AD-8).
    pub scoring: Arc<dyn ScoringRepo>,
    /// The facts and their version (AD-15).
    pub profile: Arc<dyn ProfileRepo>,
    /// The stored Preferences and the settings a run reads (S3.7).
    pub system: Arc<dyn SystemRepo>,
}

/// A run failure that stops the run itself (AD-3: the UI translates the key).
#[derive(Debug, Clone, thiserror::Error)]
pub enum DailySearchError {
    /// The Source list could not be read, so there is nothing to run.
    #[error("the source list could not be read: {0}")]
    Sources(String),
    /// The stored Preferences could not be read (the query falls back to
    /// nothing, so this is fatal only when the caller asked for it).
    #[error("the stored preferences could not be read: {0}")]
    Preferences(String),
}

impl DailySearchError {
    /// The i18n key the UI shows.
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::Sources(_) => "errors.run_report.sources",
            Self::Preferences(_) => "errors.run_report.preferences",
        }
    }
}

/// The daily search use case over its ports.
pub struct DailySearch {
    sources: Arc<dyn RunSources>,
    repos: RunRepos,
    features: Arc<ExtractFeatures>,
    clock: Arc<dyn ClockPort>,
    notifier: Option<Arc<dyn NotifierPort>>,
    observer: Option<Arc<dyn RunObserver>>,
}

impl DailySearch {
    pub fn new(
        sources: Arc<dyn RunSources>,
        repos: RunRepos,
        features: Arc<ExtractFeatures>,
        clock: Arc<dyn ClockPort>,
        notifier: Option<Arc<dyn NotifierPort>>,
        observer: Option<Arc<dyn RunObserver>>,
    ) -> Self {
        Self {
            sources,
            repos,
            features,
            clock,
            notifier,
            observer,
        }
    }

    /// Run the search (AC1): every Source, then ingestion, then the sweep, then
    /// scoring, then the report.
    pub fn run(&self, options: RunOptions) -> Result<RunReport, DailySearchError> {
        let started_at = self.clock.now_rfc3339();
        let mut available = self.sources.sources().map_err(DailySearchError::Sources)?;
        if let Some(only) = &options.only_sources {
            available.retain(|source| only.iter().any(|id| id == &source.id));
        }
        let total = available.len() as u32;
        self.observe_started(&options.run_id, total);

        let query = inputs::search_query(self.repos.system.as_ref());
        let mut reports = Vec::with_capacity(available.len());
        let mut seen: Vec<String> = Vec::new();
        let mut new_ids: Vec<String> = Vec::new();
        let mut covered: Vec<String> = Vec::new();
        let mut errors: Vec<String> = Vec::new();

        for (index, source) in available.iter().enumerate() {
            let report = self.run_source(
                source,
                &query,
                &mut seen,
                &mut new_ids,
                &mut covered,
                &mut errors,
            );
            self.observe_source(&options.run_id, &report, index as u32 + 1, total);
            reports.push(report);
        }

        let ended_at = self.clock.now_rfc3339();
        // S4.4's sweep: the Sources the run covered, the window it covered them
        // in. A Source that failed says nothing about the postings it would
        // have listed, so it is not part of the window.
        if !covered.is_empty()
            && let Err(error) = lifecycle::finish_run(
                self.repos.ingestion.as_ref(),
                &RunWindow {
                    started_at: started_at.clone(),
                    ended_at: ended_at.clone(),
                    source_ids: covered,
                },
            )
        {
            errors.push(format!("lifecycle sweep: {error}"));
        }

        let inputs = match inputs::scoring_inputs(
            self.repos.profile.as_ref(),
            self.repos.system.as_ref(),
            &ended_at,
        ) {
            Ok(inputs) => Some(inputs),
            Err(error) => {
                errors.push(format!("scoring inputs: {error}"));
                None
            }
        };

        let agent_tasks_used = self.run_agent_pass(&options, &new_ids, &mut errors);
        let (vetoed, ranked, top_matches) = match &inputs {
            Some(inputs) => self.score(&seen, &new_ids, inputs, &mut errors),
            None => (0, 0, 0),
        };
        // FR-46's product rule: a run that found top matches says so once.
        notify_top_matches(self.notifier.as_deref(), top_matches);

        let finished_at = self.clock.now_rfc3339();
        let report = RunReport {
            run_id: options.run_id.clone(),
            retry_of: options.retry_of.clone(),
            duration_ms: duration_ms(&started_at, &finished_at),
            started_at,
            finished_at,
            sources: reports,
            vetoed,
            ranked,
            agent_tasks_used,
            errors,
        };
        self.observe_finished(&report);
        Ok(report)
    }

    /// One Source's turn: its `source_run` row, its postings through dedup, and
    /// its line in the report.
    fn run_source(
        &self,
        source: &RunSource,
        query: &SearchQuery,
        seen: &mut Vec<String>,
        new_ids: &mut Vec<String>,
        covered: &mut Vec<String>,
        errors: &mut Vec<String>,
    ) -> SourceReport {
        let started_at = self.clock.now_rfc3339();
        let run_id = format!("sr_{}", ulid::Ulid::generate());
        let opened = self.repos.sources.start_run(&SourceRunRecord {
            id: run_id.clone(),
            source_id: source.id.clone(),
            status: SourceRunStatus::Running,
            started_at: started_at.clone(),
            finished_at: None,
            postings_found: 0,
            error: None,
        });
        if let Err(error) = opened {
            errors.push(format!("{}: {error}", source.id));
        }

        match self.sources.search(&source.id, query) {
            Ok(postings) => {
                let mut new = 0;
                let mut duplicates = 0;
                let now = self.clock.now_rfc3339();
                for raw in &postings {
                    match ingest_deduped(self.repos.ingestion.as_ref(), raw, &source.id, &now) {
                        Ok(Outcome::New { posting_id }) => {
                            new += 1;
                            remember(seen, &posting_id);
                            remember(new_ids, &posting_id);
                        }
                        Ok(Outcome::Merged { posting_id, .. }) => {
                            duplicates += 1;
                            remember(seen, &posting_id);
                        }
                        Err(error) => errors.push(format!("{}: {error}", source.id)),
                    }
                }
                let fetched = postings.len() as u32;
                self.close_source_run(&run_id, source, SourceRunStatus::Ok, fetched, None, errors);
                covered.push(source.id.clone());
                SourceReport {
                    source_id: source.id.clone(),
                    name: source.name.clone(),
                    outcome: SourceOutcome::Ok,
                    fetched,
                    new,
                    duplicates,
                    error: None,
                }
            }
            Err(error) => {
                let reason = error.to_string();
                self.close_source_run(
                    &run_id,
                    source,
                    SourceRunStatus::Failed,
                    0,
                    Some(&reason),
                    errors,
                );
                SourceReport {
                    source_id: source.id.clone(),
                    name: source.name.clone(),
                    outcome: SourceOutcome::Failed,
                    fetched: 0,
                    new: 0,
                    duplicates: 0,
                    error: Some(reason),
                }
            }
        }
    }

    /// Finish a `source_run` row and notify exactly the run that turns a streak
    /// into three failures (S4.8 AC3).
    fn close_source_run(
        &self,
        run_id: &str,
        source: &RunSource,
        status: SourceRunStatus,
        postings_found: u32,
        error: Option<&str>,
        errors: &mut Vec<String>,
    ) {
        let finished_at = self.clock.now_rfc3339();
        // The history *before* this run: the streak rule counts the runs that
        // already ended, and this row is the one just finishing.
        let history = match self.repos.sources.recent_runs(&source.id, HEALTH_WINDOW) {
            Ok(runs) => runs,
            Err(error) => {
                errors.push(format!("{}: {error}", source.id));
                Vec::new()
            }
        };
        if let Err(error) = self.repos.sources.finish_run(
            run_id,
            &finished_at,
            status,
            i64::from(postings_found),
            error,
        ) {
            errors.push(format!("{}: {error}", source.id));
        }
        if let Some(notifier) = &self.notifier {
            health::notify_if_streak_ended(notifier.as_ref(), &history, status);
        }
    }

    /// The agent feature pass, bounded by the run's budget (FR-20, S2.5).
    fn run_agent_pass(
        &self,
        options: &RunOptions,
        new_ids: &[String],
        errors: &mut Vec<String>,
    ) -> u32 {
        let Some(pass) = &options.agent else {
            return 0;
        };
        let mut used = 0;
        for posting_id in new_ids.iter().take(pass.max_tasks as usize) {
            used += 1;
            let request = ExtractRequest {
                cli: pass.cli.clone(),
                model: pass.model.clone(),
                budget: RunBudget {
                    wall_clock: Some(AGENT_TASK_WALL_CLOCK),
                    ..RunBudget::default()
                },
            };
            if let Err(error) = self.features.run(posting_id, request) {
                errors.push(format!("agent features for {posting_id}: {error}"));
            }
        }
        used
    }

    /// Gates and cards for every Offer the run saw (E6). A vetoed Offer is
    /// still scored: the Inbox shows it under Excluded, with its card's Gates.
    ///
    /// Every seen Offer is scored, not only the new ones: a posting that
    /// predates this slice, or whose earlier scoring failed, still needs a card,
    /// and `ScoringService::score` answers a current card from the store (FR-61)
    /// without re-computing it. The report's counters, though, describe **this
    /// run's new Offers** (`new_ids`): `vetoed` and `ranked` are what the run
    /// added, not a re-tally of the whole inbox (reviewer-1 F5).
    fn score(
        &self,
        posting_ids: &[String],
        new_ids: &[String],
        inputs: &ScoringInputs,
        errors: &mut Vec<String>,
    ) -> (u32, u32, u32) {
        let service = ScoringService::new(Arc::clone(&self.repos.scoring), Arc::clone(&self.clock));
        let mut vetoed = 0;
        let mut ranked = 0;
        let mut top_matches = 0;

        for posting_id in posting_ids {
            let is_new = new_ids.iter().any(|new| new == posting_id);
            let offer = match self.repos.ingestion.get_offer(posting_id) {
                Ok(Some(offer)) => offer,
                Ok(None) => continue,
                Err(error) => {
                    errors.push(format!("{posting_id}: {error}"));
                    continue;
                }
            };
            let gate_report = gates::run(&offer, &inputs.gates);
            if is_new
                && gate_report
                    .results
                    .iter()
                    .any(|result| result.verdict == GateVerdict::Veto)
            {
                vetoed += 1;
            }
            let features = match self.features.features(posting_id) {
                Ok(features) => features,
                Err(error) => {
                    errors.push(format!("features for {posting_id}: {error}"));
                    continue;
                }
            };
            let request = ScoreRequest {
                posting_id: posting_id.clone(),
                profile_version: inputs.profile_version,
                prefs_version: inputs.prefs_version,
                features,
                posting: offer,
                profile: inputs.profile.clone(),
                prefs: inputs.prefs.clone(),
                market: inputs.market.clone(),
                semantic: None,
                gates: gate_report,
            };
            match service.score(request) {
                Ok(Some(card)) if is_new => {
                    ranked += 1;
                    if is_top_match(&card) {
                        top_matches += 1;
                    }
                }
                Ok(Some(_)) => {}
                Ok(None) => {}
                Err(error) => errors.push(format!("scoring {posting_id}: {error}")),
            }
        }
        (vetoed, ranked, top_matches)
    }

    fn observe_started(&self, run_id: &str, total: u32) {
        if let Some(observer) = &self.observer {
            observer.run_started(run_id, total);
        }
    }

    fn observe_source(&self, run_id: &str, report: &SourceReport, done: u32, total: u32) {
        if let Some(observer) = &self.observer {
            observer.source_finished(run_id, report, done, total);
        }
    }

    fn observe_finished(&self, report: &RunReport) {
        if let Some(observer) = &self.observer {
            observer.run_finished(report);
        }
    }
}

/// FR-46: an Offer is a **top match** when its card says Strong or Good and is
/// a real judgement — a Low Confidence card is a lead, not a verdict (FR-57),
/// and a lead is not worth waking the user for.
fn is_top_match(row: &ScoreCardRow) -> bool {
    !row.card.is_lead() && matches!(row.card.verdict, BandVerdict::Strong | BandVerdict::Good)
}

/// Show the run's one top-matches notification (FR-46). Nothing when the run
/// found none, and a notifier that refuses never fails the run: the report is
/// already stored by then.
fn notify_top_matches(notifier: Option<&dyn NotifierPort>, count: u32) {
    if count == 0 {
        return;
    }
    if let Some(notifier) = notifier {
        let _ = notifier.notify(Notification::TopMatches { count });
    }
}

/// Remember an id once, in arrival order.
fn remember(ids: &mut Vec<String>, id: &str) {
    if !ids.iter().any(|seen| seen == id) {
        ids.push(id.to_string());
    }
}

/// The run's wall-clock duration, in whole milliseconds (`0` when either end
/// is unreadable, which a report never treats as an error).
fn duration_ms(started_at: &str, finished_at: &str) -> i64 {
    let Ok(start) = OffsetDateTime::parse(started_at, &Rfc3339) else {
        return 0;
    };
    let Ok(end) = OffsetDateTime::parse(finished_at, &Rfc3339) else {
        return 0;
    };
    i64::try_from((end - start).whole_milliseconds()).unwrap_or(i64::MAX)
}

#[cfg(test)]
pub(crate) mod testing;
#[cfg(test)]
mod tests;
