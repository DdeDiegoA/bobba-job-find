//! Daily search tests (S5.2): the run's decisions, against in-memory ports.

use std::sync::Arc;

use boba_domain::source::{RunStatus as SourceRunStatus, SourceError};

use boba_domain::scoring::factors::{Factor, FactorScore, FactorWeights};
use boba_domain::scoring::gates::{Confidence, Evidence, GateKind, GateReport};
use boba_domain::scoring::rank::{MODEL_VERSION, ScoreCardRow, Verdict, card as build_card};

use crate::ingest::normalize::testing::FakeIngestionRepo;
use crate::ports::agent::{AgentPort, AgentTask, CancelToken, RunHandle};
use crate::ports::clock::ClockPort;
use crate::ports::repo::ingestion::IngestionRepo;
use crate::ports::repo::profile::ProfileRepo;
use crate::ports::repo::scoring::ScoringRepo;
use crate::ports::repo::sources::SourcesRepo;
use crate::ports::repo::system::SystemRepo;
use crate::posting_features::ExtractFeatures;
use crate::profile::testing::FakeProfileRepo;
use crate::scoring::testing::FakeScoringRepo;
use crate::system::testing::FakeSystemRepo;

use super::report::{RunReport, SourceOutcome};
use super::testing::{
    RecordingNotifier, RecordingObserver, RecordingSourcesRepo, ScriptedSources, StepClock, onsite,
    raw, source,
};
use super::{DailySearch, RunObserver, RunOptions, RunRepos};
use crate::ports::notifier::Notification;

/// An `AgentPort` that never proposes anything: the deterministic features are
/// all the tests here need, and a clean run is a served pass.
#[derive(Default)]
pub struct SilentAgent {
    tasks: std::sync::Mutex<Vec<AgentTask>>,
}

impl SilentAgent {
    /// How many agent tasks the run dispatched.
    pub fn tasks(&self) -> usize {
        self.tasks.lock().unwrap().len()
    }

    /// The prompt of every task the run dispatched, in dispatch order.
    pub fn prompts(&self) -> Vec<String> {
        self.tasks
            .lock()
            .unwrap()
            .iter()
            .map(|task| task.prompt.clone())
            .collect()
    }
}

impl AgentPort for SilentAgent {
    fn run(&self, task: AgentTask) -> RunHandle {
        self.tasks.lock().unwrap().push(task);
        let (sender, events) = std::sync::mpsc::channel();
        drop(sender);
        RunHandle {
            run_id: "run_fake".to_string(),
            events,
            cancel: CancelToken::new(),
        }
    }
}

/// Everything a run is built from, so each test only spells its own part.
struct Harness {
    sources: Arc<ScriptedSources>,
    source_runs: Arc<RecordingSourcesRepo>,
    ingestion: Arc<FakeIngestionRepo>,
    observer: Arc<RecordingObserver>,
    agent: Arc<SilentAgent>,
    clock: Arc<StepClock>,
    system: Arc<FakeSystemRepo>,
    scoring: Arc<FakeScoringRepo>,
}

impl Harness {
    fn new(sources: ScriptedSources) -> Self {
        Self {
            sources: Arc::new(sources),
            source_runs: Arc::new(RecordingSourcesRepo::default()),
            ingestion: Arc::new(FakeIngestionRepo::default()),
            observer: Arc::new(RecordingObserver::default()),
            agent: Arc::new(SilentAgent::default()),
            clock: Arc::new(StepClock::new()),
            system: Arc::new(FakeSystemRepo::default()),
            scoring: Arc::new(FakeScoringRepo::default()),
        }
    }

    /// A harness whose store already holds one `setting` row.
    fn with_setting(sources: ScriptedSources, key: &str, value: &str) -> Self {
        let mut system = FakeSystemRepo::default();
        system.seed_setting(key, value);
        Self {
            system: Arc::new(system),
            ..Self::new(sources)
        }
    }

    /// A harness whose stored Preferences are the given document (S3.7's shape).
    fn with_prefs(sources: ScriptedSources, prefs: serde_json::Value) -> Self {
        let mut system = FakeSystemRepo::default();
        system.seed_setting("preferences", &prefs.to_string());
        Self {
            system: Arc::new(system),
            ..Self::new(sources)
        }
    }

