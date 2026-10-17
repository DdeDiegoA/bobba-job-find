//! Capability gating (S3.3): what works without an Agent CLI (FR-4,
//! EXPERIENCE §9.14) and which untrusted-content tasks the chosen CLI cannot
//! run (ADR-010, §9.15).
//!
//! The model is pure: it reads the engine the user chose and the lockdown
//! verdict the probe produced, never the store or the UI. Deterministic work
//! (Sources, dedup, scoring, the Tracker, manual Applications and a manual
//! Profile) is never gated; AI-only work is gated on a working CLI and, for
//! tasks that read third-party content, on verified `tool_lockdown`.
//!
//! One snapshot, computed once per UI session, is what the shared
//! `useCapability` hook and `Gated` component render (AC4); every feature
//! slice reads it instead of re-implementing disabled states.

use serde::{Deserialize, Serialize};

use crate::ports::agent::AgentTaskKind;

/// Every capability the UI gates actions on (AC4).
///
/// Deterministic ids name work that runs without an LLM; AI ids name one
/// [`AgentTaskKind`] whose availability decides them. `profile_edit` is manual
/// Profile entry: it stays available in Limited mode, which is what makes the
/// Profile manual when extraction is off (AC2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    // Deterministic: no agent needed.
    Sources,
    Dedup,
    Scoring,
    Tracker,
    ApplicationsManual,
    ProfileEdit,
    // AI-only: the task each one needs.
    ProfileExtraction,
    ProfileGaps,
    ProfileTranslation,
    PostingExtraction,
    Tailoring,
    ApplicationReview,
    ScoreNarration,
    AnswerQuestions,
    EmailClassification,
}

impl Capability {
    /// Every capability, in the order the snapshot lists them.
    pub const ALL: [Capability; 15] = [
        Capability::Sources,
        Capability::Dedup,
        Capability::Scoring,
        Capability::Tracker,
        Capability::ApplicationsManual,
        Capability::ProfileEdit,
        Capability::ProfileExtraction,
        Capability::ProfileGaps,
        Capability::ProfileTranslation,
        Capability::PostingExtraction,
        Capability::Tailoring,
        Capability::ApplicationReview,
        Capability::ScoreNarration,
        Capability::AnswerQuestions,
        Capability::EmailClassification,
    ];

    /// Stable id the UI and the hook use (`useCapability("tailoring")`).
    pub const fn id(self) -> &'static str {
        match self {
            Capability::Sources => "sources",
            Capability::Dedup => "dedup",
            Capability::Scoring => "scoring",
            Capability::Tracker => "tracker",
            Capability::ApplicationsManual => "applications_manual",
            Capability::ProfileEdit => "profile_edit",
            Capability::ProfileExtraction => "profile_extraction",
            Capability::ProfileGaps => "profile_gaps",
            Capability::ProfileTranslation => "profile_translation",
            Capability::PostingExtraction => "posting_extraction",
            Capability::Tailoring => "tailoring",
            Capability::ApplicationReview => "application_review",
            Capability::ScoreNarration => "score_narration",
            Capability::AnswerQuestions => "answer_questions",
            Capability::EmailClassification => "email_classification",
        }
    }

    /// The agent task this capability needs, or `None` when it is deterministic.
    ///
    /// Profile entry has two capabilities on the same task on purpose: the gap
    /// interview and the CV extraction are both `profile_import`, and gating
    /// them separately keeps the UI free to name its own actions.
    pub const fn task(self) -> Option<AgentTaskKind> {
        match self {
            Capability::Sources
            | Capability::Dedup
            | Capability::Scoring
            | Capability::Tracker
            | Capability::ApplicationsManual
            | Capability::ProfileEdit => None,
            Capability::ProfileExtraction | Capability::ProfileGaps => {
                Some(AgentTaskKind::ProfileImport)
            }
            Capability::ProfileTranslation => Some(AgentTaskKind::TranslateProfile),
            Capability::PostingExtraction => Some(AgentTaskKind::ExtractPostings),
            Capability::Tailoring => Some(AgentTaskKind::TailorApplication),
            Capability::ApplicationReview => Some(AgentTaskKind::ReviewApplication),
            Capability::ScoreNarration => Some(AgentTaskKind::NarrateScores),
            Capability::AnswerQuestions => Some(AgentTaskKind::AnswerQuestions),
            Capability::EmailClassification => Some(AgentTaskKind::ClassifyEmails),
        }
    }
}

/// Why an AI-only capability is unavailable: the reason the UI shows next to
/// the disabled action (AC2, AC3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum UnavailableReason {
    /// No working Agent CLI: Limited mode, "Connect an agent" (FR-4).
    NoAgent,
    /// The chosen CLI has no verified `tool_lockdown` for this task (ADR-010).
    CliUnsupported,
}

/// One capability as the UI sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CapabilityState {
    /// [`Capability::id`].
    pub id: String,
    pub available: bool,
    /// Why it is off; never set when it is on.
    pub reason: Option<UnavailableReason>,
}

/// What the UI gates on for this session (AC4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CapabilitySnapshot {
    /// No agent can run: every AI-only capability is off (FR-4 Limited mode).
    pub limited: bool,
    /// Every capability, in [`Capability::ALL`] order.
    pub capabilities: Vec<CapabilityState>,
}

/// The chosen engine as the gate sees it (ADR-010, FR-4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChosenAgent {
    /// The probe found the chosen CLI on this machine.
    pub installed: bool,
    /// The CLI reported healthy; an auth-required or broken CLI is no engine.
    pub healthy: bool,
    /// Tasks the lockdown gate refuses on this install, from the probe.
    pub unsupported_tasks: Vec<AgentTaskKind>,
}

impl ChosenAgent {
    /// Whether the chosen engine can run any task at all.
    fn works(&self) -> bool {
        self.installed && self.healthy
    }
}

/// The gating snapshot for one chosen engine (or none at all).
///
/// `None` — no choice stored, or a choice this build cannot read — is Limited
/// mode: AI actions are off with [`UnavailableReason::NoAgent`], never left
/// clickable to fail later (EXPERIENCE §9.15).
pub fn snapshot(chosen: Option<ChosenAgent>) -> CapabilitySnapshot {
    let working = chosen.as_ref().filter(|agent| agent.works());
    let limited = working.is_none();
    let capabilities = Capability::ALL
        .into_iter()
        .map(|capability| state(capability, working))
        .collect();
    CapabilitySnapshot {
        limited,
        capabilities,
    }
}

/// One capability's state for a chosen engine: deterministic work always runs;
/// AI work needs a working CLI, and untrusted tasks need its lockdown too.
fn state(capability: Capability, working: Option<&ChosenAgent>) -> CapabilityState {
    let (available, reason) = match capability.task() {
        None => (true, None),
        Some(task) => match working {
            None => (false, Some(UnavailableReason::NoAgent)),
            Some(agent) if agent.unsupported_tasks.contains(&task) => {
                (false, Some(UnavailableReason::CliUnsupported))
            }
            Some(_) => (true, None),
        },
    };
    CapabilityState {
        id: capability.id().to_string(),
        available,
        reason,
    }
}

#[cfg(test)]
mod tests;
