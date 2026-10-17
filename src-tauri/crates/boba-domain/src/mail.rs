//! Mail types that cross the sync port (S8.3, FR-38/104, AD-14).
//!
//! `boba-mail` normalises a fetched message (Graph, IMAP or the `.eml`
//! importer) into [`RawMessage`]; the scoped pipeline decides whether it relates
//! to an Application and hands only those to the `MessageSink` port. These types
//! live here because they cross that port, and `boba-store` implements the port
//! without depending on the adapter (AD-1, AD-14).

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
