//! Email connection commands (S8.1, FR-35/37/39/40; AC1–AC4).
//!
//! Thin wrappers: validate the input, drive the authorisation flow or the ports,
//! and map the failure onto the typed IPC error (AD-3). The security-critical
//! parts — PKCE S256, the single-use `state`, loopback-only redirect — live in
//! `boba_mail::oauth`, so the flow cannot be re-implemented differently by the
//! UI. The store arrives through [`wiring::port`] (ADR-012), so a locked store is
//! `errors.store.locked` instead of a panic.
//!
//! Reading mail is the sync owner's job (S8.3): this slice connects, switches the
//! mode and disconnects.

use boba_app::error::{AppError, AppResult, ErrorCode};
use boba_app::jobs::{JobHandler, JobKind};
use boba_app::ports::clock::ClockPort;
use boba_app::ports::repo::apply::ApplyRepo;
use boba_app::ports::repo::email::{
    ClientIdKind, EmailAccount, EmailMode, EmailRepo, MailProvider, OFFLINE_ACCESS_SCOPE,
};
use boba_app::ports::repo::ingestion::IngestionRepo;
use boba_app::ports::repo::outbox::OutboxRepo;
use boba_app::ports::repo::system::SystemRepo;
use boba_app::ports::secret::SecretPort;
use boba_app::system::SystemClock;
use boba_mail::gmail::{
    self, BUNDLED_GMAIL_CLIENT_ID, BUNDLED_GMAIL_CLIENT_SECRET, GMAIL_SEND_SCOPE, GmailSecret,
};
use boba_mail::graph::GraphMail;
use boba_mail::imap::{self, Auth, ImapError, ImapSettings};
use boba_mail::oauth::{
    self, BUNDLED_GRAPH_CLIENT_ID, OAuthError as FlowError, PendingFlow, ProviderEndpoints,
    STATE_TTL, TokenSet, loopback,
};
use boba_mail::smtp::{self, ImapSmtpSecret, SmtpError, SmtpSettings};
use boba_mail::sync::{self, MessageScope, MessageSink, RawMessage, SyncRequest};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Runtime};

use crate::wiring;

/// What the Connections screen shows for one mailbox.
///
/// Deliberately without the keychain reference: the UI never needs it, and a
/// token or a reference never crosses IPC (AD-13, FR-40).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ConnectionView {
    /// Connection id.
    pub id: String,
    /// Provider the mailbox is connected to.
    pub provider: MailProvider,
    /// The mailbox address.
    pub address: String,
    /// Send-only or read+send (AC3).
    pub mode: EmailMode,
    /// Whether this connection uses Boba's bundled client or the user's own.
    pub client_id_kind: ClientIdKind,
    /// The delegated scopes granted at consent (AC1).
    pub scopes: Vec<String>,
    /// When the mailbox was connected.
    pub connected_at: String,
}

impl From<EmailAccount> for ConnectionView {
    fn from(account: EmailAccount) -> Self {
        Self {
            id: account.id,
            provider: account.provider,
            address: account.address,
            mode: account.mode,
            client_id_kind: account.client_id_kind,
            scopes: account.scopes,
            connected_at: account.connected_at,
        }
    }
}

/// The Connect Outlook form: address, mode, and an optional BYO client id (AC1).
#[derive(Debug, Clone, Deserialize, Type)]
pub struct ConnectOutlookInput {
    /// The mailbox address, as typed by the user. Boba never asks for a
    /// `User.Read` scope to guess it (NFR-5).
    pub address: String,
    /// Send-only or read+send; it decides the scopes that are requested (AC3).
    pub mode: EmailMode,
    /// The user's own app-registration client id, when they have one.
    pub client_id: Option<String>,
}

