//! IMAP/SMTP adapter — the submission side (S8.2, FR-36, architecture §11).
//!
//! Two things live here:
//!
//! * [`test_connection`] — the connection test AC2 requires **before the account is
//!   saved**. It authenticates for real (app password or `XOAUTH2`) and then quits;
//!   nothing is sent. `lettre`'s own `test_connection` is only a `NOOP`, which would
//!   accept a wrong password, so the credential check is spelled out here.
//! * [`SmtpMail`] — the `MailPort` for an IMAP/SMTP account: it sends through the
//!   provider's submission endpoint with `lettre`, which builds the MIME message
//!   (encoded words for the ES/EN subject, attachments) — the same message builder
//!   the Gmail adapter hands to the Gmail API.
//!
//! Sending is an outbound action, so the core drives it through the port (AD-11);
//! an agent never reaches this module.

use std::sync::Arc;

use base64::Engine as _;
use boba_app::ports::mail::{MailError, MailFuture, MailPort, OutgoingMessage};
use boba_app::ports::repo::email::{EmailAccount, EmailRepo, MailProvider};
use boba_app::ports::secret::SecretPort;
use lettre::Transport as _;
use lettre::message::header::ContentType;
use lettre::message::{Attachment, Message, MultiPart, SinglePart};
use lettre::transport::smtp::SmtpTransport;
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use crate::imap::{Auth, ImapSettings, Security};

/// The SMTP side of an IMAP/SMTP connection (FR-36).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmtpSettings {
    /// Submission host, e.g. `smtp.fastmail.com`.
    pub host: String,
    /// Submission port: 465 for implicit TLS, 587 for `STARTTLS`.
    pub port: u16,
    /// Implicit TLS or `STARTTLS`.
    pub security: Security,
    /// The login for the submission endpoint, usually the address itself.
    pub username: String,
}

impl SmtpSettings {
    /// Reject settings the probe cannot even attempt, so the UI shows "complete
    /// the form" instead of a connection timeout.
    pub fn validate(&self) -> Result<(), SmtpError> {
        let ok = !self.host.trim().is_empty() && self.port != 0 && !self.username.trim().is_empty();
        if ok { Ok(()) } else { Err(SmtpError::Settings) }
    }
}

/// What the keychain holds for one IMAP/SMTP connection (AD-13, FR-40).
///
/// One entry per connection, addressed by the account's opaque `secret_ref`: both
/// endpoints and the one credential that authenticates them. Nothing here ever
/// reaches the database, a log or an export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImapSmtpSecret {
    /// How to reach the mailbox read-only (S8.3 syncs through it).
    pub imap: ImapSettings,
    /// How to send.
    pub smtp: SmtpSettings,
    /// The app password or the OAuth access token, shared by both protocols.
    pub credential: Auth,
}

impl ImapSmtpSecret {
    /// The keychain representation.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Parse the keychain representation. An entry that cannot be read is the same
    /// as no entry: the user reconnects rather than sending half a credential.
    pub fn from_json(json: &str) -> Result<Self, SmtpError> {
        serde_json::from_str(json).map_err(|_| SmtpError::Settings)
    }
}

/// Failures of the submission endpoint. No variant repeats server text: the
/// server has seen the credential and could echo it back.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SmtpError {
    /// The server could not be reached.
    #[error("the mail server could not be reached")]
    Connect,
    /// The TLS handshake failed, or `STARTTLS` was refused.
    #[error("the connection could not be secured")]
    Tls,
    /// The server rejected the credential.
    #[error("the mail server refused the credentials")]
    Credentials,
    /// The server refused a command the probe needs, or the message.
    #[error("the mail server refused the request")]
    Refused,
    /// The server answered something this client does not understand, or the
    /// invitation to `STARTTLS` was missing.
    #[error("the mail server answered unexpectedly")]
    Protocol,
    /// Host, port or username is missing, or the account cannot be read.
    #[error("the mail account settings are incomplete")]
    Settings,
}