    fn service(&self) -> DailySearch {
        let features = Arc::new(ExtractFeatures::new(
            Arc::clone(&self.ingestion) as Arc<dyn IngestionRepo>,
            Arc::clone(&self.profile()) as Arc<dyn ProfileRepo>,
            Arc::clone(&self.agent) as Arc<dyn AgentPort>,
            Arc::clone(&self.clock) as Arc<dyn ClockPort>,
        ));
        DailySearch::new(
            Arc::clone(&self.sources) as Arc<dyn super::RunSources>,
            RunRepos {
                sources: Arc::clone(&self.source_runs) as Arc<dyn SourcesRepo>,
                ingestion: Arc::clone(&self.ingestion) as Arc<dyn IngestionRepo>,
                scoring: Arc::clone(&self.scoring()) as Arc<dyn ScoringRepo>,
                profile: self.profile(),
                system: Arc::clone(&self.system) as Arc<dyn SystemRepo>,
            },
            features,
            Arc::clone(&self.clock) as Arc<dyn ClockPort>,
            None,
            Some(Arc::clone(&self.observer) as Arc<dyn RunObserver>),
        )
    }

    fn profile(&self) -> Arc<dyn ProfileRepo> {
        Arc::new(FakeProfileRepo::default())
    }

    fn scoring(&self) -> Arc<dyn ScoringRepo> {
        let scoring: Arc<dyn ScoringRepo> = self.scoring.clone();
        scoring
    }

    fn run(&self, options: RunOptions) -> RunReport {
        self.service().run(options).expect("the run finishes")
    }

    fn options() -> RunOptions {
        RunOptions {
            run_id: "run_1".to_string(),
            retry_of: None,
            only_sources: None,
            agent: None,
        }
    }
}

/// Remote-only Preferences: an on-site Offer is VETOed by the Location Gate.
fn remote_only_prefs() -> serde_json::Value {
    serde_json::json!({
        "version": 1,
        "values": {
            "roles": [],
            "locations": [],
            "working_languages": [],
            "work_modes": {"remote": true, "hybrid": false, "onsite": false},
            "salary_floor": null
        },
        "scores_stale": false
    })
}

/// AC1: every enabled Source is queried, its postings ingested and deduplicated,
/// and a failing Source is one line, never the end of the run.
#[test]
fn a_run_queries_every_source_and_totals_what_ingestion_did() {
    let harness = Harness::new(ScriptedSources::new([
        (
            source("remotive"),
            Ok(vec![
                raw("a1", "https://jobs.example.com/1", "Backend Engineer"),
                raw("a2", "https://jobs.example.com/2", "Data Engineer"),
            ]),
        ),
        (
            source("hn"),
            Err(SourceError::Http {
                url: "https://hn.example.com".to_string(),
                message: "HTTP 503".to_string(),
            }),
        ),
        // The same job the first Source returned: dedup merges, it never inserts
        // a second posting (AD-19).
        (
            source("himalayas"),
            Ok(vec![raw(
                "h1",
                "https://jobs.example.com/1",
                "Backend Engineer",
            )]),
        ),
    ]));

    let report = harness.run(Harness::options());

    assert_eq!(report.sources.len(), 3);
    assert_eq!(report.sources[0].source_id, "remotive");
    assert_eq!(report.sources[0].outcome, SourceOutcome::Ok);
    assert_eq!((report.sources[0].fetched, report.sources[0].new), (2, 2));
    assert_eq!(report.sources[1].source_id, "hn");
    assert_eq!(report.sources[1].outcome, SourceOutcome::Failed);
    assert!(report.sources[1].error.is_some());
    assert_eq!(report.sources[2].source_id, "himalayas");
    assert_eq!(report.sources[2].duplicates, 1);

    let totals = report.totals();
    assert_eq!(
        (
            totals.queried,
            totals.fetched,
            totals.new,
            totals.duplicates
        ),
        (3, 3, 2, 1)
    );
    assert!(report.is_partial());
    assert_eq!(
        report
            .failed_sources()
            .iter()
            .map(|source| source.source_id.as_str())
            .collect::<Vec<_>>(),
        ["hn"]
    );
}

