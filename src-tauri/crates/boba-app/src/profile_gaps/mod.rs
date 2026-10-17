//! Profile gap interview and bilingual drafting (S3.6, FR-6, FR-12).

pub mod gaps;
pub mod service;

pub use service::{
    ASK_TOOL, ContractError, InterviewOutcome, InterviewRequest, InterviewService, QuestionView,
    TRANSLATE_TOOL, TranslateOutcome, TranslateRequest, validate_questions, validate_translations,
};

/// A profile-gap failure. Each variant carries the i18n key the UI shows
/// (AD-3, CH7: `errors.profile_gaps.*`).
#[derive(Debug, Clone, thiserror::Error)]
pub enum ProfileGapsError {
    /// The Profile or the setting row could not be read or written.
    #[error("the profile could not be read or saved")]
    Store,
    /// The answer does not fit its question (wrong shape, blank, off-scale).
    #[error("that answer does not fit the question")]
    BadAnswer,
    /// The run proposed something outside the tool contract (AD-5).
    #[error("the agent sent questions Boba could not read")]
    Contract,
    /// The problem is the profile rule itself (no variant, blank variant).
    #[error(transparent)]
    Profile(#[from] crate::profile::ProfileError),
    /// The run ended without answering (FR-21): auth, quota, timeout, crash.
    #[error("the interview run failed: {0:?}")]
    Run(crate::ports::agent::FailureReason),
    /// The proposal was already confirmed or deleted (another window).
    #[error("unknown proposal: {0}")]
    UnknownProposal(String),
    /// The question is not one the Profile is missing (a stale step, a newer
    /// build's key).
    #[error("unknown question: {0}")]
    UnknownGap(String),
    /// The fact a translation was asked for does not exist.
    #[error("unknown fact item: {0}")]
    UnknownItem(String),
}

impl ProfileGapsError {
    /// The i18n key the UI shows.
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::Store => "errors.profile_gaps.store",
            Self::BadAnswer => "errors.profile_gaps.bad_answer",
            Self::Contract => "errors.profile_gaps.contract",
            Self::Profile(error) => error.message_key(),
            Self::Run(_) => "errors.profile_gaps.run",
            Self::UnknownProposal(_) => "errors.profile_gaps.unknown_proposal",
            Self::UnknownGap(_) => "errors.profile_gaps.unknown_gap",
            Self::UnknownItem(_) => "errors.profile_gaps.unknown_item",
        }
    }
}
