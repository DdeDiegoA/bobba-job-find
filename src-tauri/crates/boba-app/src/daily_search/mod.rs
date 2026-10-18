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

use boba_domain::posting::Offer;
use boba_domain::scoring::factors::{ScoreInputs, SemanticScore};
use boba_domain::scoring::gates::{self, Verdict as GateVerdict};
use boba_domain::scoring::rank::{self, ScoreCardRow, Verdict as BandVerdict};
use boba_domain::source::{
    RawPosting, RunStatus as SourceRunStatus, SearchQuery, SourceError, SourceRunRecord,
};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::digest::{self, DigestInput, DigestPort, NotificationSettings, TopMatch};
use crate::embeddings::EmbeddingsService;
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

/// Whether the user has asked this run to stop (FX15).
///
/// The run checks it **between Sources only**, so a Source's turn always
/// finishes its writes: cancelling never leaves a half-written posting or a
/// `source_run` row that never closed. What the run already found is kept.
pub trait RunCancel: Send + Sync {
    fn is_cancelled(&self) -> bool;
}

impl<F: Fn() -> bool + Send + Sync> RunCancel for F {
    fn is_cancelled(&self) -> bool {
        self()
    }
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
    /// The digest the run announces (S5.3, FR-45). A host without one keeps the
    /// plain top-matches notice the run has shown since S5.2.
    digest: Option<Arc<dyn DigestPort>>,
    observer: Option<Arc<dyn RunObserver>>,
    /// F3's optional on-device scorer (S6.9, AD-8). `None` in a build without
    /// the encoder; even when present, [`EmbeddingsService::similarity`] answers
    /// `None` until the user opts in, so the factor drops out and the axis
    /// renormalises (AC2).
    embeddings: Option<Arc<EmbeddingsService>>,
    /// The user's cancel request (FX15), checked between Sources.
    cancel: Option<Arc<dyn RunCancel>>,
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
            digest: None,
            observer,
            embeddings: None,
            cancel: None,
        }
    }

    /// The user's cancel request (FX15): the run stops between Sources, keeps
    /// what it found and reports itself as cancelled.
    pub fn with_cancel(mut self, cancel: Option<Arc<dyn RunCancel>>) -> Self {
        self.cancel = cancel;
        self
    }

    /// Whether the user has cancelled this run.
    fn cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|cancel| cancel.is_cancelled())
    }

    /// F3's optional scorer (S6.9): the composition root hands the mounted
    /// embeddings use case over, and the run asks it for each Offer's semantic
    /// similarity. Without it, scoring is exactly the deterministic one.
    pub fn with_embeddings(mut self, embeddings: Option<Arc<EmbeddingsService>>) -> Self {
        self.embeddings = embeddings;
        self
    }

    /// F3 for one Offer (S6.9, AD-8): `None` when this build has no encoder,
    /// the user has not opted in, or the encoder could not answer. A failed
    /// encode never fails a run — the factor drops out and renormalises.
    fn semantic(&self, offer: &Offer, inputs: &ScoringInputs) -> Option<SemanticScore> {
        let embeddings = self.embeddings.as_ref()?;
        match embeddings.similarity(offer, &inputs.profile, &inputs.prefs) {
            Ok(similarity) => similarity,
            Err(error) => {
                tracing::warn!(%error, "embeddings: F3 skipped for this Offer");
                None
            }
        }
    }

    /// Announce the run's result as the digest (S5.3, FR-45): counts by band,
    /// the top matches the user chose to see, quiet hours and all. Without a
    /// port, the run keeps S5.2's plain top-matches notice.
    pub fn with_digest(mut self, digest: Option<Arc<dyn DigestPort>>) -> Self {
        self.digest = digest;
        self
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

        let mut cancelled = false;
        for (index, source) in available.iter().enumerate() {
            // FX15: the check is here, at the seam between Sources, never
            // inside one: its writes always finish.
            if self.cancelled() {
                cancelled = true;
                break;
            }
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

        // A cancelled run does not hand work to the engine: the user asked it
        // to stop, and the deterministic scoring below already covers what the
        // Sources that ran returned.
        let agent_tasks_used = if cancelled {
            0
        } else {
            self.run_agent_pass(&options, &new_ids, inputs.as_ref(), &mut errors)
        };
        let tally = match &inputs {
            Some(inputs) => self.score(&seen, &new_ids, inputs, &mut errors),
            None => ScoreTally::default(),
        };
        // FR-46's product rule: a run that found top matches says so once. From
        // S5.3 the announcement is the digest (counts by band, quiet hours and
        // the user's choices); a host without the digest port keeps the plain
        // top-matches notice.
        // A cancelled run stays quiet: the user stopped it, so no digest.
        if !cancelled {
            self.announce(&tally, total);
        }

        let finished_at = self.clock.now_rfc3339();
        let report = RunReport {
            run_id: options.run_id.clone(),
            retry_of: options.retry_of.clone(),
            duration_ms: duration_ms(&started_at, &finished_at),
            started_at,
            finished_at,
            sources: reports,
            vetoed: tally.vetoed,
            ranked: tally.ranked,
            agent_tasks_used,
            errors,
            cancelled,
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
    ///
    /// The cap drops the **lowest-ranked** Offers, not the newest ones: the run
    /// orders its new Offers by their deterministic rank first (offline, the
    /// same factors the scoring pass reads), then spends the budget on the best
    /// of them. Past the cap, an Offer keeps the deterministic features the
    /// scoring pass gives every Offer, and the Inbox marks it "not yet analysed"
    /// (no agent pass for it).
    fn run_agent_pass(
        &self,
        options: &RunOptions,
        new_ids: &[String],
        inputs: Option<&ScoringInputs>,
        errors: &mut Vec<String>,
    ) -> u32 {
        let Some(pass) = &options.agent else {
            return 0;
        };
        let ordered = match inputs {
            Some(inputs) => self.rank_deterministically(new_ids, inputs),
            // No Profile/Preferences could be read, so there is no rank to
            // order by: the ingestion order is still deterministic.
            None => new_ids.to_vec(),
        };
        let mut used = 0;
        for posting_id in ordered.iter().take(pass.max_tasks as usize) {
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

    /// The run's new Offers, best deterministic rank first (FR-20, S2.5).
    ///
    /// A preview, not a stored card: [`rank::score`] is pure, so ordering costs
    /// no write. The real card is stored by the scoring pass after the agent
    /// had its turn, and includes what the agent proposed. Offers the model
    /// knows nothing about have no rank and sort last; ties keep the run's
    /// ingestion order.
    fn rank_deterministically(&self, new_ids: &[String], inputs: &ScoringInputs) -> Vec<String> {
        let now = self.clock.now_rfc3339();
        let mut ranked: Vec<(usize, Option<f32>, String)> = new_ids
            .iter()
            .enumerate()
            .map(|(index, posting_id)| {
                let rank = self.preview_rank(posting_id, inputs, &now);
                (index, rank, posting_id.clone())
            })
            .collect();
        ranked.sort_by(|a, b| {
            let key = |rank: Option<f32>| rank.unwrap_or(f32::NEG_INFINITY);
            key(b.1)
                .partial_cmp(&key(a.1))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });
        ranked.into_iter().map(|(_, _, id)| id).collect()
    }

    /// One Offer's deterministic rank, or `None` when it has no card (or could
    /// not be read: ordering is best-effort, the run goes on).
    fn preview_rank(&self, posting_id: &str, inputs: &ScoringInputs, now: &str) -> Option<f32> {
        let offer = self.repos.ingestion.get_offer(posting_id).ok().flatten()?;
        let features = self.features.features(posting_id).ok()?;
        let score_inputs = ScoreInputs {
            features: &features,
            posting: &offer,
            profile: &inputs.profile,
            prefs: &inputs.prefs,
            market: &inputs.market,
            semantic: self.semantic(&offer, inputs),
            now,
        };
        rank::score(&score_inputs).map(|card| card.rank)
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
    ) -> ScoreTally {
        let service = ScoringService::new(Arc::clone(&self.repos.scoring), Arc::clone(&self.clock));
        let mut tally = ScoreTally::default();

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
            let mut gate_report = gates::run(&offer, &inputs.gates);
            // S3.6 AC1: a gap the user skipped lowers the Confidence of the
            // Gate that needed it — the answer is missing, not the rule.
            crate::profile_gaps::gaps::lower_confidence(&mut gate_report, &inputs.skipped_gaps);
            if is_new
                && gate_report
                    .results
                    .iter()
                    .any(|result| result.verdict == GateVerdict::Veto)
            {
                tally.vetoed += 1;
            }
            let features = match self.features.features(posting_id) {
                Ok(features) => features,
                Err(error) => {
                    errors.push(format!("features for {posting_id}: {error}"));
                    continue;
                }
            };
            // The digest names the Offer (S5.3): keep the two fields before the
            // request takes ownership of it.
            let named = TopMatch {
                title: offer.title.clone(),
                company: offer.company.clone(),
            };
            // F3 before the request takes the Offer (S6.9, AC1).
            let semantic = self.semantic(&offer, inputs);
            let request = ScoreRequest {
                posting_id: posting_id.clone(),
                profile_version: inputs.profile_version,
                prefs_version: inputs.prefs_version,
                features,
                posting: offer,
                profile: inputs.profile.clone(),
                prefs: inputs.prefs.clone(),
                market: inputs.market.clone(),
                semantic,
                gates: gate_report,
            };
            match service.score(request) {
                Ok(Some(card)) if is_new => {
                    tally.ranked += 1;
                    match top_band(&card) {
                        Some(TopBand::Strong) => tally.strong.push(named),
                        Some(TopBand::Good) => tally.good.push(named),
                        None => {}
                    }
                }
                Ok(Some(_)) => {}
                Ok(None) => {}
                Err(error) => errors.push(format!("scoring {posting_id}: {error}")),
            }
        }
        tally
    }

    /// Announce what the run found (FR-45, S5.3): the digest when the host has
    /// one, S5.2's plain top-matches notice otherwise.
    fn announce(&self, tally: &ScoreTally, sources_checked: u32) {
        let Some(port) = &self.digest else {
            notify_top_matches(self.notifier.as_deref(), tally.top_match_count());
            return;
        };
        let settings = NotificationSettings::from_json(
            self.repos
                .system
                .setting_get(digest::settings::SETTINGS_KEY)
                .ok()
                .flatten()
                .as_deref(),
        );
        let input = DigestInput {
            strong: tally.strong.len() as u32,
            good: tally.good.len() as u32,
            sources_checked,
            top: tally.top_matches().cloned().collect(),
            hour: digest::local_hour(&self.clock.now_rfc3339()),
        };
        digest::announce(Some(port.as_ref()), &settings, input);
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

/// What the run's scoring added, for the report and the digest (S5.2, S5.3).
#[derive(Debug, Default)]
struct ScoreTally {
    /// Offers the Gates VETOed among this run's new ones.
    vetoed: u32,
    /// New Offers that got a ScoreCard this run.
    ranked: u32,
    /// New Offers the run ranked Strong, in arrival order.
    strong: Vec<TopMatch>,
    /// New Offers the run ranked Good, in arrival order.
    good: Vec<TopMatch>,
}

impl ScoreTally {
    /// FR-46/S5.3: top matches, Strong before Good (the digest caps the list).
    fn top_matches(&self) -> impl Iterator<Item = &TopMatch> {
        self.strong.iter().chain(self.good.iter())
    }

    fn top_match_count(&self) -> u32 {
        (self.strong.len() + self.good.len()) as u32
    }
}

/// The band a top match belongs to (FR-46, S5.3's "3 Strong, 5 Good").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TopBand {
    Strong,
    Good,
}

/// FR-46: an Offer is a **top match** when its card says Strong or Good and is
/// a real judgement — a Low Confidence card is a lead, not a verdict (FR-57),
/// and a lead is not worth waking the user for.
fn top_band(row: &ScoreCardRow) -> Option<TopBand> {
    if row.card.is_lead() {
        return None;
    }
    match row.card.verdict {
        BandVerdict::Strong => Some(TopBand::Strong),
        BandVerdict::Good => Some(TopBand::Good),
        _ => None,
    }
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
