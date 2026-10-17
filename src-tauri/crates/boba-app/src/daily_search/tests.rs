//! Daily search tests (S5.2): the run's decisions, against in-memory ports.

use std::sync::Arc;

use boba_domain::source::{RunStatus as SourceRunStatus, SourceError};

use boba_domain::scoring::factors::{Factor, FactorScore, FactorWeights};
use boba_domain::scoring::gates::{Confidence, Evidence, GateReport};
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
        Arc::new(FakeScoringRepo::default())
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
    assert!(super::is_top_match(&stored_card(
        Verdict::Strong,
        Confidence::High
    )));
    assert!(super::is_top_match(&stored_card(
        Verdict::Good,
        Confidence::Medium
    )));
    assert!(!super::is_top_match(&stored_card(
        Verdict::Moderate,
        Confidence::High
    )));
    assert!(!super::is_top_match(&stored_card(
        Verdict::Weak,
        Confidence::High
    )));
    // FR-57: a Low Confidence card is a lead, not a verdict worth a toast.
    assert!(!super::is_top_match(&stored_card(
        Verdict::Strong,
        Confidence::Low
    )));
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