/// Everything an email command can fail with, and how it becomes the typed IPC
/// error (AD-3). The keys live in the shared `errors` catalog the core embeds
/// (AD-21), and the UI owns the wording: no provider text, code or token is ever
/// shown, and a failure is never a raw Rust string.
#[derive(Debug, thiserror::Error)]
enum EmailFailure {
    /// The typed-in mailbox address is not an address.
    #[error("bad mailbox address")]
    Address,
    /// This build has no bundled client id and the user gave none.
    #[error("no bundled client id")]
    NoBundledClient,
    /// The system browser could not be opened.
    #[error("the browser could not be opened")]
    Browser,
    /// The connection was removed while the flow was running.
    #[error("unknown connection")]
    UnknownConnection,
    /// Reading is not covered by the granted scopes, so it must be re-authorised.
    #[error("re-authorisation needed")]
    NeedsReconsent,
    /// The keychain refused to store the tokens. Nothing was connected.
    #[error("the keychain refused the write")]
    Keychain,
    /// The user gave a client id but no client secret. Google needs the public
    /// installed-app secret at its token endpoint, so the flow cannot complete:
    /// this is refused before the browser opens, not after.
    #[error("the client secret is missing")]
    ClientSecret,
    /// The connection test before saving failed (AC2), for one of three reasons.
    #[error("the connection test failed: {0:?}")]
    Probe(ProbeOutcome),
    /// An internal step failed (the listener task died).
    #[error("internal failure")]
    Internal,
    /// The row could not be read or written.
    #[error("email store error: {0}")]
    Store(#[from] boba_app::ports::repo::email::EmailRepoError),
    /// The authorisation flow failed.
    #[error(transparent)]
    Flow(#[from] oauth::OAuthError),
}

impl EmailFailure {
    /// The machine-readable category.
    fn code(&self) -> ErrorCode {
        match self {
            Self::Address | Self::NoBundledClient | Self::ClientSecret => ErrorCode::InvalidInput,
            Self::Browser => ErrorCode::Unavailable,
            Self::UnknownConnection => ErrorCode::NotFound,
            Self::NeedsReconsent => ErrorCode::Conflict,
            Self::Probe(outcome) => match outcome {
                ProbeOutcome::Credentials => ErrorCode::Unauthorized,
                ProbeOutcome::Unreachable => ErrorCode::Unavailable,
                ProbeOutcome::Refused | ProbeOutcome::Settings => ErrorCode::InvalidInput,
            },
            Self::Keychain | Self::Internal | Self::Store(_) => ErrorCode::Internal,
            Self::Flow(flow) => match flow {
                oauth::OAuthError::Timeout | oauth::OAuthError::Transport => ErrorCode::Unavailable,
                oauth::OAuthError::Provider(_) => ErrorCode::Cancelled,
                oauth::OAuthError::TokenRejected(_) => ErrorCode::Unauthorized,
                oauth::OAuthError::StateMismatch
                | oauth::OAuthError::StateExpired
                | oauth::OAuthError::MissingCode => ErrorCode::InvalidInput,
                _ => ErrorCode::Internal,
            },
        }
    }

    /// The i18n key the UI resolves (AD-3). One place, so the i18n gate can check
    /// every key the core can emit.
    fn message_key(&self) -> &'static str {
        match self {
            Self::Address => "errors.email.address",
            Self::NoBundledClient => "errors.email.no_bundled_client",
            Self::ClientSecret => "errors.email.client_secret",
            Self::Browser => "errors.email.browser",
            Self::UnknownConnection => "errors.email.unknown_connection",
            Self::NeedsReconsent => "errors.email.needs_reconsent",
            Self::Probe(outcome) => match outcome {
                ProbeOutcome::Credentials => "errors.email.probe_credentials",
                ProbeOutcome::Unreachable => "errors.email.probe_unreachable",
                ProbeOutcome::Refused => "errors.email.probe_refused",
                ProbeOutcome::Settings => "errors.email.probe_settings",
            },
            Self::Keychain => "errors.email.keychain",
            Self::Internal => "errors.email.internal",
            Self::Store(_) => "errors.email.store",
            Self::Flow(flow) => match flow {
                FlowError::Timeout => "errors.email.timeout",
                FlowError::Listen => "errors.email.listen",
                FlowError::Provider(_) => "errors.email.refused",
                FlowError::StateMismatch | FlowError::StateExpired => "errors.email.state",
                FlowError::MissingCode => "errors.email.no_code",
                FlowError::Transport => "errors.email.offline",
                FlowError::TokenRejected(_) => "errors.email.rejected",
                FlowError::InvalidTokens => "errors.email.tokens",
                FlowError::Rng | FlowError::BadUrl => "errors.email.internal",
            },
        }
    }
}

impl From<EmailFailure> for AppError {
    fn from(failure: EmailFailure) -> Self {
        let error = AppError::new(failure.code(), failure.message_key());
        match failure {
            // The cause is a diagnostic detail, never rendered as user text.
            EmailFailure::Store(cause) => error.with_detail("cause", cause.to_string()),
            _ => error,
        }
    }
}

/// Every connected mailbox, oldest first.
#[tauri::command]
#[specta::specta]
pub fn email_accounts(app: tauri::AppHandle) -> AppResult<Vec<ConnectionView>> {
    let accounts = wiring::port::<dyn EmailRepo>(&app)?;
    let rows = accounts.list_accounts().map_err(EmailFailure::Store)?;
    Ok(rows.into_iter().map(ConnectionView::from).collect())
}

/// Connect Outlook with delegated OAuth (AC1, AC2).
///
/// Opens the system browser on the provider's authorisation URL and waits for
/// the single loopback callback. A failure to bind the port, an expired or
/// mismatched `state`, or a refused consent all surface as typed errors that
/// point the UI at troubleshooting or the user's own client id (AC2).
#[tauri::command]
#[specta::specta]
pub async fn email_connect_outlook(
    app: tauri::AppHandle,
    input: ConnectOutlookInput,
) -> AppResult<ConnectionView> {
    let accounts = wiring::port::<dyn EmailRepo>(&app)?;
    let secrets = wiring::port::<dyn SecretPort>(&app)?;

    let address = mailbox_address(&input.address)?;
    // AC1: Boba's bundled public client, or the user's own client id.
    let (client_id, client_id_kind) = bundled_or_byo(
        input.client_id.as_deref(),
        BUNDLED_GRAPH_CLIENT_ID,
        EmailFailure::NoBundledClient,
    )?;

    // The mode's mail scopes, plus the standard scope that asks for a refresh
    // token — without it the connection dies when the access token expires and
    // the daily run cannot work while the user is away (AC1, AD-13).
    let mut requested: Vec<&str> = input.mode.required_scopes().to_vec();
    requested.push(OFFLINE_ACCESS_SCOPE);

    let endpoints = ProviderEndpoints::microsoft_graph();
    let (listener, redirect_uri) = loopback::bind().map_err(EmailFailure::from)?;
    let flow = PendingFlow::new(
        endpoints.clone(),
        client_id.clone(),
        redirect_uri,
        &requested,
    )
    .map_err(EmailFailure::from)?;
    let tokens = finish_flow(&app, flow, listener).await?;

    // AC1/FR-40: the tokens go to the keychain, addressed by an opaque
    // reference; the row keeps the reference and the granted scopes only.
    let account = connection_row(
        accounts.as_ref(),
        MailProvider::MicrosoftGraph,
        address,
        input.mode,
        client_id,
        client_id_kind,
        tokens.scopes.clone(),
    )?;
    secrets
        .put(&account.secret_ref, &tokens.to_json())
        .map_err(|_| EmailFailure::Keychain)?;
    accounts
        .upsert_account(&account)
        .map_err(EmailFailure::Store)?;
    Ok(ConnectionView::from(account))
}

/// The Connect Gmail form: the address, and an optional BYO client id with its
/// public installed-app secret (AC1).
#[derive(Debug, Clone, Deserialize, Type)]
pub struct ConnectGmailInput {
    /// The mailbox address, as typed by the user. Boba never asks for a
    /// `gmail.readonly` scope to guess it (NFR-5).
    pub address: String,
    /// The user's own Google client id, when they have one ("advanced").
    pub client_id: Option<String>,
    /// The public value Google prints next to that client id. Not confidential by
    /// Google's own documentation, and required at the token endpoint.
    pub client_secret: Option<String>,
}

/// Connect Gmail with delegated OAuth (AC1).
///
/// Same loopback flow as Outlook (PKCE `S256`, single-use `state`, loopback-only),
/// with one deliberate difference: the requested scope is `gmail.send` and nothing
/// else. Boba's bundled client cannot read Gmail — reading is IMAP (S8.3), a BYO
/// client (S8.5) or a `.eml` import — so the connection is stored send-only and
/// `covers(ReadSend)` stays false.
#[tauri::command]
#[specta::specta]
pub async fn email_connect_gmail(
    app: tauri::AppHandle,
    input: ConnectGmailInput,
) -> AppResult<ConnectionView> {
    let accounts = wiring::port::<dyn EmailRepo>(&app)?;
    let secrets = wiring::port::<dyn SecretPort>(&app)?;

    let address = mailbox_address(&input.address)?;
    // AC1: the user's own client (id **and** secret), or Boba's bundled pair.
    // Resolved before the browser opens, so a half-configured client is a form
    // error instead of a failed consent screen.
    let (client_id, client_id_kind, client_secret) = gmail_client(
        input.client_id.as_deref(),
        input.client_secret.as_deref(),
        BUNDLED_GMAIL_CLIENT_ID,
        BUNDLED_GMAIL_CLIENT_SECRET,
    )?;

    let (listener, redirect_uri) = loopback::bind().map_err(EmailFailure::from)?;
    let mut flow = PendingFlow::new(
        gmail::endpoints(),
        client_id.clone(),
        redirect_uri,
        &[GMAIL_SEND_SCOPE],
    )
    .map_err(EmailFailure::from)?;
    if let Some(secret) = &client_secret {
        flow = flow.with_client_secret(secret.clone());
    }
    let tokens = finish_flow(&app, flow, listener).await?;

    let account = connection_row(
        accounts.as_ref(),
        MailProvider::Gmail,
        address,
        EmailMode::SendOnly,
        client_id,
        client_id_kind,
        tokens.scopes.clone(),
    )?;
    let stored = GmailSecret {
        tokens: tokens.clone(),
        client_secret,
    };
    secrets
        .put(&account.secret_ref, &stored.to_json())
        .map_err(|_| EmailFailure::Keychain)?;
    accounts
        .upsert_account(&account)
        .map_err(EmailFailure::Store)?;
    Ok(ConnectionView::from(account))
}

/// A typed mailbox address, or the error the UI shows.
fn mailbox_address(address: &str) -> Result<String, EmailFailure> {
    let address = address.trim().to_string();
    if address.is_empty() || !address.contains('@') {
        return Err(EmailFailure::Address);
    }
    Ok(address)
}

/// Boba's bundled public client id, or the user's own (AC1).
fn bundled_or_byo(
    byo: Option<&str>,
    bundled: Option<&str>,
    missing: EmailFailure,
) -> Result<(String, ClientIdKind), EmailFailure> {
    match byo {
        Some(id) if !id.is_empty() => Ok((id.to_string(), ClientIdKind::Byo)),
        _ => match bundled {
            Some(id) => Ok((id.to_string(), ClientIdKind::Bundled)),
            None => Err(missing),
        },
    }
}

/// Which Google client a Gmail connection uses (AC1).
///
/// The user's own client needs its **public installed-app secret**: Google
/// demands it at the token endpoint even with PKCE, so an id without one is
/// refused here — before the browser opens. Boba's bundled client exists only
/// when the build set both `BOBA_GMAIL_CLIENT_ID` and `BOBA_GMAIL_CLIENT_SECRET`
/// (never committed: a client id belongs to a registration, not to the tree).
fn gmail_client(
    byo_id: Option<&str>,
    byo_secret: Option<&str>,
    bundled_id: Option<&str>,
    bundled_secret: Option<&str>,
) -> Result<(String, ClientIdKind, Option<String>), EmailFailure> {
    let clean = |value: Option<&str>| {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };

    if let Some(id) = clean(byo_id) {
        let secret = clean(byo_secret).ok_or(EmailFailure::ClientSecret)?;
        return Ok((id, ClientIdKind::Byo, Some(secret)));
    }
    match (clean(bundled_id), clean(bundled_secret)) {
        (Some(id), Some(secret)) => Ok((id, ClientIdKind::Bundled, Some(secret))),
        _ => Err(EmailFailure::NoBundledClient),
    }
}

/// How an IMAP/SMTP account authenticates (AC2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum MailCredentialKind {
    /// The provider's app password.
    Password,
    /// An OAuth 2.0 access token, offered as `XOAUTH2`. Spelled out, so the wire
    /// value is not split into `x_o_auth2` by the case conversion.
    #[serde(rename = "xoauth2")]
    XOAuth2,
}

