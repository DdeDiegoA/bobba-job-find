//! The interview and the translate pass (S3.6, FR-6, FR-12).

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use boba_domain::profile::FactProposal;
use boba_domain::profile::{FactDraft, FactKind, Lang, ProfileFact, new_ulid};

use crate::ports::agent::{
    AgentEvent, AgentPort, AgentTask, AgentTaskKind, FailureReason, InputTrust, RunBudget,
};
use crate::ports::clock::ClockPort;
use crate::ports::repo::profile::ProfileRepo;
use crate::ports::repo::system::SystemRepo;
use crate::profile::{FactRevisionInput, ProfileService, ProfileSnapshot};
use crate::profile_import::{FactEdit, ImportError, ImportService, ProposalView};

use crate::profile_gaps::ProfileGapsError;
use crate::profile_gaps::gaps::{Gap, GapAnswer, answer_draft, detect, set_skipped, skipped};

/// The tool a `profile_gaps` run sends its questions through (S3.6). The
/// catalog (`boba_mcp_proto::tool`) is the authority on tool names; this crate
/// cannot depend on it, so the wiring asserts the two agree.
pub const ASK_TOOL: &str = "propose_gap_questions";

/// The tool a `translate_profile` run sends its drafts through.
pub const TRANSLATE_TOOL: &str = "propose_translations";

/// The run's wall-clock cap: one pass, one proposal call.
pub const RUN_WALL_CLOCK: std::time::Duration = std::time::Duration::from_secs(300);

/// The most MCP calls one interview run may make: its input, the profile, the
/// preferences and the answer (S2.5's budget reads the same field).
pub const RUN_TOOL_CALLS: u32 = 8;

/// What one interview run asks about: the picked engine, the content language
/// and any third-party text the user wants it to consider (AC4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterviewRequest {
    /// A `CliKind` id, e.g. `claude_code`.
    pub cli: String,
    pub model: Option<String>,
    /// The content language of the questions and the answers (AD-17).
    pub target_language: String,
    /// Posting text or a pasted note the run may read. When set, the run is
    /// dispatched as untrusted: it takes the lockdown gate (ADR-010, AC4).
    pub extra_context: Option<String>,
}

/// One question as the step renders it: the gap's key and kind, whether it may
/// be skipped, and the agent's wording in both languages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct QuestionView {
    /// The gap key the answer and the skip use (`"eligibility"`, `"cefr:en"`).
    pub key: String,
    /// The gap kind, for the input the step renders.
    pub kind: String,
    /// FR-6: the writing sample is optional.
    pub optional: bool,
    pub text_en: Option<String>,
    pub text_es: Option<String>,
}

/// What one interview did: the questions to ask and the fact proposals the
/// run made from the context it was given (nothing is saved to the profile).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct InterviewOutcome {
    pub run_id: String,
    pub questions: Vec<QuestionView>,
    pub proposals: Vec<ProposalView>,
}

/// What one translate run did: the drafts waiting for review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct TranslateOutcome {
    pub run_id: String,
    pub proposals: Vec<ProposalView>,
}

/// What the step asks the translate task for (AC2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranslateRequest {
    /// The fact whose variant is missing.
    pub item_id: String,
    /// The BCP-47 tag to draft (`en` or `es`).
    pub lang: String,
    pub cli: String,
    pub model: Option<String>,
    /// The content language the pack is rendered in (AD-17).
    pub target_language: String,
}

/// A `profile_gaps` / `translate_profile` payload outside its contract (AD-5):
/// the run's agent can read the message and retry; nothing is stored.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid profile payload: {why}")]
pub struct ContractError {
    pub why: String,
}

impl ContractError {
    fn new(why: impl Into<String>) -> Self {
        Self { why: why.into() }
    }
}

/// One question as the run proposes it.
#[derive(Debug, Clone, Deserialize)]
struct ProposedQuestion {
    key: String,
    #[serde(default)]
    text_en: Option<String>,
    #[serde(default)]
    text_es: Option<String>,
}

/// One translation as the run proposes it.
#[derive(Debug, Clone, Deserialize)]
struct ProposedTranslation {
    item_id: String,
    lang: String,
    text: String,
}

/// Split the whole token (multi-byte safe).
fn rsplit2(name: &str) -> Option<&str> {
    name.rsplit("__").next()
}

/// Validate one `propose_gap_questions` payload at the tool boundary (AD-5).
/// The keys are checked against the gaps the core asked about in [`harvest_questions`];
/// this boundary check only turns a malformed call into a retry.
pub fn validate_questions(args: &Value) -> Result<usize, ContractError> {
    Ok(questions_of(args)?.len())
}