/// AC2/FR-31: each Source's turn is a `source_run` row with its own outcome,
/// which is what the Connections tile and the health fold read.
#[test]
fn every_source_gets_its_source_run_history() {
    let harness = Harness::new(ScriptedSources::new([
        (
            source("remotive"),
            Ok(vec![raw(
                "a1",
                "https://jobs.example.com/1",
                "Backend Engineer",
            )]),
        ),
        (
            source("hn"),
            Err(SourceError::Http {
                url: "https://hn.example.com".to_string(),
                message: "HTTP 503".to_string(),
            }),
        ),
    ]));

    harness.run(Harness::options());

    let runs = harness.source_runs.runs();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].source_id, "remotive");
    assert_eq!(runs[0].status, SourceRunStatus::Ok);
    assert_eq!(runs[0].postings_found, 1);
    assert!(runs[0].finished_at.is_some());
    assert_eq!(runs[0].error, None);
    assert_eq!(runs[1].source_id, "hn");
    assert_eq!(runs[1].status, SourceRunStatus::Failed);
    assert!(runs[1].error.is_some());
}

/// AC1: the topbar's live progress — one report per Source as its turn ends,
/// against the total the run started with.
#[test]
fn the_run_reports_progress_per_source() {
    let harness = Harness::new(ScriptedSources::new([
        (
            source("remotive"),
            Ok(vec![raw(
                "a1",
                "https://jobs.example.com/1",
                "Backend Engineer",
            )]),
        ),
        (
            source("hn"),
            Err(SourceError::Http {
                url: "https://hn.example.com".to_string(),
                message: "HTTP 503".to_string(),
            }),
        ),
    ]));

    harness.run(Harness::options());

    assert_eq!(harness.observer.started(), [("run_1".to_string(), 2u32)]);
    let progress = harness.observer.sources();
    assert_eq!(progress.len(), 2);
    assert_eq!(progress[0].1, 1);
    assert_eq!(progress[0].2, 2);
    assert_eq!(progress[0].0.source_id, "remotive");
    assert_eq!(progress[1].1, 2);
    assert_eq!(progress[1].0.source_id, "hn");
    assert_eq!(progress[1].0.outcome, SourceOutcome::Failed);
    assert_eq!(harness.observer.finished_runs(), ["run_1".to_string()]);
}

/// FX15: a cancelled run stops between Sources — never mid-write — keeps what
/// it already found, and says it was cancelled instead of succeeding.
#[test]
fn a_cancelled_run_stops_between_sources_and_keeps_what_it_found() {
    let harness = Harness::new(ScriptedSources::new([
        (
            source("remotive"),
            Ok(vec![raw(
                "a1",
                "https://jobs.example.com/1",
                "Backend Engineer",
            )]),
        ),
        (
            source("hn"),
            Ok(vec![raw(
                "h1",
                "https://jobs.example.com/2",
                "Data Engineer",
            )]),
        ),
        (
            source("wwr"),
            Ok(vec![raw(
                "w1",
                "https://jobs.example.com/3",
                "Rust Engineer",
            )]),
        ),
    ]));
    // False on the first check (the run starts), true on the second (before
    // the second Source): the user cancelled after the first turn.
    let checks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let cancel = {
        let checks = Arc::clone(&checks);
        move || checks.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= 1
    };

    let report = harness
        .service()
        .with_cancel(Some(Arc::new(cancel)))
        .run(Harness::options())
        .expect("a cancelled run still reports");

    assert!(report.cancelled);
    assert_eq!(report.sources.len(), 1, "the second Source never started");
    assert_eq!(report.sources[0].source_id, "remotive");
    assert_eq!(report.totals().new, 1, "what it found is kept");
    assert_eq!(
        harness.sources.queries().len(),
        1,
        "no Source was queried after the cancel"
    );
    assert!(
        harness
            .source_runs
            .runs()
            .iter()
            .all(|row| row.finished_at.is_some()),
        "the Source that ran was closed, never left mid-write"
    );
    assert_eq!(harness.observer.finished_runs(), ["run_1".to_string()]);
}

/// FX15: a run nobody cancels is not marked cancelled.
#[test]
fn a_run_nobody_cancels_keeps_its_full_report() {
    let harness = Harness::new(ScriptedSources::new([
        (
            source("remotive"),
            Ok(vec![raw(
                "a1",
                "https://jobs.example.com/1",
                "Backend Engineer",
            )]),
        ),
        (
            source("hn"),
            Ok(vec![raw(
                "h1",
                "https://jobs.example.com/2",
                "Data Engineer",
            )]),
        ),
    ]));

    let report = harness
        .service()
        .with_cancel(Some(Arc::new(|| false)))
        .run(Harness::options())
        .expect("the run finishes");

    assert!(!report.cancelled);
    assert_eq!(report.sources.len(), 2);
}