/// Whether the socket is secured from the first byte or upgraded (AC2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum MailSecurity {
    /// Implicit TLS (IMAP 993, SMTP 465).
    Tls,
    /// Plain TCP upgraded with `STARTTLS` (IMAP 143, SMTP 587).
    StartTls,
}

impl From<MailSecurity> for boba_mail::imap::Security {
    fn from(security: MailSecurity) -> Self {
        match security {
            MailSecurity::Tls => Self::Tls,
            MailSecurity::StartTls => Self::StartTls,
        }
    }
}

/// The Connect IMAP/SMTP form (AC2).
#[derive(Debug, Clone, Deserialize, Type)]
pub struct ConnectImapSmtpInput {
    /// The mailbox address, as typed by the user.
    pub address: String,
    /// The login both protocols use, usually the address itself.
    pub username: String,
    /// The app password, or the access token for `XOAUTH2`.
    pub credential: String,
    /// Which of the two that is.
    pub credential_kind: MailCredentialKind,
    /// IMAP host.
    pub imap_host: String,
    /// IMAP port: 993 for TLS, 143 for `STARTTLS`.
    pub imap_port: u16,
    /// How the IMAP socket is protected.
    pub imap_security: MailSecurity,
    /// Submission host.
    pub smtp_host: String,
    /// Submission port: 465 for TLS, 587 for `STARTTLS`.
    pub smtp_port: u16,
    /// How the submission socket is protected.
    pub smtp_security: MailSecurity,
}

impl ConnectImapSmtpInput {
    /// The adapter's settings and the credential, or what is missing.
    fn parts(self) -> Result<(ImapSettings, SmtpSettings, Auth), EmailFailure> {
        let credential = match self.credential_kind {
            MailCredentialKind::Password => Auth::Password(self.credential),
            MailCredentialKind::XOAuth2 => Auth::XOAuth2(self.credential),
        };
        let username = self.username.trim().to_string();
        let imap = ImapSettings {
            host: self.imap_host.trim().to_string(),
            port: self.imap_port,
            security: self.imap_security.into(),
            username: username.clone(),
        };
        let smtp = SmtpSettings {
            host: self.smtp_host.trim().to_string(),
            port: self.smtp_port,
            security: self.smtp_security.into(),
            username,
        };
        // The adapters own the settings rules; an empty credential is not one
        // (the secret itself is only ever read by the protocol).
        let empty = match &credential {
            Auth::Password(secret) | Auth::XOAuth2(secret) => secret.trim().is_empty(),
        };
        if imap.validate().is_err() || smtp.validate().is_err() || empty {
            return Err(EmailFailure::Probe(ProbeOutcome::Settings));
        }
        Ok((imap, smtp, credential))
    }
}

/// Why a connection test failed, in the three ways the user can act on (AC2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// The server rejected the app password or the token.
    Credentials,
    /// The server could not be reached at all.
    Unreachable,
    /// The server refused the connection, or would not speak the protocol.
    Refused,
    /// The form is incomplete.
    Settings,
}

impl From<ImapError> for ProbeOutcome {
    fn from(error: ImapError) -> Self {
        match error {
            ImapError::Connect => Self::Unreachable,
            ImapError::Credentials => Self::Credentials,
            ImapError::Tls | ImapError::Refused | ImapError::Protocol | ImapError::Settings => {
                Self::Refused
            }
        }
    }
}

impl From<SmtpError> for ProbeOutcome {
    fn from(error: SmtpError) -> Self {
        match error {
            SmtpError::Connect => Self::Unreachable,
            SmtpError::Credentials => Self::Credentials,
            SmtpError::Tls | SmtpError::Refused | SmtpError::Protocol | SmtpError::Settings => {
                Self::Refused
            }
        }
    }
}

/// Connect an IMAP/SMTP account (AC2).
///
/// The connection test runs **before anything is saved**: the IMAP probe opens
/// `INBOX` read-only and the submission probe authenticates, and only when both
/// pass are the endpoints and the credential written to the keychain and the
/// connection to the store. A wrong password therefore leaves no trace.
#[tauri::command]
#[specta::specta]
pub async fn email_connect_imap_smtp(
    app: tauri::AppHandle,
    input: ConnectImapSmtpInput,
) -> AppResult<ConnectionView> {
    let accounts = wiring::port::<dyn EmailRepo>(&app)?;
    let secrets = wiring::port::<dyn SecretPort>(&app)?;
    Ok(
        connect_imap_smtp_with(&LiveProbe, accounts.as_ref(), secrets.as_ref(), input)
            .await
            .map(ConnectionView::from)?,
    )
}

/// The connection test one IMAP/SMTP account must pass before it is saved (AC2).
///
/// It is a value so the save can be exercised without a provider's TLS
/// endpoints; the real one is [`LiveProbe`].
trait MailProbe: Send + Sync {
    /// Both endpoints, IMAP first; `Err` is why the account is not saved.
    fn probe<'a>(
        &'a self,
        imap: &'a ImapSettings,
        smtp: &'a SmtpSettings,
        credential: &'a Auth,
    ) -> Pin<Box<dyn Future<Output = Result<(), ProbeOutcome>> + Send + 'a>>;
}

/// The probes as the app runs them (the real sockets, read-only).
struct LiveProbe;

impl MailProbe for LiveProbe {
    fn probe<'a>(
        &'a self,
        imap: &'a ImapSettings,
        smtp: &'a SmtpSettings,
        credential: &'a Auth,
    ) -> Pin<Box<dyn Future<Output = Result<(), ProbeOutcome>> + Send + 'a>> {
        Box::pin(async move {
            // AC2: the IMAP test never writes — it opens `INBOX` with `EXAMINE`,
            // so nothing on the server changes — and the submission test only
            // authenticates. Both run before anything is saved.
            imap::test_connection(imap, credential)
                .await
                .map_err(ProbeOutcome::from)?;
            smtp::test_connection(smtp, credential)
                .await
                .map_err(ProbeOutcome::from)
        })
    }
}

/// The connect itself, over the two ports (so it is testable without an app).
async fn connect_imap_smtp_with(
    probe: &dyn MailProbe,
    accounts: &dyn EmailRepo,
    secrets: &dyn SecretPort,
    input: ConnectImapSmtpInput,
) -> Result<EmailAccount, EmailFailure> {
    let address = mailbox_address(&input.address)?;
    let (imap, smtp, credential) = input.parts()?;

    probe
        .probe(&imap, &smtp, &credential)
        .await
        .map_err(EmailFailure::Probe)?;

    // Nothing is stored before this line: a failed probe leaves no connection.
    let account = connection_row(
        accounts,
        MailProvider::ImapSmtp,
        address,
        // An IMAP/SMTP account reads (read-only) and sends, and holds no OAuth
        // grant, so `covers` has nothing to compare.
        EmailMode::ReadSend,
        String::new(),
        ClientIdKind::Byo,
        Vec::new(),
    )?;
    let stored = ImapSmtpSecret {
        imap,
        smtp,
        credential,
    };
    secrets
        .put(&account.secret_ref, &stored.to_json())
        .map_err(|_| EmailFailure::Keychain)?;
    accounts
        .upsert_account(&account)
        .map_err(EmailFailure::Store)?;
    Ok(account)
}

