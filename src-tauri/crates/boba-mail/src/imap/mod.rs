//! IMAP connection probe (S8.2, FR-36, architecture §11).
//!
//! An IMAP/SMTP account is tested **before it is saved** (AC2): this module opens
//! the mailbox read-only (`EXAMINE`, never `SELECT` — no flag or per-user state
//! can change) and reports what it found. Reading and syncing mail is the sync
//! owner's job (S8.3); this slice proves the settings and the credential work and
//! that the connection writes nothing.
//!
//! The credential is the one the provider issued: an app password (`LOGIN`) or an
//! OAuth 2.0 access token (`XOAUTH2`). [`Auth`] and [`Security`] live here because
//! both protocols of one account start from the same credential and the same kind
//! of socket; `smtp` reuses them.

use std::fmt;

use tokio::net::TcpStream;

/// How the socket is protected. Both protocols start from plain TCP on their
/// provider port and are then wrapped or upgraded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Security {
    /// TLS from the first byte — IMAP on 993, SMTP on 465.
    Tls,
    /// Plain TCP, upgraded with `STARTTLS` before anything secret is sent —
    /// IMAP on 143, SMTP on 587.
    StartTls,
}

/// How one IMAP/SMTP account authenticates.
///
/// A password is what the provider issues for app-password access; an access
/// token is used as `XOAUTH2` where the provider supports it. Both are secrets
/// and never appear in a log, an error or a `Debug` print.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum Auth {
    /// The provider's app password.
    Password(String),
    /// An OAuth 2.0 access token (XOAUTH2).
    XOAuth2(String),
}

impl Auth {
    /// The secret itself, for the one place that hands it to the protocol.
    pub(crate) fn secret(&self) -> &str {
        match self {
            Self::Password(secret) | Self::XOAuth2(secret) => secret,
        }
    }
}

impl fmt::Debug for Auth {
    /// A password or a token is a secret: no `{:?}` may print one (AD-13, FR-40).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self {
            Self::Password(_) => "Password",
            Self::XOAuth2(_) => "XOAuth2",
        };
        f.debug_tuple(kind).field(&"<redacted>").finish()
    }
}

/// The IMAP side of an IMAP/SMTP connection (FR-36).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImapSettings {
    /// Server host name, e.g. `imap.fastmail.com`.
    pub host: String,
    /// Server port, e.g. 993.
    pub port: u16,
    /// Implicit TLS or `STARTTLS`.
    pub security: Security,
    /// The mailbox login, usually the address itself.
    pub username: String,
}

impl ImapSettings {
    /// Reject settings the probe cannot even attempt, so the UI shows "complete
    /// the form" instead of a connection timeout.
    pub fn validate(&self) -> Result<(), ImapError> {
        let ok = !self.host.trim().is_empty() && self.port != 0 && !self.username.trim().is_empty();
        if ok { Ok(()) } else { Err(ImapError::Settings) }
    }
}

/// What the probe found: the mailbox opened read-only, and how much is in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MailboxProbe {
    /// Messages in `INBOX` at the moment of the test.
    pub messages: u32,
}

/// Failures of the IMAP probe. No variant carries a credential, and none repeats
/// server text: the server has seen the password and could echo it back.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImapError {
    /// The server could not be reached.
    #[error("the mail server could not be reached")]
    Connect,
    /// The TLS handshake failed, or `STARTTLS` was refused.
    #[error("the connection could not be secured")]
    Tls,
    /// The server rejected the credential.
    #[error("the mail server refused the credentials")]
    Credentials,
    /// The server answered `NO`/`BAD` to a command the probe needs.
    #[error("the mail server refused the request")]
    Refused,
    /// The server answered something the probe does not understand.
    #[error("the mail server answered unexpectedly")]
    Protocol,
    /// Host, port or username is missing (AC2: tested before saving).
    #[error("the mail account settings are incomplete")]
    Settings,
}

/// Open `INBOX` read-only and report what is there (AC2).
///
/// The connection is closed again before returning; nothing is fetched, flagged
/// or written.
pub async fn test_connection(
    settings: &ImapSettings,
    auth: &Auth,
) -> Result<MailboxProbe, ImapError> {
    settings.validate()?;
    let (stream, greeting_read) = connect(settings).await?;
    probe(stream, settings, auth, greeting_read).await
}

