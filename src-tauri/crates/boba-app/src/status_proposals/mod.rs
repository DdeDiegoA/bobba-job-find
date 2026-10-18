//! Reply classification and Status Proposals (S8.4, FR-100..FR-103, NFR-3,
//! ADR-010).
//!
//! A stored recruiter reply is read twice, and only when it has to be:
//!
//! 1. **Deterministic first** (AC1): the EN/ES rules in `boba-mail` run offline
//!    on the stored plain text — the classifier arrives through
//!    [`ReplyClassifier`] so this crate never depends on the adapter.
//! 2. **The LLM only for uncertain cases** (AC1), and only when the user chose
//!    that mode and saw the disclosure (AC3). The run proposes through the
//!    `propose_status_change` tool; the core verifies every proposal (the
//!    message was one of the batch, the application is a candidate, the quote
//!    is in the message) and stores it tainted (AC6, ADR-010).
//!
//! What the core keeps:
//!
//! * **The class, confidence and quoted line** of each message (AC1), in
//!   `email_classification`.
//! * **A Status Proposal** when the class implies a Stage change (AC2), with
//!   the Application when the message named one unambiguously, and without it
//!   when the user must pick. Approval is the only thing that calls the Tracker
//!   transition (`source = status_proposal`).
//! * **An automatic change** only for a high-confidence Acknowledgement or
//!   Rejection (AC5), and it is logged by the status event and can be undone.
//!   The threshold is above what an LLM proposal may carry, so an automatic
//!   change is always backed by a rule the core can re-check.
//! * **Unclassified messages** in Deterministic-only mode (AC4, NFR-3): no
//!   email text leaves the machine and the message waits for manual triage.

pub mod repo;

use std::sync::Arc;

use boba_domain::application::{AppStatus, Application};
use boba_domain::mail::{
    AUTO_CONFIRM_CONFIDENCE, ClassificationMethod, LLM_CONFIDENCE_CEILING, ProposalState,
    ReplyClass, RuleMatch, StatusProposal,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use crate::applications::{OfferSnapshot, TrackerService, TransitionInput};
use crate::ports::agent::{
    AgentEvent, AgentPort, AgentTask, AgentTaskKind, FailureReason, InputTrust, RunBudget,
};
use crate::ports::clock::ClockPort;
use crate::ports::repo::apply::{ApplyRepo, ApplyRepoError};
use crate::ports::repo::ingestion::{IngestionRepo, IngestionRepoError};
use crate::ports::repo::system::{SystemRepo, SystemRepoError};
use repo::{Classification, StatusProposalRepo, StatusProposalRepoError, StoredMessage};

/// The classifier the deterministic stage uses. `boba-mail::rules` implements
/// it; a test hands in a script instead.
pub trait ReplyClassifier: Send + Sync {
    fn classify(&self, subject: &str, body: &str) -> Option<RuleMatch>;
}

/// The `setting` key holding the classification mode (`llm` or `deterministic`).
pub const SETTING_MODE: &str = "email_classification_mode";

/// The `setting` key recording that the user saw the disclosure (AC3).
pub const SETTING_DISCLOSURE_ACK: &str = "email_classification_disclosure_ack";

/// The `setting` key for auto-confirm (AC5). Off unless the user turns it on.
pub const SETTING_AUTO_CONFIRM: &str = "email_auto_confirm_high_confidence";

/// The tool a classification run proposes through. The catalog
/// (`boba_mcp_proto::tool`) is the authority on the name; this crate cannot
/// depend on it, so the wiring asserts the two agree.
pub const PROPOSE_TOOL: &str = "propose_status_change";

/// The confidence an LLM proposal is stored with. It sits below
/// [`LLM_CONFIDENCE_CEILING`] and well below auto-confirm: a run's own claim is
/// never evidence (AC5 vs AC6).
pub const LLM_CONFIDENCE: f64 = 0.85;

/// The wall-clock cap of one classification run.
pub const RUN_WALL_CLOCK: std::time::Duration = std::time::Duration::from_secs(180);

/// How much of one message body the run is given. Enough to see the verdict,
/// never the whole thread.
const BODY_PROMPT_CHARS: usize = 4000;

/// Which classifier runs after the rules (AC3, AC4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ClassificationMode {
    /// The rules first, then the user's agent CLI for uncertain cases.
    Llm,
    /// The rules only; nothing leaves the machine (NFR-3).
    Deterministic,
}

impl ClassificationMode {
    /// The stored form.
    pub const fn as_str(self) -> &'static str {
        match self {
            ClassificationMode::Llm => "llm",
            ClassificationMode::Deterministic => "deterministic",
        }
    }

    /// Parse the stored form, `None` for anything else.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "llm" => Some(Self::Llm),
            "deterministic" => Some(Self::Deterministic),
            _ => None,
        }
    }
}