/// Drive one loopback authorisation to tokens (AC1, AC2): open the user's browser
/// and take the single callback. The PKCE verifier and the `state` never leave the
/// flow, so the UI cannot re-implement the security-critical part differently.
async fn finish_flow(
    app: &tauri::AppHandle,
    mut flow: PendingFlow,
    listener: std::net::TcpListener,
) -> Result<TokenSet, EmailFailure> {
    open_in_browser(app, &flow.authorize_url().map_err(EmailFailure::from)?)?;

    // The listener accepts exactly one callback and closes (AC2). It runs on a
    // blocking thread so the flow's own deadline is the only wait.
    let callback = tokio::task::spawn_blocking(move || {
        loopback::accept_callback(&listener, STATE_TTL + Duration::from_secs(5))
    })
    .await
    .map_err(|_| EmailFailure::Internal)?
    .map_err(EmailFailure::from)?;

    let code = flow
        .take_callback(&callback, std::time::Instant::now())
        .map_err(EmailFailure::from)?;
    let scopes = flow.scopes().to_vec();

    let http = oauth::http_client().map_err(EmailFailure::from)?;
    // The code is exchanged with the PKCE verifier, which stays inside the flow.
    let response = flow
        .exchange(&http, &code)
        .await
        .map_err(EmailFailure::from)?;
    Ok(TokenSet::new(&response, &scopes))
}

/// The row for one authorised connection.
///
/// Reconnects replace the connection for the same mailbox instead of adding a
/// duplicate, and the timestamps are the store's (AD-16).
fn connection_row(
    accounts: &dyn EmailRepo,
    provider: MailProvider,
    address: String,
    mode: EmailMode,
    client_id: String,
    client_id_kind: ClientIdKind,
    scopes: Vec<String>,
) -> Result<EmailAccount, EmailFailure> {
    let now = SystemClock.now_rfc3339();
    let existing = accounts
        .find_account(provider, &address)
        .map_err(EmailFailure::Store)?;
    let id = existing
        .as_ref()
        .map_or_else(boba_store::repos::email::new_account_id, |a| a.id.clone());
    Ok(EmailAccount {
        secret_ref: format!("mail:{id}"),
        id,
        provider,
        address,
        mode,
        client_id,
        client_id_kind,
        scopes,
        connected_at: existing.map_or_else(|| now.clone(), |a| a.connected_at),
        updated_at: now,
    })
}

/// Switch a connection between send-only and read+send (AC3).
///
/// Reading is only switched on when the granted scopes cover it (NFR-5): a
/// connection authorised send-only must run the flow again, so the UI can ask
/// for `Mail.Read` instead of claiming reading works.
#[tauri::command]
#[specta::specta]
pub fn email_set_mode(
    app: tauri::AppHandle,
    id: String,
    mode: EmailMode,
) -> AppResult<ConnectionView> {
    let accounts = wiring::port::<dyn EmailRepo>(&app)?;
    let account = accounts
        .get_account(&id)
        .map_err(EmailFailure::Store)?
        .ok_or(EmailFailure::UnknownConnection)?;
    if !account.covers(mode) {
        return Err(EmailFailure::NeedsReconsent.into());
    }
    accounts
        .set_mode(&id, mode, &SystemClock.now_rfc3339())
        .map_err(EmailFailure::Store)?;
    let mut updated = account;
    updated.mode = mode;
    Ok(ConnectionView::from(updated))
}

/// Disconnect a mailbox (AC4).
///
/// Revokes the tokens where the provider supports it, then deletes them from the
/// keychain and the connection. A provider that refuses (or has no revocation
/// endpoint) never blocks the local deletion: the user asked to disconnect.
/// Purging the stored mail is the sync owner's job (S8.3).
#[tauri::command]
#[specta::specta]
pub async fn email_disconnect(app: tauri::AppHandle, id: String) -> AppResult<bool> {
    let accounts = wiring::port::<dyn EmailRepo>(&app)?;
    let secrets = wiring::port::<dyn SecretPort>(&app)?;
    Ok(disconnect(accounts.as_ref(), secrets.as_ref(), &id).await?)
}

/// The disconnect itself, over the two ports (so it is testable without an app).
///
/// Order matters: revoke where the provider can, delete the keychain entry, and
/// only then delete the row — the row is the reference to that entry, so losing
/// it while the entry remains would strand the tokens (AC4, FR-40).
async fn disconnect(
    accounts: &dyn EmailRepo,
    secrets: &dyn SecretPort,
    id: &str,
) -> Result<bool, EmailFailure> {
    let account = accounts
        .get_account(id)
        .map_err(EmailFailure::Store)?
        .ok_or(EmailFailure::UnknownConnection)?;

    revoke_tokens(secrets, &account).await?;

    // AC4: the tokens go before the row does. If the keychain refuses the
    // delete, the row is kept — it holds the only reference to the entry — so
    // Disconnect can be retried instead of stranding the tokens (FR-40).
    // Nothing stored (`Ok(false)`) is a successful disconnect, not a failure.
    if secrets
        .delete(&account.secret_ref)
        .map_err(|_| EmailFailure::Keychain)?
    {
        tracing::debug!("the stored tokens were deleted");
    }
    let removed = accounts.delete_account(id).map_err(EmailFailure::Store)?;
    Ok(removed)
}

/// Revoke the connection's tokens where the provider supports it (AC4).
///
/// Best effort by design: a provider without a revocation endpoint, a token set
/// that cannot be read and a refusal all end in "the local copy is deleted",
/// which is what the user asked for. Only building the HTTP client can fail.
async fn revoke_tokens(
    secrets: &dyn SecretPort,
    account: &EmailAccount,
) -> Result<(), EmailFailure> {
    let Some(endpoints) = ProviderEndpoints::for_provider(account.provider) else {
        return Ok(());
    };
    let Ok(Some(stored)) = secrets.get(&account.secret_ref) else {
        return Ok(());
    };
    let Some(tokens) = stored_tokens(account.provider, &stored) else {
        return Ok(());
    };

    let http = oauth::http_client().map_err(EmailFailure::from)?;
    let held = [
        tokens.refresh_token.as_deref(),
        Some(tokens.access_token.as_str()),
    ];
    for token in held.into_iter().flatten() {
        if let Err(error) = oauth::revoke(&http, &endpoints, &account.client_id, token).await {
            tracing::debug!(?error, "token revocation did not complete");
        }
    }
    Ok(())
}

/// Open the authorisation URL in the user's own browser, never in a webview.
fn open_in_browser(app: &tauri::AppHandle, url: &str) -> Result<(), EmailFailure> {
    use tauri_plugin_opener::OpenerExt as _;
    app.opener()
        .open_url(url.to_string(), None::<&str>)
        .map_err(|_| EmailFailure::Browser)
}

/// The tokens one connection's keychain entry holds, whatever else it carries
/// (S8.1 stores a Graph token set, S8.2 a Gmail entry with the public client
/// secret next to it). An entry of an unknown shape is "nothing to revoke".
fn stored_tokens(provider: MailProvider, stored: &str) -> Option<TokenSet> {
    match provider {
        MailProvider::Gmail => GmailSecret::from_json(stored)
            .ok()
            .map(|entry| entry.tokens),
        MailProvider::MicrosoftGraph | MailProvider::ImapSmtp => TokenSet::from_json(stored).ok(),
    }
}

// --- Scoped sync (S8.3, FR-38/104/113) ---

/// The `setting` row that carries one connection's sync cursor: the Unix second
/// of the newest message scanned. AD-18: a pass re-reads only newer mail, and
/// the sink's unique key makes an overlapping window harmless.
pub const SYNC_CURSOR_PREFIX: &str = "email.sync.cursor.";