/// AC1: the search request is built from the stored Preferences, not from
/// nothing: roles and locations reach the adapters.
#[test]
fn the_query_carries_the_stored_preferences() {
    let harness = Harness::with_prefs(
        ScriptedSources::new([(source("remotive"), Ok(Vec::new()))]),
        serde_json::json!({
            "version": 3,
            "values": {
                "roles": ["data engineer"],
                "locations": ["Madrid"],
                "working_languages": ["en"],
                "work_modes": {"remote": true, "hybrid": false, "onsite": false}
            },
            "scores_stale": false
        }),
    );

    harness.run(Harness::options());

    let queries = harness.sources.queries();
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].1.roles, ["data engineer"]);
    assert_eq!(queries[0].1.locations, ["Madrid"]);
    assert_eq!(queries[0].1.languages, ["en"]);
    assert_eq!(queries[0].1.remote, Some(true));
}

/// AC1/the run budget (S2.5): the agent pass is bounded; the postings past the
/// budget still get a deterministic card, they are just not analysed.
#[test]
fn the_agent_pass_stops_at_the_run_budget() {
    let harness = Harness::new(ScriptedSources::new([(
        source("remotive"),
        Ok(vec![
            raw("a1", "https://jobs.example.com/1", "Backend Engineer"),
            raw("a2", "https://jobs.example.com/2", "Data Engineer"),
            raw("a3", "https://jobs.example.com/3", "Platform Engineer"),
        ]),
    )]));

    let mut options = Harness::options();
    options.agent = Some(super::AgentPass {
        cli: "claude_code".to_string(),
        model: None,
        max_tasks: 2,
    });
    let report = harness.run(options);

    assert_eq!(report.agent_tasks_used, 2);
    assert_eq!(harness.agent.tasks(), 2);
    assert_eq!(report.totals().new, 3);
}

/// FR-20 (S2.5, orquestador ruling 2026-10-17): the budget cap drops the
/// lowest-ranked Offers, it does not simply take the first N the Sources
/// returned. Deterministic scoring is offline, so the run orders the new
/// Offers by their deterministic rank before the agent pass.
#[test]
fn the_agent_budget_keeps_the_best_ranked_offers() {
    // Same posting in every way but age: the stale one ranks lowest on the
    // Quality axis, so it is the one the budget leaves out.
    let mut stale = raw("a3", "https://jobs.example.com/3", "Platform Engineer");
    stale.posted_at = Some("2025-01-01T00:00:00Z".to_string());
    let harness = Harness::new(ScriptedSources::new([(
        source("remotive"),
        Ok(vec![
            raw("a1", "https://jobs.example.com/1", "Backend Engineer"),
            stale,
            raw("a2", "https://jobs.example.com/2", "Data Engineer"),
        ]),
    )]));

    let mut options = Harness::options();
    options.agent = Some(super::AgentPass {
        cli: "claude_code".to_string(),
        model: None,
        max_tasks: 2,
    });
    let report = harness.run(options);

    assert_eq!(report.agent_tasks_used, 2);
    let prompts = harness.agent.prompts();
    assert!(
        prompts
            .iter()
            .all(|prompt| !prompt.contains("Platform Engineer")),
        "the lowest-ranked Offer was analysed instead: {prompts:?}"
    );
    assert!(
        prompts
            .iter()
            .any(|prompt| prompt.contains("Backend Engineer"))
            && prompts
                .iter()
                .any(|prompt| prompt.contains("Data Engineer")),
        "the budget did not keep the best-ranked Offers: {prompts:?}"
    );
}

/// AC2: a VETOed Offer is still scored (it shows under Excluded), and the
/// report counts it.
#[test]
fn a_vetoed_offer_is_counted_and_still_ranked() {
    let harness = Harness::with_prefs(
        ScriptedSources::new([(
            source("remotive"),
            Ok(vec![onsite(
                "a1",
                "https://jobs.example.com/1",
                "Backend Engineer",
            )]),
        )]),
        remote_only_prefs(),
    );

    let report = harness.run(Harness::options());

    assert_eq!(report.vetoed, 1);
    assert_eq!(report.ranked, 1);
}