/// What Settings → Privacy and Today show about reply reading (AC3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct DisclosureView {
    /// The classifier the user chose.
    pub mode: ClassificationMode,
    /// Whether the plain-language disclosure has been accepted.
    pub acknowledged: bool,
    /// Whether high-confidence Acknowledgement/Rejection is applied
    /// automatically (AC5). Off by default.
    pub auto_confirm: bool,
    /// True while the mode is LLM and the disclosure has not been accepted: the
    /// first classification must show it before any text leaves the machine.
    pub needs_disclosure: bool,
}

/// One Application a proposal may move (AC2). Only what the picker shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct CandidateView {
    pub application_id: String,
    pub company: String,
    pub title: String,
    /// The stored status (`applied`, `interview`, …).
    pub status: String,
}

/// One proposal as Today's needs-you queue shows it (AC2).
///
/// `quoted_line` is always rendered as plain text (AC6, ADR-010): it is the
/// message's words, never markup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct StatusProposalView {
    pub id: String,
    /// `acknowledgement`, `rejection`, `interview_invite`, `assessment`, `offer`.
    pub class: String,
    pub confidence: f64,
    /// The message line the class was read from, verbatim. Plain text only.
    pub quoted_line: String,
    /// ADR-010: `true` when the class or the quote came from the LLM.
    pub untrusted_origin: bool,
    pub message_id: String,
    pub message_subject: String,
    pub message_from: String,
    pub received_at: String,
    /// The Application the proposal moves, when the message named one.
    pub application: Option<CandidateView>,
    /// The Applications the user may pick from (the matches, or every open
    /// Application when nothing matched). Empty when `application` is set.
    pub candidates: Vec<CandidateView>,
    /// Whether approving it needs the user to pick an Application first.
    pub needs_pick: bool,
    /// The status the proposal moves the card to.
    pub to_status: String,
    pub created_at: String,
}

/// What one classification pass did (AC1..AC5).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ClassifyOutcome {
    /// Messages that had not been read before this pass.
    pub messages: u32,
    /// Messages the rules classified.
    pub classified: u32,
    /// Status Proposals created.
    pub proposals: u32,
    /// Proposals auto-confirm applied (AC5).
    pub auto_applied: u32,
    /// The auto-applied proposals, so the screen can offer Undo (AC5).
    pub auto_applied_ids: Vec<String>,
    /// Messages no classifier could place; they wait for manual triage (AC4).
    pub uncertain: u32,
    /// LLM proposals the core refused (a quote not in the message, an unknown
    /// application): the run's output is a proposal, never an instruction.
    pub dropped: u32,
    /// Whether the agent CLI was asked at all.
    pub llm_asked: bool,
}

/// What one approval did (AC2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ApproveOutcome {
    pub proposal_id: String,
    pub application_id: String,
    pub from_status: String,
    pub to_status: String,
}

/// What a batch approval did (AC2: approve singly or in a batch).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct BatchOutcome {
    /// The proposals that moved their card.
    pub approved: Vec<String>,
    /// The proposals that need the user to pick an Application first.
    pub needs_pick: Vec<String>,
    /// The proposals that could not be approved (stale, unknown, already
    /// decided, or the card already sat where the proposal points).
    pub failed: Vec<String>,
}