/// How many messages one run reads per connection (the readers cap too).
const SYNC_READ_LIMIT: usize = 100;

/// Mount the `email_sync` job. Called as one line from `port_list` (ADR-012).
pub fn mount<R: Runtime>(app: &AppHandle<R>) {
    let registered = crate::scheduler::register_handler(
        app,
        JobKind::EmailSync,
        Arc::new(EmailSyncHandler::new(app)),
    );
    if !registered {
        tracing::warn!("email sync handler was not registered");
    }
}

/// The scheduled reply sync (FR-38): read+send connections only, scoped to the
/// Applications, incremental from the stored cursor.
///
/// A locked store never reaches this handler — the scheduler records the slot
/// `skipped_locked` and notifies (S5.1, ADR-009); the ports resolving here is
/// the proof the store is open.
pub struct EmailSyncHandler<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> EmailSyncHandler<R> {
    /// A handler for the app it will read its ports from.
    pub fn new(app: &AppHandle<R>) -> Self {
        Self { app: app.clone() }
    }
}

impl<R: Runtime> JobHandler for EmailSyncHandler<R> {
    fn run(&self, job: JobKind) -> Result<(), String> {
        if job != JobKind::EmailSync {
            return Err(format!("{} is not this handler's job", job.as_str()));
        }
        let port = |key: &str, error: AppError| format!("{key}: {}", error.message_key);
        let accounts =
            wiring::port::<dyn EmailRepo>(&self.app).map_err(|error| port("email repo", error))?;
        let secrets = wiring::port::<dyn SecretPort>(&self.app)
            .map_err(|error| port("secret port", error))?;
        let sink = wiring::port::<dyn MessageSink>(&self.app)
            .map_err(|error| port("message sink", error))?;
        let system = wiring::port::<dyn SystemRepo>(&self.app)
            .map_err(|error| port("system repo", error))?;
        let apply =
            wiring::port::<dyn ApplyRepo>(&self.app).map_err(|error| port("apply repo", error))?;
        let outbox = wiring::port::<dyn OutboxRepo>(&self.app)
            .map_err(|error| port("outbox repo", error))?;
        let offers = wiring::port::<dyn IngestionRepo>(&self.app)
            .map_err(|error| port("ingestion repo", error))?;

        let connected: Vec<EmailAccount> = accounts
            .list_accounts()
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|account| account.mode == EmailMode::ReadSend)
            .collect();
        if connected.is_empty() {
            return Ok(());
        }

        let scope = sync_scope(apply.as_ref(), outbox.as_ref(), offers.as_ref())?;
        let now = i64::try_from(oauth::now_unix()).unwrap_or(i64::MAX);
        let graph = GraphMail::new(Arc::clone(&secrets), Arc::clone(&accounts))
            .map_err(|error| error.to_string())?;

        // S8.4 follow-up (orquestador): one connection's failure must not abort
        // the others. Each account runs its own pass; the errors are collected
        // and reported together after every account was tried.
        let mut failures: Vec<String> = Vec::new();
        for account in connected {
            if let Err(error) = sync_account(
                &account,
                &graph,
                secrets.as_ref(),
                &scope,
                now,
                sink.as_ref(),
                system.as_ref(),
            ) {
                failures.push(format!("{}: {error}", account.id));
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("; "))
        }
    }
}

/// One connection's pass (S8.3/S8.4): fetch, filter and store; its `Err` is
/// that account's, never the run's.
#[allow(clippy::too_many_arguments)]
fn sync_account(
    account: &EmailAccount,
    graph: &GraphMail,
    secrets: &dyn SecretPort,
    scope: &MessageScope,
    now: i64,
    sink: &dyn MessageSink,
    system: &dyn SystemRepo,
) -> Result<(), String> {
    let after = read_cursor(system, &account.id)?;
    let since = after.unwrap_or_else(|| scope.cutoff(now));
    let messages = match account.provider {
        MailProvider::MicrosoftGraph => tauri::async_runtime::block_on(graph.fetch_inbox(
            &account.id,
            since,
            now,
            SYNC_READ_LIMIT,
        ))
        .map_err(|error| error.to_string())?,
        MailProvider::ImapSmtp => {
            let stored = secrets
                .get(&account.secret_ref)
                .map_err(|_| "the keychain could not be read".to_string())?
                .ok_or_else(|| "the connection needs to be authorised again".to_string())?;
            let secret = ImapSmtpSecret::from_json(&stored).map_err(|error| error.to_string())?;
            tauri::async_runtime::block_on(boba_mail::imap::fetch_inbox(
                &secret.imap,
                &secret.credential,
                since,
                now,
                SYNC_READ_LIMIT,
            ))
            .map_err(|error| error.to_string())?
        }
        // Gmail's bundled client has no read scope; the BYO read lands with
        // S8.5.
        MailProvider::Gmail => return Ok(()),
    };
    sync_pass(&account.id, messages, scope, now, after, sink, system)?;
    Ok(())
}

/// Run one connection's pass and persist its cursor. Split out so the wiring
/// (handler → `sync::run` → sink → cursor) is exercised without a mailbox.
fn sync_pass(
    account_id: &str,
    messages: Vec<RawMessage>,
    scope: &MessageScope,
    now: i64,
    after: Option<i64>,
    sink: &dyn MessageSink,
    system: &dyn SystemRepo,
) -> Result<usize, String> {
    let request = SyncRequest::incremental(account_id, now, after);
    let report = sync::run(messages, scope, &request, sink).map_err(|error| error.to_string())?;
    if let Some(next) = report.next_after
        && Some(next) != after
    {
        let updated_at = ClockPort::now_rfc3339(&boba_app::system::SystemClock);
        system
            .setting_put(&cursor_key(account_id), &next.to_string(), &updated_at)
            .map_err(|error| error.to_string())?;
    }
    Ok(report.stored)
}

/// The scope for one pass: the domains applications were sent to (and the
/// company hosts their Offers live on), plus the `Message-ID`s of what Boba
/// sent, so a reply in one of those threads is recognised.
fn sync_scope(
    apply: &dyn ApplyRepo,
    outbox: &dyn OutboxRepo,
    offers: &dyn IngestionRepo,
) -> Result<MessageScope, String> {
    let mut domains: Vec<String> = Vec::new();
    let mut sent: Vec<String> = Vec::new();
    for application in apply.applications().map_err(|error| error.to_string())? {
        for attempt in outbox
            .attempts_for(&application.id)
            .map_err(|error| error.to_string())?
        {
            if let Some(domain) = company_domain(&attempt.recipient) {
                domains.push(domain);
            }
            if let Some(message_id) = attempt.message_id {
                sent.push(message_id);
            }
        }
        // The Offer may still exist; a missing one only costs a domain.
        if let Ok(Some(offer)) = offers.get_offer(&application.posting_id) {
            if let Some(email) = offer.apply.email.as_deref()
                && let Some(domain) = company_domain(email)
            {
                domains.push(domain);
            }
            if let Some(host) = url_host(&offer.apply.url)
                && !is_excluded_host(&host)
            {
                domains.push(host);
            }
        }
    }
    domains.sort();
    domains.dedup();
    sent.sort();
    sent.dedup();
    Ok(MessageScope::new(domains, Vec::<String>::new(), sent))
}

/// Hosts that are never a company's own domain: free mailboxes and job boards.
/// A recipient at one of them (a relay, a board's own inbox) must not open the
/// scope to every mail that domain carries.
const EXCLUDED_HOSTS: &[&str] = &[
    // Free mailbox providers.
    "gmail.com",
    "googlemail.com",
    "outlook.com",
    "hotmail.com",
    "live.com",
    "yahoo.com",
    "yahoo.es",
    "yahoo.com.mx",
    "icloud.com",
    "me.com",
    "proton.me",
    "protonmail.com",
    "aol.com",
    "gmx.com",
    "gmx.net",
    "mail.com",
    "zoho.com",
    // Job boards and aggregators.
    "linkedin.com",
    "indeed.com",
    "glassdoor.com",
    "monster.com",
    "ziprecruiter.com",
    "dice.com",
    "wellfound.com",
    "angel.co",
    "infojobs.net",
    "infojobs.com.br",
    "computrabajo.com",
    "bumeran.com",
    "elempleo.com",
    "occ.com.mx",
];

