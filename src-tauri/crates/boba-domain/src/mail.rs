//! Mail types that cross the sync and classification ports (S8.3, S8.4,
//! FR-38/104, AD-14).
//!
//! `boba-mail` normalises a fetched message (Graph, IMAP or the `.eml`
//! importer) into [`RawMessage`]; the scoped pipeline decides whether it relates
//! to an Application and hands only those to the `MessageSink` port. These types
//! live here because they cross that port, and `boba-store` implements the port
//! without depending on the adapter (AD-1, AD-14).
//!
//! S8.4 adds the classification vocabulary ([`ReplyClass`], [`RuleMatch`],
//! [`ClassificationMethod`]) and the Status Proposal row ([`StatusProposal`]):
//! the rules live in `boba-mail`, the use case in `boba-app` and the storage in
//! `boba-store`, so the shape they share is here.

/// The lowercased domain of an address, without a trailing root dot.
fn normalize_domain(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('@')
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

/// Which part of a message made it through the scope filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchReason {
    /// The sender is on a domain of a company the user applied to.
    CompanyDomain,
    /// The sender is a known ATS.
    AtsSender,
    /// The message is a reply to a thread the app started.
    SentThread,
}

impl MatchReason {
    /// The stored text form (mirrors `email_message.matched_by`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CompanyDomain => "company_domain",
            Self::AtsSender => "ats_sender",
            Self::SentThread => "sent_thread",
        }
    }
}

/// Where a stored message entered the pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImportOrigin {
    /// Fetched from a connected mailbox during a sync pass.
    #[default]
    Sync,
    /// Handed in by the user as an `.eml` file or pasted text (FR-104).
    Eml,
}

impl ImportOrigin {
    /// The stored text form (mirrors `email_message.origin`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sync => "sync",
            Self::Eml => "eml",
        }
    }
}

/// One message as an adapter (Graph, IMAP, Gmail or the `.eml` importer)
/// normalised it. Bodies are plain text only; HTML never travels further
/// (AD-7).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawMessage {
    /// The id the source assigned, stable across a re-delivery. For an `.eml`
    /// file this is its Message-ID or a hash of the bytes.
    pub provider_id: String,
    /// The `Message-ID` header, normalised (no angle brackets, lowercase).
    pub message_id: Option<String>,
    /// `In-Reply-To` ids, normalised like `message_id`.
    pub in_reply_to: Vec<String>,
    /// `References` ids, normalised like `message_id`.
    pub references: Vec<String>,
    /// The `From` address, as the header carried it.
    pub from_address: String,
    /// The display name in `From`, when the header had one.
    pub from_name: Option<String>,
    /// The `To` addresses.
    pub to_addresses: Vec<String>,
    /// The `Subject` line.
    pub subject: String,
    /// The `Date` header, in Unix seconds (UTC).
    pub received_at: i64,
    /// The plain-text body, already stripped of markup.
    pub body_text: String,
    /// Whether the message carries attachments (metadata only: attachments are
    /// never stored).
    pub has_attachments: bool,
}

impl RawMessage {
    /// The lowercased domain of [`RawMessage::from_address`], when it has one.
    pub fn sender_domain(&self) -> Option<String> {
        let (_, domain) = self.from_address.trim().rsplit_once('@')?;
        let domain = normalize_domain(domain);
        (!domain.is_empty()).then_some(domain)
    }
}

/// A message that passed the scope filter, handed to the storage adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredMessage<'a> {
    /// The normalised message.
    pub message: &'a RawMessage,
    /// Why it was kept.
    pub matched_by: MatchReason,
    /// Where it came from.
    pub origin: ImportOrigin,
    /// The connection the message was fetched from; `None` for an import.
    pub account_id: Option<&'a str>,
}

/// Failures of one pass.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SyncError {
    /// The storage adapter could not write a matched message.
    #[error("the email store failed: {0}")]
    Store(String),
}

// ---- Reply classification (S8.4, FR-100) ----------------------------------

/// What a recruiter reply says (FR-100, AC1). The stored form is the text
/// below; a class this build does not know is never stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReplyClass {
    /// The application arrived: "we received your application".
    Acknowledgement,
    /// The application will not proceed.
    Rejection,
    /// The company wants to talk to the user.
    InterviewInvite,
    /// A test, challenge or take-home comes next.
    Assessment,
    /// A job offer.
    Offer,
    /// The company needs something from the user before continuing.
    RequestForInfo,
    /// The rule engine could not tell (only an LLM may propose this class).
    Other,
}