/// A status-proposal failure (AD-3: the UI translates `message_key`).
#[derive(Debug, Clone, thiserror::Error)]
pub enum StatusProposalsError {
    #[error(transparent)]
    Repo(#[from] StatusProposalRepoError),
    #[error(transparent)]
    Apply(#[from] ApplyRepoError),
    #[error(transparent)]
    Postings(#[from] IngestionRepoError),
    #[error(transparent)]
    Settings(#[from] SystemRepoError),
    #[error(transparent)]
    Tracker(#[from] crate::applications::TrackerError),
    /// The user picked LLM mode but has not seen the disclosure yet (AC3).
    #[error("the reply-reading disclosure has not been accepted")]
    DisclosureRequired,
    /// The proposal is not one this build holds (a stale queue).
    #[error("unknown status proposal: {0}")]
    UnknownProposal(String),
    /// The proposal was already approved, dismissed or undone.
    #[error("status proposal {0} was already decided")]
    NotPending(String),
    /// Only an applied proposal can be undone.
    #[error("status proposal {0} moved no card")]
    NotUndoable(String),
    /// The card moved since the proposal was applied; undoing it now would be a
    /// guess, so the user moves it by hand.
    #[error("the card moved after the proposal was applied")]
    StaleCard,
    /// The message did not name one Application unambiguously (AC2).
    #[error("status proposal {0} needs an Application")]
    NeedsApplication(String),
    /// The card already sits where the proposal points.
    #[error("the card is already in that Stage")]
    AlreadyThere,
    /// The run ended without a usable proposal (FR-21).
    #[error("the classification run failed: {0:?}")]
    Run(FailureReason),
}

impl StatusProposalsError {
    /// The i18n key the UI shows.
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::Repo(_) => "errors.status_proposals.store",
            Self::Apply(_) | Self::Postings(_) | Self::Settings(_) | Self::Tracker(_) => {
                "errors.status_proposals.store"
            }
            Self::DisclosureRequired => "errors.status_proposals.disclosure_required",
            Self::UnknownProposal(_) => "errors.status_proposals.unknown_proposal",
            Self::NotPending(_) => "errors.status_proposals.not_pending",
            Self::NotUndoable(_) => "errors.status_proposals.not_undoable",
            Self::StaleCard => "errors.status_proposals.stale",
            Self::NeedsApplication(_) => "errors.status_proposals.needs_application",
            Self::AlreadyThere => "errors.status_proposals.already_there",
            Self::Run(_) => "errors.status_proposals.run",
        }
    }
}

/// The reply-classification and Status Proposal use cases over the owner ports.
pub struct StatusProposalsService {
    repo: Arc<dyn StatusProposalRepo>,
    apply: Arc<dyn ApplyRepo>,
    postings: Arc<dyn IngestionRepo>,
    system: Arc<dyn SystemRepo>,
    agents: Arc<dyn AgentPort>,
    rules: Arc<dyn ReplyClassifier>,
    clock: Arc<dyn ClockPort>,
}

impl StatusProposalsService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repo: Arc<dyn StatusProposalRepo>,
        apply: Arc<dyn ApplyRepo>,
        postings: Arc<dyn IngestionRepo>,
        system: Arc<dyn SystemRepo>,
        agents: Arc<dyn AgentPort>,
        rules: Arc<dyn ReplyClassifier>,
        clock: Arc<dyn ClockPort>,
    ) -> Self {
        Self {
            repo,
            apply,
            postings,
            system,
            agents,
            rules,
            clock,
        }
    }

    // ---- settings and disclosure (AC3, AC4) -------------------------------

    /// The reply-reading settings the disclosure screen shows.
    pub fn disclosure(&self) -> Result<DisclosureView, StatusProposalsError> {
        let mode = self
            .system
            .setting_get(SETTING_MODE)?
            .and_then(|value| ClassificationMode::parse(&value))
            .unwrap_or(ClassificationMode::Llm);
        let acknowledged = is_true(self.system.setting_get(SETTING_DISCLOSURE_ACK)?.as_deref());
        let auto_confirm = is_true(self.system.setting_get(SETTING_AUTO_CONFIRM)?.as_deref());
        Ok(DisclosureView {
            mode,
            acknowledged,
            auto_confirm,
            needs_disclosure: mode == ClassificationMode::Llm && !acknowledged,
        })
    }

    /// Save what the user chose in the disclosure or in Settings → Privacy
    /// (AC3). Accepting is one-way: the disclosure is shown once, and turning
    /// it off again is not a thing the user can ask for.
    pub fn save_settings(
        &self,
        mode: ClassificationMode,
        auto_confirm: bool,
        acknowledge: bool,
    ) -> Result<DisclosureView, StatusProposalsError> {
        let now = self.clock.now_rfc3339();
        self.system.setting_put(SETTING_MODE, mode.as_str(), &now)?;
        self.system.setting_put(
            SETTING_AUTO_CONFIRM,
            if auto_confirm { "true" } else { "false" },
            &now,
        )?;
        if acknowledge {
            self.system
                .setting_put(SETTING_DISCLOSURE_ACK, "true", &now)?;
        }
        self.disclosure()
    }

    // ---- classification (AC1, AC4) ----------------------------------------

    /// Read every message no classifier has read yet.
    ///
    /// `cli`/`model` are the engine to ask for the uncertain cases; `None`
    /// leaves them for manual triage. In Deterministic mode they are never
    /// used: nothing leaves the machine (AC4, NFR-3).
    pub fn classify(
        &self,
        cli: Option<&str>,
        model: Option<String>,
    ) -> Result<ClassifyOutcome, StatusProposalsError> {
        let settings = self.disclosure()?;
        if settings.mode == ClassificationMode::Llm && !settings.acknowledged {
            return Err(StatusProposalsError::DisclosureRequired);
        }

        let messages = self.repo.messages_unclassified()?;
        let now = self.clock.now_rfc3339();
        let mut outcome = ClassifyOutcome {
            messages: u32::try_from(messages.len()).unwrap_or(u32::MAX),
            ..ClassifyOutcome::default()
        };

        let mut uncertain: Vec<StoredMessage> = Vec::new();
        for message in &messages {
            match self.rules.classify(&message.subject, &message.body_text) {
                Some(matched) => self.record(
                    message,
                    matched.class,
                    matched.confidence,
                    &matched.quoted_line,
                    ClassificationMethod::Deterministic,
                    settings.auto_confirm,
                    &now,
                    &mut outcome,
                )?,
                None => uncertain.push(message.clone()),
            }
        }

        let deterministic = outcome.classified;
        let ask_llm =
            settings.mode == ClassificationMode::Llm && !uncertain.is_empty() && cli.is_some();
        if ask_llm {
            let cli = cli.expect("checked by ask_llm");
            outcome.llm_asked = true;
            let answers = self.run_llm(cli, model, &uncertain)?;
            for answer in answers {
                let Some(message) = uncertain.iter().find(|row| row.id == answer.email_id) else {
                    // A message that was not in this batch: the run's output is
                    // a proposal, never an instruction.
                    outcome.dropped += 1;
                    continue;
                };
                let Some(class) = class_for_status(&answer.status) else {
                    outcome.dropped += 1;
                    continue;
                };
                let Some(quote) = verified_quote(message, &answer.quote) else {
                    outcome.dropped += 1;
                    continue;
                };
                // The application must be one this message could plausibly
                // name: the core disposes, the run only proposes.
                let candidates = self.candidates(message)?;
                if !candidates.iter().any(|c| c.application_id == answer.app_id) {
                    outcome.dropped += 1;
                    continue;
                }
                self.record(
                    message,
                    class,
                    LLM_CONFIDENCE.min(LLM_CONFIDENCE_CEILING),
                    &quote,
                    ClassificationMethod::Llm,
                    settings.auto_confirm,
                    &now,
                    &mut outcome,
                )?;
            }
        }

        // What the LLM did not place either waits for manual triage (AC4).
        let placed = (outcome.classified - deterministic) as usize;
        outcome.uncertain =
            u32::try_from(uncertain.len().saturating_sub(placed)).unwrap_or(u32::MAX);
        Ok(outcome)
    }

    /// Store the verdict on one message and, when the class implies a Stage
    /// change, the proposal that offers to make it (AC1, AC2, AC5).
    #[allow(clippy::too_many_arguments)]
    fn record(
        &self,
        message: &StoredMessage,
        class: ReplyClass,
        confidence: f64,
        quoted_line: &str,
        method: ClassificationMethod,
        auto_confirm: bool,
        now: &str,
        outcome: &mut ClassifyOutcome,
    ) -> Result<(), StatusProposalsError> {
        let untrusted_origin = method.produces_untrusted_text();
        self.repo.classification_put(&Classification {
            message_id: message.id.clone(),
            class,
            confidence,
            quoted_line: quoted_line.to_string(),
            method,
            untrusted_origin,
            classified_at: now.to_string(),
        })?;
        outcome.classified += 1;

        let Some(to) = class.proposed_status() else {
            return Ok(());
        };
        let candidates = self.candidates(message)?;
        let application = (candidates.len() == 1).then(|| candidates[0].clone());
        let from_status = match &application {
            Some(candidate) => match AppStatus::parse(&candidate.status) {
                Some(status) => {
                    // A move the core would refuse is not proposed at all.
                    if forward_move(class, status).is_none() {
                        return Ok(());
                    }
                    Some(status)
                }
                None => return Ok(()),
            },
            None => None,
        };
        // An ambiguous proposal is only worth the user's time when at least one
        // Application could actually take the move.
        if application.is_none()
            && !candidates.iter().any(|candidate| {
                AppStatus::parse(&candidate.status)
                    .is_some_and(|status| forward_move(class, status).is_some())
            })
        {
            return Ok(());
        }

        let proposal = StatusProposal {
            id: format!("sp_{}", ulid::Ulid::generate()),
            message_id: message.id.clone(),
            application_id: application.as_ref().map(|c| c.application_id.clone()),
            from_status: from_status.map(|status| status.as_str().to_string()),
            to_status: to.to_string(),
            class,
            confidence,
            quoted_line: quoted_line.to_string(),
            untrusted_origin,
            state: ProposalState::Pending,
            created_at: now.to_string(),
            decided_at: None,
        };
        if !self.repo.proposal_insert(&proposal)? {
            return Ok(());
        }
        outcome.proposals += 1;

        // AC5: a high-confidence Acknowledgement or Rejection is applied at
        // once, when the user asked for it and the Application is unambiguous.
        let (Some(application), Some(from)) = (proposal.application_id.as_deref(), from_status)
        else {
            return Ok(());
        };
        if auto_confirm && class.auto_confirmable() && confidence >= AUTO_CONFIRM_CONFIDENCE {
            let Some(target) = AppStatus::parse(&proposal.to_status) else {
                return Ok(());
            };
            self.move_card(application, from, target, class)?;
            self.repo.proposal_decide(
                &proposal.id,
                Some(application),
                Some(from.as_str()),
                ProposalState::AutoApplied,
                now,
            )?;
            outcome.auto_applied += 1;
            outcome.auto_applied_ids.push(proposal.id.clone());
        }
        Ok(())
    }

    /// Ask the agent CLI about the uncertain messages (AC1).
    fn run_llm(
        &self,
        cli: &str,
        model: Option<String>,
        messages: &[StoredMessage],
    ) -> Result<Vec<LlmAnswer>, StatusProposalsError> {
        let pairs: Vec<(StoredMessage, Vec<CandidateView>)> = messages
            .iter()
            .map(|message| {
                self.candidates(message)
                    .map(|candidates| (message.clone(), candidates))
            })
            .collect::<Result<_, _>>()?;
        let handle = self.agents.run(AgentTask {
            kind: AgentTaskKind::ClassifyEmails,
            // Email is third-party text: the lockdown gate decides whether the
            // chosen CLI may read it (ADR-010).
            input_trust: InputTrust::Untrusted,
            cli: cli.to_string(),
            model,
            prompt: prompt(&pairs),
            // The run writes no deliverable: the label is only what the pack
            // renders, and classification is language-neutral.
            target_language: "en".to_string(),
            budget: RunBudget {
                wall_clock: Some(RUN_WALL_CLOCK),
                ..RunBudget::default()
            },
            env: Vec::new(),
        });
        let events: Vec<AgentEvent> = handle.events.iter().collect();
        if let Some(reason) = failure(&events) {
            return Err(StatusProposalsError::Run(reason));
        }
        Ok(answers(&events))
    }

    // ---- the queue (AC2) ---------------------------------------------------

    /// The proposals waiting for the user, newest first.
    pub fn proposals(&self) -> Result<Vec<StatusProposalView>, StatusProposalsError> {
        let mut views = Vec::new();
        for proposal in self.repo.proposals(ProposalState::Pending)? {
            views.push(self.view(proposal)?);
        }
        Ok(views)
    }

    /// Approve one proposal, moving its card (AC2).
    ///
    /// `application_id` is the user's pick, required when the proposal was
    /// ambiguous. The stored status is read now, so a card the user moved by
    /// hand in the meantime is never moved against their will: the move is
    /// from what is stored to what the proposal decided.
    pub fn approve(
        &self,
        id: &str,
        application_id: Option<&str>,
    ) -> Result<ApproveOutcome, StatusProposalsError> {
        let proposal = self
            .repo
            .proposal_get(id)?
            .ok_or_else(|| StatusProposalsError::UnknownProposal(id.to_string()))?;
        if proposal.state != ProposalState::Pending {
            return Err(StatusProposalsError::NotPending(id.to_string()));
        }
        let application_id = application_id
            .map(str::to_string)
            .or_else(|| proposal.application_id.clone())
            .ok_or_else(|| StatusProposalsError::NeedsApplication(id.to_string()))?;
        let application = self
            .apply
            .application_get(&application_id)?
            .ok_or_else(|| StatusProposalsError::UnknownProposal(application_id.clone()))?;
        let from = application.status;
        let to = AppStatus::parse(&proposal.to_status)
            .ok_or_else(|| StatusProposalsError::UnknownProposal(id.to_string()))?;
        let target = forward_move(proposal.class, from).filter(|target| *target == to);
        let Some(target) = target else {
            // The card already sits at or past what the reply implies.
            return Err(StatusProposalsError::AlreadyThere);
        };

        let now = self.clock.now_rfc3339();
        self.move_card(&application_id, from, target, proposal.class)?;
        self.repo.proposal_decide(
            id,
            Some(&application_id),
            Some(from.as_str()),
            ProposalState::Approved,
            &now,
        )?;
        Ok(ApproveOutcome {
            proposal_id: id.to_string(),
            application_id,
            from_status: from.as_str().to_string(),
            to_status: target.as_str().to_string(),
        })
    }

    /// Approve several proposals at once (AC2). Every item is tried: one that
    /// needs a pick, one whose card already moved, or one the store cannot read
    /// is reported in the outcome, never fatal — the items that already moved
    /// must stay visible to the user.
    pub fn approve_many(&self, ids: &[String]) -> Result<BatchOutcome, StatusProposalsError> {
        let mut outcome = BatchOutcome::default();
        for id in ids {
            match self.approve(id, None) {
                Ok(_) => outcome.approved.push(id.clone()),
                Err(StatusProposalsError::NeedsApplication(_)) => {
                    outcome.needs_pick.push(id.clone());
                }
                // A failure is this item's, whatever its kind: the items that
                // already moved stay reported (the user must see them), and the
                // next one is still tried. Aborting here would hide a card that
                // has already changed.
                Err(_) => outcome.failed.push(id.clone()),
            }
        }
        Ok(outcome)
    }

    /// Dismiss one proposal: nothing moves, and the message is not asked about
    /// again (the classification stays).
    pub fn dismiss(&self, id: &str) -> Result<(), StatusProposalsError> {
        let proposal = self
            .repo
            .proposal_get(id)?
            .ok_or_else(|| StatusProposalsError::UnknownProposal(id.to_string()))?;
        if proposal.state != ProposalState::Pending {
            return Err(StatusProposalsError::NotPending(id.to_string()));
        }
        let now = self.clock.now_rfc3339();
        self.repo
            .proposal_decide(id, None, None, ProposalState::Dismissed, &now)?;
        Ok(())
    }

    /// Undo an applied proposal (AC5), automatic or approved.
    ///
    /// Only when the card still sits where the proposal left it: once the user
    /// moved it again, going back is a guess and the Tracker is the place to
    /// fix it.
    pub fn undo(&self, id: &str) -> Result<(), StatusProposalsError> {
        let proposal = self
            .repo
            .proposal_get(id)?
            .ok_or_else(|| StatusProposalsError::UnknownProposal(id.to_string()))?;
        if !proposal.state.moved_the_card() {
            return Err(StatusProposalsError::NotUndoable(id.to_string()));
        }
        let application_id = proposal
            .application_id
            .clone()
            .ok_or_else(|| StatusProposalsError::NotUndoable(id.to_string()))?;
        let from = proposal
            .from_status
            .as_deref()
            .and_then(AppStatus::parse)
            .ok_or_else(|| StatusProposalsError::NotUndoable(id.to_string()))?;
        let to = AppStatus::parse(&proposal.to_status)
            .ok_or_else(|| StatusProposalsError::NotUndoable(id.to_string()))?;
        let application = self
            .apply
            .application_get(&application_id)?
            .ok_or_else(|| StatusProposalsError::UnknownProposal(application_id.clone()))?;
        if application.status != to {
            return Err(StatusProposalsError::StaleCard);
        }
        let now = self.clock.now_rfc3339();
        self.move_card(&application_id, to, from, proposal.class)?;
        self.repo
            .proposal_decide(id, None, None, ProposalState::Undone, &now)?;
        Ok(())
    }

    // ---- internals ---------------------------------------------------------

    /// The Tracker transition every approval goes through (AD-16): this use
    /// case never writes `application.status` itself.
    fn move_card(
        &self,
        application_id: &str,
        from: AppStatus,
        to: AppStatus,
        class: ReplyClass,
    ) -> Result<(), StatusProposalsError> {
        let tracker = TrackerService::new(Arc::clone(&self.apply), Arc::clone(&self.clock));
        tracker.transition(TransitionInput {
            application_id: application_id.to_string(),
            from_status: from.as_str().to_string(),
            to_status: to.as_str().to_string(),
            source: "status_proposal".to_string(),
            // The history records what the core decided, not the message's
            // words: an untrusted quote never becomes a timeline line (AD-7).
            reason: Some(format!("reply:{}", class.as_str())),
        })?;
        Ok(())
    }

    /// The Applications this message could be about (AC2).
    ///
    /// A match is either the sender's domain (the address Boba mailed or the
    /// Offer's host) or the company's name inside the sender's domain. When
    /// nothing matches, every open Application is offered so the user can pick.
    fn candidates(
        &self,
        message: &StoredMessage,
    ) -> Result<Vec<CandidateView>, StatusProposalsError> {
        candidates_for(message, self.apply.as_ref(), self.postings.as_ref())
    }

    /// One proposal as the queue shows it.
    fn view(&self, proposal: StatusProposal) -> Result<StatusProposalView, StatusProposalsError> {
        let message = self.repo.message_get(&proposal.message_id)?;
        let (subject, from, received_at) = match &message {
            Some(message) => (
                message.subject.clone(),
                message.from_address.clone(),
                message.received_at.clone(),
            ),
            None => (String::new(), String::new(), proposal.created_at.clone()),
        };
        let mut candidates = Vec::new();
        if proposal.application_id.is_none()
            && let Some(message) = &message
        {
            candidates = self.candidates(message)?;
        }
        let application = proposal
            .application_id
            .as_deref()
            .and_then(|id| self.apply.application_get(id).ok().flatten())
            .map(|application| {
                let snapshot = snapshot(&application);
                CandidateView {
                    application_id: application.id.clone(),
                    company: snapshot.company,
                    title: snapshot.title,
                    status: application.status.as_str().to_string(),
                }
            });
        Ok(StatusProposalView {
            id: proposal.id,
            class: proposal.class.as_str().to_string(),
            confidence: proposal.confidence,
            quoted_line: proposal.quoted_line,
            untrusted_origin: proposal.untrusted_origin,
            message_id: proposal.message_id,
            message_subject: subject,
            message_from: from,
            received_at,
            needs_pick: proposal.application_id.is_none(),
            application,
            candidates,
            to_status: proposal.to_status,
            created_at: proposal.created_at,
        })
    }
}

/// One `propose_status_change` call the core accepted (S8.4).
///
/// The agent names the words; the core returns the message's own line as the
/// quote (ADR-010: the core disposes). Nothing is written here — the use case
/// stores the proposal off the run's event stream, and the user approves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedProposal {
    pub email_id: String,
    pub app_id: String,
    /// The message's own line, never the run's transcription of it.
    pub quote: String,
}