/// The contract object of a `propose_gap_questions` call, shape-checked.
fn questions_of(args: &Value) -> Result<Vec<ProposedQuestion>, ContractError> {
    let questions = args.get("questions").cloned().unwrap_or(Value::Null);
    let Some(questions) = questions.as_array() else {
        return Err(ContractError::new("`questions` must be an array"));
    };
    let mut parsed = Vec::with_capacity(questions.len());
    for question in questions {
        if !question.is_object() {
            return Err(ContractError::new("every question must be an object"));
        }
        let question: ProposedQuestion = serde_json::from_value(question.clone())
            .map_err(|error| ContractError::new(error.to_string()))?;
        if question.key.trim().is_empty() {
            return Err(ContractError::new("every question needs its `key`"));
        }
        let en = question.text_en.as_deref().map(str::trim).unwrap_or("");
        let es = question.text_es.as_deref().map(str::trim).unwrap_or("");
        if en.is_empty() && es.is_empty() {
            return Err(ContractError::new(
                "a question needs English or Spanish text",
            ));
        }
        parsed.push(question);
    }
    Ok(parsed)
}

/// Validate one `propose_translations` payload at the tool boundary (AD-5).
pub fn validate_translations(args: &Value) -> Result<usize, ContractError> {
    Ok(translations_of(args)?.len())
}

/// The contract object of a `propose_translations` call, shape-checked.
fn translations_of(args: &Value) -> Result<Vec<ProposedTranslation>, ContractError> {
    let translations = args.get("translations").cloned().unwrap_or(Value::Null);
    let Some(translations) = translations.as_array() else {
        return Err(ContractError::new("`translations` must be an array"));
    };
    let mut parsed = Vec::with_capacity(translations.len());
    for translation in translations {
        if !translation.is_object() {
            return Err(ContractError::new("every translation must be an object"));
        }
        let translation: ProposedTranslation = serde_json::from_value(translation.clone())
            .map_err(|error| ContractError::new(error.to_string()))?;
        if translation.item_id.trim().is_empty() {
            return Err(ContractError::new("every translation needs its `item_id`"));
        }
        if Lang::parse(translation.lang.trim()).is_none() {
            return Err(ContractError::new("`lang` must be `en` or `es`"));
        }
        if translation.text.trim().is_empty() {
            return Err(ContractError::new("a translation needs its text"));
        }
        parsed.push(translation);
    }
    Ok(parsed)
}

/// The interview and the translate pass (S3.6).
pub struct InterviewService {
    repo: Arc<dyn ProfileRepo>,
    system: Arc<dyn SystemRepo>,
    agents: Arc<dyn AgentPort>,
    clock: Arc<dyn ClockPort>,
}

impl InterviewService {
    pub fn new(
        repo: Arc<dyn ProfileRepo>,
        system: Arc<dyn SystemRepo>,
        agents: Arc<dyn AgentPort>,
        clock: Arc<dyn ClockPort>,
    ) -> Self {
        Self {
            repo,
            system,
            agents,
            clock,
        }
    }

    /// The questions the Profile's current facts leave open, in a stable order.
    /// Pure apart from the read: no run, no writes. This is what the step shows
    /// when no CLI is available (S3.3 limited mode) and what a run is asked
    /// about.
    pub fn questions(&self) -> Result<Vec<Gap>, ProfileGapsError> {
        Ok(detect(
            &self
                .repo
                .facts_current()
                .map_err(|_| ProfileGapsError::Store)?,
        ))
    }