/// F5: a run's counters describe its new Offers, not a re-tally of everything
/// the store already had. Re-seeing the same posting adds no "new" count.
#[test]
fn a_rerun_does_not_recount_offers_the_store_already_had() {
    let harness = Harness::new(ScriptedSources::new([(
        source("remotive"),
        Ok(vec![raw(
            "a1",
            "https://jobs.example.com/1",
            "Backend Engineer",
        )]),
    )]));

    let first = harness.run(Harness::options());
    assert_eq!(
        (first.totals().new, first.totals().duplicates, first.ranked),
        (1, 0, 1)
    );

    let mut second_options = Harness::options();
    second_options.run_id = "run_2".to_string();
    let second = harness.run(second_options);
    assert_eq!(
        (second.totals().new, second.totals().duplicates),
        (0, 1),
        "the same posting merges instead of being new"
    );
    assert_eq!(
        (second.vetoed, second.ranked),
        (0, 0),
        "a known Offer is re-read, not recounted as this run's work"
    );
}

/// A retry queries only the Sources the first run could not reach.
#[test]
fn a_retry_queries_only_the_failed_sources() {
    let harness = Harness::new(ScriptedSources::new([
        (source("remotive"), Ok(Vec::new())),
        (source("hn"), Ok(Vec::new())),
    ]));

    let mut options = Harness::options();
    options.run_id = "run_retry".to_string();
    options.retry_of = Some("run_1".to_string());
    options.only_sources = Some(vec!["hn".to_string()]);
    let report = harness.run(options);

    assert_eq!(report.sources.len(), 1);
    assert_eq!(report.sources[0].source_id, "hn");
    assert_eq!(report.retry_of.as_deref(), Some("run_1"));
    assert_eq!(
        harness
            .sources
            .queries()
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
        ["hn"]
    );
}

/// S4.4's sweep runs at the end of the run: a posting of a Source the run
/// covered, not seen this time, accrues a missing run.
#[test]
fn the_run_sweeps_the_lifecycle_of_the_sources_it_covered() {
    let harness = Harness::new(ScriptedSources::new([(
        source("remotive"),
        Ok(vec![raw(
            "a1",
            "https://jobs.example.com/1",
            "Backend Engineer",
        )]),
    )]));

    let first = harness.run(Harness::options());
    assert_eq!(first.errors, Vec::<String>::new());
    let posting = harness
        .ingestion
        .offers(10, 0)
        .expect("read")
        .into_iter()
        .next()
        .expect("one posting");

    // The gap between the run's own window and now is what marks a miss: the
    // posting was last seen at the run's start, so it is not missing yet.
    let lifecycle = harness
        .ingestion
        .lifecycle(&posting.id)
        .expect("read")
        .expect("the store fills a lifecycle row as it ingests");
    assert_eq!(lifecycle.missing_runs, 0);
}

/// AC3: the non-AI stages are bounded — the pipeline over the default Sources on
/// fixtures finishes well inside three minutes.
#[test]
fn the_non_ai_stages_finish_within_three_minutes_on_fixtures() {
    let mut script = Vec::new();
    for source_id in [
        "remotive",
        "himalayas",
        "remoteok",
        "arbeitnow",
        "wwr",
        "hn",
        "rss:jobs-example-com-feed-xml",
    ] {
        let postings = (0..60)
            .map(|n| {
                // Every fixture posting is its own job: a shared description
                // would make dedup merge them all into one (AD-19).
                boba_domain::source::RawPosting {
                    description_html: Some(format!(
                        "<p>{source_id} role {n}: Rust, PostgreSQL and Kubernetes.</p>"
                    )),
                    ..raw(
                        &format!("{source_id}-{n}"),
                        &format!("https://jobs.example.com/{source_id}/{n}"),
                        &format!("{source_id} Engineer {n}"),
                    )
                }
            })
            .collect();
        script.push((source(source_id), Ok(postings)));
    }
    let harness = Harness::new(ScriptedSources::new(script));

    let started = std::time::Instant::now();
    let report = harness.run(Harness::options());
    let elapsed = started.elapsed();

    assert_eq!(report.totals().new, 420);
    assert!(
        elapsed < std::time::Duration::from_secs(180),
        "the non-AI stages took {elapsed:?}"
    );
}