/// Test the submission endpoint before the account is saved (AC2).
///
/// Connects, secures the socket (`STARTTLS` where the account asks for it),
/// authenticates, and quits — a wrong app password or token fails here, not at the
/// first application.
pub async fn test_connection(settings: &SmtpSettings, auth: &Auth) -> Result<(), SmtpError> {
    settings.validate()?;
    let (stream, greeting_read) = connect(settings).await?;
    probe(stream, settings, auth, greeting_read).await
}

/// The secured socket, and whether the 220 greeting has already been read
/// (`STARTTLS` upgrades mid-session, so there is none after the upgrade).
async fn connect(settings: &SmtpSettings) -> Result<(SecuredStream, bool), SmtpError> {
    // The network-activity view (S10.4, NFR-1): host and time only.
    boba_app::network_log::record_host(boba_app::network_log::Purpose::Email, settings.host.trim());
    let tcp = TcpStream::connect((settings.host.as_str(), settings.port))
        .await
        .map_err(|_| SmtpError::Connect)?;
    let connector = tokio_native_tls::TlsConnector::from(
        native_tls::TlsConnector::new().map_err(|_| SmtpError::Tls)?,
    );

    match settings.security {
        Security::Tls => {
            let stream = connector
                .connect(&settings.host, tcp)
                .await
                .map_err(|_| SmtpError::Tls)?;
            Ok((stream, false))
        }
        Security::StartTls => {
            // The upgrade happens before any credential is sent. A server that
            // does not invite STARTTLS is abandoned instead of downgraded.
            let (read, mut write) = tokio::io::split(tcp);
            let mut reader = BufReader::new(read);
            let (code, _) = read_reply(&mut reader).await?;
            if code != 220 {
                return Err(SmtpError::Refused);
            }
            let capabilities = greet(&mut reader, &mut write, settings).await?;
            if !capabilities.to_uppercase().contains("STARTTLS") {
                return Err(SmtpError::Protocol);
            }
            send(&mut write, "STARTTLS").await?;
            let (code, _) = read_reply(&mut reader).await?;
            if code != 220 {
                return Err(SmtpError::Tls);
            }
            let stream = connector
                .connect(&settings.host, reader.into_inner().unsplit(write))
                .await
                .map_err(|_| SmtpError::Tls)?;
            Ok((stream, true))
        }
    }
}

/// The socket this module works on, whichever way it was secured.
type SecuredStream = tokio_native_tls::TlsStream<TcpStream>;

/// The SMTP conversation: greeting, `EHLO`, authenticate, `QUIT`. Nothing is
/// sent — a connection test never has an envelope.
///
/// Generic over the socket so the protocol is exercised against a real server in
/// the tests; [`connect`] is the only part that needs a certificate.
async fn probe<S>(
    stream: S,
    settings: &SmtpSettings,
    auth: &Auth,
    greeting_read: bool,
) -> Result<(), SmtpError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let (read, mut write) = tokio::io::split(stream);
    let mut reader = BufReader::new(read);
    if !greeting_read {
        let (code, _) = read_reply(&mut reader).await?;
        if code != 220 {
            return Err(SmtpError::Refused);
        }
    }

    let capabilities = greet(&mut reader, &mut write, settings).await?;
    authenticate(&mut reader, &mut write, settings, auth, &capabilities).await?;

    // The probe has its answer; a server that slams the door on QUIT does not
    // change it.
    let _ = send(&mut write, "QUIT").await;
    let _ = read_reply(&mut reader).await;
    Ok(())
}

/// `EHLO` with the account's own domain, and the capabilities it answers with.
async fn greet<R, W>(
    reader: &mut R,
    write: &mut W,
    settings: &SmtpSettings,
) -> Result<String, SmtpError>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    send(write, &format!("EHLO {}", hello_name(settings))).await?;
    let (code, text) = read_reply(reader).await?;
    if code != 250 {
        return Err(SmtpError::Refused);
    }
    Ok(text)
}