/// Whether `host` is (or is a subdomain of) one of [`EXCLUDED_HOSTS`].
fn is_excluded_host(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    EXCLUDED_HOSTS
        .iter()
        .any(|excluded| host == *excluded || host.ends_with(&format!(".{excluded}")))
}

/// The domain of an email address, lowercased; `None` when it has none.
fn email_domain(address: &str) -> Option<String> {
    let (_, domain) = address.trim().rsplit_once('@')?;
    let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    (!domain.is_empty()).then_some(domain)
}

/// The company domain of an address: its domain unless that is a free mailbox
/// or a job board.
fn company_domain(address: &str) -> Option<String> {
    let domain = email_domain(address)?;
    (!is_excluded_host(&domain)).then_some(domain)
}

/// The host of an apply URL, when it parses.
fn url_host(url: &str) -> Option<String> {
    url::Url::parse(url).ok()?.host_str().map(str::to_string)
}

/// The cursor for one connection.
fn read_cursor(system: &dyn SystemRepo, account_id: &str) -> Result<Option<i64>, String> {
    system
        .setting_get(&cursor_key(account_id))
        .map_err(|error| error.to_string())?
        .map(|value| {
            value
                .parse::<i64>()
                .map_err(|_| format!("the sync cursor of {account_id} is not a timestamp"))
        })
        .transpose()
}

