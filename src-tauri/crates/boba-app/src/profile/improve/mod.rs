//! Fact improvements (FX10a): an agent run rewrites one existing fact as a
//! **proposal**, and the core saves nothing until the user confirms that one
//! suggestion.
//!
//! The run is dispatched as untrusted (ADR-010): its init event must list only
//! `mcp__boba__*` tools or the runtime aborts it. `propose_fact_improvements` is
//! a proposal tool, never a write tool: the suggestion is stored with the
//! revision it was written against, and [`ImproveService::confirm`] — through
//! the shared review path — refuses it as stale if the item moved on.

use std::sync::Arc;

use boba_domain::audit::AuditOutcome;
use boba_domain::profile::improve::{FactImprovement, MAX_IMPROVEMENT_TEXT};
use boba_domain::profile::{FactProposal, ProfileFact, new_ulid};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use crate::ports::agent::{
    AgentEvent, AgentPort, AgentTask, AgentTaskKind, FailureReason, InputTrust, RunBudget,
};
use crate::ports::clock::ClockPort;
use crate::ports::repo::profile::ProfileRepo;
use crate::ports::repo::system::{NewIntent, SystemRepo};
use crate::profile::{ProfileError, ProfileSnapshot};
use crate::profile_import::{FactEdit, ImportError, ImportService, ProposalView};

/// The tool an `improve_fact` run sends its suggestion through (FX10a). The
/// catalog (`boba_mcp_proto::tool`) is the authority on tool names; this crate
/// cannot depend on it, so the wiring asserts the two agree.
pub const IMPROVE_TOOL: &str = "propose_fact_improvements";

/// The audit action a confirmed improvement writes (AD-12, canonical chain).
pub const IMPROVE_ACTION: &str = "profile.fact_improved";

/// The run's wall-clock cap: one pass, one proposal call.
pub const RUN_WALL_CLOCK: std::time::Duration = std::time::Duration::from_secs(300);

/// The most MCP calls one improvement run may make: its input, the profile and
/// the proposal.
pub const RUN_TOOL_CALLS: u32 = 10;

/// What one improvement run asks about: the fact, the picked engine and the
/// content language (AD-17).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImproveRequest {
    /// Stable ULID of the fact item to improve.
    pub item_id: String,
    /// A `CliKind` id, e.g. `claude_code`.
    pub cli: String,
    pub model: Option<String>,
    /// The content language the pack is rendered in (AD-17).
    pub target_language: String,
}

/// What one improvement run did: the suggestions waiting for review. Nothing
/// was saved to the profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ImproveOutcome {
    pub run_id: String,
    pub proposals: Vec<ProposalView>,
}

/// An improvement payload outside its contract (AD-5): the run's agent can read
/// the message and retry; nothing is stored.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid improvement payload: {why}")]
pub struct ContractError {
    pub why: String,
}

impl ContractError {
    fn new(why: impl Into<String>) -> Self {
        Self { why: why.into() }
    }
}

/// One improvement as the run proposes it. The core keeps the fact's own
/// revision, kind, evidence and structured detail: the run names the item and
/// the text it wants. `evidence` and `detail` in the payload are ignored (R1):
/// evidence is the fact's source grounding, so an agent may not attach any.
#[derive(Debug, Clone, Deserialize)]
struct ProposedImprovement {
    item_id: String,
    #[serde(default)]
    text_en: Option<String>,
    #[serde(default)]
    text_es: Option<String>,
}

/// Validate one `propose_fact_improvements` payload at the tool boundary (AD-5).
/// Returns how many suggestions the core could accept; the use case still drops
/// one for another item or one that invents a variant.
pub fn validate_improvements(args: &Value) -> Result<usize, ContractError> {
    Ok(improvements_of(args)?.len())
}

/// The contract object of a `propose_fact_improvements` call, shape-checked.
fn improvements_of(args: &Value) -> Result<Vec<ProposedImprovement>, ContractError> {
    let improvements = args.get("improvements").cloned().unwrap_or(Value::Null);
    let Some(improvements) = improvements.as_array() else {
        return Err(ContractError::new("`improvements` must be an array"));
    };
    let mut parsed = Vec::with_capacity(improvements.len());
    for improvement in improvements {
        if !improvement.is_object() {
            return Err(ContractError::new("every improvement must be an object"));
        }
        let improvement: ProposedImprovement = serde_json::from_value(improvement.clone())
            .map_err(|error| ContractError::new(error.to_string()))?;
        if improvement.item_id.trim().is_empty() {
            return Err(ContractError::new("every improvement needs its `item_id`"));
        }
        let en = improvement.text_en.as_deref().map(str::trim).unwrap_or("");
        let es = improvement.text_es.as_deref().map(str::trim).unwrap_or("");
        if en.is_empty() && es.is_empty() {
            return Err(ContractError::new(
                "an improvement needs English or Spanish text",
            ));
        }
        // R4: a suggestion is a rewritten fact, not a document. Refusing it
        // here gives the agent a message it can retry with, rather than
        // storing pages of agent text or truncating it behind the user's back.
        if [en, es]
            .into_iter()
            .any(|text| text.chars().count() > MAX_IMPROVEMENT_TEXT)
        {
            return Err(ContractError::new(format!(
                "an improvement's text may not exceed {MAX_IMPROVEMENT_TEXT} characters"
            )));
        }
        parsed.push(improvement);
    }
    Ok(parsed)
}