/// Check one `propose_status_change` call against the data the run was given
/// (S8.4, ADR-010): the message is one this pass has not read yet (that is the
/// batch the run's input carried), the status is one of the four the tool may
/// propose, the quote is a line of that message, and the application is one of
/// that message's candidates. A refusal goes back to the agent in the existing
/// tool-error shape, so a bad call costs the run a retry instead of being
/// stored or silently dropped.
///
/// The handler calls this; nothing here writes, moves a card or reads anything
/// but the batch and the Tracker.
pub fn validate_agent_proposal(
    repo: &dyn StatusProposalRepo,
    apply: &dyn ApplyRepo,
    postings: &dyn IngestionRepo,
    args: &Value,
) -> Result<AcceptedProposal, String> {
    let text = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let email_id = text("email_id")
        .ok_or_else(|| "`propose_status_change` needs the message's `email_id`".to_string())?;
    let app_id =
        text("app_id").ok_or_else(|| "`propose_status_change` needs an `app_id`".to_string())?;
    let status =
        text("status").ok_or_else(|| "`propose_status_change` needs a `status`".to_string())?;
    let quote = text("quote")
        .ok_or_else(|| "`propose_status_change` needs the `quote` that shows it".to_string())?;
    if class_for_status(&status).is_none() {
        return Err(format!(
            "`{status}` is not one of applied, interview, offer, rejected"
        ));
    }
    // The batch is exactly what `classify` asked about: the messages no
    // classifier has read. `classify` stores the verdict only after the run
    // ends, so during the run this set is the run's own.
    let message = repo
        .messages_unclassified()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|message| message.id == email_id)
        .ok_or_else(|| format!("`{email_id}` is not in this run's batch"))?;
    let quote = verified_quote(&message, &quote)
        .ok_or_else(|| format!("the quote is not a line of `{email_id}`"))?;
    let candidates =
        candidates_for(&message, apply, postings).map_err(|error| error.to_string())?;
    if !candidates
        .iter()
        .any(|candidate| candidate.application_id == app_id)
    {
        return Err(format!(
            "`{app_id}` is not one of `{email_id}`'s applications"
        ));
    }
    Ok(AcceptedProposal {
        email_id,
        app_id,
        quote,
    })
}