/// AC1 (S3.6): a gap interview question the user skipped lowers the
/// Confidence of the Gate that needed the answer, on the card the run stores —
/// and only that Gate, only by one step.
#[test]
fn a_skipped_gap_lowers_the_gate_confidence_on_the_stored_card() {
    let posting = || {
        let mut posting = raw("a1", "https://jobs.example.com/1", "Backend Engineer");
        posting.description_html =
            Some("<p>Must be authorized to work in the EU. We build APIs in Rust.</p>".to_string());
        posting
    };

    // Without the skip the gate is sure of itself.
    let sure = Harness::new(ScriptedSources::new([(
        source("remotive"),
        Ok(vec![posting()]),
    )]));
    sure.run(Harness::options());
    let id = sure.ingestion.offers(10, 0).expect("read")[0].id.clone();
    let card = sure.scoring.cards_for(&id).expect("read").remove(0);
    let eligibility = card.gates.gate(GateKind::Eligibility).expect("eligibility");
    assert_eq!(
        eligibility.verdict,
        boba_domain::scoring::gates::Verdict::Veto
    );
    assert_eq!(eligibility.confidence, Confidence::High);

    // A skipped eligibility question lowers exactly that Gate.
    let skipped = Harness::with_setting(
        ScriptedSources::new([(source("remotive"), Ok(vec![posting()]))]),
        "profile_gaps.skipped",
        r#"["eligibility"]"#,
    );
    skipped.run(Harness::options());
    let id = skipped.ingestion.offers(10, 0).expect("read")[0].id.clone();
    let card = skipped.scoring.cards_for(&id).expect("read").remove(0);
    let eligibility = card.gates.gate(GateKind::Eligibility).expect("eligibility");
    assert_eq!(eligibility.confidence, Confidence::Medium);
    let language = card.gates.gate(GateKind::Language).expect("language");
    assert_eq!(language.confidence, Confidence::High);
}

/// A stored card with a chosen band and confidence, so the top-match rule can
/// be read on its own.
fn stored_card(verdict: Verdict, confidence: Confidence) -> ScoreCardRow {
    let factors: Vec<FactorScore> = Factor::ALL
        .into_iter()
        .map(|factor| {
            FactorScore::scored(
                factor,
                88.0,
                FactorWeights::DEFAULT.of(factor),
                Confidence::High,
                vec![Evidence::Field("description_text".to_string())],
            )
        })
        .collect();
    let card = build_card(MODEL_VERSION, factors, &FactorWeights::DEFAULT).expect("a card");
    let mut row = ScoreCardRow::new(
        "sc_1",
        "pst_1",
        boba_domain::scoring::rank::CardVersions {
            profile_version: 1,
            prefs_version: 1,
            model_version: MODEL_VERSION,
        },
        card,
        GateReport { results: vec![] },
        "2026-10-17T08:00:00Z",
    );
    row.card.verdict = verdict;
    row.card.confidence = confidence;
    row
}

/// FR-46: only a Strong or Good card that is a real judgement is a top match.
#[test]
fn only_strong_or_good_judged_cards_are_top_matches() {
    assert_eq!(
        super::top_band(&stored_card(Verdict::Strong, Confidence::High)),
        Some(super::TopBand::Strong)
    );
    assert_eq!(
        super::top_band(&stored_card(Verdict::Good, Confidence::Medium)),
        Some(super::TopBand::Good)
    );
    assert_eq!(
        super::top_band(&stored_card(Verdict::Moderate, Confidence::High)),
        None
    );
    assert_eq!(
        super::top_band(&stored_card(Verdict::Weak, Confidence::High)),
        None
    );
    // FR-57: a Low Confidence card is a lead, not a verdict worth a toast.
    assert_eq!(
        super::top_band(&stored_card(Verdict::Strong, Confidence::Low)),
        None
    );
}

/// A `DigestPort` that keeps what it was asked to show (S5.3).
#[derive(Default)]
struct RecordingDigestPort {
    shown: std::sync::Mutex<Vec<crate::digest::Digest>>,
}

impl RecordingDigestPort {
    fn shown(&self) -> Vec<crate::digest::Digest> {
        self.shown.lock().unwrap().clone()
    }
}