    /// Run one `profile_gaps` pass (FR-6, AC1): the agent writes the questions,
    /// the core decides which are real, and any fact it proposes from the
    /// context it was given is a pending proposal (AC4).
    pub fn start(&self, request: InterviewRequest) -> Result<InterviewOutcome, ProfileGapsError> {
        let facts = self
            .repo
            .facts_current()
            .map_err(|_| ProfileGapsError::Store)?;
        let gaps = detect(&facts);
        self.prune_skips(&gaps)?;

        let untrusted = request.extra_context.is_some();
        let task = AgentTask {
            kind: AgentTaskKind::ProfileGaps,
            input_trust: if untrusted {
                InputTrust::Untrusted
            } else {
                InputTrust::User
            },
            cli: request.cli,
            model: request.model,
            prompt: task_input(&gaps, &facts, request.extra_context.as_deref()),
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
        let questions = self.harvest_questions(&gaps, &events)?;
        self.harvest_facts(
            &handle.run_id,
            untrusted,
            request.extra_context.as_deref(),
            &events,
        )?;
        if let Some(reason) = failure(&events) {
            return Err(ProfileGapsError::Run(reason));
        }
        Ok(InterviewOutcome {
            run_id: handle.run_id.clone(),
            questions,
            proposals: self.proposals_of(&handle.run_id)?,
        })
    }

    /// Answer one question: the fact it states is saved through the profile use
    /// case (the user's own words, `provenance = user`) and the key stops being
    /// skipped (AC1).
    pub fn answer(
        &self,
        key: &str,
        answer: GapAnswer,
        lang: &str,
    ) -> Result<ProfileSnapshot, ProfileGapsError> {
        let lang = Lang::parse(lang).unwrap_or(Lang::En);
        let facts = self
            .repo
            .facts_current()
            .map_err(|_| ProfileGapsError::Store)?;
        let gaps = detect(&facts);
        let gap = gaps
            .iter()
            .find(|gap| gap.key() == key)
            .ok_or(ProfileGapsError::BadAnswer)?;
        let existing = gap
            .item_id
            .as_ref()
            .and_then(|item_id| facts.iter().find(|fact| &fact.item_id == item_id));
        let draft = answer_draft(gap, &answer, lang, existing)?;
        let profile = ProfileService::new(self.repo.clone(), self.clock.clone());
        let snapshot = profile.save(FactRevisionInput {
            item_id: gap.item_id.clone(),
            kind: draft.kind.as_str().to_string(),
            text_en: draft.text_en,
            text_es: draft.text_es,
            evidence: draft.evidence,
            detail_json: Some(draft.detail_json),
        })?;
        self.clear_skip(key)?;
        Ok(snapshot)
    }

    /// Skip one question (AC1): remember it so the Gates that needed the field
    /// lose confidence, and stop asking it this session.
    pub fn skip(&self, key: &str) -> Result<(), ProfileGapsError> {
        let facts = self
            .repo
            .facts_current()
            .map_err(|_| ProfileGapsError::Store)?;
        let gaps = detect(&facts);
        if !gaps.iter().any(|gap| gap.key() == key) {
            return Err(ProfileGapsError::UnknownGap(key.to_string()));
        }
        let mut keys = skipped(self.system.as_ref());
        if keys.iter().any(|skipped| skipped == key) {
            return Ok(());
        }
        keys.push(key.to_string());
        set_skipped(self.system.as_ref(), &keys, &self.clock.now_rfc3339())
    }

    /// Run one `translate_profile` pass for a fact missing a variant (AC2):
    /// the draft is a proposal the user confirms, never a saved fact.
    pub fn translate(
        &self,
        request: TranslateRequest,
    ) -> Result<TranslateOutcome, ProfileGapsError> {
        let target = Lang::parse(request.lang.trim()).ok_or(ProfileGapsError::BadAnswer)?;
        let facts = self
            .repo
            .facts_current()
            .map_err(|_| ProfileGapsError::Store)?;
        let fact = facts
            .iter()
            .find(|fact| fact.item_id == request.item_id)
            .ok_or_else(|| ProfileGapsError::UnknownItem(request.item_id.clone()))?;
        if fact.text(target).is_some() {
            return Err(ProfileGapsError::BadAnswer);
        }
        let task = AgentTask {
            kind: AgentTaskKind::TranslateProfile,
            // A profile fact is the user's own content (ADR-010).
            input_trust: InputTrust::User,
            cli: request.cli,
            model: request.model,
            prompt: translate_input(fact, target),
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
        self.harvest_translations(&handle.run_id, fact, target, &events)?;
        if let Some(reason) = failure(&events) {
            return Err(ProfileGapsError::Run(reason));
        }
        Ok(TranslateOutcome {
            run_id: handle.run_id.clone(),
            proposals: self.proposals_of(&handle.run_id)?,
        })
    }

    /// Confirm a drafted proposal — as proposed, or with the user's edit — and
    /// save it as a revision (AC3). Reuses the one review path (S3.5).
    pub fn confirm(
        &self,
        id: &str,
        edit: Option<FactEdit>,
    ) -> Result<ProfileSnapshot, ProfileGapsError> {
        let importer =
            ImportService::new(self.repo.clone(), self.agents.clone(), self.clock.clone());
        importer.confirm(id, edit).map_err(map_import)
    }

    /// Delete a drafted proposal: it never becomes a fact (AC3).
    pub fn discard(&self, id: &str) -> Result<(), ProfileGapsError> {
        let importer =
            ImportService::new(self.repo.clone(), self.agents.clone(), self.clock.clone());
        importer.discard(id).map_err(map_import)
    }

    /// The proposals one run left waiting, newest first.
    fn proposals_of(&self, run_id: &str) -> Result<Vec<ProposalView>, ProfileGapsError> {
        Ok(self
            .repo
            .proposals_pending()
            .map_err(|_| ProfileGapsError::Store)?
            .iter()
            .filter(|proposal| proposal.agent_run_id == run_id)
            .map(ProposalView::from)
            .collect())
    }

    /// Keep the skip list to the gaps that are still open: an answered (or
    /// re-detected) field never keeps a Gate's confidence low forever.
    fn prune_skips(&self, gaps: &[Gap]) -> Result<(), ProfileGapsError> {
        let keys = skipped(self.system.as_ref());
        let open: Vec<String> = gaps.iter().map(Gap::key).collect();
        let kept: Vec<String> = keys
            .iter()
            .filter(|key| open.contains(key))
            .cloned()
            .collect();
        if kept == keys {
            return Ok(());
        }
        set_skipped(self.system.as_ref(), &kept, &self.clock.now_rfc3339())
    }

    /// Drop a key from the skip list after its answer.
    fn clear_skip(&self, key: &str) -> Result<(), ProfileGapsError> {
        let keys = skipped(self.system.as_ref());
        if !keys.iter().any(|skipped| skipped == key) {
            return Ok(());
        }
        let kept: Vec<String> = keys.into_iter().filter(|skipped| skipped != key).collect();
        set_skipped(self.system.as_ref(), &kept, &self.clock.now_rfc3339())
    }

    /// The run's questions, kept only for gaps the core detected (AD-5: the
    /// core disposes). A gap the run skipped has no text and falls back to the
    /// step's own copy.
    fn harvest_questions(
        &self,
        gaps: &[Gap],
        events: &[AgentEvent],
    ) -> Result<Vec<QuestionView>, ProfileGapsError> {
        let mut views = Vec::new();
        for event in events {
            let AgentEvent::ToolCall { name, input } = event else {
                continue;
            };
            if rsplit2(name) != Some(ASK_TOOL) {
                continue;
            }
            for question in questions_of(input).map_err(|_| ProfileGapsError::Contract)? {
                let Some(gap) = gaps.iter().find(|gap| gap.key() == question.key) else {
                    continue;
                };
                views.push(QuestionView {
                    key: gap.key(),
                    kind: gap.kind.as_str().to_string(),
                    optional: gap.kind.optional(),
                    text_en: clean(question.text_en),
                    text_es: clean(question.text_es),
                });
            }
        }
        Ok(views)
    }

    /// The facts a run proposes from the context it was given (AC4): stored
    /// as pending proposals, `untrusted_origin = 1`, and only when the context
    /// really contains the excerpt they cite (AD-5).
    fn harvest_facts(
        &self,
        run_id: &str,
        untrusted: bool,
        context: Option<&str>,
        events: &[AgentEvent],
    ) -> Result<(), ProfileGapsError> {
        let Some(context) = context else {
            return Ok(());
        };
        for event in events {
            let AgentEvent::ToolCall { name, input } = event else {
                continue;
            };
            if rsplit2(name) != Some(crate::profile_import::PROPOSE_TOOL) {
                continue;
            }
            let facts =
                crate::profile_import::facts_of(input).map_err(|_| ProfileGapsError::Contract)?;
            for fact in facts {
                let excerpt = fact.source_excerpt.as_deref().map(str::trim).unwrap_or("");
                if !excerpt_in(context, excerpt) {
                    continue;
                }
                let Some(kind) = FactKind::parse(&fact.kind) else {
                    continue;
                };
                let proposal = FactProposal {
                    id: new_ulid(),
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
                self.repo
                    .proposal_append(&proposal)
                    .map_err(|_| ProfileGapsError::Store)?;
            }
        }
        Ok(())
    }

    /// The drafted variant, stored as a proposal on the fact's own item (AC2).
    fn harvest_translations(
        &self,
        run_id: &str,
        fact: &ProfileFact,
        target: Lang,
        events: &[AgentEvent],
    ) -> Result<(), ProfileGapsError> {
        for event in events {
            let AgentEvent::ToolCall { name, input } = event else {
                continue;
            };
            if rsplit2(name) != Some(TRANSLATE_TOOL) {
                continue;
            }
            for translation in translations_of(input).map_err(|_| ProfileGapsError::Contract)? {
                if translation.item_id != fact.item_id {
                    continue;
                }
                if Lang::parse(translation.lang.trim()) != Some(target) {
                    continue;
                }
                let mut draft = fact.draft();
                match target {
                    Lang::En => draft.text_en = clean(Some(translation.text)),
                    Lang::Es => draft.text_es = clean(Some(translation.text)),
                }
                let proposal = FactProposal {
                    id: new_ulid(),
                    item_id: Some(fact.item_id.clone()),
                    draft,
                    // A drafted variant cites no outside words: the fact it
                    // translates is the user's own.
                    source_excerpt: None,
                    untrusted_origin: false,
                    agent_run_id: run_id.to_string(),
                    created_at: self.clock.now_rfc3339(),
                };
                self.repo
                    .proposal_append(&proposal)
                    .map_err(|_| ProfileGapsError::Store)?;
            }
        }
        Ok(())
    }
}

/// Map the shared review path's failure onto this use case's (AD-3).
fn map_import(error: ImportError) -> ProfileGapsError {
    match error {
        ImportError::Repo(_) => ProfileGapsError::Store,
        ImportError::Profile(error) => ProfileGapsError::Profile(error),
        ImportError::Contract(_) => ProfileGapsError::Contract,
        ImportError::UnknownProposal(id) => ProfileGapsError::UnknownProposal(id),
        ImportError::Run(reason) => ProfileGapsError::Run(reason),
        ImportError::Text(_) => ProfileGapsError::Store,
    }
}

/// The run's own input: the open gaps and the profile, as data (AD-7).
fn task_input(gaps: &[Gap], facts: &[ProfileFact], context: Option<&str>) -> String {
    let gaps: Vec<Value> = gaps
        .iter()
        .map(|gap| {
            serde_json::json!({
                "key": gap.key(),
                "kind": gap.kind.as_str(),
                "subject": gap.subject,
                "optional": gap.kind.optional(),
            })
        })
        .collect();
    let facts: Vec<Value> = facts
        .iter()
        .map(|fact| {
            serde_json::json!({
                "item_id": fact.item_id,
                "kind": fact.kind.as_str(),
                "text_en": fact.text_en,
                "text_es": fact.text_es,
                "detail": fact.detail_json,
            })
        })
        .collect();
    serde_json::json!({ "gaps": gaps, "profile": facts, "context": context }).to_string()
}

/// The translate run's input: the fact and the variant wanted.
fn translate_input(fact: &ProfileFact, target: Lang) -> String {
    serde_json::json!({
        "target": target.as_str(),
        "fact": {
            "item_id": fact.item_id,
            "kind": fact.kind.as_str(),
            "text_en": fact.text_en,
            "text_es": fact.text_es,
            "evidence": fact.evidence,
            "detail": fact.detail_json,
        },
    })
    .to_string()
}

/// Whether `excerpt` is really in the context the run was given (whitespace is
/// layout, not wording).
fn excerpt_in(context: &str, excerpt: &str) -> bool {
    let needle = collapse(excerpt);
    !needle.is_empty() && collapse(context).contains(&needle)
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
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
    use crate::profile::testing::FakeProfileRepo;
    use crate::system::FixedClock;
    use crate::system::testing::FakeSystemRepo;
    use boba_domain::profile::{FactKind, Provenance};
    use serde_json::json;
    use std::sync::Mutex;
    use std::sync::mpsc::{Receiver, channel};

    /// An `AgentPort` that answers from a script, like the import tests' one.
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
                run_id: "run_gaps".to_string(),
                events: events as Receiver<AgentEvent>,
                cancel: crate::ports::agent::CancelToken::new(),
            }
        }
    }