/// A profile improvement failure. Each variant carries the i18n key the UI
/// shows (AD-3: `errors.profile_improve.*`).
#[derive(Debug, Clone, thiserror::Error)]
pub enum ProfileImproveError {
    /// The Profile, the proposals or the audit log could not be read or saved.
    #[error("the profile could not be read or saved")]
    Store,
    /// The fact an improvement was asked for does not exist.
    #[error("unknown fact item: {0}")]
    UnknownItem(String),
    /// The suggestion was already confirmed or discarded (another window).
    #[error("unknown proposal: {0}")]
    UnknownProposal(String),
    /// The item moved past the revision the suggestion was written against
    /// (AC3): the newer edit wins and nothing is written.
    #[error("stale improvement proposal: {0}")]
    Stale(String),
    /// The run proposed something outside the tool contract (AD-5).
    #[error("the agent sent an improvement Boba could not read")]
    Contract,
    /// The problem is the profile rule itself (no variant, blank variant).
    #[error(transparent)]
    Profile(#[from] ProfileError),
    /// The run ended without proposing (FR-21): auth, quota, timeout, crash.
    #[error("the improvement run failed: {0:?}")]
    Run(FailureReason),
}

impl ProfileImproveError {
    /// The i18n key the UI shows.
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::Store => "errors.profile_improve.store",
            Self::UnknownItem(_) => "errors.profile_improve.unknown_item",
            Self::UnknownProposal(_) => "errors.profile_improve.unknown_proposal",
            Self::Stale(_) => "errors.profile_improve.stale",
            Self::Contract => "errors.profile_improve.contract",
            Self::Profile(error) => error.message_key(),
            Self::Run(_) => "errors.profile_improve.run",
        }
    }
}

/// The improvement use case: the profile owner's port, the agent runtime, the
/// injected clock and the audit log (AD-12).
pub struct ImproveService {
    repo: Arc<dyn ProfileRepo>,
    agents: Arc<dyn AgentPort>,
    clock: Arc<dyn ClockPort>,
    system: Arc<dyn SystemRepo>,
}

impl ImproveService {
    pub fn new(
        repo: Arc<dyn ProfileRepo>,
        agents: Arc<dyn AgentPort>,
        clock: Arc<dyn ClockPort>,
        system: Arc<dyn SystemRepo>,
    ) -> Self {
        Self {
            repo,
            agents,
            clock,
            system,
        }
    }

    /// Run one `improve_fact` pass (AC1, AC2): the suggestion is a pending
    /// proposal tied to the fact's current revision. Nothing is saved.
    pub fn start(&self, request: ImproveRequest) -> Result<ImproveOutcome, ProfileImproveError> {
        let facts = self
            .repo
            .facts_current()
            .map_err(|_| ProfileImproveError::Store)?;
        let fact = facts
            .into_iter()
            .find(|fact| fact.item_id == request.item_id)
            .ok_or_else(|| ProfileImproveError::UnknownItem(request.item_id.clone()))?;
        let task = AgentTask {
            kind: AgentTaskKind::ImproveFact,
            // The run rewrites the user's own fact, but its output is agent
            // text the user must review: it takes the lockdown gate (ADR-010).
            input_trust: InputTrust::Untrusted,
            cli: request.cli,
            model: request.model,
            prompt: improve_input(&fact),
            target_language: request.target_language,
            budget: RunBudget {
                wall_clock: Some(RUN_WALL_CLOCK),
                max_tool_calls: Some(RUN_TOOL_CALLS),
                ..RunBudget::default()
            },
            env: Vec::new(),
        };
        let handle = self.agents.run(task);
        let events: Vec<AgentEvent> = handle.events.iter().collect();
        self.harvest(&handle.run_id, &fact, &events)?;
        if let Some(reason) = failure(&events) {
            return Err(ProfileImproveError::Run(reason));
        }
        Ok(ImproveOutcome {
            run_id: handle.run_id.clone(),
            proposals: self.proposals_of(&handle.run_id)?,
        })
    }