/// The name the probe greets with: the account's own domain, so a provider that
/// filters `EHLO` names sees something plausible.
fn hello_name(settings: &SmtpSettings) -> String {
    match settings.username.rsplit_once('@') {
        Some((_, domain)) if !domain.is_empty() => domain.to_string(),
        _ => "localhost".to_string(),
    }
}

/// Offer the account's credential with the mechanism its server advertises, and
/// fail on a rejection: this is what makes the test worth running (AC2).
async fn authenticate<R, W>(
    reader: &mut R,
    write: &mut W,
    settings: &SmtpSettings,
    auth: &Auth,
    capabilities: &str,
) -> Result<(), SmtpError>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let advertised = capabilities.to_uppercase();
    match auth {
        Auth::XOAuth2(_) if advertised.contains("XOAUTH2") => {
            let response = sasl(&xoauth2(&settings.username, auth.secret()));
            send(write, &format!("AUTH XOAUTH2 {response}")).await?;
            accepted(read_reply(reader).await?)
        }
        Auth::Password(_) if advertised.contains("PLAIN") => {
            let response = sasl(&format!("\0{}\0{}", settings.username, auth.secret()));
            send(write, &format!("AUTH PLAIN {response}")).await?;
            accepted(read_reply(reader).await?)
        }
        Auth::Password(_) if advertised.contains("LOGIN") => {
            send(write, "AUTH LOGIN").await?;
            expect(reader, 334).await?;
            send(write, &sasl(&settings.username)).await?;
            expect(reader, 334).await?;
            send(write, &sasl(auth.secret())).await?;
            accepted(read_reply(reader).await?)
        }
        // The server offers no mechanism that carries this credential; sending
        // it anyway would be guesswork.
        _ => Err(SmtpError::Protocol),
    }
}

/// The RFC 1734 initial response for `XOAUTH2` (Google's profile).
fn xoauth2(username: &str, token: &str) -> String {
    format!("user={username}\x01auth=Bearer {token}\x01\x01")
}

fn sasl(response: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(response)
}

/// `235` is an accepted credential; another 5xx is a refusal.
fn accepted(reply: (u16, String)) -> Result<(), SmtpError> {
    match reply.0 {
        235 => Ok(()),
        code if (500..600).contains(&code) => Err(SmtpError::Credentials),
        _ => Err(SmtpError::Protocol),
    }
}

async fn expect<R: AsyncBufRead + Unpin>(reader: &mut R, code: u16) -> Result<(), SmtpError> {
    if read_reply(reader).await?.0 == code {
        Ok(())
    } else {
        Err(SmtpError::Protocol)
    }
}

/// One command line, CRLF-terminated.
async fn send<W: AsyncWrite + Unpin>(write: &mut W, command: &str) -> Result<(), SmtpError> {
    let line = format!("{command}\r\n");
    write
        .write_all(line.as_bytes())
        .await
        .map_err(|_| SmtpError::Connect)
}