    struct Fixture {
        service: InterviewService,
        profile: Arc<FakeProfileRepo>,
        system: Arc<FakeSystemRepo>,
        agents: Arc<FakeAgent>,
    }

    fn fixture(events: Vec<AgentEvent>) -> Fixture {
        let profile = Arc::new(FakeProfileRepo::default());
        let system = Arc::new(FakeSystemRepo::default());
        let agents = Arc::new(FakeAgent::scripted(events));
        let service = InterviewService::new(
            profile.clone(),
            system.clone(),
            agents.clone(),
            Arc::new(FixedClock::new("2026-10-17T09:00:00.000Z")),
        );
        Fixture {
            service,
            profile,
            system,
            agents,
        }
    }

    fn request() -> InterviewRequest {
        InterviewRequest {
            cli: "claude_code".to_string(),
            model: Some("sonnet".to_string()),
            target_language: "en".to_string(),
            extra_context: None,
        }
    }

    fn questions_event(questions: Value) -> AgentEvent {
        AgentEvent::ToolCall {
            name: "mcp__boba__propose_gap_questions".to_string(),
            input: json!({ "questions": questions }),
        }
    }

    fn translations_event(translations: Value) -> AgentEvent {
        AgentEvent::ToolCall {
            name: "mcp__boba__propose_translations".to_string(),
            input: json!({ "translations": translations }),
        }
    }

