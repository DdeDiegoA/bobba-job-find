//! Master CV import use case (S3.5, FR-5, AD-5): the user's own PDF/DOCX/ZIP
//! files become profile facts the agent proposes and the user confirms.
//!
//! Nothing is saved without confirmation (AC1): a run's proposals are stored in
//! `profile_fact_proposal`, the review UI reads them with their source excerpt,
//! and only [`ImportService::confirm`] writes a `profile_fact` (through the
//! profile use case, so the normal versioning rules apply).
//!
//! A CV is the user's own input, so the run declares [`InputTrust::User`] and
//! every proposal it produces carries `untrusted_origin = 0` (AC4, ADR-010).

pub mod extract;

use std::sync::Arc;

use boba_domain::profile::{FactDraft, FactKind, FactProposal, new_ulid};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use crate::ports::agent::{
    AgentEvent, AgentPort, AgentTask, AgentTaskKind, FailureReason, InputTrust, RunBudget,
};
use crate::ports::clock::ClockPort;
use crate::ports::repo::profile::{ProfileRepo, ProfileRepoError};
use crate::profile::{FactRevisionInput, ProfileError, ProfileService, ProfileSnapshot};

pub use extract::{Extracted, ImportFile, TextError, extract, read_files};

/// The tool a run sends its proposals through. The catalog
/// (`boba_mcp_proto::tool`) is the authority on tool names; this crate cannot
/// depend on it, so the wiring asserts the two agree
/// (`wiring::agent::tests::profile_import_proposals_listen_for_the_catalog_tool`).
pub const PROPOSE_TOOL: &str = "propose_profile_facts";

/// The run's wall-clock cap: a CV import is a single agent pass with one
/// proposal call, so a run that has not answered in five minutes is dead.
pub const RUN_WALL_CLOCK: std::time::Duration = std::time::Duration::from_secs(300);

/// One fact as the agent proposes it. The shape the skill documents; a payload
/// outside it costs the run a retry at the tool boundary (AD-5).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ProposedFact {
    /// Section id: `identity`, `experience`, `skill`, `education`,
    /// `certification`, `language` or `eligibility`.
    pub kind: String,
    #[serde(default)]
    pub text_en: Option<String>,
    #[serde(default)]
    pub text_es: Option<String>,
    #[serde(default)]
    pub evidence: Option<String>,
    /// Kind-specific fields (organisation, dates, CEFR level, …) as JSON.
    #[serde(default)]
    pub detail: Option<Value>,
    /// The words of the CV this fact was read from (AC1). Required: the core
    /// verifies it against the extracted text before storing anything.
    #[serde(default)]
    pub source_excerpt: Option<String>,
}

/// A `propose_profile_facts` payload outside the contract (AD-5). The run's
/// agent can read the message and retry; nothing is stored.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid profile proposal: {why}")]
pub struct ContractError {
    pub why: String,
}

impl ContractError {
    fn new(why: impl Into<String>) -> Self {
        Self { why: why.into() }
    }
}

/// Validate one `propose_profile_facts` payload at the tool boundary (AD-5):
/// every fact names a known section, carries at least one variant and a source
/// excerpt. Returns how many facts the core would accept.
///
/// The excerpt is **not** checked here — the extracted CV text is the use
/// case's, and only [`ImportService::harvest`] holds it. This boundary check is
/// what turns a malformed call into a retry instead of a silent drop.
pub fn validate_proposal(args: &Value) -> Result<usize, ContractError> {
    Ok(facts_of(args)?.len())
}

/// The contract object of a `propose_profile_facts` call, shape-checked.
pub fn facts_of(args: &Value) -> Result<Vec<ProposedFact>, ContractError> {
    let facts = args.get("facts").cloned().unwrap_or(Value::Null);
    let Some(facts) = facts.as_array() else {
        return Err(ContractError::new("`facts` must be an array"));
    };
    let mut parsed = Vec::with_capacity(facts.len());
    for fact in facts {
        if !fact.is_object() {
            return Err(ContractError::new("every fact must be an object"));
        }
        let fact: ProposedFact = serde_json::from_value(fact.clone())
            .map_err(|error| ContractError::new(error.to_string()))?;
        check_fact(&fact)?;
        parsed.push(fact);
    }
    Ok(parsed)
}