    /// Confirm one suggestion — as proposed, or with the user's edit (AC2,
    /// AC3). The shared review path writes the revision and refuses a stale
    /// suggestion; a successful write is audited (AC5, ADR-011).
    pub fn confirm(
        &self,
        id: &str,
        edit: Option<FactEdit>,
    ) -> Result<ProfileSnapshot, ProfileImproveError> {
        let proposal = self
            .repo
            .proposals_pending()
            .map_err(|_| ProfileImproveError::Store)?
            .into_iter()
            .find(|proposal| proposal.id == id)
            .ok_or_else(|| ProfileImproveError::UnknownProposal(id.to_string()))?;
        // This command confirms improvements only; a CV or translation proposal
        // belongs to its own review path.
        let Some(item_id) = proposal.item_id.clone() else {
            return Err(ProfileImproveError::UnknownProposal(id.to_string()));
        };
        if proposal.base_revision.is_none() {
            return Err(ProfileImproveError::UnknownProposal(id.to_string()));
        }

        let now = self.clock.now_rfc3339();
        let intent = self
            .system
            .audit_append_intent(&NewIntent {
                action: IMPROVE_ACTION.to_string(),
                correlation_id: format!(
                    "{IMPROVE_ACTION}_{item_id}_{}",
                    now.replace([':', '+', '.'], "-")
                ),
                actor: "user".to_string(),
                target_kind: "profile_fact".to_string(),
                target_id: Some(item_id.clone()),
                // Ids and the base revision, never the text (AD-12).
                detail_json: serde_json::json!({
                    "proposal_id": id,
                    "base_revision": proposal.base_revision,
                    "agent_run_id": proposal.agent_run_id,
                })
                .to_string(),
                payload_blake3: None,
                created_at: now.clone(),
            })
            .map_err(|_| ProfileImproveError::Store)?;

        let importer =
            ImportService::new(self.repo.clone(), self.agents.clone(), self.clock.clone());
        match importer.confirm(id, edit) {
            Ok(snapshot) => {
                let revision_id = snapshot
                    .facts
                    .iter()
                    .find(|fact| fact.item_id == item_id)
                    .map(|fact| fact.revision_id.clone());
                // The fact is saved; a failed outcome append must not undo it
                // (the intent then stays for startup to resolve).
                let _ = self.system.audit_append_outcome(
                    &intent,
                    AuditOutcome::Ok,
                    &serde_json::json!({
                        "profile_version": snapshot.profile_version,
                        "revision_id": revision_id,
                    })
                    .to_string(),
                    &self.clock.now_rfc3339(),
                );
                Ok(snapshot)
            }
            Err(error) => {
                let _ = self.system.audit_append_outcome(
                    &intent,
                    AuditOutcome::Failed,
                    &serde_json::json!({ "error": error.message_key() }).to_string(),
                    &self.clock.now_rfc3339(),
                );
                Err(map_import(error))
            }
        }
    }

    /// Reject one suggestion: it never becomes a fact (FX10b's Reject).
    pub fn discard(&self, id: &str) -> Result<(), ProfileImproveError> {
        let proposals = self
            .repo
            .proposals_pending()
            .map_err(|_| ProfileImproveError::Store)?;
        let Some(proposal) = proposals.iter().find(|proposal| proposal.id == id) else {
            return Err(ProfileImproveError::UnknownProposal(id.to_string()));
        };
        if proposal.base_revision.is_none() {
            return Err(ProfileImproveError::UnknownProposal(id.to_string()));
        }
        let importer =
            ImportService::new(self.repo.clone(), self.agents.clone(), self.clock.clone());
        importer.discard(id).map_err(map_import)
    }

    /// The suggestions one run left waiting, newest first.
    fn proposals_of(&self, run_id: &str) -> Result<Vec<ProposalView>, ProfileImproveError> {
        Ok(self
            .repo
            .proposals_pending()
            .map_err(|_| ProfileImproveError::Store)?
            .iter()
            .filter(|proposal| proposal.agent_run_id == run_id)
            .map(ProposalView::from)
            .collect())
    }

