//! `EmailRepo` — owns `email_account` (AD-13, AD-16). Only this area writes that
//! table; other modules read through its queries. The adapter is `boba-store`.
//!
//! The row holds **connection metadata only**: the granted scopes, the mode and
//! an opaque `secret_ref`. Tokens live in the OS keychain, keyed by that
//! reference, and never reach this table, a log or an export (AD-13, FR-40).

use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;

/// Which mailbox provider a connection belongs to (architecture §11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum MailProvider {
    /// Outlook.com / Microsoft 365 over Microsoft Graph.
    MicrosoftGraph,
    /// Gmail over the user's own or Boba's desktop OAuth client (S8.2).
    Gmail,
    /// Any provider over IMAP/SMTP (S8.2).
    ImapSmtp,
}

impl MailProvider {
    /// The stored text form (mirrors the `email_account.provider` CHECK).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MicrosoftGraph => "microsoft_graph",
            Self::Gmail => "gmail",
            Self::ImapSmtp => "imap_smtp",
        }
    }

    /// Parse the stored text form.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "microsoft_graph" => Some(Self::MicrosoftGraph),
            "gmail" => Some(Self::Gmail),
            "imap_smtp" => Some(Self::ImapSmtp),
            _ => None,
        }
    }
}

/// Whether a connection may read the mailbox or only send (FR-37, AC3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum EmailMode {
    /// Send only: reply parsing is off (FR-37).
    SendOnly,
    /// Read and send.
    ReadSend,
}

impl EmailMode {
    /// The stored text form (mirrors the `email_account.mode` CHECK).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SendOnly => "send_only",
            Self::ReadSend => "read_send",
        }
    }

    /// Parse the stored text form.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "send_only" => Some(Self::SendOnly),
            "read_send" => Some(Self::ReadSend),
            _ => None,
        }
    }

    /// The delegated **mail** scopes this mode needs, and nothing more (AC1,
    /// NFR-5).
    ///
    /// Delegated only: Boba never asks for app-only or admin consent, and it
    /// never asks for `User.Read` — the mailbox address is typed by the user.
    pub const fn required_scopes(self) -> &'static [&'static str] {
        match self {
            Self::SendOnly => &["Mail.Send"],
            Self::ReadSend => &["Mail.Read", "Mail.Send"],
        }
    }
}

/// The standard OIDC scope that keeps a connection usable while the user is
/// away.
///
/// It is not a mail permission and grants no access on its own: it is what asks
/// the provider for a **refresh token**, which this design needs (AD-13 keeps
/// refresh tokens in the keychain; the daily run and the reply sync work while
/// the user is not looking). Microsoft Graph issues a refresh token only when
/// the authorisation request asks for it.
pub const OFFLINE_ACCESS_SCOPE: &str = "offline_access";

/// Where the OAuth client id came from (AC1: bundled public client, or the
/// user's own).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ClientIdKind {
    /// Boba's bundled public desktop client.
    Bundled,
    /// The user's own app registration (BYO client id).
    Byo,
}

impl ClientIdKind {
    /// The stored text form (mirrors the `email_account.client_id_kind` CHECK).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bundled => "bundled",
            Self::Byo => "byo",
        }
    }

    /// Parse the stored text form.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "bundled" => Some(Self::Bundled),
            "byo" => Some(Self::Byo),
            _ => None,
        }
    }
}

/// One connected mailbox (AD-16 owner area: email).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct EmailAccount {
    /// ULID of the connection.
    pub id: String,
    /// Provider this connection talks to.
    pub provider: MailProvider,
    /// The mailbox address, as typed by the user (no `User.Read` scope needed).
    pub address: String,
    /// Send-only or read+send (AC3).
    pub mode: EmailMode,
    /// The OAuth client id the tokens belong to (public by definition, §11).
    pub client_id: String,
    /// Whether that client id is Boba's bundled one or the user's own.
    pub client_id_kind: ClientIdKind,
    /// The delegated scopes that were actually requested and granted (AC1).
    pub scopes: Vec<String>,
    /// Keychain entry holding the tokens. An opaque reference, never a token
    /// (AD-13, FR-40).
    pub secret_ref: String,
    /// When the connection was first authorised.
    pub connected_at: String,
    /// Last change (mode switch, re-consent).
    pub updated_at: String,
}

impl EmailAccount {
    /// Whether the granted scopes cover `mode`; a mode switch that needs more
    /// scopes must re-run the OAuth flow (AC1/NFR-5: scopes follow the mode).
    pub fn covers(&self, mode: EmailMode) -> bool {
        mode.required_scopes()
            .iter()
            .all(|scope| self.scopes.iter().any(|granted| granted == scope))
    }
}

/// A repo-level failure, stringified so the port stays free of the adapter's
/// error types.
#[derive(Debug, Clone, Error)]
#[error("email store error: {0}")]
pub struct EmailRepoError(pub String);

impl EmailRepoError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// The `email` owner area (AD-16).
pub trait EmailRepo: Send + Sync {
    /// Insert or replace one connection.
    fn upsert_account(&self, account: &EmailAccount) -> Result<(), EmailRepoError>;

    fn get_account(&self, id: &str) -> Result<Option<EmailAccount>, EmailRepoError>;

    /// The connection for one mailbox, so reconnecting replaces it instead of
    /// adding a duplicate.
    fn find_account(
        &self,
        provider: MailProvider,
        address: &str,
    ) -> Result<Option<EmailAccount>, EmailRepoError>;

    fn list_accounts(&self) -> Result<Vec<EmailAccount>, EmailRepoError>;

    /// Switch one connection between send-only and read+send (AC3). Fails when
    /// the id is unknown.
    fn set_mode(&self, id: &str, mode: EmailMode, updated_at: &str) -> Result<(), EmailRepoError>;

    /// Remove one connection; returns whether a row was removed. Purging the
    /// stored mail of that connection belongs to the sync owner (S8.3).
    fn delete_account(&self, id: &str) -> Result<bool, EmailRepoError>;
}