/// One reply: its status code and the text, with `250-…` continuations joined.
async fn read_reply<R: AsyncBufRead + Unpin>(reader: &mut R) -> Result<(u16, String), SmtpError> {
    let mut text = String::new();
    loop {
        let mut line = String::new();
        if reader
            .read_line(&mut line)
            .await
            .map_err(|_| SmtpError::Connect)?
            == 0
        {
            return Err(SmtpError::Connect);
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.len() < 3 {
            return Err(SmtpError::Protocol);
        }
        let (digits, rest) = line.split_at(3);
        let code: u16 = digits.parse().map_err(|_| SmtpError::Protocol)?;
        match rest.strip_prefix('-') {
            // "250-STARTTLS" continues; the last line has a space (or nothing).
            Some(continuation) => {
                text.push_str(continuation);
                text.push(' ');
            }
            None => {
                text.push_str(rest.trim_start());
                return Ok((code, text));
            }
        }
    }
}

/// The submission endpoint for one account: implicit TLS on 465, `STARTTLS` on
/// 587, with the credential the connection stores.
fn transport_for(settings: &SmtpSettings, credential: &Auth) -> Result<SmtpTransport, SmtpError> {
    settings.validate()?;
    let builder = match settings.security {
        Security::Tls => SmtpTransport::relay(&settings.host),
        Security::StartTls => SmtpTransport::starttls_relay(&settings.host),
    }
    .map_err(|_| SmtpError::Settings)?;
    let mut builder = builder.port(settings.port).credentials(Credentials::new(
        settings.username.clone(),
        credential.secret().to_string(),
    ));
    if matches!(credential, Auth::XOAuth2(_)) {
        // A token is only ever offered as XOAUTH2; lettre would otherwise try
        // PLAIN with the token as a password.
        builder = builder.authentication(vec![Mechanism::Xoauth2]);
    }
    Ok(builder.build())
}

/// The RFC 5322 message for one outgoing message: the tailored body plus the
/// attachments, encoded by `lettre` (encoded words for the subject, base64 for
/// the files).
fn message(from: &str, outgoing: &OutgoingMessage) -> Result<Message, SmtpError> {
    let mut builder = Message::builder()
        .from(from.parse().map_err(|_| SmtpError::Settings)?)
        .subject(outgoing.subject.clone());
    for address in &outgoing.to {
        builder = builder.to(address.parse().map_err(|_| SmtpError::Settings)?);
    }

    let body = SinglePart::plain(outgoing.body_text.clone());
    let built = if outgoing.attachments.is_empty() {
        builder.singlepart(body)
    } else {
        let mut parts = MultiPart::mixed().singlepart(body);
        for attachment in &outgoing.attachments {
            let content_type =
                ContentType::parse(&attachment.mime).unwrap_or(ContentType::TEXT_PLAIN);
            parts = parts.singlepart(
                Attachment::new(attachment.filename.clone())
                    .body(attachment.bytes.clone(), content_type),
            );
        }
        builder.multipart(parts)
    };
    built.map_err(|_| SmtpError::Settings)
}

/// The RFC 5322 bytes of one outgoing message: what SMTP puts on the wire, and
/// what the Gmail API takes as `raw` (S8.2). One builder, so both sends carry the
/// same message.
pub(crate) fn rfc5322(from: &str, outgoing: &OutgoingMessage) -> Result<Vec<u8>, SmtpError> {
    message(from, outgoing).map(|email| email.formatted())
}

/// Send one already-built message through `transport`.
fn deliver(
    transport: &SmtpTransport,
    from: &str,
    outgoing: &OutgoingMessage,
) -> Result<(), SmtpError> {
    let email = message(from, outgoing)?;
    transport.send(&email).map(|_| ()).map_err(|error| {
        // lettre repeats the server's text; that text comes from a server that
        // has just seen the message, so it stops here.
        if error.is_tls() {
            SmtpError::Tls
        } else if error.is_permanent() {
            SmtpError::Refused
        } else {
            SmtpError::Connect
        }
    })
}

/// How a submission failure reaches the UI (AD-3): "authorise again" for a
/// credential the provider no longer accepts, otherwise a transport or a refusal.
fn mail_error(error: SmtpError) -> MailError {
    match error {
        SmtpError::Settings | SmtpError::Credentials => MailError::Reauthorize,
        SmtpError::Refused => {
            MailError::Rejected("the mail server refused the message".to_string())
        }
        SmtpError::Connect | SmtpError::Tls | SmtpError::Protocol => {
            MailError::Transport("the mail provider could not be reached".to_string())
        }
    }
}

/// The adapter that sends for an IMAP/SMTP connection.
pub struct SmtpMail {
    secrets: Arc<dyn SecretPort>,
    accounts: Arc<dyn EmailRepo>,
}

impl SmtpMail {
    /// The adapter as the app wires it.
    pub fn new(secrets: Arc<dyn SecretPort>, accounts: Arc<dyn EmailRepo>) -> Self {
        Self { secrets, accounts }
    }

    /// The account and the keychain blob its send needs, ready to talk to it.
    /// Anything unreadable is "authorise again", never a half credential.
    fn connection(&self, account_id: &str) -> Result<(EmailAccount, ImapSmtpSecret), MailError> {
        let account = self
            .accounts
            .get_account(account_id)
            .map_err(|_| MailError::UnknownAccount)?
            .ok_or(MailError::UnknownAccount)?;
        if account.provider != MailProvider::ImapSmtp {
            return Err(MailError::UnknownAccount);
        }
        let stored = self
            .secrets
            .get(&account.secret_ref)
            .map_err(|_| MailError::Reauthorize)?
            .ok_or(MailError::Reauthorize)?;
        let secret = ImapSmtpSecret::from_json(&stored).map_err(mail_error)?;
        Ok((account, secret))
    }
}

impl MailPort for SmtpMail {
    fn send<'a>(
        &'a self,
        account_id: &'a str,
        message: &'a OutgoingMessage,
    ) -> MailFuture<'a, Result<(), MailError>> {
        Box::pin(async move {
            let (account, secret) = self.connection(account_id)?;
            let transport = transport_for(&secret.smtp, &secret.credential).map_err(mail_error)?;
            deliver(&transport, &account.address, message).map_err(mail_error)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::sync::Mutex;

    use boba_app::ports::mail::Attachment;
    use boba_app::ports::repo::email::{ClientIdKind, EmailMode, EmailRepoError};
    use boba_app::ports::secret::SecretError;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::{TcpListener, TcpStream};

    use super::*;

    const PASSWORD: &str = "app-password";
    const TOKEN: &str = "the-token";
    const PDF: &[u8] = b"%PDF-1.4 \x00\x01binary\xff";

    /// A one-shot SMTP server: greeting, then `reply(line)` for every command line.
    /// `DATA` is read to the terminating dot by the helper. Every line is recorded.
    async fn fake_smtp(
        reply: impl Fn(&str) -> String + Send + Sync + 'static,
    ) -> (SocketAddr, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let recorded = seen.clone();
        tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("accept");
            let (read, mut write) = socket.into_split();
            write
                .write_all(b"220 boba.test ESMTP\r\n")
                .await
                .expect("greet");
            let mut lines = BufReader::new(read).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                recorded.lock().expect("lock").push(line.clone());
                if line == "DATA" {
                    if write.write_all(b"354 end with .\r\n").await.is_err() {
                        break;
                    }
                    while let Ok(Some(body_line)) = lines.next_line().await {
                        let end = body_line == ".";
                        recorded.lock().expect("lock").push(body_line);
                        if end {
                            break;
                        }
                    }
                    if write.write_all(b"250 2.0.0 queued\r\n").await.is_err() {
                        break;
                    }
                    continue;
                }
                let answer = reply(&line);
                if answer.is_empty() || write.write_all(answer.as_bytes()).await.is_err() {
                    break;
                }
            }
        });
        (addr, seen)
    }

    fn settings(addr: SocketAddr) -> SmtpSettings {
        SmtpSettings {
            host: addr.ip().to_string(),
            port: addr.port(),
            security: Security::Tls,
            username: "me@example.com".to_string(),
        }
    }

    fn outgoing() -> OutgoingMessage {
        OutgoingMessage {
            to: vec!["recruiter@example.com".to_string()],
            subject: "Application: Backend Engineer".to_string(),
            body_text: "Hello,\n\nPlease find my CV attached.".to_string(),
            attachments: vec![Attachment {
                filename: "cv.pdf".to_string(),
                mime: "application/pdf".to_string(),
                bytes: PDF.to_vec(),
            }],
        }
    }

    /// The probe talks to a real socket: the test drives the plain-TCP
    /// conversation ([`probe`]) — the TLS wrap around it is the only part that
    /// needs a certificate.
    #[tokio::test]
    async fn an_app_password_authenticates_and_quits() {
        let (addr, seen) = fake_smtp(|line| {
            if line.starts_with("EHLO") {
                "250-boba.test\r\n250-AUTH PLAIN LOGIN\r\n250 OK\r\n".to_string()
            } else if line.starts_with("AUTH PLAIN") {
                "235 2.7.0 accepted\r\n".to_string()
            } else if line.starts_with("QUIT") {
                "221 bye\r\n".to_string()
            } else {
                "250 ok\r\n".to_string()
            }
        })
        .await;

        let stream = TcpStream::connect(addr).await.expect("connect");
        probe(
            stream,
            &settings(addr),
            &Auth::Password(PASSWORD.to_string()),
            false,
        )
        .await
        .expect("probe");

        let commands = seen.lock().expect("lock").clone();
        let expected = base64::engine::general_purpose::STANDARD
            .encode(format!("\0{}\0{PASSWORD}", "me@example.com"));
        assert!(
            commands
                .iter()
                .any(|line| line == &format!("AUTH PLAIN {expected}")),
            "the app password is offered as SASL PLAIN: {commands:?}"
        );
        assert!(
            commands.iter().any(|line| line.starts_with("EHLO")),
            "the probe greets the server first: {commands:?}"
        );
        assert!(
            commands.iter().any(|line| line.starts_with("QUIT")),
            "the probe closes the connection: {commands:?}"
        );
        assert!(
            !commands.iter().any(|line| line.starts_with("MAIL FROM")),
            "a connection test never sends mail: {commands:?}"
        );
    }

    #[tokio::test]
    async fn xoauth2_offers_the_bearer_token() {
        let (addr, seen) = fake_smtp(|line| {
            if line.starts_with("EHLO") {
                "250-boba.test\r\n250-AUTH XOAUTH2 PLAIN\r\n250 OK\r\n".to_string()
            } else if line.starts_with("AUTH XOAUTH2") {
                "235 2.7.0 accepted\r\n".to_string()
            } else if line.starts_with("QUIT") {
                "221 bye\r\n".to_string()
            } else {
                "250 ok\r\n".to_string()
            }
        })
        .await;

        let stream = TcpStream::connect(addr).await.expect("connect");
        probe(
            stream,
            &settings(addr),
            &Auth::XOAuth2(TOKEN.to_string()),
            false,
        )
        .await
        .expect("probe");

        let commands = seen.lock().expect("lock").clone();
        let expected = base64::engine::general_purpose::STANDARD.encode(format!(
            "user=me@example.com\x01auth=Bearer {TOKEN}\x01\x01"
        ));
        assert!(
            commands
                .iter()
                .any(|line| line == &format!("AUTH XOAUTH2 {expected}")),
            "the token is offered as XOAUTH2: {commands:?}"
        );
    }

    #[tokio::test]
    async fn a_refused_password_is_credentials_and_says_nothing_more() {
        let (addr, _) = fake_smtp(|line| {
            if line.starts_with("EHLO") {
                "250-boba.test\r\n250 AUTH PLAIN LOGIN\r\n".to_string()
            } else {
                "535 5.7.8 username and password not accepted\r\n".to_string()
            }
        })
        .await;

        let stream = TcpStream::connect(addr).await.expect("connect");
        let error = probe(
            stream,
            &settings(addr),
            &Auth::Password("wrong-password".to_string()),
            false,
        )
        .await
        .expect_err("refused");
        assert_eq!(error, SmtpError::Credentials);
        assert!(!format!("{error}").contains("wrong-password"));
        assert!(!format!("{error}").contains("5.7.8"));
    }

    /// AC2's "test runs first" is only worth anything if it fails on a bad
    /// password; `NOOP` would pass here.
    #[tokio::test]
    async fn the_credential_is_really_checked() {
        let (addr, seen) = fake_smtp(|line| {
            if line.starts_with("EHLO") {
                "250-boba.test\r\n250 AUTH PLAIN\r\n".to_string()
            } else if line.starts_with("AUTH PLAIN") {
                "535 5.7.8 no\r\n".to_string()
            } else {
                "250 ok\r\n".to_string()
            }
        })
        .await;

        let stream = TcpStream::connect(addr).await.expect("connect");
        assert_eq!(
            probe(
                stream,
                &settings(addr),
                &Auth::Password("nope".into()),
                false
            )
            .await,
            Err(SmtpError::Credentials)
        );
        assert!(
            seen.lock()
                .expect("lock")
                .iter()
                .any(|line| line.starts_with("AUTH PLAIN")),
            "the test authenticates, it does not just say hello"
        );
    }

    /// The message goes out through the same transport the app builds, against a
    /// real submission-server conversation.
    #[tokio::test]
    async fn a_message_reaches_the_submission_server_with_its_attachment() {
        let (addr, seen) = fake_smtp(|line| {
            if line.starts_with("EHLO") {
                "250-boba.test\r\n250-8BITMIME\r\n250 AUTH PLAIN\r\n".to_string()
            } else if line.starts_with("MAIL FROM") {
                "250 2.1.0 ok\r\n".to_string()
            } else if line.starts_with("RCPT TO") {
                "250 2.1.5 ok\r\n".to_string()
            } else if line.starts_with("QUIT") {
                "221 bye\r\n".to_string()
            } else {
                "250 ok\r\n".to_string()
            }
        })
        .await;

        let transport = SmtpTransport::builder_dangerous(addr.ip().to_string())
            .port(addr.port())
            .timeout(Some(std::time::Duration::from_secs(5)))
            .build();
        // lettre's submission client is blocking, so it runs off the runtime:
        // the fake server task must keep answering while the send is in flight.
        let sent =
            tokio::task::spawn_blocking(move || deliver(&transport, "me@example.com", &outgoing()))
                .await
                .expect("join");
        sent.expect("sent");

        let lines = seen.lock().expect("lock").clone();
        assert!(
            lines
                .iter()
                .any(|line| line == "MAIL FROM:<me@example.com>"),
            "the envelope comes from the connection's address: {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "RCPT TO:<recruiter@example.com>"),
            "the recipient is the application address: {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "Subject: Application: Backend Engineer"),
            "the subject reaches the wire: {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "Content-Disposition: attachment; filename=\"cv.pdf\""),
            "the attachment is announced: {lines:?}"
        );
        assert!(
            lines.iter().any(|line| line.starts_with(
                base64::engine::general_purpose::STANDARD
                    .encode(PDF)
                    .as_str()
            )),
            "the attachment body is base64: {lines:?}"
        );
    }

    #[test]
    fn incomplete_settings_never_reach_the_network() {
        let mut incomplete = SmtpSettings {
            host: String::new(),
            port: 587,
            security: Security::StartTls,
            username: "me@example.com".to_string(),
        };
        assert_eq!(incomplete.validate(), Err(SmtpError::Settings));
        incomplete.host = "smtp.example.com".to_string();
        incomplete.port = 0;
        assert_eq!(incomplete.validate(), Err(SmtpError::Settings));
        incomplete.port = 587;
        incomplete.username = "  ".to_string();
        assert_eq!(incomplete.validate(), Err(SmtpError::Settings));
        incomplete.username = "me@example.com".to_string();
        assert_eq!(incomplete.validate(), Ok(()));
    }

    #[test]
    fn the_connection_blob_round_trips_and_never_prints_the_credential() {
        let secret = ImapSmtpSecret {
            imap: ImapSettings {
                host: "imap.example.com".to_string(),
                port: 993,
                security: Security::Tls,
                username: "me@example.com".to_string(),
            },
            smtp: SmtpSettings {
                host: "smtp.example.com".to_string(),
                port: 587,
                security: Security::StartTls,
                username: "me@example.com".to_string(),
            },
            credential: Auth::Password(PASSWORD.to_string()),
        };
        let stored = secret.to_json();
        assert!(!stored.is_empty());
        assert_eq!(ImapSmtpSecret::from_json(&stored).expect("read"), secret);
        assert!(!format!("{secret:?}").contains(PASSWORD));
        assert_eq!(
            ImapSmtpSecret::from_json("not json"),
            Err(SmtpError::Settings)
        );
    }

    // --- The MailPort -------------------------------------------------------

    struct StubSecrets(Option<String>);

    impl SecretPort for StubSecrets {
        fn put(&self, _reference: &str, _value: &str) -> Result<(), SecretError> {
            unimplemented!("the send path never writes a secret")
        }

        fn get(&self, _reference: &str) -> Result<Option<String>, SecretError> {
            Ok(self.0.clone())
        }

        fn delete(&self, _reference: &str) -> Result<bool, SecretError> {
            unimplemented!("the send path never deletes a secret")
        }
    }

    struct StubAccounts(Option<EmailAccount>);

    impl EmailRepo for StubAccounts {
        fn upsert_account(&self, _account: &EmailAccount) -> Result<(), EmailRepoError> {
            unimplemented!("the send path never writes an account")
        }

        fn get_account(&self, id: &str) -> Result<Option<EmailAccount>, EmailRepoError> {
            Ok(self.0.clone().filter(|account| account.id == id))
        }

        fn find_account(
            &self,
            _provider: MailProvider,
            _address: &str,
        ) -> Result<Option<EmailAccount>, EmailRepoError> {
            Ok(None)
        }

        fn list_accounts(&self) -> Result<Vec<EmailAccount>, EmailRepoError> {
            Ok(self.0.clone().into_iter().collect())
        }

        fn set_mode(
            &self,
            _id: &str,
            _mode: EmailMode,
            _updated_at: &str,
        ) -> Result<(), EmailRepoError> {
            unimplemented!("the send path never changes the mode")
        }

        fn delete_account(&self, _id: &str) -> Result<bool, EmailRepoError> {
            unimplemented!("the send path never deletes an account")
        }
    }

    fn smtp_account() -> EmailAccount {
        EmailAccount {
            id: "01SMTP".to_string(),
            provider: MailProvider::ImapSmtp,
            address: "me@example.com".to_string(),
            mode: EmailMode::ReadSend,
            client_id: String::new(),
            client_id_kind: ClientIdKind::Byo,
            scopes: Vec::new(),
            secret_ref: "mail:01SMTP".to_string(),
            connected_at: "2026-10-16T00:00:00Z".to_string(),
            updated_at: "2026-10-16T00:00:00Z".to_string(),
        }
    }

    fn mail(account: Option<EmailAccount>, stored: Option<&str>) -> SmtpMail {
        SmtpMail::new(
            Arc::new(StubSecrets(stored.map(str::to_string))),
            Arc::new(StubAccounts(account)),
        )
    }

    #[tokio::test]
    async fn a_connection_of_another_provider_is_not_sent_through() {
        let mut graph = smtp_account();
        graph.provider = MailProvider::MicrosoftGraph;
        let sender = mail(Some(graph), Some("{}"));
        assert_eq!(
            sender.send("01SMTP", &outgoing()).await,
            Err(MailError::UnknownAccount)
        );

        let sender = mail(None, Some("{}"));
        assert_eq!(
            sender.send("ghost", &outgoing()).await,
            Err(MailError::UnknownAccount)
        );
    }

    #[tokio::test]
    async fn an_unreadable_connection_blob_asks_to_reconnect() {
        for stored in [None, Some("{}"), Some("not json")] {
            let sender = mail(Some(smtp_account()), stored);
            assert_eq!(
                sender.send("01SMTP", &outgoing()).await,
                Err(MailError::Reauthorize),
                "stored: {stored:?}"
            );
        }
    }
}