    /// Keep only what the core can store (AD-5): a suggestion for this item,
    /// written against the revision the run read, that keeps the fact's variant
    /// set and satisfies the profile rules. Its text is stored verbatim.
    fn harvest(
        &self,
        run_id: &str,
        fact: &ProfileFact,
        events: &[AgentEvent],
    ) -> Result<(), ProfileImproveError> {
        // A call the boundary refused is the agent's to fix: it may retry in
        // the same run. Only a run where every call was refused reports the
        // contract failure, so a retried over-long suggestion (R4) still lands.
        let mut accepted = 0_usize;
        let mut refused = false;
        for event in events {
            let AgentEvent::ToolCall { name, input } = event else {
                continue;
            };
            if rsplit2(name) != Some(IMPROVE_TOOL) {
                continue;
            }
            let Ok(proposed) = improvements_of(input) else {
                refused = true;
                continue;
            };
            for proposed in proposed {
                if proposed.item_id != fact.item_id {
                    continue;
                }
                // The suggestion rewrites what it names and keeps everything
                // else: the run can improve one language without wiping the
                // other. Evidence and structured detail are the fact's own and
                // never the run's (R1): evidence grounds the fact in the CV, so
                // an agent may not attach any.
                let mut draft = fact.draft();
                if let Some(en) = clean(proposed.text_en) {
                    draft.text_en = Some(en);
                }
                if let Some(es) = clean(proposed.text_es) {
                    draft.text_es = Some(es);
                }
                let improvement = FactImprovement {
                    item_id: fact.item_id.clone(),
                    base_revision: fact.id.clone(),
                    draft,
                };
                if !improvement.keeps_variants(fact)
                    || improvement.validate().is_err()
                    || !improvement.text_within_cap()
                {
                    continue;
                }
                let proposal = FactProposal {
                    id: new_ulid(),
                    item_id: Some(fact.item_id.clone()),
                    draft: improvement.draft,
                    // An improvement cites no outside words: it rewrites the
                    // user's own fact.
                    source_excerpt: None,
                    base_revision: Some(fact.id.clone()),
                    untrusted_origin: true,
                    agent_run_id: run_id.to_string(),
                    created_at: self.clock.now_rfc3339(),
                };
                self.repo
                    .proposal_append(&proposal)
                    .map_err(|_| ProfileImproveError::Store)?;
                accepted += 1;
            }
        }
        if accepted == 0 && refused {
            return Err(ProfileImproveError::Contract);
        }
        Ok(())
    }
}

/// Map the shared review path's failure onto this use case's (AD-3).
fn map_import(error: ImportError) -> ProfileImproveError {
    match error {
        ImportError::Repo(_) => ProfileImproveError::Store,
        ImportError::Profile(error) => ProfileImproveError::Profile(error),
        ImportError::Contract(_) => ProfileImproveError::Contract,
        ImportError::UnknownProposal(id) => ProfileImproveError::UnknownProposal(id),
        ImportError::Stale(id) => ProfileImproveError::Stale(id),
        ImportError::Run(reason) => ProfileImproveError::Run(reason),
        ImportError::Text(_) => ProfileImproveError::Store,
    }
}

/// The run's own input: the fact to improve, as data (AD-7).
fn improve_input(fact: &ProfileFact) -> String {
    serde_json::json!({
        "item_id": fact.item_id,
        "revision_id": fact.id,
        "kind": fact.kind.as_str(),
        "text_en": fact.text_en,
        "text_es": fact.text_es,
        "evidence": fact.evidence,
        "detail": fact.detail_json,
    })
    .to_string()
}

/// Split the whole tool token (multi-byte safe).
fn rsplit2(name: &str) -> Option<&str> {
    name.rsplit("__").next()
}