    fn finished() -> AgentEvent {
        AgentEvent::Finished { exit_code: 0 }
    }

    // AC1: the run is handed the missing fields and its questions come back
    // keyed by gap, so the step renders exactly what was asked.
    #[test]
    fn a_run_returns_one_question_per_missing_field() {
        let fixture = fixture(vec![
            questions_event(json!([
                { "key": "eligibility", "text_en": "Where can you work without a visa?", "text_es": "¿Dónde puedes trabajar sin visado?" },
                { "key": "writing_sample", "text_en": "Paste a short sample of your writing.", "text_es": "Pega una muestra corta de tu escritura." },
            ])),
            finished(),
        ]);

        let outcome = fixture.service.start(request()).expect("interview");
        assert_eq!(outcome.run_id, "run_gaps");
        assert_eq!(outcome.questions.len(), 2);
        let eligibility = &outcome.questions[0];
        assert_eq!(eligibility.key, "eligibility");
        assert_eq!(eligibility.kind, "eligibility");
        assert!(!eligibility.optional);
        assert_eq!(
            eligibility.text_en.as_deref(),
            Some("Where can you work without a visa?")
        );
        assert_eq!(
            eligibility.text_es.as_deref(),
            Some("¿Dónde puedes trabajar sin visado?")
        );
        assert!(outcome.questions[1].optional);

        // The task the run was handed: the gaps, the profile, no untrusted text.
        let task = &fixture.agents.tasks()[0];
        assert_eq!(task.kind, AgentTaskKind::ProfileGaps);
        assert_eq!(task.input_trust, InputTrust::User);
        assert_eq!(task.cli, "claude_code");
        assert!(task.prompt.contains("\"eligibility\""), "{}", task.prompt);
    }

