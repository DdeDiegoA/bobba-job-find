//! Posting features use case (AC1, FR-61, AD-5): read an Offer's features
//! deterministically, let an agent run propose more, and cache both per (Offer,
//! Profile version).
//!
//! The cache is the point of the type (FR-61): [`features`] is the only read,
//! and it either returns what this Profile version already has or runs the
//! deterministic pass and stores it. Nothing here re-extracts a version it has
//! already read, and [`run`] never starts a second agent pass for one that has
//! been served.
//!
//! This service is the **single writer** of features (AD-5: the agent proposes,
//! the core disposes). A run's proposals arrive as its own tool calls on the
//! event stream and [`harvest`] stores them — the MCP tool handler cannot,
//! because what a proposal is stored *against* (the Profile version, and the
//! merge with the deterministic claims) lives here, and a handler at the tool
//! boundary has neither.
//!
//! [`features`]: ExtractFeatures::features

use std::sync::Arc;

use boba_domain::posting::Offer;
use boba_domain::posting_features::{
    AgentRunRef, Claim, FeatureError, FeatureSource, PendingPosting, PostingFeatures, Proposal,
    deterministic, merge, resolve, verified_counts,
};
use serde_json::Value;

use crate::ports::agent::{
    AgentEvent, AgentPort, AgentTask, AgentTaskKind, FailureReason, InputTrust, RunBudget,
};
use crate::ports::clock::ClockPort;
use crate::ports::repo::ingestion::{IngestionRepo, IngestionRepoError};
use crate::ports::repo::profile::{ProfileRepo, ProfileRepoError};

/// Validate one `propose_posting_features` payload at the tool boundary (AD-5):
/// the contract's shape, and every quote against the posting's own text.
///
/// Returns `(accepted, dropped)`: how many claims the core would store, and how
/// many the model sent with a quote the posting does not contain. The endpoint
/// hands an `Err` back to the agent, which is what makes a bad payload cost the
/// run one of its retries instead of being stored or silently lost; the use case
/// stores the accepted ones from the run's event stream.
pub fn validate_proposal(offer: &Offer, args: &Value) -> Result<(usize, usize), FeatureError> {
    Ok(verified_counts(
        &proposal_of(args)?.claims()?,
        &offer.description_text,
    ))
}

/// The contract object of a `propose_posting_features` call, shape-checked.
fn proposal_of(args: &Value) -> Result<Proposal, FeatureError> {
    let features = args.get("features").cloned().unwrap_or(Value::Null);
    // serde would also accept a positional sequence for a struct; the contract
    // is a named object, so anything else is refused here rather than read as
    // "the model sent nothing".
    if !features.is_object() {
        return Err(shape_error("`features` must be an object"));
    }
    serde_json::from_value(features).map_err(|error| shape_error(&error.to_string()))
}

/// The claims of one proposal whose quote is really in `offer`'s text, each
/// attributed to the run that proposed it (AD-5).
fn verified_claims(
    offer: &Offer,
    run: &AgentRunRef,
    args: &Value,
) -> Result<Vec<Claim>, FeatureError> {
    let proposed = proposal_of(args)?.claims()?;
    Ok(resolve(&proposed, &offer.description_text, run))
}

/// The posting a proposal is about, as the tool takes it.
pub fn proposal_id(args: &Value) -> Option<&str> {
    args.get("id").and_then(Value::as_str)
}

/// The tool a run sends its proposals through. The catalog
/// (`boba_mcp_proto::tool`) is the authority on tool names; this crate cannot
/// depend on it, so the wiring asserts the two agree
/// (`wiring::agent::tests::the_feature_harvest_listens_for_the_catalog_tool`).
pub const PROPOSE_TOOL: &str = "propose_posting_features";

/// How the caller wants the agent pass run: which CLI and model the user picked,
/// and the budget it may spend (architecture §5.4).
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractRequest {
    /// A `CliKind` id, e.g. `claude_code`.
    pub cli: String,
    pub model: Option<String>,
    pub budget: RunBudget,
}