fn cursor_key(account_id: &str) -> String {
    format!("{SYNC_CURSOR_PREFIX}{account_id}")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use boba_app::ports::secret::SecretError;
    use boba_store::db::{Store, StoreConfig};
    use boba_store::keys::DbKey;
    use std::sync::Mutex;

    use super::*;

    /// Fixed key, so nothing touches the OS keychain (S8.1, like the wiring tests).
    const KEY_HEX: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

    fn open_store(tag: &str) -> Arc<Store> {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir: PathBuf =
            std::env::temp_dir().join(format!("boba-email-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let key = DbKey::from_hex(KEY_HEX).expect("test key");
        let config = StoreConfig::new(dir.join("boba.db"), dir.join("snapshots"));
        Arc::new(Store::open_with_key(config, key).expect("store opens"))
    }

    fn connection() -> EmailAccount {
        EmailAccount {
            id: "01ACC".into(),
            provider: MailProvider::MicrosoftGraph,
            address: "me@example.com".into(),
            mode: EmailMode::ReadSend,
            client_id: "client-1".into(),
            client_id_kind: ClientIdKind::Bundled,
            scopes: vec!["Mail.Read".into(), "Mail.Send".into()],
            secret_ref: "mail:01ACC".into(),
            connected_at: "2026-10-16T00:00:00Z".into(),
            updated_at: "2026-10-16T00:00:00Z".into(),
        }
    }

    /// A keychain stub: `get` and `delete` answer from the fields.
    struct StubSecrets {
        stored: Option<String>,
        delete: Result<bool, SecretError>,
    }

    impl SecretPort for StubSecrets {
        fn put(&self, _reference: &str, _value: &str) -> Result<(), SecretError> {
            unimplemented!("connect is not exercised here")
        }

        fn get(&self, _reference: &str) -> Result<Option<String>, SecretError> {
            Ok(self.stored.clone())
        }

        fn delete(&self, _reference: &str) -> Result<bool, SecretError> {
            self.delete.clone()
        }
    }

    fn seeded(tag: &str) -> (Arc<Store>, EmailAccount) {
        let store = open_store(tag);
        let account = connection();
        store.upsert_account(&account).expect("seed");
        (store, account)
    }

    /// F1: a keychain that refuses the delete must not strand the tokens. The row
    /// keeps the reference, so Disconnect can be retried (AC4, FR-40).
    #[tokio::test]
    async fn a_keychain_that_refuses_the_delete_keeps_the_connection() {
        let (store, account) = seeded("refuse");
        let secrets = StubSecrets {
            stored: None,
            delete: Err(SecretError::Keychain),
        };

        let failure = disconnect(store.as_ref(), &secrets, &account.id)
            .await
            .expect_err("a refused delete must fail the command");

        let error = AppError::from(failure);
        assert_eq!(error.message_key, "errors.email.keychain");
        assert_eq!(error.code, ErrorCode::Internal);
        assert_eq!(
            store.get_account(&account.id).expect("read").map(|a| a.id),
            Some(account.id.clone()),
            "the row survives, so the orphaned tokens can still be deleted"
        );
    }

    /// A keychain with nothing stored is a successful disconnect, not a failure.
    #[tokio::test]
    async fn an_absent_keychain_entry_still_disconnects() {
        let (store, account) = seeded("absent");
        let secrets = StubSecrets {
            stored: None,
            delete: Ok(false),
        };

        assert!(
            disconnect(store.as_ref(), &secrets, &account.id)
                .await
                .expect("disconnect")
        );
        assert!(store.get_account(&account.id).expect("read").is_none());
    }

    /// A provider without a revocation endpoint (Graph) still deletes the tokens
    /// and the row: revocation is best effort, deletion is not (AC4).
    #[tokio::test]
    async fn stored_tokens_are_deleted_even_without_a_revoke_endpoint() {
        let (store, account) = seeded("stored");
        let tokens = TokenSet {
            access_token: "at".into(),
            refresh_token: Some("rt".into()),
            expires_at_unix: 0,
            scopes: vec!["Mail.Send".into()],
        };
        let secrets = StubSecrets {
            stored: Some(tokens.to_json()),
            delete: Ok(true),
        };

        assert!(
            disconnect(store.as_ref(), &secrets, &account.id)
                .await
                .expect("disconnect")
        );
        assert!(store.get_account(&account.id).expect("read").is_none());
    }

    /// A keychain that records what was written, so "nothing was saved" is
    /// checked rather than assumed.
    #[derive(Default)]
    struct RecordingSecrets(Mutex<Vec<(String, String)>>);

    impl RecordingSecrets {
        /// The references the connection wrote to.
        fn writes(&self) -> Vec<String> {
            self.0
                .lock()
                .expect("lock")
                .iter()
                .map(|(reference, _)| reference.clone())
                .collect()
        }

        /// The last value written, so the entry itself can be read back.
        fn last_value(&self) -> Option<String> {
            self.0
                .lock()
                .expect("lock")
                .last()
                .map(|(_, value)| value.clone())
        }
    }

    impl SecretPort for RecordingSecrets {
        fn put(&self, reference: &str, value: &str) -> Result<(), SecretError> {
            self.0
                .lock()
                .expect("lock")
                .push((reference.to_string(), value.to_string()));
            Ok(())
        }

        fn get(&self, _reference: &str) -> Result<Option<String>, SecretError> {
            Ok(None)
        }

        fn delete(&self, _reference: &str) -> Result<bool, SecretError> {
            Ok(false)
        }
    }

    fn imap_smtp_form(port: u16) -> ConnectImapSmtpInput {
        ConnectImapSmtpInput {
            address: "me@example.com".to_string(),
            username: "me@example.com".to_string(),
            credential: "app-password".to_string(),
            credential_kind: MailCredentialKind::Password,
            imap_host: "127.0.0.1".to_string(),
            imap_port: port,
            imap_security: MailSecurity::Tls,
            smtp_host: "127.0.0.1".to_string(),
            smtp_port: port,
            smtp_security: MailSecurity::Tls,
        }
    }

    /// A connection test whose answer the test chooses.
    struct StubProbe(Option<ProbeOutcome>);

    impl MailProbe for StubProbe {
        fn probe<'a>(
            &'a self,
            _imap: &'a ImapSettings,
            _smtp: &'a SmtpSettings,
            _credential: &'a Auth,
        ) -> Pin<Box<dyn Future<Output = Result<(), ProbeOutcome>> + Send + 'a>> {
            let answer = self.0;
            Box::pin(async move { answer.map_or(Ok(()), Err) })
        }
    }

    /// AC2: the connection test runs first, so a mailbox that does not answer
    /// leaves no connection and no keychain entry behind.
    #[tokio::test]
    async fn a_failed_probe_persists_nothing() {
        let store = open_store("probe");
        let secrets = RecordingSecrets::default();

        // Port 1 is closed: the IMAP probe cannot even connect.
        let failure =
            connect_imap_smtp_with(&LiveProbe, store.as_ref(), &secrets, imap_smtp_form(1))
                .await
                .expect_err("the probe must fail");

        let error = AppError::from(failure);
        assert_eq!(error.code, ErrorCode::Unavailable);
        assert_eq!(error.message_key, "errors.email.probe_unreachable");
        assert!(
            store.list_accounts().expect("read").is_empty(),
            "a failed test must not save the connection"
        );
        assert!(
            secrets.writes().is_empty(),
            "a failed test must not write a keychain entry"
        );
    }

    /// A refused credential is the other way the test fails, and it too writes
    /// nothing (AC2).
    #[tokio::test]
    async fn a_refused_credential_is_reported_and_saves_nothing() {
        let store = open_store("refused");
        let secrets = RecordingSecrets::default();

        let failure = connect_imap_smtp_with(
            &StubProbe(Some(ProbeOutcome::Credentials)),
            store.as_ref(),
            &secrets,
            imap_smtp_form(993),
        )
        .await
        .expect_err("a refused credential must fail");

        let error = AppError::from(failure);
        assert_eq!(error.code, ErrorCode::Unauthorized);
        assert_eq!(error.message_key, "errors.email.probe_credentials");
        assert!(store.list_accounts().expect("read").is_empty());
        assert!(secrets.writes().is_empty());
    }

    /// AC2: when both tests pass, the connection is saved — the endpoints and the
    /// credential in the keychain, the account in the store (never the reverse).
    #[tokio::test]
    async fn a_passing_probe_saves_the_account_and_its_keychain_entry() {
        let store = open_store("saved");
        let secrets = RecordingSecrets::default();

        let mut form = imap_smtp_form(993);
        form.imap_host = "imap.example.com".to_string();
        form.smtp_host = "smtp.example.com".to_string();
        form.smtp_port = 587;
        form.smtp_security = MailSecurity::StartTls;
        form.credential_kind = MailCredentialKind::XOAuth2;
        form.credential = "the-token".to_string();

        let account = connect_imap_smtp_with(&StubProbe(None), store.as_ref(), &secrets, form)
            .await
            .expect("both tests passed");

        assert_eq!(account.provider, MailProvider::ImapSmtp);
        assert_eq!(account.address, "me@example.com");
        assert_eq!(account.mode, EmailMode::ReadSend);
        assert!(
            account.scopes.is_empty(),
            "an IMAP/SMTP account holds no OAuth grant"
        );

        let stored = store
            .get_account(&account.id)
            .expect("read")
            .expect("saved");
        assert_eq!(stored.id, account.id);

        // The keychain entry is what the sender and the sync owner will read:
        // both endpoints, and the credential that was probed.
        assert_eq!(secrets.writes(), vec![account.secret_ref.clone()]);
        let entry = ImapSmtpSecret::from_json(&secrets.last_value().expect("value")).expect("json");
        assert_eq!(entry.imap.host, "imap.example.com");
        assert_eq!(entry.imap.port, 993);
        assert_eq!(entry.smtp.host, "smtp.example.com");
        assert_eq!(entry.smtp.port, 587);
        assert_eq!(Auth::XOAuth2("the-token".to_string()), entry.credential);
    }

    /// The IMAP/SMTP form maps onto the adapter's settings, and refuses an
    /// incomplete one before any network call.
    #[test]
    fn an_incomplete_imap_smtp_form_is_refused_as_settings() {
        let mut form = imap_smtp_form(993);
        form.imap_host = "  ".to_string();
        assert!(matches!(
            form.parts(),
            Err(EmailFailure::Probe(ProbeOutcome::Settings))
        ));

        let mut form = imap_smtp_form(993);
        form.smtp_port = 0;
        assert!(matches!(
            form.parts(),
            Err(EmailFailure::Probe(ProbeOutcome::Settings))
        ));

        let mut form = imap_smtp_form(993);
        form.credential = "   ".to_string();
        assert!(matches!(
            form.parts(),
            Err(EmailFailure::Probe(ProbeOutcome::Settings))
        ));

        let mut form = imap_smtp_form(993);
        form.credential_kind = MailCredentialKind::XOAuth2;
        form.credential = "the-token".to_string();
        let (imap, smtp, credential) = form.parts().expect("complete");
        assert_eq!(imap.port, 993);
        assert_eq!(smtp.port, 993);
        assert_eq!(credential, Auth::XOAuth2("the-token".to_string()));
    }

    /// F2: a BYO client id without its public secret cannot complete the flow, so
    /// it is refused before the browser opens.
    #[test]
    fn a_client_id_without_its_secret_is_refused() {
        assert!(matches!(
            gmail_client(
                Some(" client-id "),
                None,
                Some("bundled"),
                Some("bundled-secret")
            ),
            Err(EmailFailure::ClientSecret)
        ));
        assert!(matches!(
            gmail_client(Some("client-id"), Some("  "), None, None),
            Err(EmailFailure::ClientSecret)
        ));
        // With its secret, the user's own client wins over the bundled one.
        assert!(matches!(
            gmail_client(
                Some(" client-id "),
                Some(" public-secret "),
                Some("bundled"),
                Some("bundled-secret")
            ),
            Ok((id, ClientIdKind::Byo, Some(secret)))
                if id == "client-id" && secret == "public-secret"
        ));
    }

    /// F3: the bundled path exists only when the build set both values.
    #[test]
    fn the_bundled_client_needs_its_id_and_its_secret() {
        assert!(matches!(
            gmail_client(None, None, Some("bundled"), Some("bundled-secret")),
            Ok((id, ClientIdKind::Bundled, Some(secret)))
                if id == "bundled" && secret == "bundled-secret"
        ));
        assert!(matches!(
            gmail_client(None, None, Some("bundled"), None),
            Err(EmailFailure::NoBundledClient)
        ));
        assert!(matches!(
            gmail_client(None, None, None, Some("bundled-secret")),
            Err(EmailFailure::NoBundledClient)
        ));
        assert!(matches!(
            gmail_client(None, None, None, None),
            Err(EmailFailure::NoBundledClient)
        ));
    }

    /// An unknown id is a typed NotFound, never a panic (AD-3).
    #[tokio::test]
    async fn disconnecting_an_unknown_connection_is_not_found() {
        let store = open_store("unknown");
        let secrets = StubSecrets {
            stored: None,
            delete: Ok(false),
        };

        let failure = disconnect(store.as_ref(), &secrets, "ghost")
            .await
            .expect_err("unknown");
        let error = AppError::from(failure);
        assert_eq!(error.code, ErrorCode::NotFound);
        assert_eq!(error.message_key, "errors.email.unknown_connection");
    }

    // --- Scoped sync (S8.3, FR-38) ---

    use boba_app::ports::repo::system::{NewIntent, SettingRecord, SystemRepoError};
    use boba_domain::audit::{AuditOutcome, AuditRow, ChainStatus};

    #[derive(Default)]
    struct SyncSink(Mutex<Vec<RawMessage>>);

    impl MessageSink for SyncSink {
        fn store(
            &self,
            message: boba_mail::sync::StoredMessage<'_>,
        ) -> Result<(), boba_mail::sync::SyncError> {
            self.0.lock().unwrap().push(message.message.clone());
            Ok(())
        }
    }

    impl SyncSink {
        fn messages(&self) -> Vec<RawMessage> {
            self.0.lock().unwrap().clone()
        }
    }

    /// The settings half of `SystemRepo`, which is all a sync pass touches.
    #[derive(Default)]
    struct MemorySettings(std::sync::Mutex<std::collections::BTreeMap<String, SettingRecord>>);

    impl SystemRepo for MemorySettings {
        fn setting_get(&self, key: &str) -> Result<Option<String>, SystemRepoError> {
            Ok(self.0.lock().unwrap().get(key).map(|row| row.value.clone()))
        }

        fn setting_put(
            &self,
            key: &str,
            value: &str,
            updated_at: &str,
        ) -> Result<(), SystemRepoError> {
            self.0.lock().unwrap().insert(
                key.to_string(),
                SettingRecord {
                    key: key.to_string(),
                    value: value.to_string(),
                    updated_at: updated_at.to_string(),
                },
            );
            Ok(())
        }

        fn setting_list(&self) -> Result<Vec<SettingRecord>, SystemRepoError> {
            Ok(self.0.lock().unwrap().values().cloned().collect())
        }

        fn setting_delete(&self, key: &str) -> Result<bool, SystemRepoError> {
            Ok(self.0.lock().unwrap().remove(key).is_some())
        }

        fn audit_append_intent(&self, _: &NewIntent) -> Result<AuditRow, SystemRepoError> {
            unreachable!("the mail sync does not write the audit log")
        }

        fn audit_append_outcome(
            &self,
            _: &AuditRow,
            _: AuditOutcome,
            _: &str,
            _: &str,
        ) -> Result<AuditRow, SystemRepoError> {
            unreachable!("the mail sync does not write the audit log")
        }

        fn audit_head_hash(&self) -> Result<Option<String>, SystemRepoError> {
            unreachable!("the mail sync does not read the audit log")
        }

        fn audit_verify_chain(&self) -> Result<ChainStatus, SystemRepoError> {
            unreachable!("the mail sync does not read the audit log")
        }

        fn audit_unresolved_intents(&self) -> Result<Vec<AuditRow>, SystemRepoError> {
            unreachable!("the mail sync does not read the audit log")
        }

        fn audit_anchor_get(&self) -> Result<Option<String>, SystemRepoError> {
            unreachable!("the mail sync does not read the audit log")
        }

        fn audit_anchor_is_known(&self, _: &str) -> Result<bool, SystemRepoError> {
            unreachable!("the mail sync does not read the audit log")
        }

        fn audit_anchor_set(&self, _: &str) -> Result<(), SystemRepoError> {
            unreachable!("the mail sync does not write the audit log")
        }
    }

    const NOW: i64 = 1_760_000_000;
    const DAY: i64 = 24 * 60 * 60;

    fn incoming(id: &str, from: &str, received_at: i64) -> RawMessage {
        RawMessage {
            provider_id: id.to_string(),
            from_address: from.to_string(),
            subject: "hello".to_string(),
            body_text: "body".to_string(),
            received_at,
            ..RawMessage::default()
        }
    }

    fn sync_scope_for() -> MessageScope {
        MessageScope::new(
            ["acme.example".to_string()],
            Vec::<String>::new(),
            ["sent-1@boba.local".to_string()],
        )
    }

    #[test]
    fn a_pass_goes_through_run_and_the_sink_and_stores_the_cursor() {
        let sink = SyncSink::default();
        let system = MemorySettings::default();
        let stored = sync_pass(
            "acc-1",
            vec![
                incoming("m1", "ana@acme.example", NOW - DAY),
                incoming("m2", "friend@personal.example", NOW - DAY),
            ],
            &sync_scope_for(),
            NOW,
            None,
            &sink,
            &system,
        )
        .expect("pass");

        assert_eq!(stored, 1);
        assert_eq!(sink.messages().len(), 1);
        assert_eq!(sink.messages()[0].provider_id, "m1");
        assert_eq!(
            read_cursor(&system, "acc-1").expect("cursor"),
            Some(NOW - DAY)
        );
    }

    #[test]
    fn a_second_pass_skips_mail_older_than_the_stored_cursor() {
        let sink = SyncSink::default();
        let system = MemorySettings::default();
        let scope = sync_scope_for();
        let first = sync_pass(
            "acc-1",
            vec![incoming("m1", "ana@acme.example", NOW - 10 * DAY)],
            &scope,
            NOW,
            None,
            &sink,
            &system,
        )
        .expect("first");
        assert_eq!(first, 1);

        let cursor = read_cursor(&system, "acc-1").expect("cursor");
        let second = sync_pass(
            "acc-1",
            vec![
                incoming("m0", "ana@acme.example", NOW - 20 * DAY),
                // A message in the cursor's own second is genuinely ambiguous
                // (two mails can share a timestamp), so it is re-offered; the
                // sink's unique key is what makes it one row.
                incoming("m1", "ana@acme.example", NOW - 10 * DAY),
                incoming("m2", "talent@acme.example", NOW - DAY),
            ],
            &scope,
            NOW,
            cursor,
            &sink,
            &system,
        )
        .expect("second");
        assert_eq!(
            second, 2,
            "the older message is skipped, the other two are offered"
        );
        let stored_ids: Vec<String> = sink
            .messages()
            .into_iter()
            .map(|message| message.provider_id)
            .collect();
        assert!(!stored_ids.contains(&"m0".to_string()), "{stored_ids:?}");
        assert_eq!(
            read_cursor(&system, "acc-1").expect("cursor"),
            Some(NOW - DAY)
        );
    }

    #[test]
    fn a_corrupt_cursor_is_an_error_not_a_silent_rescan() {
        let system = MemorySettings::default();
        system
            .setting_put(&cursor_key("acc-1"), "not-a-timestamp", "t")
            .expect("seed");
        assert!(read_cursor(&system, "acc-1").is_err());
    }

    #[test]
    fn company_domains_come_from_the_addresses_applications_were_sent_to() {
        assert_eq!(
            email_domain("Talent@ACME.example."),
            Some("acme.example".to_string())
        );
        assert_eq!(email_domain("not-an-address"), None);
        assert_eq!(
            company_domain("talent@acme.example"),
            Some("acme.example".to_string())
        );
    }

    #[test]
    fn a_multi_label_company_domain_is_kept_whole() {
        // No parent step: `careers.acme.co.uk` must never become `co.uk`, and
        // `acme.com.mx` must never become `com.mx`.
        assert_eq!(
            company_domain("talent@acme.co.uk"),
            Some("acme.co.uk".to_string())
        );
        assert_eq!(
            company_domain("talent@acme.com.mx"),
            Some("acme.com.mx".to_string())
        );
        assert_eq!(
            url_host("https://careers.acme.co.uk/jobs/1"),
            Some("careers.acme.co.uk".to_string())
        );
        assert!(!is_excluded_host("careers.acme.co.uk"));
        assert!(!is_excluded_host("acme.com.mx"));
    }

    #[test]
    fn free_mailboxes_and_job_boards_are_not_company_domains() {
        for address in [
            "someone@gmail.com",
            "reply@mail.gmail.com",
            "jobs@linkedin.com",
            "careers@indeed.com",
            "talent@outlook.com",
        ] {
            assert_eq!(company_domain(address), None, "{address}");
        }
        assert!(is_excluded_host("linkedin.com"));
        assert!(!is_excluded_host("notlinkedin.com"));
    }

    #[test]
    fn an_offer_on_a_job_board_host_adds_no_company_domain() {
        let board = url_host("https://www.linkedin.com/jobs/view/1").expect("host");
        assert!(is_excluded_host(&board));
        let company = url_host("https://careers.acme.example/jobs/1").expect("host");
        assert!(!is_excluded_host(&company));
    }
}