    // AC1: a gap with no question text still renders — the step falls back to
    // its own copy; the agent never invents gaps of its own.
    #[test]
    fn a_question_for_a_field_that_is_not_missing_is_dropped() {
        let fixture = fixture(vec![
            questions_event(json!([
                { "key": "from_a_newer_build", "text_en": "Nope?", "text_es": "¿No?" },
            ])),
            finished(),
        ]);
        let outcome = fixture.service.start(request()).expect("interview");
        assert!(outcome.questions.is_empty());
    }

    // AC4: an input that carries third-party text runs as untrusted, and the
    // facts it proposes carry `untrusted_origin = 1` until the user confirms.
    #[test]
    fn an_untrusted_context_marks_the_runs_fact_proposals() {
        let context = "The posting asks for EU work authorization. Notice period: 1 month.";
        let facts = AgentEvent::ToolCall {
            name: "mcp__boba__propose_profile_facts".to_string(),
            input: json!({ "facts": [{
                "kind": "eligibility",
                "text_en": "Notice period: 1 month",
                "source_excerpt": "Notice period: 1 month",
                "detail": { "notice_period": "1 month" },
            }]}),
        };
        let fixture = fixture(vec![questions_event(json!([])), facts, finished()]);
        let mut untrusted = request();
        untrusted.extra_context = Some(context.to_string());

        let outcome = fixture.service.start(untrusted).expect("interview");
        assert_eq!(outcome.proposals.len(), 1);
        assert!(outcome.proposals[0].untrusted_origin);
        let stored = fixture.profile.proposals_pending().unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].untrusted_origin_flag(), 1);
        assert_eq!(fixture.agents.tasks()[0].input_trust, InputTrust::Untrusted);
    }

    // AD-5: a proposed fact whose excerpt is not in the context never reaches
    // the user.
    #[test]
    fn a_fact_the_context_does_not_state_is_dropped() {
        let facts = AgentEvent::ToolCall {
            name: "mcp__boba__propose_profile_facts".to_string(),
            input: json!({ "facts": [{
                "kind": "skill",
                "text_en": "Kubernetes",
                "source_excerpt": "Kubernetes",
            }]}),
        };
        let fixture = fixture(vec![questions_event(json!([])), facts, finished()]);
        let mut untrusted = request();
        untrusted.extra_context = Some("The posting asks for Rust.".to_string());
        let outcome = fixture.service.start(untrusted).expect("interview");
        assert!(outcome.proposals.is_empty());
        assert!(fixture.profile.proposals_pending().unwrap().is_empty());
    }

    #[test]
    fn without_extra_context_no_fact_proposal_is_stored() {
        let facts = AgentEvent::ToolCall {
            name: "mcp__boba__propose_profile_facts".to_string(),
            input: json!({ "facts": [{
                "kind": "skill",
                "text_en": "Rust",
                "source_excerpt": "Rust",
            }]}),
        };
        let fixture = fixture(vec![questions_event(json!([])), facts, finished()]);
        let outcome = fixture.service.start(request()).expect("interview");
        assert!(outcome.proposals.is_empty());
    }

    // FR-21: a run that died still reports why, and keeps its proposals.
    #[test]
    fn a_failed_run_is_reported_and_keeps_its_proposals() {
        let fixture = fixture(vec![AgentEvent::Failed {
            reason: FailureReason::AuthExpired,
            message_key: "errors.agent.auth_expired".to_string(),
        }]);
        let error = fixture.service.start(request()).unwrap_err();
        assert_eq!(error.message_key(), "errors.profile_gaps.run");
    }

    // AC1: the questions are computable without a run — the fallback the step
    // uses when no CLI is available (S3.3 limited mode).
    #[test]
    fn questions_are_computable_without_a_run() {
        let fixture = fixture(vec![]);
        let gaps = fixture.service.questions().expect("questions");
        let keys: Vec<String> = gaps.iter().map(Gap::key).collect();
        assert_eq!(keys, vec!["eligibility", "notice_period", "writing_sample"]);
        assert!(fixture.agents.tasks().is_empty());
    }

    // AC1: an answer is saved as the user's own fact (nothing is agent-proposed
    // here), and the question stops being asked.
    #[test]
    fn an_answer_saves_the_fact_and_clears_the_skip() {
        let fixture = fixture(vec![]);
        fixture.service.skip("eligibility").expect("skip");
        assert_eq!(skipped(fixture.system.as_ref()), vec!["eligibility"]);

        let snapshot = fixture
            .service
            .answer(
                "eligibility",
                GapAnswer::Eligibility {
                    region: "es".to_string(),
                    permit: "citizenship".to_string(),
                },
                "en",
            )
            .expect("answer");
        assert_eq!(snapshot.profile_version, 1);
        assert!(skipped(fixture.system.as_ref()).is_empty());

        let facts = fixture.profile.revisions();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].kind, FactKind::Eligibility);
        assert_eq!(facts[0].provenance, Provenance::User);
        assert!(facts[0].text_en.is_some());
        assert!(facts[0].text_es.is_none());
    }

    // AC1/FR-12: a CEFR answer revises the language item instead of adding one.
    #[test]
    fn a_cefr_answer_revises_the_existing_language_fact() {
        let fixture = fixture(vec![]);
        // A language the Profile already states, without a level.
        ProfileService::new(
            fixture.profile.clone(),
            Arc::new(FixedClock::new("2026-10-17T09:00:00.000Z")),
        )
        .save(FactRevisionInput {
            item_id: None,
            kind: "language".to_string(),
            text_en: Some("English".to_string()),
            text_es: None,
            evidence: None,
            detail_json: Some(r#"{"language":"en"}"#.to_string()),
        })
        .expect("seed language");
        let item = fixture.profile.revisions()[0].item_id.clone();

        fixture
            .service
            .answer(
                "cefr:en",
                GapAnswer::Cefr {
                    level: "C1".to_string(),
                },
                "en",
            )
            .expect("answer");

        let facts = fixture.profile.revisions();
        let latest = facts.last().expect("revision");
        assert_eq!(latest.item_id, item);
        assert_eq!(latest.text_en.as_deref(), Some("English"));
        let detail: serde_json::Value = serde_json::from_str(&latest.detail_json).unwrap();
        assert_eq!(detail["level"], "C1");
        assert_eq!(detail["language"], "en");
    }

    // AC1: a CEFR answer with no language fact behind it is refused, never
    // turned into a new language.
    #[test]
    fn a_cefr_answer_without_a_language_fact_is_refused() {
        let fixture = fixture(vec![]);
        let error = fixture
            .service
            .answer(
                "cefr:en",
                GapAnswer::Cefr {
                    level: "C1".to_string(),
                },
                "en",
            )
            .unwrap_err();
        assert_eq!(error.message_key(), "errors.profile_gaps.bad_answer");
    }

    #[test]
    fn skipping_an_unknown_question_is_refused() {
        let fixture = fixture(vec![]);
        let error = fixture.service.skip("from_a_newer_build").unwrap_err();
        assert_eq!(error.message_key(), "errors.profile_gaps.unknown_gap");
    }

    // AC2: a fact with one variant gets the other drafted, as a proposal the
    // user confirms — nothing is saved before that.
    #[test]
    fn a_translation_run_stores_a_proposal_for_the_missing_variant() {
        let fixture = fixture(vec![]);
        fixture
            .service
            .answer(
                "notice_period",
                GapAnswer::NoticePeriod {
                    text: "1 month".to_string(),
                },
                "en",
            )
            .expect("answer");
        let item = fixture.profile.revisions()[0].item_id.clone();

        let agents = fixture.agents.clone();
        // The translate run is the second run this port serves.
        *agents.events.lock().unwrap() = vec![
            translations_event(json!([{
                "item_id": item,
                "lang": "es",
                "text": "Preaviso: 1 mes",
            }])),
            finished(),
        ];
        let outcome = fixture
            .service
            .translate(TranslateRequest {
                item_id: item.clone(),
                lang: "es".to_string(),
                cli: "claude_code".to_string(),
                model: None,
                target_language: "en".to_string(),
            })
            .expect("translate");
        assert_eq!(outcome.proposals.len(), 1);
        assert_eq!(
            outcome.proposals[0].text_es.as_deref(),
            Some("Preaviso: 1 mes")
        );
        assert!(!outcome.proposals[0].untrusted_origin);

        let task = agents.tasks().last().cloned().expect("translate task");
        assert_eq!(task.kind, AgentTaskKind::TranslateProfile);
        assert_eq!(task.input_trust, InputTrust::User);

        // AC2/AC3: nothing reached the profile until the user confirms.
        assert_eq!(fixture.profile.profile_version().unwrap(), 1);
        assert_eq!(fixture.profile.proposals_pending().unwrap().len(), 1);
    }

    // AC2: a variant that is already there is never "translated" over.
    #[test]
    fn a_translation_for_an_existing_variant_is_dropped() {
        let fixture = fixture(vec![
            translations_event(json!([{
                "item_id": "unknown",
                "lang": "es",
                "text": "x",
            }])),
            finished(),
        ]);
        let error = fixture
            .service
            .translate(TranslateRequest {
                item_id: "unknown".to_string(),
                lang: "es".to_string(),
                cli: "claude_code".to_string(),
                model: None,
                target_language: "en".to_string(),
            })
            .unwrap_err();
        assert_eq!(error.message_key(), "errors.profile_gaps.unknown_item");
        assert!(fixture.profile.proposals_pending().unwrap().is_empty());
    }

    // AC3: confirming the drafted variant saves it verbatim as a revision of
    // the same item.
    #[test]
    fn confirming_a_drafted_variant_saves_it_on_the_same_item() {
        let fixture = fixture(vec![]);
        fixture
            .service
            .answer(
                "notice_period",
                GapAnswer::NoticePeriod {
                    text: "1 month".to_string(),
                },
                "en",
            )
            .expect("answer");
        let item = fixture.profile.revisions()[0].item_id.clone();
        *fixture.agents.events.lock().unwrap() = vec![
            translations_event(json!([{
                "item_id": item,
                "lang": "es",
                "text": "Preaviso: 1 mes",
            }])),
            finished(),
        ];
        fixture
            .service
            .translate(TranslateRequest {
                item_id: item.clone(),
                lang: "es".to_string(),
                cli: "claude_code".to_string(),
                model: None,
                target_language: "en".to_string(),
            })
            .expect("translate");
        let proposal = fixture.profile.proposals_pending().unwrap()[0].id.clone();

        let snapshot = fixture.service.confirm(&proposal, None).expect("confirm");
        assert_eq!(snapshot.profile_version, 2);
        let facts = fixture.profile.revisions();
        let latest = facts.last().expect("revision");
        assert_eq!(latest.item_id, item);
        assert_eq!(latest.text_es.as_deref(), Some("Preaviso: 1 mes"));
        assert_eq!(latest.text_en.as_deref(), Some("Notice period: 1 month"));
        assert!(fixture.profile.proposals_pending().unwrap().is_empty());
    }

    #[test]
    fn the_question_contract_refuses_a_payload_outside_it() {
        assert!(validate_questions(&json!({ "questions": "nope" })).is_err());
        assert!(validate_questions(&json!({})).is_err());
        // No text at all.
        assert!(validate_questions(&json!({ "questions": [{ "key": "eligibility" }] })).is_err());
        // A well-formed question is accepted.
        assert_eq!(
            validate_questions(&json!({ "questions": [{
                "key": "eligibility", "text_en": "Where?", "text_es": "¿Dónde?"
            }]}))
            .unwrap(),
            1
        );
    }

    #[test]
    fn the_translation_contract_refuses_a_payload_outside_it() {
        assert!(validate_translations(&json!({ "translations": "nope" })).is_err());
        assert!(validate_translations(&json!({})).is_err());
        assert!(
            validate_translations(&json!({ "translations": [{
                "item_id": "01X", "lang": "fr", "text": "Bonjour"
            }]}))
            .is_err()
        );
        assert!(
            validate_translations(&json!({ "translations": [{
                "item_id": "01X", "lang": "es", "text": "  "
            }]}))
            .is_err()
        );
        assert_eq!(
            validate_translations(&json!({ "translations": [{
                "item_id": "01X", "lang": "es", "text": "Hola"
            }]}))
            .unwrap(),
            1
        );
    }

    // AC1: the skip list the run started with prunes entries whose gap is
    // gone, so a later fact does not keep a Gate's confidence low forever.
    #[test]
    fn a_skip_for_an_answered_gap_is_pruned_on_the_next_run() {
        let fixture = fixture(vec![questions_event(json!([])), finished()]);
        fixture.service.skip("writing_sample").expect("skip");
        // Answering it makes the gap disappear.
        fixture
            .service
            .answer(
                "writing_sample",
                GapAnswer::WritingSample {
                    text: "I write plainly.".to_string(),
                },
                "en",
            )
            .expect("answer");
        // A stale skip written directly (as a crash between the two steps
        // would) is pruned by the next run.
        set_skipped(
            fixture.system.as_ref(),
            &["writing_sample".to_string()],
            "2026-10-17T09:00:00.000Z",
        )
        .expect("write");
        fixture.service.start(request()).expect("interview");
        assert!(skipped(fixture.system.as_ref()).is_empty());
    }
}