/// A feature failure the UI shows through a `message_key` (AD-3).
#[derive(Debug, Clone, thiserror::Error)]
pub enum FeaturesError {
    #[error(transparent)]
    Repo(#[from] IngestionRepoError),
    #[error(transparent)]
    Profile(#[from] ProfileRepoError),
    /// The id names no stored Offer.
    #[error("unknown offer: {0}")]
    UnknownOffer(String),
    /// A `propose_posting_features` payload outside the contract. The run's
    /// agent can fix it and retry (AC2 of the endpoint's own budget).
    #[error(transparent)]
    Contract(boba_domain::posting_features::FeatureError),
    /// The run ended without a result (FR-21): auth, quota, timeout, crash. The
    /// deterministic features are stored anyway; the pass stays pending.
    #[error("the extraction run failed: {0:?}")]
    Run(FailureReason),
}

impl FeaturesError {
    /// The i18n key the UI shows.
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::Repo(_) | Self::Profile(_) => "errors.posting_features.store",
            Self::UnknownOffer(_) => "errors.posting_features.unknown_offer",
            Self::Contract(_) => "errors.posting_features.contract",
            Self::Run(_) => "errors.posting_features.run",
        }
    }
}

/// The feature use case: the ingestion and profile owners' ports, the agent
/// runtime, and the injected clock (AD-17).
pub struct ExtractFeatures {
    repo: Arc<dyn IngestionRepo>,
    profile: Arc<dyn ProfileRepo>,
    agents: Arc<dyn AgentPort>,
    clock: Arc<dyn ClockPort>,
}

impl ExtractFeatures {
    pub fn new(
        repo: Arc<dyn IngestionRepo>,
        profile: Arc<dyn ProfileRepo>,
        agents: Arc<dyn AgentPort>,
        clock: Arc<dyn ClockPort>,
    ) -> Self {
        Self {
            repo,
            profile,
            agents,
            clock,
        }
    }

    /// The features of one Offer at the current Profile version (AC1, AC2):
    /// what is cached for `(offer, version)`, or the deterministic pass — which
    /// is stored, so the next call is a cache hit (FR-61).
    pub fn features(&self, offer_id: &str) -> Result<PostingFeatures, FeaturesError> {
        let version = self.profile_version()?;
        if let Some(cached) = self.repo.features(offer_id, version)? {
            return Ok(cached);
        }
        let offer = self.offer(offer_id)?;
        let mut set = PostingFeatures::new(offer_id.to_string(), version);
        set.extend(deterministic(&offer));
        self.repo
            .save_deterministic_features(&set, &self.clock.now_rfc3339())?;
        Ok(set)
    }

    /// Extract one Offer: the deterministic pass always, then **one** agent run
    /// within its budget — never a second one for a Profile version that has
    /// been served (FR-61).
    ///
    /// A run that failed leaves the pass pending (a later run retries it) and
    /// reports why; a run that finished is harvested, and whatever it proposed
    /// is stored as [`boba_domain::posting_features::FeatureSource::Llm`] claims.
    pub fn run(
        &self,
        offer_id: &str,
        request: ExtractRequest,
    ) -> Result<PostingFeatures, FeaturesError> {
        let set = self.features(offer_id)?;
        if self
            .repo
            .features_agent_done(offer_id, set.profile_version)?
        {
            return Ok(set);
        }
        let (run, events) = self.dispatch(&self.offer(offer_id)?, request)?;
        let stored = self.harvest(&run, &events)?;
        if let Some(reason) = failure(&events) {
            return Err(FeaturesError::Run(reason));
        }
        if stored == 0 {
            // A clean run that proposed nothing is still a served pass: this
            // version must not be asked again (FR-61).
            let set = self.features(offer_id)?;
            self.repo
                .save_agent_features(&set, &self.clock.now_rfc3339())?;
        }
        self.features(offer_id)
    }

    /// Store every proposal a run made, from its event stream (AC1), tagging
    /// each claim with the run that proposed it (AD-5). Returns how many claims
    /// were stored.
    ///
    /// A proposal whose quote is not in the posting is dropped, not stored
    /// (AD-5); a payload outside the contract is an error, so the run is
    /// visible rather than silently ignored. This is also the seam a batch run
    /// uses: drain the handle's events and harvest them the same way.
    pub fn harvest(
        &self,
        run: &AgentRunRef,
        events: &[AgentEvent],
    ) -> Result<usize, FeaturesError> {
        let mut stored = 0;
        for event in events {
            let AgentEvent::ToolCall { name, input } = event else {
                continue;
            };
            if !is_proposal(name) {
                continue;
            }
            stored += self.store_proposal(run, input)?;
        }
        Ok(stored)
    }