/// The Applications one message could be about (AC2).
///
/// A match is either the sender's domain (the address Boba mailed or the
/// Offer's host) or the company's name inside the sender's domain. When
/// nothing matches, every open Application is offered so the user can pick.
fn candidates_for(
    message: &StoredMessage,
    apply: &dyn ApplyRepo,
    postings: &dyn IngestionRepo,
) -> Result<Vec<CandidateView>, StatusProposalsError> {
    let domain = sender_domain(&message.from_address);
    let mut matched = Vec::new();
    let mut open = Vec::new();
    for application in apply.applications()? {
        let snapshot = snapshot(&application);
        let view = CandidateView {
            application_id: application.id.clone(),
            company: snapshot.company.clone(),
            title: snapshot.title.clone(),
            status: application.status.as_str().to_string(),
        };
        let offer = postings.get_offer(&application.posting_id).ok().flatten();
        let matches = domain.as_deref().is_some_and(|domain| {
            offer.as_ref().is_some_and(|offer| {
                offer_matches(offer, domain) || company_matches(&snapshot.company, domain)
            })
        });
        if matches {
            matched.push(view.clone());
        }
        if !application.status.is_terminal() {
            open.push(view);
        }
    }
    if matched.is_empty() {
        return Ok(open);
    }
    Ok(matched)
}