impl ReplyClass {
    /// Every class, in the order the UI lists them.
    pub const ALL: [ReplyClass; 7] = [
        ReplyClass::Acknowledgement,
        ReplyClass::Rejection,
        ReplyClass::InterviewInvite,
        ReplyClass::Assessment,
        ReplyClass::Offer,
        ReplyClass::RequestForInfo,
        ReplyClass::Other,
    ];

    /// The stored form (mirrors `email_classification.class`).
    pub fn as_str(self) -> &'static str {
        match self {
            ReplyClass::Acknowledgement => "acknowledgement",
            ReplyClass::Rejection => "rejection",
            ReplyClass::InterviewInvite => "interview_invite",
            ReplyClass::Assessment => "assessment",
            ReplyClass::Offer => "offer",
            ReplyClass::RequestForInfo => "request_for_info",
            ReplyClass::Other => "other",
        }
    }

    /// Parse the stored form, `None` for anything else.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|class| class.as_str() == value)
    }

    /// The `AppStatus` this class moves an Application to, when it implies one
    /// (AC2). `None` for a class the core never turns into a Status Proposal.
    pub fn proposed_status(self) -> Option<&'static str> {
        match self {
            ReplyClass::Acknowledgement => Some("applied"),
            ReplyClass::Rejection => Some("rejected"),
            ReplyClass::InterviewInvite | ReplyClass::Assessment => Some("interview"),
            ReplyClass::Offer => Some("offer"),
            ReplyClass::RequestForInfo | ReplyClass::Other => None,
        }
    }

    /// Whether this class is one auto-confirm may apply (AC5: Acknowledgement
    /// and Rejection only).
    pub const fn auto_confirmable(self) -> bool {
        matches!(self, ReplyClass::Acknowledgement | ReplyClass::Rejection)
    }
}

/// One match from the deterministic EN/ES rules (FR-100, AC1).
#[derive(Debug, Clone, PartialEq)]
pub struct RuleMatch {
    /// The class the matched phrase points at.
    pub class: ReplyClass,
    /// How sure the rule is, in `0.0..=1.0`. A value below
    /// [`UNCERTAIN_CONFIDENCE`] is an uncertain case: only the LLM may be asked.
    pub confidence: f64,
    /// The line of the message the phrase was read from, verbatim.
    pub quoted_line: String,
}

/// The confidence under which a deterministic match is "uncertain" (AC1): the
/// rules still run, but the case is offered to the LLM when one is allowed.
pub const UNCERTAIN_CONFIDENCE: f64 = 0.75;

/// The confidence auto-confirm needs (AC5). It is above what an LLM proposal
/// may carry ([`LLM_CONFIDENCE_CEILING`]), so an automatic status change is
/// always backed by a deterministic rule the core can re-check.
pub const AUTO_CONFIRM_CONFIDENCE: f64 = 0.95;

/// The most confidence an LLM proposal may carry. The core caps what it reads
/// from a run: a claimed `1.0` is not evidence, and the cap keeps an untrusted
/// sender's words from triggering auto-confirm.
pub const LLM_CONFIDENCE_CEILING: f64 = 0.9;

/// Who classified a message (mirrors `email_classification.method`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassificationMethod {
    /// The EN/ES rules in `boba-mail`, offline.
    Deterministic,
    /// The user's agent CLI read the message text.
    Llm,
}

impl ClassificationMethod {
    /// The stored form.
    pub const fn as_str(self) -> &'static str {
        match self {
            ClassificationMethod::Deterministic => "deterministic",
            ClassificationMethod::Llm => "llm",
        }
    }

    /// Parse the stored form.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "deterministic" => Some(Self::Deterministic),
            "llm" => Some(Self::Llm),
            _ => None,
        }
    }

    /// Whether text this method produced is untrusted (ADR-010, AC6): LLM text
    /// always is, a deterministic rule's quote is the message's own words and
    /// carries the message's own taint.
    pub const fn produces_untrusted_text(self) -> bool {
        matches!(self, ClassificationMethod::Llm)
    }
}