impl crate::digest::DigestPort for RecordingDigestPort {
    fn show(
        &self,
        digest: &crate::digest::Digest,
    ) -> Result<(), crate::ports::notifier::NotifierError> {
        self.shown.lock().unwrap().push(digest.clone());
        Ok(())
    }
}

impl Harness {
    /// The run with a digest port (S5.3): what the run hands the notification.
    fn service_with_digest(&self, port: Arc<dyn crate::digest::DigestPort>) -> DailySearch {
        self.service().with_digest(Some(port))
    }
}

/// S5.3 AC1: a finished run hands the digest the numbers it ranked, once.
#[test]
fn a_finished_run_hands_the_digest_its_numbers_once() {
    let harness = Harness::with_setting(
        ScriptedSources::new([
            (
                source("remotive"),
                Ok(vec![raw(
                    "a1",
                    "https://jobs.example.com/1",
                    "Backend Engineer",
                )]),
            ),
            (
                source("hn"),
                Err(SourceError::Http {
                    url: "https://hn.example.com".to_string(),
                    message: "HTTP 503".to_string(),
                }),
            ),
        ]),
        "notifications.settings",
        // The fixture's card is not a top match: ask for the empty-run digest
        // so the wiring is what this test reads.
        r#"{"notify_when_empty":true}"#,
    );
    let port = Arc::new(RecordingDigestPort::default());

    harness
        .service_with_digest(Arc::clone(&port) as Arc<dyn crate::digest::DigestPort>)
        .run(Harness::options())
        .expect("the run finishes");

    let shown = port.shown();
    assert_eq!(shown.len(), 1, "one digest per run");
    assert_eq!(shown[0].sources_checked, 2);
    assert_eq!((shown[0].strong, shown[0].good), (0, 0));
    assert!(shown[0].top.is_empty());
}

/// S5.3 AC2: nothing new means no digest under the default settings.
#[test]
fn an_empty_run_with_default_settings_shows_no_digest() {
    let harness = Harness::new(ScriptedSources::new([(
        source("remotive"),
        Ok(vec![raw(
            "a1",
            "https://jobs.example.com/1",
            "Backend Engineer",
        )]),
    )]));
    let port = Arc::new(RecordingDigestPort::default());

    harness
        .service_with_digest(Arc::clone(&port) as Arc<dyn crate::digest::DigestPort>)
        .run(Harness::options())
        .expect("the run finishes");

    assert!(port.shown().is_empty());
}

/// S5.3 AC2: quiet hours silence the run's digest even when it has news to tell.
#[test]
fn quiet_hours_silence_the_digest() {
    // The run clock is fixed at 08:00 UTC; the window is built around whatever
    // that is in the machine's local time, so the test holds in any timezone.
    let hour = crate::digest::local_hour("2026-10-17T08:00:00Z");
    let settings = serde_json::json!({
        "notify_when_empty": true,
        "quiet_hours": {"enabled": true, "from": hour, "to": (hour + 1) % 24, "notify_during": false},
    })
    .to_string();
    let harness = Harness::with_setting(
        ScriptedSources::new([(
            source("remotive"),
            Ok(vec![raw(
                "a1",
                "https://jobs.example.com/1",
                "Backend Engineer",
            )]),
        )]),
        "notifications.settings",
        &settings,
    );
    let port = Arc::new(RecordingDigestPort::default());

    harness
        .service_with_digest(Arc::clone(&port) as Arc<dyn crate::digest::DigestPort>)
        .run(Harness::options())
        .expect("the run finishes");

    assert!(port.shown().is_empty());
}

/// FR-46: one notification when the run found top matches, none when it did
/// not, and a missing notifier is not a failure.
#[test]
fn the_run_notifies_top_matches_once_and_stays_quiet_without_them() {
    let notifier = RecordingNotifier::default();
    super::notify_top_matches(Some(&notifier), 0);
    assert!(notifier.shown().is_empty());

    super::notify_top_matches(Some(&notifier), 3);
    assert_eq!(notifier.shown(), [Notification::TopMatches { count: 3 }]);

    super::notify_top_matches(None, 2);
}

/// An encoder that answers a fixed similarity, so the run's F3 wiring can be
/// read without a model on disk.
#[derive(Default)]
struct FixedEncoder {
    installed: std::sync::atomic::AtomicBool,
}