/// The task input the classification run reads (AD-7: plain text, bounded).
fn prompt(messages: &[(StoredMessage, Vec<CandidateView>)]) -> String {
    let mut prompt = String::from(
        "Classify recruiter replies. For each message, decide the application status it \
         implies and call `propose_status_change` with: the message's `email_id`; an \
         `app_id` from that message's listed applications; a `status` of `applied`, \
         `interview`, `offer` or `rejected`; and as `quote` the exact line of the message \
         that shows it, copied verbatim. Propose nothing when the message does not clearly \
         imply one of those statuses.\n",
    );
    for (message, candidates) in messages {
        let ids: Vec<String> = candidates
            .iter()
            .map(|candidate| {
                format!(
                    "{} ({} — {})",
                    candidate.application_id, candidate.company, candidate.title
                )
            })
            .collect();
        prompt.push_str(&format!(
            "\n---\nemail_id: {}\napplications: {}\nsubject: {}\nbody:\n{}\n",
            message.id,
            ids.join(", "),
            message.subject,
            bound(&message.body_text, BODY_PROMPT_CHARS),
        ));
    }
    prompt
}

/// One `propose_status_change` call, as the run made it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LlmAnswer {
    email_id: String,
    app_id: String,
    status: String,
    quote: String,
}

/// The proposals in a run's event stream.
fn answers(events: &[AgentEvent]) -> Vec<LlmAnswer> {
    events
        .iter()
        .filter_map(|event| match event {
            // The CLI's stream names an MCP tool `mcp__boba__<tool>`; the bare
            // name is what the catalog and the tests use (FX16).
            AgentEvent::ToolCall { name, input } if is_proposal(name) => {
                let text = |key: &str| {
                    input
                        .get(key)
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_string)
                };
                Some(LlmAnswer {
                    email_id: text("email_id")?,
                    app_id: text("app_id")?,
                    status: text("status")?,
                    quote: text("quote")?,
                })
            }
            _ => None,
        })
        .collect()
}