    /// The postings still waiting for an agent pass at the current Profile
    /// version, newest first: what `get_pending_postings` serves.
    pub fn pending(&self, limit: usize) -> Result<Vec<PendingPosting>, FeaturesError> {
        Ok(self.repo.features_pending(self.profile_version()?, limit)?)
    }

    /// The current `profile.version` (AD-15), the second half of the cache key.
    pub fn profile_version(&self) -> Result<u32, FeaturesError> {
        Ok(self.profile.profile_version()?)
    }

    /// One stored Offer, or [`FeaturesError::UnknownOffer`].
    fn offer(&self, offer_id: &str) -> Result<Offer, FeaturesError> {
        self.repo
            .get_offer(offer_id)?
            .ok_or_else(|| FeaturesError::UnknownOffer(offer_id.to_string()))
    }

    /// Run the task and collect its events, with the identity every claim it
    /// proposes will carry (AD-5). The run is synchronous: this use case has
    /// nothing else to do while it waits, and every later stage wants the
    /// features, not a handle.
    fn dispatch(
        &self,
        offer: &Offer,
        request: ExtractRequest,
    ) -> Result<(AgentRunRef, Vec<AgentEvent>), FeaturesError> {
        let task = AgentTask {
            kind: AgentTaskKind::ExtractPostings,
            // A posting is third-party text (ADR-010): the run may only start on
            // a CLI with a verified tool lockdown (S2.3b's gate, not this one).
            input_trust: InputTrust::Untrusted,
            cli: request.cli,
            model: request.model,
            prompt: task_input(offer),
            target_language: offer.lang.clone(),
            budget: request.budget,
            env: Vec::new(),
        };
        let (cli, model) = (task.cli.clone(), task.model.clone());
        let handle = self.agents.run(task);
        // The id and CLI the runtime opened the run with, so a stored claim
        // points at the `agent_run` row that produced it (AD-5).
        let run = AgentRunRef {
            run_id: handle.run_id.clone(),
            cli,
            model,
        };
        Ok((run, handle.events.iter().collect()))
    }

    /// Store one `propose_posting_features` call: read the posting it is about,
    /// verify every quote against that posting's text, merge the survivors with
    /// what the core already knows, and write them as the run's claims (AC1).
    fn store_proposal(&self, run: &AgentRunRef, args: &Value) -> Result<usize, FeaturesError> {
        let id = proposal_id(args)
            .ok_or_else(|| FeaturesError::Contract(missing_id()))?
            .to_string();
        // The same check the tool boundary runs, here against the stored posting:
        // an unverifiable quote stores nothing (AD-5).
        let offer = self.offer(&id)?;
        let verified = verified_claims(&offer, run, args).map_err(FeaturesError::Contract)?;
        if verified.is_empty() {
            return Ok(0);
        }
        let set = self.features(&id)?;
        let mut merged = PostingFeatures::new(id, set.profile_version);
        merged.extend(merge(set.claims.clone(), verified));
        self.repo
            .save_agent_features(&merged, &self.clock.now_rfc3339())?;
        Ok(merged
            .claims
            .iter()
            .filter(|claim| claim.source == FeatureSource::Llm)
            .count())
    }
}

/// The run's own input: the posting, as data (AD-7). The id travels with the
/// text so the run can name the posting in its proposal.
fn task_input(offer: &Offer) -> String {
    serde_json::json!({
        "id": offer.id,
        "lang": offer.lang,
        "title": offer.title,
        "company": offer.company,
        "text": offer.description_text,
    })
    .to_string()
}

/// Whether a tool call is this use case's proposal. Drivers report MCP calls
/// under the shim's own prefix (`mcp__boba__…`), so the name is matched by its
/// last segment.
fn is_proposal(name: &str) -> bool {
    name.rsplit("__").next() == Some(PROPOSE_TOOL)
}

/// The failure a run ended with, if any.
fn failure(events: &[AgentEvent]) -> Option<FailureReason> {
    events.iter().find_map(|event| match event {
        AgentEvent::Failed { reason, .. } => Some(*reason),
        _ => None,
    })
}

fn missing_id() -> FeatureError {
    shape_error("a proposal needs the posting's `id`")
}