/// The shape rules a fact must satisfy before it can ever be stored.
fn check_fact(fact: &ProposedFact) -> Result<(), ContractError> {
    if FactKind::parse(&fact.kind).is_none() {
        return Err(ContractError::new(format!(
            "unknown profile section `{}`",
            fact.kind
        )));
    }
    let en = fact.text_en.as_deref().map(str::trim).unwrap_or("");
    let es = fact.text_es.as_deref().map(str::trim).unwrap_or("");
    if en.is_empty() && es.is_empty() {
        return Err(ContractError::new("a fact needs English or Spanish text"));
    }
    if let Some(Value::Object(_)) | None = fact.detail.as_ref() {
    } else {
        return Err(ContractError::new("`detail` must be an object"));
    }
    match fact.source_excerpt.as_deref().map(str::trim) {
        Some(excerpt) if !excerpt.is_empty() => Ok(()),
        _ => Err(ContractError::new("every fact needs its `source_excerpt`")),
    }
}

/// What the step asks a run to do: the picked engine and the files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportRequest {
    /// A `CliKind` id, e.g. `claude_code`.
    pub cli: String,
    pub model: Option<String>,
    /// The wizard's language; the skill writes section text in the CV's own
    /// language, so this only labels the run's content language (AD-17).
    pub target_language: String,
    pub files: Vec<ImportFile>,
}

/// What one import did: the proposals now waiting for review, and how many
/// facts the core stored or dropped (an excerpt not in the CV is dropped, AD-5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ImportOutcome {
    pub proposals: Vec<ProposalView>,
    pub accepted: u32,
    pub dropped: u32,
    /// The files the run read, in merge order.
    pub files: Vec<String>,
}

/// One proposal as the review UI shows it. Every text field is the stored
/// string verbatim: the UI renders it as plain text, never truncated and never
/// interpreted as markup (AC3, AD-7, AD-15).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ProposalView {
    pub id: String,
    /// Section id, as `profile:sections.<kind>` labels it.
    pub kind: String,
    pub text_en: Option<String>,
    pub text_es: Option<String>,
    pub evidence: Option<String>,
    pub detail_json: String,
    /// The CV words the fact was read from (AC1).
    pub source_excerpt: Option<String>,
    /// Always `false` for a CV import (AC4): the user's own input.
    pub untrusted_origin: bool,
    pub created_at: String,
}

impl From<&FactProposal> for ProposalView {
    fn from(proposal: &FactProposal) -> Self {
        Self {
            id: proposal.id.clone(),
            kind: proposal.draft.kind.as_str().to_string(),
            text_en: proposal.draft.text_en.clone(),
            text_es: proposal.draft.text_es.clone(),
            evidence: proposal.draft.evidence.clone(),
            detail_json: proposal.draft.detail_json.clone(),
            source_excerpt: proposal.source_excerpt.clone(),
            untrusted_origin: proposal.untrusted_origin,
            created_at: proposal.created_at.clone(),
        }
    }
}

/// What the user changed in the review card before confirming (AC1). The three
/// text fields are the **new values**; `None` clears that field, so an edit
/// never keeps a variant the user deleted. `detail_json` is different: the card
/// does not edit it, so `None` keeps the proposed detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct FactEdit {
    pub text_en: Option<String>,
    pub text_es: Option<String>,
    pub evidence: Option<String>,
    pub detail_json: Option<String>,
}