/// The secured socket, and whether the server greeting has already been consumed
/// (`STARTTLS` upgrades mid-session, so there is none after the upgrade).
async fn connect(settings: &ImapSettings) -> Result<(SecuredStream, bool), ImapError> {
    let tcp = TcpStream::connect((settings.host.as_str(), settings.port))
        .await
        .map_err(|_| ImapError::Connect)?;
    let connector = tokio_native_tls::TlsConnector::from(
        native_tls::TlsConnector::new().map_err(|_| ImapError::Tls)?,
    );

    match settings.security {
        Security::Tls => {
            let stream = connector
                .connect(&settings.host, tcp)
                .await
                .map_err(|_| ImapError::Tls)?;
            Ok((stream, false))
        }
        Security::StartTls => {
            // The upgrade happens on the plain socket, before any credential is
            // sent: greeting, `STARTTLS`, then the handshake.
            let mut client = async_imap::Client::new(tcp);
            match client.read_response().await {
                Ok(Some(_)) => {}
                _ => return Err(ImapError::Connect),
            }
            client
                .run_command_and_check_ok("STARTTLS", None)
                .await
                .map_err(|_| ImapError::Tls)?;
            let stream = connector
                .connect(&settings.host, client.into_inner())
                .await
                .map_err(|_| ImapError::Tls)?;
            Ok((stream, true))
        }
    }
}

/// The socket this module works on, whichever way it was secured.
type SecuredStream = tokio_native_tls::TlsStream<TcpStream>;

/// The IMAP conversation: greeting, authenticate, `EXAMINE INBOX`, `LOGOUT`.
///
/// Generic over the socket so the protocol is exercised against a real server in
/// the tests; [`connect`] is the only part that needs a certificate.
async fn probe<S>(
    stream: S,
    settings: &ImapSettings,
    auth: &Auth,
    greeting_read: bool,
) -> Result<MailboxProbe, ImapError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + fmt::Debug + Send,
{
    let mut client = async_imap::Client::new(stream);
    if !greeting_read {
        match client.read_response().await {
            Ok(Some(_)) => {}
            _ => return Err(ImapError::Connect),
        }
    }

    // A refused credential is the one failure the UI must name precisely, and
    // the server's own text is not repeated: it has seen the credential.
    let mut session = match auth {
        Auth::Password(password) => client
            .login(settings.username.as_str(), password.as_str())
            .await
            .map_err(|_| ImapError::Credentials)?,
        Auth::XOAuth2(token) => client
            .authenticate("XOAUTH2", XOAuth2::new(&settings.username, token))
            .await
            .map_err(|_| ImapError::Credentials)?,
    };

    // AC2: `EXAMINE`, never `SELECT` — the mailbox is opened read-only, so the
    // test cannot change a flag or any per-user state.
    let mailbox = session
        .examine("INBOX")
        .await
        .map_err(|_| ImapError::Refused)?;
    let messages = mailbox.exists;

    // The probe has its answer; a refused LOGOUT does not change it.
    let _ = session.logout().await;
    Ok(MailboxProbe { messages })
}

/// The SASL `XOAUTH2` response (Google's profile): the bearer token in the
/// `user=…\x01auth=Bearer …\x01\x01` form, base64-encoded by the client.
struct XOAuth2 {
    response: String,
}

impl XOAuth2 {
    fn new(username: &str, token: &str) -> Self {
        Self {
            response: format!("user={username}\x01auth=Bearer {token}\x01\x01"),
        }
    }
}

impl async_imap::Authenticator for XOAuth2 {
    type Response = String;