/// A Status Proposal row (FR-102, AC2). `application_id` is `None` when the
/// message did not name one Application unambiguously: the user picks (AC2).
#[derive(Debug, Clone, PartialEq)]
pub struct StatusProposal {
    /// Proposal id (`sp_…`).
    pub id: String,
    /// The stored message it was read from.
    pub message_id: String,
    /// The Application it proposes to move, or `None` when ambiguous.
    pub application_id: Option<String>,
    /// The status the Application had when the proposal was made.
    pub from_status: Option<String>,
    /// The status the proposal moves it to.
    pub to_status: String,
    /// The class that implied the move.
    pub class: ReplyClass,
    /// Confidence of the class, capped by [`LLM_CONFIDENCE_CEILING`] for LLM.
    pub confidence: f64,
    /// The message line the class was read from, verbatim.
    pub quoted_line: String,
    /// ADR-010: `true` when the class or the quote came from the LLM. A quote
    /// is always rendered as plain text.
    pub untrusted_origin: bool,
    /// Where the proposal stands.
    pub state: ProposalState,
    /// When it was made (UTC RFC 3339, AD-17).
    pub created_at: String,
    /// When it was approved, dismissed or undone.
    pub decided_at: Option<String>,
}

/// Where a Status Proposal stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProposalState {
    /// Waiting in Today's needs-you queue.
    Pending,
    /// The user approved it; the Application moved.
    Approved,
    /// The user dismissed it; nothing moved.
    Dismissed,
    /// Auto-confirm applied it (AC5). It can still be undone.
    AutoApplied,
    /// An applied proposal (automatic or not) was undone.
    Undone,
}

impl ProposalState {
    /// Every state.
    pub const ALL: [ProposalState; 5] = [
        ProposalState::Pending,
        ProposalState::Approved,
        ProposalState::Dismissed,
        ProposalState::AutoApplied,
        ProposalState::Undone,
    ];

    /// The stored form (mirrors `status_proposal.state`).
    pub fn as_str(self) -> &'static str {
        match self {
            ProposalState::Pending => "pending",
            ProposalState::Approved => "approved",
            ProposalState::Dismissed => "dismissed",
            ProposalState::AutoApplied => "auto_applied",
            ProposalState::Undone => "undone",
        }
    }

    /// Parse the stored form.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.as_str() == value)
    }

    /// Whether the Application actually moved for this proposal, so only Undo
    /// (not approval) still applies to it.
    pub const fn moved_the_card(self) -> bool {
        matches!(self, ProposalState::Approved | ProposalState::AutoApplied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_reply_class_round_trips_and_parses_nothing_else() {
        for class in ReplyClass::ALL {
            assert_eq!(ReplyClass::parse(class.as_str()), Some(class));
        }
        assert_eq!(
            ReplyClass::parse("Rejection"),
            None,
            "the form is lower case"
        );
        assert_eq!(ReplyClass::parse("spam"), None);
    }

    #[test]
    fn only_stage_changing_classes_name_a_status() {
        assert_eq!(
            ReplyClass::Acknowledgement.proposed_status(),
            Some("applied")
        );
        assert_eq!(ReplyClass::Rejection.proposed_status(), Some("rejected"));
        assert_eq!(
            ReplyClass::InterviewInvite.proposed_status(),
            Some("interview")
        );
        assert_eq!(ReplyClass::Assessment.proposed_status(), Some("interview"));
        assert_eq!(ReplyClass::Offer.proposed_status(), Some("offer"));
        assert_eq!(ReplyClass::RequestForInfo.proposed_status(), None);
        assert_eq!(ReplyClass::Other.proposed_status(), None);
        assert!(ReplyClass::Acknowledgement.auto_confirmable());
        assert!(ReplyClass::Rejection.auto_confirmable());
        assert!(!ReplyClass::InterviewInvite.auto_confirmable());
    }

    #[test]
    fn every_proposal_state_round_trips() {
        for state in ProposalState::ALL {
            assert_eq!(ProposalState::parse(state.as_str()), Some(state));
        }
        assert_eq!(ProposalState::parse("applied"), None);
        assert!(ProposalState::Approved.moved_the_card());
        assert!(ProposalState::AutoApplied.moved_the_card());
        assert!(!ProposalState::Pending.moved_the_card());
        assert!(!ProposalState::Undone.moved_the_card());
    }

    #[test]
    fn an_llm_proposal_can_never_reach_the_auto_confirm_threshold() {
        // AC5 reads "high-confidence": the core decides what that means, and an
        // LLM's own number must never clear it (AC6, ADR-010).
        const { assert!(AUTO_CONFIRM_CONFIDENCE > LLM_CONFIDENCE_CEILING) };
        const { assert!(UNCERTAIN_CONFIDENCE < AUTO_CONFIRM_CONFIDENCE) };
        assert!(ClassificationMethod::Llm.produces_untrusted_text());
        assert!(!ClassificationMethod::Deterministic.produces_untrusted_text());
        assert_eq!(
            ClassificationMethod::parse(ClassificationMethod::Llm.as_str()),
            Some(ClassificationMethod::Llm)
        );
    }
}