/// A profile-import failure. Each variant carries the key the UI shows (AD-3).
#[derive(Debug, Clone, thiserror::Error)]
pub enum ImportError {
    #[error(transparent)]
    Repo(#[from] ProfileRepoError),
    #[error(transparent)]
    Profile(#[from] ProfileError),
    #[error(transparent)]
    Text(#[from] TextError),
    #[error(transparent)]
    Contract(#[from] ContractError),
    /// The proposal was already confirmed or deleted (another window, a stale
    /// list).
    #[error("unknown proposal: {0}")]
    UnknownProposal(String),
    /// The run ended without proposing (FR-21): auth, quota, timeout, crash.
    #[error("the import run failed: {0:?}")]
    Run(FailureReason),
}

impl ImportError {
    /// The i18n key the UI shows.
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::Repo(_) => "errors.profile_import.store",
            Self::Profile(error) => error.message_key(),
            Self::Text(error) => error.message_key(),
            Self::Contract(_) => "errors.profile_import.contract",
            Self::UnknownProposal(_) => "errors.profile_import.unknown_proposal",
            Self::Run(_) => "errors.profile_import.run",
        }
    }
}

/// The import use case: the profile owner's port, the agent runtime and the
/// injected clock (AD-17).
pub struct ImportService {
    repo: Arc<dyn ProfileRepo>,
    agents: Arc<dyn AgentPort>,
    clock: Arc<dyn ClockPort>,
}

impl ImportService {
    pub fn new(
        repo: Arc<dyn ProfileRepo>,
        agents: Arc<dyn AgentPort>,
        clock: Arc<dyn ClockPort>,
    ) -> Self {
        Self {
            repo,
            agents,
            clock,
        }
    }

    /// Extract the files and run one `profile_import` pass (AC1). The run's
    /// proposals are stored as pending proposals; a failed run still keeps what
    /// it proposed before it died, but the error is returned so the step shows
    /// why (FR-21).
    pub fn start(&self, request: ImportRequest) -> Result<ImportOutcome, ImportError> {
        let extracted = extract::extract(&request.files)?;
        let task = AgentTask {
            kind: AgentTaskKind::ProfileImport,
            // A CV is the user's own input (AC4): no lockdown gate applies.
            input_trust: InputTrust::User,
            cli: request.cli,
            model: request.model,
            prompt: task_input(&extracted),
            target_language: request.target_language,
            budget: RunBudget {
                wall_clock: Some(RUN_WALL_CLOCK),
                ..RunBudget::default()
            },
            env: Vec::new(),
        };
        let handle = self.agents.run(task);
        let events: Vec<AgentEvent> = handle.events.iter().collect();
        let (accepted, dropped) = self.harvest(&handle.run_id, false, &extracted.text, &events)?;
        if let Some(reason) = failure(&events) {
            return Err(ImportError::Run(reason));
        }
        Ok(ImportOutcome {
            proposals: self.proposals()?,
            accepted,
            dropped,
            files: extracted.files,
        })
    }

    /// Store every proposal a run made, from its event stream (AC1, AC4).
    /// Returns `(accepted, dropped)`: a fact whose excerpt is not in the CV's
    /// text is dropped, never stored (AD-5: the core disposes).
    pub fn harvest(
        &self,
        run_id: &str,
        untrusted: bool,
        source: &str,
        events: &[AgentEvent],
    ) -> Result<(u32, u32), ImportError> {
        let mut accepted = 0;
        let mut dropped = 0;
        for event in events {
            let AgentEvent::ToolCall { name, input } = event else {
                continue;
            };
            if !is_proposal(name) {
                continue;
            }
            let facts = facts_of(input)?;
            for fact in facts {
                let excerpt = fact
                    .source_excerpt
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or("")
                    .to_string();
                if !excerpt_in(source, &excerpt) {
                    dropped += 1;
                    continue;
                }
                self.store(run_id, untrusted, fact, &excerpt)?;
                accepted += 1;
            }
        }
        Ok((accepted, dropped))
    }

    /// The proposals waiting for review, newest first (AC1).
    pub fn proposals(&self) -> Result<Vec<ProposalView>, ImportError> {
        Ok(self
            .repo
            .proposals_pending()?
            .iter()
            .map(ProposalView::from)
            .collect())
    }

    /// The user confirmed a proposal (possibly edited): save the fact through
    /// the profile use case — the normal versioning rules, `provenance = user` —
    /// and drop the proposal (AC1: nothing is saved before this).
    pub fn confirm(
        &self,
        proposal_id: &str,
        edit: Option<FactEdit>,
    ) -> Result<ProfileSnapshot, ImportError> {
        let proposal = self
            .repo
            .proposals_pending()?
            .into_iter()
            .find(|proposal| proposal.id == proposal_id)
            .ok_or_else(|| ImportError::UnknownProposal(proposal_id.to_string()))?;
        let draft = proposal.draft;
        let (text_en, text_es, evidence, detail_json) = match edit {
            None => (
                draft.text_en,
                draft.text_es,
                draft.evidence,
                Some(draft.detail_json),
            ),
            Some(edit) => (
                edit.text_en,
                edit.text_es,
                edit.evidence,
                // The review card edits the wording, not the structured
                // detail: `None` keeps what the run proposed.
                Some(edit.detail_json.unwrap_or(draft.detail_json)),
            ),
        };
        let input = FactRevisionInput {
            item_id: proposal.item_id,
            kind: draft.kind.as_str().to_string(),
            text_en,
            text_es,
            evidence,
            detail_json,
        };
        let profile = ProfileService::new(self.repo.clone(), self.clock.clone());
        profile.save(input)?;
        self.repo.proposal_delete(proposal_id)?;
        // Read after the delete: the saved snapshot was taken while the
        // proposal still existed, and the step shows the new count.
        Ok(profile.snapshot()?)
    }

    /// The user deleted a proposal: it never becomes a fact (AC1).
    pub fn discard(&self, proposal_id: &str) -> Result<(), ImportError> {
        if self.repo.proposal_delete(proposal_id)? {
            return Ok(());
        }
        Err(ImportError::UnknownProposal(proposal_id.to_string()))
    }

    /// Store one verified fact as a pending proposal (AC1, AC4).
    fn store(
        &self,
        run_id: &str,
        untrusted: bool,
        fact: ProposedFact,
        excerpt: &str,
    ) -> Result<(), ImportError> {
        let kind = FactKind::parse(&fact.kind).expect("checked by the contract");
        let proposal = FactProposal {
            id: new_ulid(),
            // A CV import always creates new items; translating an existing one
            // is the translate task's (S3.6).
            item_id: None,
            draft: FactDraft {
                kind,
                text_en: clean(fact.text_en),
                text_es: clean(fact.text_es),
                evidence: clean(fact.evidence),
                detail_json: fact
                    .detail
                    .map(|detail| detail.to_string())
                    .unwrap_or_else(|| "{}".to_string()),
            },
            source_excerpt: Some(excerpt.to_string()),
            untrusted_origin: untrusted,
            agent_run_id: run_id.to_string(),
            created_at: self.clock.now_rfc3339(),
        };
        self.repo.proposal_append(&proposal)?;
        Ok(())
    }
}

/// The run's own input: the files' text, as data (AD-7). `get_task_input`
/// serves it to the run.
fn task_input(extracted: &Extracted) -> String {
    serde_json::json!({
        "files": extracted.files,
        "text": extracted.text,
    })
    .to_string()
}

/// Whether a tool call is this use case's proposal. Drivers report MCP calls
/// under the shim's own prefix (`mcp__boba__…`), so the name is matched by its
/// last segment.
fn is_proposal(name: &str) -> bool {
    name.rsplit("__").next() == Some(PROPOSE_TOOL)
}

/// Whether `excerpt` is really in the extracted CV text. Whitespace is
/// collapsed (a PDF's line breaks are layout, not wording); the words must be
/// there verbatim.
fn excerpt_in(source: &str, excerpt: &str) -> bool {
    let needle = collapse(excerpt);
    !needle.is_empty() && collapse(source).contains(&needle)
}

/// Collapse every whitespace run to one space, trimmed.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Trimmed `None` for a blank string: the UI shows "missing variant", not "".
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
    use crate::profile::testing::FakeProfileRepo;
    use crate::system::FixedClock;
    use boba_domain::profile::{FactKind, Lang, ProfileFact, Provenance};
    use serde_json::json;
    use std::sync::Mutex;
    use std::sync::mpsc::{Receiver, channel};

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
                run_id: "run_import".to_string(),
                events: events as Receiver<AgentEvent>,
                cancel: crate::ports::agent::CancelToken::new(),
            }
        }
    }

    const CV: &str = "Ada Lovelace\nBackend engineer at Nimbus since 2020\n\
                      Skills: Rust, SQL, distributed systems\n\
                      AWS Certified Solutions Architect";

    fn service(agents: Arc<FakeAgent>) -> (ImportService, Arc<FakeProfileRepo>) {
        let profile = Arc::new(FakeProfileRepo::default());
        (
            ImportService::new(
                profile.clone(),
                agents,
                Arc::new(FixedClock::new("2026-10-17T09:00:00.000Z")),
            ),
            profile,
        )
    }

    fn request() -> ImportRequest {
        ImportRequest {
            cli: "claude_code".to_string(),
            model: Some("sonnet".to_string()),
            target_language: "en".to_string(),
            files: vec![ImportFile {
                name: "cv.txt".to_string(),
                bytes: CV.as_bytes().to_vec(),
            }],
        }
    }

    fn proposal(facts: Value) -> AgentEvent {
        AgentEvent::ToolCall {
            name: "mcp__boba__propose_profile_facts".to_string(),
            input: json!({ "facts": facts }),
        }
    }

    fn a_skill() -> Value {
        json!({
            "kind": "skill",
            "text_en": "Rust",
            "evidence": "Skills: Rust",
            "source_excerpt": "Skills: Rust, SQL, distributed systems",
        })
    }

    fn an_experience() -> Value {
        json!({
            "kind": "experience",
            "text_en": "Backend engineer at Nimbus since 2020",
            "source_excerpt": "Backend engineer at Nimbus since 2020",
            "detail": { "organisation": "Nimbus", "start": "2020" },
        })
    }

    // AC1/AC4: the run reads the CV as user input and its proposals are stored
    // pending, not saved to the profile.
    #[test]
    fn a_run_stores_its_verified_facts_as_pending_proposals() {
        let agents = Arc::new(FakeAgent::scripted(vec![
            AgentEvent::Started {
                cli: "claude_code".to_string(),
                model: Some("sonnet".to_string()),
            },
            proposal(json!([a_skill(), an_experience()])),
            AgentEvent::Finished { exit_code: 0 },
        ]));
        let (service, profile) = service(agents.clone());

        let outcome = service.start(request()).expect("import");
        assert_eq!(outcome.accepted, 2);
        assert_eq!(outcome.dropped, 0);
        assert_eq!(outcome.files, vec!["cv.txt"]);
        assert_eq!(outcome.proposals.len(), 2);
        assert_eq!(profile.proposals_pending().unwrap().len(), 2);

        // AC4: the CV is the user's own input.
        for proposal in profile.proposals_pending().unwrap() {
            assert!(!proposal.untrusted_origin, "a CV is trusted input");
            assert_eq!(proposal.untrusted_origin_flag(), 0);
            assert_eq!(proposal.agent_run_id, "run_import");
            assert_eq!(proposal.created_at, "2026-10-17T09:00:00.000Z");
        }
        // AC1: nothing reached the profile.
        assert!(profile.revisions().is_empty());
        assert_eq!(profile.profile_version().unwrap(), 0);

        // The task the run was handed: the user's input, the CV text, the picked
        // engine.
        let task = &agents.tasks()[0];
        assert_eq!(task.kind, AgentTaskKind::ProfileImport);
        assert_eq!(task.input_trust, InputTrust::User);
        assert_eq!(task.cli, "claude_code");
        assert_eq!(task.model.as_deref(), Some("sonnet"));
        assert_eq!(task.target_language, "en");
        assert!(
            task.prompt.contains("Backend engineer at Nimbus"),
            "{}",
            task.prompt
        );
    }

    // AD-5: a fact whose excerpt is not in the CV is dropped, never stored.
    #[test]
    fn a_fact_with_an_excerpt_the_cv_does_not_contain_is_dropped() {
        let invented = json!({
            "kind": "certification",
            "text_en": "Kubernetes Administrator",
            "source_excerpt": "Kubernetes Administrator",
        });
        let agents = Arc::new(FakeAgent::scripted(vec![
            proposal(json!([a_skill(), invented])),
            AgentEvent::Finished { exit_code: 0 },
        ]));
        let (service, profile) = service(agents);

        let outcome = service.start(request()).expect("import");
        assert_eq!(outcome.accepted, 1);
        assert_eq!(outcome.dropped, 1);
        let stored = profile.proposals_pending().unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].draft.text_en.as_deref(), Some("Rust"));
    }

    // AD-5: whitespace in the excerpt is layout, not wording — a match across
    // a line break still counts.
    #[test]
    fn an_excerpt_across_a_line_break_matches_the_cv_text() {
        let across = json!({
            "kind": "skill",
            "text_en": "distributed systems",
            "source_excerpt": "Rust, SQL,\n   distributed systems",
        });
        let agents = Arc::new(FakeAgent::scripted(vec![
            proposal(json!([across])),
            AgentEvent::Finished { exit_code: 0 },
        ]));
        let (service, _) = service(agents);
        let outcome = service.start(request()).expect("import");
        assert_eq!(outcome.accepted, 1);
        assert_eq!(outcome.dropped, 0);
    }

    // FR-21: a run that died still keeps what it proposed, and the step learns
    // why it failed.
    #[test]
    fn a_failed_run_is_reported_and_keeps_its_proposals() {
        let agents = Arc::new(FakeAgent::scripted(vec![
            proposal(json!([a_skill()])),
            AgentEvent::Failed {
                reason: FailureReason::AuthExpired,
                message_key: "errors.agent.auth_expired".to_string(),
            },
        ]));
        let (service, profile) = service(agents);
        let error = service.start(request()).unwrap_err();
        assert!(matches!(
            error,
            ImportError::Run(FailureReason::AuthExpired)
        ));
        assert_eq!(error.message_key(), "errors.profile_import.run");
        assert_eq!(profile.proposals_pending().unwrap().len(), 1);
    }

    // A clean run that proposes nothing is not an error.
    #[test]
    fn a_run_that_proposes_nothing_is_an_empty_outcome() {
        let agents = Arc::new(FakeAgent::scripted(vec![
            proposal(json!([])),
            AgentEvent::Finished { exit_code: 0 },
        ]));
        let (service, profile) = service(agents);
        let outcome = service.start(request()).expect("import");
        assert_eq!(outcome.accepted, 0);
        assert!(outcome.proposals.is_empty());
        assert!(profile.proposals_pending().unwrap().is_empty());
    }

    // AC2's message path: a scanned file never reaches the agent.
    #[test]
    fn a_file_without_text_fails_before_any_run_starts() {
        let agents = Arc::new(FakeAgent::scripted(vec![]));
        let (service, _) = service(agents.clone());
        let mut bad = request();
        bad.files = vec![ImportFile {
            name: "scan.pdf".to_string(),
            bytes: b"%PDF-1.4\nnot really".to_vec(),
        }];
        let error = service.start(bad).unwrap_err();
        assert_eq!(error.message_key(), "errors.profile_import.no_text");
        assert!(agents.tasks().is_empty(), "no run for unreadable text");
    }

    #[test]
    fn the_tool_boundary_refuses_a_payload_outside_the_contract() {
        // Not an array.
        assert!(validate_proposal(&json!({ "facts": "nope" })).is_err());
        // No facts key at all.
        assert!(validate_proposal(&json!({})).is_err());
        // Unknown section.
        assert!(validate_proposal(&json!({ "facts": [{ "kind": "hobby", "text_en": "Chess", "source_excerpt": "Chess" }] })).is_err());
        // No variant.
        assert!(
            validate_proposal(
                &json!({ "facts": [{ "kind": "skill", "text_en": " ", "source_excerpt": "x" }] })
            )
            .is_err()
        );
        // No excerpt: the fact could not be shown with its source (AC1).
        assert!(
            validate_proposal(&json!({ "facts": [{ "kind": "skill", "text_en": "Rust" }] }))
                .is_err()
        );
        // A well-formed payload is accepted.
        assert_eq!(
            validate_proposal(&json!({ "facts": [a_skill()] })).unwrap(),
            1
        );
        // An empty proposal is legal: a CV that says nothing new.
        assert_eq!(validate_proposal(&json!({ "facts": [] })).unwrap(), 0);
    }

    // AC1/AC3: confirming saves the stored text verbatim and drops the proposal.
    #[test]
    fn confirming_saves_the_fact_as_the_user_reviewed_it() {
        let agents = Arc::new(FakeAgent::scripted(vec![
            proposal(json!([an_experience()])),
            AgentEvent::Finished { exit_code: 0 },
        ]));
        let (service, profile) = service(agents);
        let outcome = service.start(request()).expect("import");
        let id = outcome.proposals[0].id.clone();

        let snapshot = service.confirm(&id, None).expect("confirm");
        assert_eq!(snapshot.profile_version, 1);
        assert_eq!(snapshot.pending_proposals, 0);
        assert!(profile.proposals_pending().unwrap().is_empty());

        let fact = &profile.revisions()[0];
        assert_eq!(
            fact.text_en.as_deref(),
            Some("Backend engineer at Nimbus since 2020")
        );
        assert_eq!(fact.kind, FactKind::Experience);
        assert_eq!(fact.provenance, Provenance::User);
        assert_eq!(
            fact.detail_json,
            r#"{"organisation":"Nimbus","start":"2020"}"#
        );
        assert!(fact.text(Lang::Es).is_none());
    }

    // AC1/AC3: an edited fact is what gets saved.
    #[test]
    fn confirming_an_edit_saves_the_edited_text() {
        let with_detail = json!({
            "kind": "skill",
            "text_en": "Rust",
            "source_excerpt": "Skills: Rust",
            "detail": { "level": "advanced" },
        });
        let agents = Arc::new(FakeAgent::scripted(vec![
            proposal(json!([with_detail])),
            AgentEvent::Finished { exit_code: 0 },
        ]));
        let (service, profile) = service(agents);
        let id = service.start(request()).expect("import").proposals[0]
            .id
            .clone();

        service
            .confirm(
                &id,
                Some(FactEdit {
                    text_en: Some("Rust and SQL".to_string()),
                    text_es: Some("Rust y SQL".to_string()),
                    evidence: None,
                    detail_json: None,
                }),
            )
            .expect("confirm");

        let fact: ProfileFact = profile.revisions().remove(0);
        assert_eq!(fact.text_en.as_deref(), Some("Rust and SQL"));
        assert_eq!(fact.text_es.as_deref(), Some("Rust y SQL"));
        assert!(fact.evidence.is_none());
        // The card edits wording only: the run's structured detail survives.
        assert_eq!(fact.detail_json, "{\"level\":\"advanced\"}");
    }

    // AC1: deleting a proposal never saves anything.
    #[test]
    fn discarding_a_proposal_saves_nothing() {
        let agents = Arc::new(FakeAgent::scripted(vec![
            proposal(json!([a_skill()])),
            AgentEvent::Finished { exit_code: 0 },
        ]));
        let (service, profile) = service(agents);
        let id = service.start(request()).expect("import").proposals[0]
            .id
            .clone();

        service.discard(&id).expect("discard");
        assert!(profile.proposals_pending().unwrap().is_empty());
        assert!(profile.revisions().is_empty());
    }

    #[test]
    fn a_stale_proposal_id_is_not_found() {
        let agents = Arc::new(FakeAgent::default());
        let (service, _) = service(agents);
        let error = service.discard("01NOTTHERE").unwrap_err();
        assert_eq!(
            error.message_key(),
            "errors.profile_import.unknown_proposal"
        );
        let error = service.confirm("01NOTTHERE", None).unwrap_err();
        assert_eq!(
            error.message_key(),
            "errors.profile_import.unknown_proposal"
        );
    }

    // AC1: a proposal updated by hand still has to satisfy the profile rules.
    #[test]
    fn an_edit_that_empties_both_variants_is_refused_by_the_profile_rule() {
        let agents = Arc::new(FakeAgent::scripted(vec![
            proposal(json!([a_skill()])),
            AgentEvent::Finished { exit_code: 0 },
        ]));
        let (service, profile) = service(agents);
        let id = service.start(request()).expect("import").proposals[0]
            .id
            .clone();

        let error = service
            .confirm(
                &id,
                Some(FactEdit {
                    text_en: None,
                    text_es: None,
                    evidence: None,
                    detail_json: None,
                }),
            )
            .unwrap_err();
        assert_eq!(error.message_key(), "errors.profile.no_variant");
        // The proposal is still there for the user to fix.
        assert_eq!(profile.proposals_pending().unwrap().len(), 1);
    }

    #[test]
    fn the_view_is_the_stored_text_verbatim() {
        let long = "Led the migration of the billing pipeline to a distributed \
                    architecture across three teams, cutting p99 latency by half \
                    and removing sixteen hours of nightly batch work.";
        let agents = Arc::new(FakeAgent::scripted(vec![
            proposal(json!([{
                "kind": "experience",
                "text_en": long,
                "source_excerpt": long,
            }])),
            AgentEvent::Finished { exit_code: 0 },
        ]));
        let (service, _) = service(agents);
        let mut cv = request();
        cv.files[0].bytes = long.as_bytes().to_vec();
        let view = &service.start(cv).expect("import").proposals[0];
        assert_eq!(view.text_en.as_deref(), Some(long));
        assert_eq!(view.source_excerpt.as_deref(), Some(long));
    }
}