    fn process(&mut self, _challenge: &[u8]) -> String {
        self.response.clone()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use base64::Engine as _;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

    use super::*;

    /// A one-shot IMAP server: it sends `greeting`, then answers every command
    /// line with `reply(tag, command)`. Every raw line it received is recorded.
    async fn fake_imap(
        greeting: &'static str,
        reply: impl Fn(&str, &str) -> String + Send + Sync + 'static,
    ) -> (std::net::SocketAddr, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let recorded = seen.clone();
        tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("accept");
            let (read, mut write) = socket.into_split();
            write.write_all(greeting.as_bytes()).await.expect("greet");
            let mut lines = BufReader::new(read).lines();
            let mut tag = String::new();
            while let Ok(Some(line)) = lines.next_line().await {
                recorded.lock().expect("lock").push(line.clone());
                // A continuation response carries no tag of its own, so the
                // reply to it is tagged with the command it continues.
                if let Some((own_tag, _)) = line.split_once(' ') {
                    tag = own_tag.to_string();
                }
                let command = line
                    .split_once(' ')
                    .map(|(_, rest)| rest)
                    .unwrap_or(line.as_str());
                let answer = reply(&tag, command);
                if answer.is_empty() {
                    break;
                }
                if write.write_all(answer.as_bytes()).await.is_err() {
                    break;
                }
            }
        });
        (addr, seen)
    }

    fn settings(addr: std::net::SocketAddr) -> ImapSettings {
        ImapSettings {
            host: addr.ip().to_string(),
            port: addr.port(),
            security: Security::Tls,
            username: "me@example.com".to_string(),
        }
    }

    /// The probe talks to a real socket, so the test drives the plain-TCP
    /// conversation directly ([`probe`]) — the TLS wrap around it is the only
    /// part that needs a certificate.
    #[tokio::test]
    async fn login_opens_the_mailbox_read_only() {
        let (addr, seen) = fake_imap("* OK [CAPABILITY IMAP4rev1] ready\r\n", |tag, command| {
            if command.starts_with("LOGIN") {
                format!("{tag} OK LOGIN completed\r\n")
            } else if command.starts_with("EXAMINE") {
                format!("* 3 EXISTS\r\n* OK [READ-ONLY] done\r\n{tag} OK EXAMINE completed\r\n")
            } else if command.starts_with("LOGOUT") {
                format!("* BYE\r\n{tag} OK LOGOUT completed\r\n")
            } else {
                format!("{tag} BAD unknown\r\n")
            }
        })
        .await;

        let stream = TcpStream::connect(addr).await.expect("connect");
        let probe = probe(
            stream,
            &settings(addr),
            &Auth::Password("app-password".to_string()),
            false,
        )
        .await
        .expect("probe");

        assert_eq!(probe.messages, 3);
        let commands = seen.lock().expect("lock").clone();
        assert!(
            commands
                .iter()
                .any(|line| line.contains("EXAMINE \"INBOX\"")),
            "the mailbox is opened read-only: {commands:?}"
        );
        assert!(
            !commands.iter().any(|line| line.contains("SELECT")),
            "SELECT would open the mailbox read-write: {commands:?}"
        );
        assert!(
            commands.iter().any(|line| line.contains("LOGOUT")),
            "the connection is closed again: {commands:?}"
        );
    }

    #[tokio::test]
    async fn xoauth2_sends_the_bearer_token() {
        let (addr, seen) = fake_imap("* OK ready\r\n", |tag, command| {
            if command.starts_with("AUTHENTICATE XOAUTH2") {
                "+ \r\n".to_string()
            } else if command.starts_with("EXAMINE") {
                format!("* 0 EXISTS\r\n{tag} OK EXAMINE completed\r\n")
            } else if line_is_bearer(command) {
                format!("{tag} OK AUTHENTICATE completed\r\n")
            } else if command.starts_with("LOGOUT") {
                format!("{tag} OK LOGOUT completed\r\n")
            } else {
                format!("{tag} BAD unknown\r\n")
            }
        })
        .await;

        let stream = TcpStream::connect(addr).await.expect("connect");
        let probe = probe(
            stream,
            &settings(addr),
            &Auth::XOAuth2("the-token".into()),
            false,
        )
        .await
        .expect("probe");
        assert_eq!(probe.messages, 0);

        let commands = seen.lock().expect("lock").clone();
        let expected = base64::engine::general_purpose::STANDARD
            .encode("user=me@example.com\x01auth=Bearer the-token\x01\x01");
        assert!(
            commands.iter().any(|line| line == &expected),
            "the XOAUTH2 response is the RFC profile: {commands:?}"
        );
    }

    /// The continuation response carries no tag, so it arrives with the whole
    /// base64 string as its first token.
    fn line_is_bearer(command: &str) -> bool {
        !command.contains(' ')
    }

    #[tokio::test]
    async fn a_refused_login_is_credentials_not_a_panic() {
        let (addr, _) = fake_imap("* OK ready\r\n", |tag, _| {
            format!("{tag} NO [AUTHENTICATIONFAILED] invalid credentials\r\n")
        })
        .await;

        let stream = TcpStream::connect(addr).await.expect("connect");
        let error = probe(
            stream,
            &settings(addr),
            &Auth::Password("wrong".to_string()),
            false,
        )
        .await
        .expect_err("refused");
        assert_eq!(error, ImapError::Credentials);
        assert!(!format!("{error}").contains("wrong"));
    }

    #[test]
    fn incomplete_settings_never_reach_the_network() {
        let mut incomplete = ImapSettings {
            host: "  ".to_string(),
            port: 993,
            security: Security::Tls,
            username: "me@example.com".to_string(),
        };
        assert_eq!(incomplete.validate(), Err(ImapError::Settings));

        incomplete.host = "imap.example.com".to_string();
        incomplete.port = 0;
        assert_eq!(incomplete.validate(), Err(ImapError::Settings));

        incomplete.port = 993;
        incomplete.username = String::new();
        assert_eq!(incomplete.validate(), Err(ImapError::Settings));

        incomplete.username = "me@example.com".to_string();
        assert_eq!(incomplete.validate(), Ok(()));
    }

    #[test]
    fn a_password_is_never_printed() {
        let auth = Auth::Password("hunter2".to_string());
        assert!(!format!("{auth:?}").contains("hunter2"));
        assert_eq!(auth.secret(), "hunter2");
    }

    #[test]
    fn settings_and_credentials_round_trip_through_the_stored_form() {
        let stored = serde_json::json!({
            "host": "imap.example.com",
            "port": 993,
            "security": "start_tls",
            "username": "me@example.com"
        });
        let settings: ImapSettings = serde_json::from_value(stored).expect("settings");
        assert_eq!(settings.security, Security::StartTls);
        assert_eq!(settings.port, 993);

        let auth: Auth =
            serde_json::from_str(&serde_json::to_string(&Auth::XOAuth2("t".into())).unwrap())
                .expect("auth");
        assert_eq!(auth, Auth::XOAuth2("t".to_string()));
    }
}