fn shape_error(why: &str) -> FeatureError {
    FeatureError::Shape {
        why: why.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::normalize::testing::FakeIngestionRepo;
    use crate::profile::testing::FakeProfileRepo;
    use crate::system::FixedClock;
    use boba_domain::posting::{ApplyHints, Provenance, WorkMode};
    use std::sync::Mutex;
    use std::sync::mpsc::{Receiver, channel};
    use std::time::Duration;

    /// An `AgentPort` that answers from a script: the events one run streams, in
    /// order, and every task it was handed.
    #[derive(Default)]
    struct FakeAgent {
        events: Mutex<Vec<AgentEvent>>,
        tasks: Mutex<Vec<AgentTask>>,
    }

    impl FakeAgent {
        fn scripted(events: Vec<AgentEvent>) -> Self {
            Self {
                events: Mutex::new(events),
                tasks: Mutex::new(Vec::new()),
            }
        }

        fn tasks(&self) -> Vec<AgentTask> {
            self.tasks.lock().unwrap().clone()
        }

        fn runs(&self) -> usize {
            self.tasks.lock().unwrap().len()
        }
    }

    impl AgentPort for FakeAgent {
        fn run(&self, task: AgentTask) -> crate::ports::agent::RunHandle {
            self.tasks.lock().unwrap().push(task);
            let (sender, events) = channel();
            for event in self.events.lock().unwrap().iter() {
                let _ = sender.send(event.clone());
            }
            drop(sender);
            crate::ports::agent::RunHandle {
                run_id: "run_test".to_string(),
                events: events as Receiver<AgentEvent>,
                cancel: crate::ports::agent::CancelToken::new(),
            }
        }
    }

    fn offer(id: &str, title: &str, body: &str) -> Offer {
        Offer {
            id: id.to_string(),
            source_id: "manual".to_string(),
            external_id: id.to_string(),
            title: title.to_string(),
            company: "Boba Labs".to_string(),
            locations: Vec::new(),
            work_mode: WorkMode::Unknown,
            money: None,
            posted_at: None,
            deadline: None,
            first_seen: "2026-10-16T00:00:00Z".to_string(),
            last_seen: "2026-10-16T00:00:00Z".to_string(),
            lang: "en".to_string(),
            description_html: String::new(),
            description_text: body.to_string(),
            requirements: Vec::new(),
            apply: ApplyHints {
                url: "https://example.com/job".to_string(),
                email: None,
            },
        }
    }

    fn seeded(offers: &[Offer]) -> Arc<FakeIngestionRepo> {
        let repo = Arc::new(FakeIngestionRepo::default());
        for one in offers {
            repo.insert_offer(
                one,
                &Provenance {
                    posting_id: one.id.clone(),
                    source_id: "manual".to_string(),
                    external_id: one.external_id.clone(),
                    url: one.apply.url.clone(),
                    raw: "{}".to_string(),
                    fetched_at: "2026-10-16T00:00:00Z".to_string(),
                },
            )
            .expect("seed an offer");
        }
        repo
    }

    /// The service under test, with the handle a test asserts through.
    fn service(
        repo: &Arc<FakeIngestionRepo>,
        agents: Arc<FakeAgent>,
    ) -> (ExtractFeatures, Arc<FakeProfileRepo>) {
        let profile = Arc::new(FakeProfileRepo::default());
        (
            ExtractFeatures::new(
                repo.clone(),
                profile.clone(),
                agents,
                Arc::new(FixedClock::new("2026-10-16T12:00:00.000Z")),
            ),
            profile,
        )
    }

    fn request() -> ExtractRequest {
        ExtractRequest {
            cli: "claude_code".to_string(),
            model: Some("haiku".to_string()),
            budget: RunBudget {
                wall_clock: Some(Duration::from_secs(60)),
                max_tool_calls: Some(10),
                ..RunBudget::default()
            },
        }
    }

    /// The identity a harvested claim must carry (AD-5): the run the fake port
    /// hands back is `run_test`, and the request names the CLI and model.
    fn run() -> AgentRunRef {
        AgentRunRef {
            run_id: "run_test".to_string(),
            cli: "claude_code".to_string(),
            model: Some("haiku".to_string()),
        }
    }

    fn proposal(id: &str, body: serde_json::Value) -> AgentEvent {
        AgentEvent::ToolCall {
            name: "mcp__boba__propose_posting_features".to_string(),
            input: serde_json::json!({"id": id, "features": body}),
        }
    }

    fn finished() -> AgentEvent {
        AgentEvent::Finished { exit_code: 0 }
    }

    #[test]
    fn a_cold_offer_is_read_deterministically_and_cached() {
        let repo = seeded(&[offer("pst_1", "Senior Rust Engineer", "Rust and SQL.")]);
        let (service, _) = service(&repo, Arc::new(FakeAgent::default()));

        let first = service.features("pst_1").unwrap();
        assert_eq!(
            first.values(boba_domain::posting_features::FeatureKind::Skill),
            vec!["Rust", "SQL"]
        );
        assert_eq!(
            first.profile_version, 0,
            "profile.version starts at 0 (AD-15)"
        );
        assert_eq!(repo.deterministic_saves(), 1);

        let second = service.features("pst_1").unwrap();
        assert_eq!(
            second, first,
            "the same inputs give the same features (FR-61)"
        );
        assert_eq!(
            repo.deterministic_saves(),
            1,
            "a cached version is not read again"
        );
    }

    #[test]
    fn the_deterministic_pass_fills_the_postings_own_requirements() {
        let repo = seeded(&[offer("pst_1", "Engineer", "Rust and TypeScript.")]);
        let (service, _) = service(&repo, Arc::new(FakeAgent::default()));

        service.features("pst_1").unwrap();

        let stored = repo.get_offer("pst_1").unwrap().expect("stored");
        assert_eq!(stored.requirements, vec!["Rust", "TypeScript"]);
    }

    #[test]
    fn a_new_profile_version_is_read_again() {
        let repo = seeded(&[offer("pst_1", "Engineer", "Rust.")]);
        let (service, profile) = service(&repo, Arc::new(FakeAgent::default()));

        assert_eq!(service.features("pst_1").unwrap().profile_version, 0);
        profile.set_version(3);
        let reread = service.features("pst_1").unwrap();
        assert_eq!(
            reread.profile_version, 3,
            "the cache key is (offer, version)"
        );
        assert_eq!(repo.deterministic_saves(), 2);
    }

    #[test]
    fn an_unknown_offer_is_refused() {
        let repo = seeded(&[]);
        let (service, _) = service(&repo, Arc::new(FakeAgent::default()));
        assert!(matches!(
            service.features("pst_missing").unwrap_err(),
            FeaturesError::UnknownOffer(id) if id == "pst_missing"
        ));
    }

    #[test]
    fn a_clean_run_harvests_its_proposals() {
        let repo = seeded(&[offer("pst_1", "Engineer", "We build APIs in Rust.")]);
        let agents = Arc::new(FakeAgent::scripted(vec![
            proposal(
                "pst_1",
                serde_json::json!({
                    "skills": [{"value": "Kubernetes", "quote": "We build APIs in Rust."}],
                    "seniority": {"value": "mid", "quote": "We build APIs in Rust."}
                }),
            ),
            finished(),
        ]));
        let (service, _) = service(&repo, agents.clone());

        let set = service.run("pst_1", request()).unwrap();

        assert!(set.has_llm(), "{set:?}");
        assert_eq!(
            set.values(boba_domain::posting_features::FeatureKind::Skill),
            vec!["Kubernetes", "Rust"]
        );
        assert_eq!(
            set.first(boba_domain::posting_features::FeatureKind::Seniority)
                .map(|claim| claim.value.as_str()),
            Some("mid")
        );
        assert_eq!(
            repo.stored_claims("pst_1", 0).len(),
            3,
            "the deterministic claims and the run's both landed"
        );
        assert!(repo.features_agent_done("pst_1", 0).unwrap());

        // AD-5: every claim the run proposed names the run, CLI and model behind
        // it — threaded from the run handle through the event stream.
        let agent_claims: Vec<Claim> = repo
            .stored_claims("pst_1", 0)
            .into_iter()
            .filter(|claim| claim.source == FeatureSource::Llm)
            .collect();
        assert_eq!(agent_claims.len(), 2, "{agent_claims:?}");
        for claim in agent_claims {
            assert_eq!(claim.run(), Some(&run()));
        }
        assert!(
            repo.stored_claims("pst_1", 0)
                .iter()
                .filter(|claim| claim.source == FeatureSource::Det)
                .all(|claim| claim.run().is_none()),
            "the core's own reads have no run"
        );
    }

    #[test]
    fn the_run_is_handed_the_posting_text_inside_the_untrusted_contract() {
        let repo = seeded(&[offer("pst_1", "Backend Engineer", "Rust everywhere.")]);
        let agents = Arc::new(FakeAgent::scripted(vec![finished()]));
        let (service, _) = service(&repo, agents.clone());

        service.run("pst_1", request()).unwrap();

        let task = agents.tasks().pop().expect("one run");
        assert_eq!(task.kind, AgentTaskKind::ExtractPostings);
        assert_eq!(task.input_trust, InputTrust::Untrusted);
        assert!(
            task.reads_untrusted(),
            "a posting is third-party text (AD-7)"
        );
        assert_eq!(task.cli, "claude_code");
        assert_eq!(task.model.as_deref(), Some("haiku"));
        assert_eq!(
            task.target_language, "en",
            "the posting's language, not the UI's"
        );
        assert_eq!(task.budget.max_tool_calls, Some(10), "the caller's budget");
        let input: Value = serde_json::from_str(&task.prompt).expect("the run's own input is JSON");
        assert_eq!(input["id"], "pst_1");
        assert_eq!(input["text"], "Rust everywhere.");
    }

    #[test]
    fn a_served_version_is_never_run_again() {
        let repo = seeded(&[offer("pst_1", "Engineer", "Rust.")]);
        let agents = Arc::new(FakeAgent::scripted(vec![
            proposal(
                "pst_1",
                serde_json::json!({"skills": [{"value": "Docker", "quote": "Rust."}]}),
            ),
            finished(),
        ]));
        let (service, _) = service(&repo, agents.clone());

        let first = service.run("pst_1", request()).unwrap();
        let second = service.run("pst_1", request()).unwrap();

        assert_eq!(agents.runs(), 1, "one run per (offer, profile version)");
        assert_eq!(second, first);
        assert!(second.has_llm());
    }

    #[test]
    fn a_clean_run_that_proposed_nothing_still_serves_the_version() {
        let repo = seeded(&[offer("pst_1", "Engineer", "Competitive salary.")]);
        let agents = Arc::new(FakeAgent::scripted(vec![finished()]));
        let (service, _) = service(&repo, agents.clone());

        let set = service.run("pst_1", request()).unwrap();

        assert!(!set.has_llm());
        assert!(repo.features_agent_done("pst_1", 0).unwrap());
        service.run("pst_1", request()).unwrap();
        assert_eq!(
            agents.runs(),
            1,
            "an empty answer is still an answer (FR-61)"
        );
        assert!(service.pending(10).unwrap().is_empty());
    }

    #[test]
    fn a_failed_run_keeps_the_deterministic_features_and_stays_pending() {
        let repo = seeded(&[offer("pst_1", "Engineer", "Rust.")]);
        let agents = Arc::new(FakeAgent::scripted(vec![AgentEvent::Failed {
            reason: FailureReason::AuthExpired,
            message_key: "errors.agent.auth_expired".to_string(),
        }]));
        let (service, _) = service(&repo, agents);

        let error = service.run("pst_1", request()).unwrap_err();

        assert!(matches!(
            error,
            FeaturesError::Run(FailureReason::AuthExpired)
        ));
        assert_eq!(error.message_key(), "errors.posting_features.run");
        assert_eq!(
            service
                .features("pst_1")
                .unwrap()
                .values(boba_domain::posting_features::FeatureKind::Skill),
            vec!["Rust"],
            "what the core read itself is kept"
        );
        assert!(!repo.features_agent_done("pst_1", 0).unwrap());
        assert_eq!(
            service.pending(10).unwrap().len(),
            1,
            "a later run retries it"
        );
    }

    #[test]
    fn a_proposal_whose_quote_is_not_in_the_posting_is_dropped() {
        let repo = seeded(&[offer("pst_1", "Engineer", "Competitive salary.")]);
        let agents = Arc::new(FakeAgent::scripted(vec![
            proposal(
                "pst_1",
                serde_json::json!({"skills": [{"value": "Rust", "quote": "Rust services!"}]}),
            ),
            finished(),
        ]));
        let (service, _) = service(&repo, agents);

        let set = service.run("pst_1", request()).unwrap();

        assert!(
            !set.has_llm(),
            "an unverifiable quote stores nothing: {set:?}"
        );
        assert!(repo.stored_claims("pst_1", 0).is_empty());
        assert!(repo.features_agent_done("pst_1", 0).unwrap());
    }

    #[test]
    fn a_proposal_outside_the_contract_is_an_error_and_marks_nothing() {
        let repo = seeded(&[offer("pst_1", "Engineer", "Rust.")]);
        let agents = Arc::new(FakeAgent::scripted(vec![
            proposal("pst_1", serde_json::json!({"salary": []})),
            finished(),
        ]));
        let (service, _) = service(&repo, agents);

        let error = service.run("pst_1", request()).unwrap_err();

        assert!(matches!(error, FeaturesError::Contract(_)), "{error}");
        assert_eq!(error.message_key(), "errors.posting_features.contract");
        assert!(!repo.features_agent_done("pst_1", 0).unwrap());
    }

    #[test]
    fn a_batch_of_proposals_is_harvested_as_one_report() {
        let repo = seeded(&[
            offer("pst_1", "Engineer", "Rust and SQL."),
            offer("pst_2", "Designer", "Figma and research."),
        ]);
        let (service, _) = service(&repo, Arc::new(FakeAgent::default()));

        let stored = service
            .harvest(
                &run(),
                &[
                proposal(
                    "pst_1",
                    serde_json::json!({"skills": [{"value": "Kubernetes", "quote": "Rust and SQL."}]}),
                ),
                    proposal(
                        "pst_2",
                        serde_json::json!({"notes": [{"value": "remote", "quote": "Figma and research."}]}),
                    ),
                ],
            )
            .unwrap();

        assert_eq!(stored, 2);
        assert!(repo.features_agent_done("pst_1", 0).unwrap());
        assert!(repo.features_agent_done("pst_2", 0).unwrap());
        assert_eq!(repo.stored_claims("pst_2", 0).len(), 1);
    }

    #[test]
    fn pending_lists_what_awaits_a_pass_newest_first() {
        let mut older = offer("pst_1", "Engineer", "Rust.");
        older.first_seen = "2026-10-01T00:00:00Z".to_string();
        let repo = seeded(&[older, offer("pst_2", "Designer", "Figma.")]);
        let (service, _) = service(&repo, Arc::new(FakeAgent::default()));

        let pending = service.pending(1).unwrap();

        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "pst_2");
        assert_eq!(pending[0].text, "Figma.");
        assert_eq!(service.pending(10).unwrap().len(), 2);
    }

    #[test]
    fn the_tool_boundary_refuses_a_payload_outside_the_contract() {
        let stable = offer("pst_1", "Engineer", "Rust and SQL.");

        let (accepted, dropped) = validate_proposal(
            &stable,
            &serde_json::json!({
                "id": "pst_1",
                "features": {"skills": [{"value": "Kubernetes", "quote": "Rust and SQL."}]}
            }),
        )
        .unwrap();
        assert_eq!((accepted, dropped), (1, 0));

        let (accepted, dropped) = validate_proposal(
            &stable,
            &serde_json::json!({
                "id": "pst_1",
                "features": {"skills": [{"value": "Kubernetes", "quote": "nowhere in the posting"}]}
            }),
        )
        .unwrap();
        assert_eq!(
            (accepted, dropped),
            (0, 1),
            "an unfound quote is reported, not stored"
        );

        assert!(matches!(
            validate_proposal(&stable, &serde_json::json!({"id": "pst_1", "features": []})),
            Err(FeatureError::Shape { .. })
        ));
        assert_eq!(
            proposal_id(&serde_json::json!({"id": "pst_1"})),
            Some("pst_1")
        );
        assert_eq!(proposal_id(&serde_json::json!({})), None);
    }

    #[test]
    fn a_tool_call_that_is_not_a_proposal_is_left_alone() {
        let repo = seeded(&[offer("pst_1", "Engineer", "Rust.")]);
        let (service, _) = service(&repo, Arc::new(FakeAgent::default()));

        let stored = service
            .harvest(
                &run(),
                &[
                    AgentEvent::ToolCall {
                        name: "mcp__boba__get_pending_postings".to_string(),
                        input: serde_json::json!({"limit": 5}),
                    },
                    AgentEvent::Text {
                        text: "done".to_string(),
                    },
                ],
            )
            .unwrap();

        assert_eq!(stored, 0);
        assert!(repo.stored_claims("pst_1", 0).is_empty());
    }
}