impl crate::ports::embedding::EmbeddingPort for FixedEncoder {
    fn model_id(&self) -> &str {
        boba_domain::scoring::f3::MODEL_ID
    }

    fn destination(&self) -> std::path::PathBuf {
        std::path::PathBuf::from("/tmp/boba-models")
    }

    fn installed(&self) -> bool {
        self.installed.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn install(
        &self,
    ) -> Result<boba_domain::scoring::f3::ModelInstall, crate::ports::embedding::EmbeddingError>
    {
        self.installed
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(boba_domain::scoring::f3::ModelInstall {
            model_id: boba_domain::scoring::f3::MODEL_ID.to_string(),
            destination: self.destination(),
            dimension: boba_domain::scoring::f3::MODEL_DIMENSION,
        })
    }

    fn similarity(
        &self,
        _profile_text: &str,
        _offer_text: &str,
    ) -> Result<boba_domain::scoring::factors::SemanticScore, crate::ports::embedding::EmbeddingError>
    {
        boba_domain::scoring::factors::SemanticScore::new(0.82)
            .ok_or_else(|| crate::ports::embedding::EmbeddingError::Encode("bad".into()))
    }
}

/// Preferences that declare a role, so the Profile side of F3 has something to
/// encode even when the Profile itself is empty.
fn role_prefs() -> serde_json::Value {
    serde_json::json!({
        "version": 1,
        "values": {
            "roles": ["Backend Engineer"],
            "locations": [],
            "working_languages": [],
            "work_modes": {"remote": true, "hybrid": true, "onsite": true},
            "salary_floor": null
        },
        "scores_stale": false
    })
}

/// S6.9 AC1: with an opted-in encoder, the card the run stores carries F3.
#[test]
fn an_opted_in_encoder_feeds_f3_into_the_stored_card() {
    let harness = Harness::with_prefs(
        ScriptedSources::new([(
            source("remotive"),
            Ok(vec![raw(
                "a1",
                "https://jobs.example.com/1",
                "Backend Engineer",
            )]),
        )]),
        role_prefs(),
    );
    let encoder = Arc::new(FixedEncoder::default());
    let embeddings = Arc::new(crate::embeddings::EmbeddingsService::new(
        Some(Arc::clone(&encoder) as Arc<dyn crate::ports::embedding::EmbeddingPort>),
        Arc::clone(&harness.system) as Arc<dyn SystemRepo>,
        Arc::clone(&harness.clock) as Arc<dyn ClockPort>,
    ));
    embeddings.opt_in().expect("the opt-in downloads");

    harness
        .service()
        .with_embeddings(Some(embeddings))
        .run(Harness::options())
        .expect("the run finishes");

    let id = harness.ingestion.offers(10, 0).expect("read")[0].id.clone();
    let card = harness.scoring.cards_for(&id).expect("read").remove(0);
    let semantic = card.card.factor(Factor::Semantic).expect("F3 on the card");
    assert_eq!(semantic.score, Some(82.0));
}

/// S6.9 AC2/AC4: without an encoder (or before the opt-in) the card stores the
/// same F3 gap it always stored, and the axis renormalises.
#[test]
fn without_an_opted_in_encoder_f3_stays_out_of_the_card() {
    let harness = Harness::with_prefs(
        ScriptedSources::new([(
            source("remotive"),
            Ok(vec![raw(
                "a1",
                "https://jobs.example.com/1",
                "Backend Engineer",
            )]),
        )]),
        role_prefs(),
    );
    let encoder = Arc::new(FixedEncoder::default());
    let embeddings = Arc::new(crate::embeddings::EmbeddingsService::new(
        Some(Arc::clone(&encoder) as Arc<dyn crate::ports::embedding::EmbeddingPort>),
        Arc::clone(&harness.system) as Arc<dyn SystemRepo>,
        Arc::clone(&harness.clock) as Arc<dyn ClockPort>,
    ));

    harness
        .service()
        .with_embeddings(Some(embeddings))
        .run(Harness::options())
        .expect("the run finishes");

    let id = harness.ingestion.offers(10, 0).expect("read")[0].id.clone();
    let card = harness.scoring.cards_for(&id).expect("read").remove(0);
    let semantic = card.card.factor(Factor::Semantic).expect("F3 on the card");
    assert_eq!(semantic.score, None);
    assert_eq!(semantic.effective_weight, 0.0);
}