/// Whether a tool call in the run's stream is the proposal tool. The stream
/// carries the CLI's name for it (`mcp__boba__propose_status_change`); the
/// catalog's bare name is the last segment.
fn is_proposal(name: &str) -> bool {
    name.rsplit("__").next() == Some(PROPOSE_TOOL)
}

/// The failure a run reported, if it reported one (FR-21).
fn failure(events: &[AgentEvent]) -> Option<FailureReason> {
    events.iter().find_map(|event| match event {
        AgentEvent::Failed { reason, .. } => Some(*reason),
        _ => None,
    })
}

/// The class a proposed status stands for.
fn class_for_status(status: &str) -> Option<ReplyClass> {
    match status.trim().to_ascii_lowercase().as_str() {
        "applied" => Some(ReplyClass::Acknowledgement),
        "rejected" => Some(ReplyClass::Rejection),
        "interview" => Some(ReplyClass::InterviewInvite),
        "offer" => Some(ReplyClass::Offer),
        _ => None,
    }
}

/// The move a class implies from the stored status, or `None` when there is
/// nothing to propose (AC2): Boba never moves a card backwards or past where it
/// already is.
fn forward_move(class: ReplyClass, from: AppStatus) -> Option<AppStatus> {
    use AppStatus::{Applied, Drafted, Interview, Offer, Rejected, Saved};
    let terminal = matches!(
        from,
        AppStatus::Hired | AppStatus::Rejected | AppStatus::Withdrawn | AppStatus::OfferDeclined
    );
    match class {
        ReplyClass::Acknowledgement => matches!(from, Saved | Drafted).then_some(Applied),
        ReplyClass::Rejection => (!terminal).then_some(Rejected),
        ReplyClass::InterviewInvite | ReplyClass::Assessment => {
            matches!(from, Saved | Drafted | Applied | AppStatus::NoResponse).then_some(Interview)
        }
        ReplyClass::Offer => matches!(
            from,
            Saved | Drafted | Applied | Interview | AppStatus::NoResponse
        )
        .then_some(Offer),
        ReplyClass::RequestForInfo | ReplyClass::Other => None,
    }
}

