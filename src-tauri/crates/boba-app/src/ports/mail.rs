//! `MailPort` — send email through a connected mailbox (Graph / Gmail
//! `gmail.send` / IMAP-SMTP) (AD-13, architecture §11). Adapter: `boba-mail`.
//!
//! Sending is an outbound action, so the core drives it: only the apply engine
//! (through the Outbox) calls this port, never an agent (AD-11). The message
//! body and its attachments are the payload; the tokens stay in the keychain and
//! are resolved by the adapter.

use std::future::Future;
use std::pin::Pin;

/// A boxed future so the trait stays object-safe (`dyn MailPort`) while its
/// methods are async; Rust 2024 async-fn-in-trait is not dyn-safe on its own.
pub type MailFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// One file attached to an outgoing message (the tailored CV or letter).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    /// File name as the recipient sees it.
    pub filename: String,
    /// IANA media type, e.g. `application/pdf`.
    pub mime: String,
    /// The file bytes.
    pub bytes: Vec<u8>,
}

/// One message to send from a connected mailbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingMessage {
    /// Recipient addresses.
    pub to: Vec<String>,
    /// Subject line.
    pub subject: String,
    /// Plain-text body.
    pub body_text: String,
    /// Attachments, in order.
    pub attachments: Vec<Attachment>,
}

/// Failures from the mail adapter.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MailError {
    /// No connection with that id (the user disconnected it).
    #[error("unknown mail connection")]
    UnknownAccount,
    /// The connection holds no usable token and could not be refreshed; the UI
    /// asks the user to reconnect.
    #[error("the mail connection needs to be authorised again")]
    Reauthorize,
    /// The provider rejected the message (bad address, blocked, quota).
    #[error("the provider rejected the message: {0}")]
    Rejected(String),
    /// The request never completed (offline, DNS, TLS, timeout).
    #[error("the mail provider could not be reached: {0}")]
    Transport(String),
}

/// Send email as a connected mailbox.
pub trait MailPort: Send + Sync {
    /// Send `message` from the connection `account_id`.
    fn send<'a>(
        &'a self,
        account_id: &'a str,
        message: &'a OutgoingMessage,
    ) -> MailFuture<'a, Result<(), MailError>>;
}

// ---- Scoped mail reading (S8.3, FR-38/104) --------------------------------
//
// The pipeline's message shape lives in `boba_domain::mail` (AD-14); this port
// is where a matched message stops. `boba-store` implements it without being
// allowed to depend on the `boba-mail` adapter (AD-1), and `boba-mail`
// re-exports both the types and the trait so the engine keeps one vocabulary.

use boba_domain::mail::{StoredMessage, SyncError};

/// Where a matched message goes. The `email` adapter writes it to
/// `email_message`; tests collect it in memory.
///
/// `&self` (not `&mut self`) so one sink can be mounted as `Arc<dyn
/// MessageSink>` and shared by the `email_sync` job; the adapter serialises its
/// own writes.
pub trait MessageSink: Send + Sync {
    /// Persist one matched message. A failure stops the pass: the caller
    /// records the error and retries from its cursor.
    fn store(&self, message: StoredMessage<'_>) -> Result<(), SyncError>;
}