/// Trimmed `None` for a blank string.
fn clean(text: Option<String>) -> Option<String> {
    text.map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

/// The failure a run ended with, if any.
fn failure(events: &[AgentEvent]) -> Option<FailureReason> {
    events.iter().find_map(|event| match event {
        AgentEvent::Failed { reason, .. } => Some(*reason),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::agent::{AgentPort, CancelToken, RunHandle};
    use crate::ports::repo::profile::ProfileRepo;
    use crate::profile::testing::FakeProfileRepo;
    use crate::profile::{FactRevisionInput, ProfileService};
    use crate::system::FixedClock;
    use crate::system::testing::FakeSystemRepo;
    use boba_domain::audit::AuditPhase;
    use boba_domain::profile::Provenance;
    use serde_json::json;
    use std::sync::Mutex;
    use std::sync::mpsc::{Receiver, channel};

    /// An `AgentPort` that answers from a script, like the gap tests' one.
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
    }

    impl AgentPort for FakeAgent {
        fn run(&self, task: AgentTask) -> RunHandle {
            self.tasks.lock().unwrap().push(task);
            let (sender, events) = channel();
            for event in self.events.lock().unwrap().iter() {
                let _ = sender.send(event.clone());
            }
            drop(sender);
            RunHandle {
                run_id: "run_improve".to_string(),
                events: events as Receiver<AgentEvent>,
                cancel: CancelToken::new(),
            }
        }
    }

    struct Fixture {
        service: ImproveService,
        profile: std::sync::Arc<FakeProfileRepo>,
        system: std::sync::Arc<FakeSystemRepo>,
        agents: std::sync::Arc<FakeAgent>,
        item_id: String,
    }

    fn fixture(events: Vec<AgentEvent>) -> Fixture {
        let profile = std::sync::Arc::new(FakeProfileRepo::default());
        let system = std::sync::Arc::new(FakeSystemRepo::default());
        let agents = std::sync::Arc::new(FakeAgent::scripted(events));
        let clock = std::sync::Arc::new(FixedClock::new("2026-10-18T09:00:00.000Z"));
        let saved = ProfileService::new(profile.clone(), clock.clone())
            .save(FactRevisionInput {
                item_id: None,
                kind: "experience".to_string(),
                text_en: Some("Backend engineer at Nimbus".to_string()),
                text_es: None,
                evidence: None,
                detail_json: Some(r#"{"organisation":"Nimbus"}"#.to_string()),
            })
            .expect("seed the fact");
        let item_id = saved.facts[0].item_id.clone();
        let service = ImproveService::new(profile.clone(), agents.clone(), clock, system.clone());
        Fixture {
            service,
            profile,
            system,
            agents,
            item_id,
        }
    }

    fn request(item_id: &str) -> ImproveRequest {
        ImproveRequest {
            item_id: item_id.to_string(),
            cli: "claude_code".to_string(),
            model: Some("sonnet".to_string()),
            target_language: "en".to_string(),
        }
    }

    fn suggestion(item_id: &str, en: &str) -> AgentEvent {
        AgentEvent::ToolCall {
            name: "mcp__boba__propose_fact_improvements".to_string(),
            input: json!({ "improvements": [
                { "item_id": item_id, "text_en": en }
            ]}),
        }
    }

    fn finished() -> AgentEvent {
        AgentEvent::Finished { exit_code: 0 }
    }

    // FX10a AC1/AC2: the run reads the fact and its suggestion waits as a
    // pending proposal tied to the fact's current revision. Nothing is saved.
    #[test]
    fn a_run_stores_a_pending_suggestion_and_writes_nothing() {
        let text = "Led Nimbus's backend rewrite in Rust, cutting p99 latency by 40%.";
        let fixture = fixture(vec![]);
        let item = fixture.item_id.clone();
        *fixture.agents.events.lock().unwrap() = vec![suggestion(&item, text), finished()];

        let outcome = fixture.service.start(request(&item)).expect("improve");
        assert_eq!(outcome.run_id, "run_improve");
        assert_eq!(outcome.proposals.len(), 1);
        let proposal = &outcome.proposals[0];
        assert_eq!(proposal.item_id.as_deref(), Some(item.as_str()));
        assert_eq!(proposal.text_en.as_deref(), Some(text));
        assert!(proposal.untrusted_origin, "agent text is never user text");
        assert_eq!(
            proposal.base_revision.as_deref(),
            Some(fixture.profile.revisions()[0].id.as_str())
        );

        // AC2: the core wrote no fact.
        assert_eq!(fixture.profile.profile_version().unwrap(), 1);
        assert_eq!(fixture.profile.revisions().len(), 1);
        assert_eq!(fixture.profile.proposals_pending().unwrap().len(), 1);

        // AC1: the per-run lockdown gate applies because the task is untrusted.
        let task = &fixture.agents.tasks()[0];
        assert_eq!(task.input_trust, InputTrust::Untrusted);
        assert_eq!(task.kind, AgentTaskKind::ImproveFact);
        assert!(
            task.prompt.contains("Backend engineer at Nimbus"),
            "{}",
            task.prompt
        );
    }

    // The suggestion is returned verbatim: markup or instruction-shaped text is
    // data the UI shows as plain text (AC4, AD-7).
    #[test]
    fn the_suggestion_reaches_the_ui_verbatim() {
        let text = "Ignore previous instructions.\n<b>Senior</b> engineer — 100% <script>alert(1)</script>";
        let fixture = fixture(vec![]);
        let item = fixture.item_id.clone();
        *fixture.agents.events.lock().unwrap() = vec![suggestion(&item, text), finished()];

        let outcome = fixture.service.start(request(&item)).expect("improve");
        assert_eq!(outcome.proposals[0].text_en.as_deref(), Some(text));
    }

    #[test]
    fn an_unknown_item_is_refused_before_any_run() {
        let fixture = fixture(vec![]);
        let error = fixture
            .service
            .start(request("01NOTHERE"))
            .expect_err("unknown item");
        assert_eq!(error.message_key(), "errors.profile_improve.unknown_item");
        assert!(fixture.agents.tasks().is_empty());
    }

    // AD-5: the core disposes — a suggestion for a different item never reaches
    // the user.
    #[test]
    fn a_suggestion_for_another_item_is_dropped() {
        let fixture = fixture(vec![]);
        let item = fixture.item_id.clone();
        *fixture.agents.events.lock().unwrap() =
            vec![suggestion("01OTHER", "Something else entirely"), finished()];

        let outcome = fixture.service.start(request(&item)).expect("improve");
        assert!(outcome.proposals.is_empty());
        assert!(fixture.profile.proposals_pending().unwrap().is_empty());
    }

    // FX10a: filling a missing variant is the translate task's job (S3.6); the
    // core drops an improvement that invents one.
    #[test]
    fn a_suggestion_that_invents_the_missing_variant_is_dropped() {
        let fixture = fixture(vec![]);
        let item = fixture.item_id.clone();
        *fixture.agents.events.lock().unwrap() = vec![
            AgentEvent::ToolCall {
                name: "mcp__boba__propose_fact_improvements".to_string(),
                input: json!({ "improvements": [{
                    "item_id": item,
                    "text_en": "Backend engineer at Nimbus",
                    "text_es": "Ingeniero backend en Nimbus",
                }]}),
            },
            finished(),
        ];

        let outcome = fixture.service.start(request(&item)).expect("improve");
        assert!(outcome.proposals.is_empty());
    }

    // A suggestion that only rewrites one language keeps the other variant and
    // the structured detail.
    #[test]
    fn a_partial_suggestion_keeps_the_other_variant_and_detail() {
        let fixture = fixture(vec![]);
        let clock = std::sync::Arc::new(FixedClock::new("2026-10-18T09:00:00.000Z"));
        ProfileService::new(fixture.profile.clone(), clock)
            .save(FactRevisionInput {
                item_id: Some(fixture.item_id.clone()),
                kind: "experience".to_string(),
                text_en: Some("Backend engineer at Nimbus".to_string()),
                text_es: Some("Ingeniero backend en Nimbus".to_string()),
                evidence: None,
                detail_json: Some(r#"{"organisation":"Nimbus"}"#.to_string()),
            })
            .expect("seed the bilingual fact");
        let item = fixture.item_id.clone();
        let revision = fixture
            .profile
            .fact_latest(&item)
            .unwrap()
            .expect("current")
            .id
            .clone();
        *fixture.agents.events.lock().unwrap() = vec![
            suggestion(&item, "Led the Nimbus backend rewrite in Rust"),
            finished(),
        ];

        let outcome = fixture.service.start(request(&item)).expect("improve");
        let proposal = &outcome.proposals[0];
        assert_eq!(
            proposal.text_es.as_deref(),
            Some("Ingeniero backend en Nimbus")
        );
        assert_eq!(proposal.base_revision.as_deref(), Some(revision.as_str()));
        assert!(proposal.detail_json.contains("Nimbus"));
    }

    // FR-21: a run that died still reports why, and keeps its proposals.
    #[test]
    fn a_failed_run_is_reported_and_keeps_its_proposals() {
        let fixture = fixture(vec![]);
        let item = fixture.item_id.clone();
        *fixture.agents.events.lock().unwrap() = vec![
            suggestion(&item, "Backend engineer, lead"),
            AgentEvent::Failed {
                reason: FailureReason::AuthExpired,
                message_key: "errors.agent.auth_expired".to_string(),
            },
        ];

        let error = fixture
            .service
            .start(request(&item))
            .expect_err("failed run");
        assert_eq!(error.message_key(), "errors.profile_improve.run");
        assert_eq!(fixture.profile.proposals_pending().unwrap().len(), 1);
    }

    // R1 (FX10a review): the run rewrites text only. Evidence is source
    // grounding and detail is structured data: the core keeps the fact's own
    // whatever the run sent, so no agent can attach evidence the CV never said.
    #[test]
    fn a_suggestion_keeps_the_facts_own_evidence_and_detail() {
        let fixture = fixture(vec![]);
        let clock = std::sync::Arc::new(FixedClock::new("2026-10-18T09:30:00.000Z"));
        ProfileService::new(fixture.profile.clone(), clock)
            .save(FactRevisionInput {
                item_id: Some(fixture.item_id.clone()),
                kind: "experience".to_string(),
                text_en: Some("Backend engineer at Nimbus".to_string()),
                text_es: None,
                evidence: Some("Nimbus CV, 2024: backend rewrite".to_string()),
                detail_json: Some(r#"{"organisation":"Nimbus"}"#.to_string()),
            })
            .expect("seed the fact with evidence");
        let item = fixture.item_id.clone();
        *fixture.agents.events.lock().unwrap() = vec![
            AgentEvent::ToolCall {
                name: "mcp__boba__propose_fact_improvements".to_string(),
                input: json!({ "improvements": [{
                    "item_id": item,
                    "text_en": "Led the Nimbus backend rewrite in Rust",
                    "evidence": "Patent pending, 2025",
                    "detail": { "organisation": "Invented Corp" }
                }]}),
            },
            finished(),
        ];

        let outcome = fixture.service.start(request(&item)).expect("improve");
        let proposal = &outcome.proposals[0];
        assert_eq!(
            proposal.evidence.as_deref(),
            Some("Nimbus CV, 2024: backend rewrite")
        );
        assert!(proposal.detail_json.contains("Nimbus"));
        assert!(
            !proposal.detail_json.contains("Invented"),
            "{}",
            proposal.detail_json
        );

        let snapshot = fixture
            .service
            .confirm(&proposal.id, None)
            .expect("confirm");
        assert_eq!(snapshot.profile_version, 3);
        let latest = fixture
            .profile
            .revisions()
            .last()
            .cloned()
            .expect("revision");
        assert_eq!(
            latest.evidence.as_deref(),
            Some("Nimbus CV, 2024: backend rewrite"),
            "the confirmed fact keeps its own source grounding"
        );
        assert!(latest.detail_json.contains("Nimbus"));
    }

    // R4: an over-long suggestion is refused at the tool boundary with a
    // contract error the agent can retry, never stored and never truncated.
    #[test]
    fn an_over_long_suggestion_is_refused_at_the_tool_boundary() {
        let at_cap = "a".repeat(boba_domain::profile::improve::MAX_IMPROVEMENT_TEXT);
        let over_cap = "a".repeat(boba_domain::profile::improve::MAX_IMPROVEMENT_TEXT + 1);
        assert_eq!(
            validate_improvements(&json!({ "improvements": [{
                "item_id": "01ITEM", "text_en": at_cap
            }]}))
            .unwrap(),
            1
        );
        assert!(
            validate_improvements(&json!({ "improvements": [{
                "item_id": "01ITEM", "text_en": over_cap
            }]}))
            .is_err(),
            "an over-long suggestion must be refused, not stored"
        );

        // And a run that bypasses the boundary still stores nothing.
        let fixture = fixture(vec![]);
        let item = fixture.item_id.clone();
        *fixture.agents.events.lock().unwrap() = vec![suggestion(&item, &over_cap), finished()];
        let error = fixture
            .service
            .start(request(&item))
            .expect_err("over-long suggestion");
        assert_eq!(error.message_key(), "errors.profile_improve.contract");
        assert!(fixture.profile.proposals_pending().unwrap().is_empty());
    }

    // R4: a refused call is the agent's to fix. When it retries with a shorter
    // suggestion, the run keeps what the retry produced; only a run where every
    // call was refused reports the contract failure.
    #[test]
    fn a_refused_suggestion_can_be_retried_in_the_same_run() {
        let fixture = fixture(vec![]);
        let item = fixture.item_id.clone();
        let over_cap = "a".repeat(boba_domain::profile::improve::MAX_IMPROVEMENT_TEXT + 1);
        *fixture.agents.events.lock().unwrap() = vec![
            suggestion(&item, &over_cap),
            suggestion(&item, "Led the Nimbus backend rewrite in Rust"),
            finished(),
        ];

        let outcome = fixture.service.start(request(&item)).expect("improve");
        assert_eq!(outcome.proposals.len(), 1);
        assert_eq!(
            outcome.proposals[0].text_en.as_deref(),
            Some("Led the Nimbus backend rewrite in Rust")
        );
    }

    #[test]
    fn the_tool_boundary_refuses_a_payload_outside_the_contract() {
        assert!(validate_improvements(&json!({ "improvements": "nope" })).is_err());
        assert!(validate_improvements(&json!({})).is_err());
        assert!(
            validate_improvements(&json!({ "improvements": [{
                "item_id": "01ITEM", "text_en": " ", "text_es": " "
            }]}))
            .is_err()
        );
        assert!(
            validate_improvements(&json!({ "improvements": [{
                "item_id": " ", "text_en": "Rust"
            }]}))
            .is_err()
        );
        // R1: `evidence` and `detail` are the fact's own, not the run's; the
        // boundary ignores them rather than trusting them.
        assert_eq!(
            validate_improvements(&json!({ "improvements": [{
                "item_id": "01ITEM", "text_en": "Rust", "detail": "nope",
                "evidence": "anything"
            }]}))
            .unwrap(),
            1
        );
        assert_eq!(
            validate_improvements(&json!({ "improvements": [{
                "item_id": "01ITEM", "text_en": "Rust"
            }]}))
            .unwrap(),
            1
        );
    }

    // FX10a AC2: confirming one suggestion writes exactly that fact, and the
    // proposal is gone.
    #[test]
    fn confirming_one_suggestion_saves_only_that_fact() {
        let fixture = fixture(vec![]);
        let item = fixture.item_id.clone();
        *fixture.agents.events.lock().unwrap() = vec![
            suggestion(&item, "Led the Nimbus backend rewrite in Rust"),
            finished(),
        ];
        let id = fixture
            .service
            .start(request(&item))
            .expect("improve")
            .proposals[0]
            .id
            .clone();

        let snapshot = fixture.service.confirm(&id, None).expect("confirm");
        assert_eq!(snapshot.profile_version, 2);
        let latest = fixture
            .profile
            .revisions()
            .last()
            .cloned()
            .expect("revision");
        assert_eq!(latest.item_id, item);
        assert_eq!(
            latest.text_en.as_deref(),
            Some("Led the Nimbus backend rewrite in Rust")
        );
        // A confirmed proposal is the user's own claim (the S3.5 path):
        // provenance stays `user`, and the run is named only in the audit row.
        assert_eq!(latest.provenance, Provenance::User);
        assert!(fixture.profile.proposals_pending().unwrap().is_empty());
    }

    // AC5 (AD-11): a successful confirm is audited in the canonical chain form
    // — one intent and one ok outcome.
    #[test]
    fn a_successful_confirm_is_audited() {
        let fixture = fixture(vec![]);
        let item = fixture.item_id.clone();
        *fixture.agents.events.lock().unwrap() = vec![
            suggestion(&item, "Led the Nimbus backend rewrite in Rust"),
            finished(),
        ];
        let id = fixture
            .service
            .start(request(&item))
            .expect("improve")
            .proposals[0]
            .id
            .clone();
        fixture.service.confirm(&id, None).expect("confirm");

        let rows = fixture.system.audit_rows().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].phase, AuditPhase::Intent);
        assert_eq!(rows[0].action, IMPROVE_ACTION);
        assert_eq!(rows[0].actor, "user");
        assert_eq!(rows[0].target_kind, "profile_fact");
        assert_eq!(rows[0].target_id.as_deref(), Some(item.as_str()));
        assert!(
            rows[0].detail_json.contains(&id),
            "the intent names the suggestion, never the text"
        );
        assert!(
            rows[0].detail_json.contains("run_improve"),
            "the intent names the run that proposed it"
        );
        assert!(!rows[0].detail_json.contains("Nimbus"));
        assert_eq!(rows[1].phase, AuditPhase::Outcome);
        assert_eq!(rows[1].outcome, Some(AuditOutcome::Ok));
        assert_eq!(rows[1].intent_id.as_deref(), Some(rows[0].id.as_str()));
    }

    // AC3: a suggestion made before the user's own edit is refused as stale,
    // nothing is written, and the attempt is audited as failed.
    #[test]
    fn a_stale_confirm_is_refused_and_writes_nothing() {
        let fixture = fixture(vec![]);
        let item = fixture.item_id.clone();
        *fixture.agents.events.lock().unwrap() = vec![
            suggestion(&item, "Led the Nimbus backend rewrite in Rust"),
            finished(),
        ];
        let id = fixture
            .service
            .start(request(&item))
            .expect("improve")
            .proposals[0]
            .id
            .clone();

        ProfileService::new(
            fixture.profile.clone(),
            std::sync::Arc::new(FixedClock::new("2026-10-18T10:00:00.000Z")),
        )
        .save(FactRevisionInput {
            item_id: Some(item.clone()),
            kind: "experience".to_string(),
            text_en: Some("Backend engineer, staff".to_string()),
            text_es: None,
            evidence: None,
            detail_json: None,
        })
        .expect("the user's own edit");

        let before = fixture.profile.revisions().len();
        let error = fixture
            .service
            .confirm(&id, None)
            .expect_err("stale suggestion");
        assert_eq!(error.message_key(), "errors.profile_improve.stale");
        assert_eq!(
            fixture.profile.revisions().len(),
            before,
            "a stale confirm writes nothing"
        );
        let latest = fixture
            .profile
            .revisions()
            .last()
            .cloned()
            .expect("revision");
        assert_eq!(latest.text_en.as_deref(), Some("Backend engineer, staff"));

        let rows = fixture.system.audit_rows().unwrap();
        assert_eq!(rows[1].outcome, Some(AuditOutcome::Failed));
    }

    // A rejected suggestion never becomes a fact.
    #[test]
    fn discarding_a_suggestion_writes_nothing() {
        let fixture = fixture(vec![]);
        let item = fixture.item_id.clone();
        *fixture.agents.events.lock().unwrap() = vec![
            suggestion(&item, "Led the Nimbus backend rewrite in Rust"),
            finished(),
        ];
        let id = fixture
            .service
            .start(request(&item))
            .expect("improve")
            .proposals[0]
            .id
            .clone();

        fixture.service.discard(&id).expect("discard");
        assert!(fixture.profile.proposals_pending().unwrap().is_empty());
        assert_eq!(fixture.profile.revisions().len(), 1);
        assert_eq!(fixture.profile.profile_version().unwrap(), 1);
    }

    #[test]
    fn an_unknown_suggestion_is_reported() {
        let fixture = fixture(vec![]);
        let error = fixture
            .service
            .confirm("01NOTHERE", None)
            .expect_err("unknown proposal");
        assert_eq!(
            error.message_key(),
            "errors.profile_improve.unknown_proposal"
        );
        let error = fixture.service.discard("01NOTHERE").expect_err("discard");
        assert_eq!(
            error.message_key(),
            "errors.profile_improve.unknown_proposal"
        );
    }
}