/// The run's quote, taken from the message itself: the run names the words,
/// the core returns the message's own line. A quote that is not in the message
/// drops the proposal (ADR-010: the core disposes).
fn verified_quote(message: &StoredMessage, quote: &str) -> Option<String> {
    let needle = quote.trim().to_lowercase();
    if needle.is_empty() {
        return None;
    }
    let line = message
        .body_text
        .lines()
        .chain(std::iter::once(message.subject.as_str()))
        .map(str::trim)
        .find(|line| !line.is_empty() && line.to_lowercase().contains(&needle))?;
    Some(bound(line, 300))
}

/// The lowercased domain of an address.
fn sender_domain(address: &str) -> Option<String> {
    let (_, domain) = address.trim().rsplit_once('@')?;
    let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    (!domain.is_empty()).then_some(domain)
}

/// Whether the Offer was applied to at that domain: its apply email or the
/// host of its apply URL, subdomains included.
fn offer_matches(offer: &boba_domain::posting::Offer, domain: &str) -> bool {
    let email_domain = offer.apply.email.as_deref().and_then(sender_domain);
    let host = url_host(&offer.apply.url);
    email_domain
        .as_deref()
        .is_some_and(|candidate| related(candidate, domain))
        || host
            .as_deref()
            .is_some_and(|candidate| related(candidate, domain))
}

/// Whether the company's name appears in the sender's domain (`acme` in
/// `acme.com`). Tokens shorter than four characters are skipped: they match
/// everything.
fn company_matches(company: &str, domain: &str) -> bool {
    let host = domain.split('.').next().unwrap_or(domain);
    company
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|token| token.chars().count() >= 4)
        .any(|token| host.contains(&token))
}

/// Two domains are related when one is the other or a subdomain of it.
fn related(left: &str, right: &str) -> bool {
    left == right || left.ends_with(&format!(".{right}")) || right.ends_with(&format!(".{left}"))
}

/// The host of an URL, without scheme, userinfo or port.
fn url_host(url: &str) -> Option<String> {
    let rest = url.trim();
    let rest = rest.split_once("://").map_or(rest, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = authority.split(':').next()?.trim().trim_end_matches('.');
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// The task-input bound (AD-7: never the whole thread).
fn bound(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let clipped: String = text.chars().take(max_chars).collect();
    format!("{clipped}…")
}

/// The Offer snapshot frozen on an Application, or an empty card when the row
/// is one this build cannot read (the same tolerance the Tracker keeps).
fn snapshot(application: &Application) -> OfferSnapshot {
    serde_json::from_str(&application.offer_snapshot_json).unwrap_or(OfferSnapshot {
        title: String::new(),
        company: String::new(),
        apply_url: None,
        lang: None,
        deadline: None,
    })
}

/// A stored `true`.
fn is_true(value: Option<&str>) -> bool {
    value == Some("true")
}

#[cfg(test)]
pub mod testing;
#[cfg(test)]
mod tests;
