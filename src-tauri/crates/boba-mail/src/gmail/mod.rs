//! Gmail send-only adapter (S8.2, AC1, FR-34, architecture §11).
//!
//! Boba's bundled Google client is a **public desktop client** and asks for
//! exactly one scope, `gmail.send` (AC1): it can send as the user and read
//! nothing. Reading Gmail needs a BYO client (S8.5), IMAP (S8.3) or a `.eml`
//! import. CASA stays deferred.
//!
//! The connect flow is the shared one in [`crate::oauth`] — loopback-only, PKCE
//! `S256`, single-use `state`. Google's one difference is that its token endpoint
//! demands the public installed-app "secret" its console issues for a Desktop-app
//! client; Google's own documentation says that value is "obviously not treated as
//! a secret", so it is a build-time value here (or the user's own, BYO) and it is
//! only ever sent to `oauth2.googleapis.com/token`.
//!
//! The message itself is built by [`crate::smtp::rfc5322`] — the same RFC 5322
//! bytes the SMTP path puts on the wire — and handed to the Gmail API as `raw`.

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE;
use boba_app::ports::mail::{MailError, MailFuture, MailPort, OutgoingMessage};
use boba_app::ports::repo::email::{EmailAccount, EmailRepo, MailProvider};
use boba_app::ports::secret::SecretPort;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::oauth::{self, OAuthError, ProviderEndpoints, TokenSet};

/// Boba's bundled Gmail client id, set at build time (`BOBA_GMAIL_CLIENT_ID`).
///
/// Public by definition — a desktop client has no secret and security never relies
/// on it. Registering the client is a release task (S10.5); until it exists the
/// bundled path is unavailable and the UI offers the user's own client id.
pub const BUNDLED_GMAIL_CLIENT_ID: Option<&str> = option_env!("BOBA_GMAIL_CLIENT_ID");

/// The public value Google issues next to a Desktop-app client id, set at build
/// time (`BOBA_GMAIL_CLIENT_SECRET`). Google requires it at the token endpoint and
/// documents that it is not confidential.
pub const BUNDLED_GMAIL_CLIENT_SECRET: Option<&str> = option_env!("BOBA_GMAIL_CLIENT_SECRET");

/// The one scope the bundled client asks Google for (AC1, NFR-5).
///
/// It is a full URL, unlike Graph's `Mail.Send`. Nothing else is requested:
/// `gmail.send` cannot read, and Google returns a refresh token to installed apps
/// without asking for `offline_access`.
pub const GMAIL_SEND_SCOPE: &str = "https://www.googleapis.com/auth/gmail.send";

/// Google's endpoints for this provider (the shared loopback flow dials them).
pub fn endpoints() -> ProviderEndpoints {
    ProviderEndpoints::gmail()
}

/// The Google hosts the flow talks to, in one place for the tests and the UI's
/// troubleshooting copy.
pub fn authorization_host() -> &'static str {
    "accounts.google.com"
}

/// What the keychain holds for one Gmail connection (AD-13, FR-40).
///
/// The tokens, plus the public client secret this connection uses: Boba's bundled
/// one, or the user's own when they brought a client id (BYO). One entry per
/// connection, addressed by the account's opaque `secret_ref`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmailSecret {
    /// The current tokens.
    pub tokens: TokenSet,
    /// The public installed-app secret, when this connection has one.
    #[serde(default)]
    pub client_secret: Option<String>,
}

impl std::fmt::Debug for GmailSecret {
    /// Tokens are secrets: the entry never prints them (AD-13, FR-40).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GmailSecret")
            .field("tokens", &self.tokens)
            .field(
                "client_secret",
                &self
                    .client_secret
                    .as_ref()
                    .map(|_| "<public client secret>"),
            )
            .finish()
    }
}

impl GmailSecret {
    /// The keychain representation.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Parse the keychain representation. An entry that cannot be read is the same
    /// as no entry: the user reconnects rather than sending with half a credential.
    pub fn from_json(json: &str) -> Result<Self, OAuthError> {
        serde_json::from_str(json).map_err(|_| OAuthError::InvalidTokens)
    }
}

/// Gmail's `MailPort`: sends through the Gmail API with the connection's token.
pub struct GmailMail {
    http: reqwest::Client,
    endpoints: ProviderEndpoints,
    secrets: Arc<dyn SecretPort>,
    accounts: Arc<dyn EmailRepo>,
}

impl GmailMail {
    /// The adapter as the app wires it: Google's endpoints and a client that
    /// identifies Boba honestly. Fails only when no HTTP client can be built.
    pub fn new(
        secrets: Arc<dyn SecretPort>,
        accounts: Arc<dyn EmailRepo>,
    ) -> Result<Self, OAuthError> {
        Ok(Self::with_endpoints(
            oauth::http_client()?,
            ProviderEndpoints::gmail(),
            secrets,
            accounts,
        ))
    }

    /// The adapter against explicit endpoints (tests, or a wiremock server).
    pub fn with_endpoints(
        http: reqwest::Client,
        endpoints: ProviderEndpoints,
        secrets: Arc<dyn SecretPort>,
        accounts: Arc<dyn EmailRepo>,
    ) -> Self {
        Self {
            http,
            endpoints,
            secrets,
            accounts,
        }
    }

    /// The Gmail `users.messages.send` body: the RFC 5322 message, base64url.
    pub fn send_body(raw: &[u8]) -> Value {
        json!({ "raw": URL_SAFE.encode(raw) })
    }

    /// A usable access token for `account`, renewing it when it is near expiry
    /// (AC1: the tokens never leave the keychain and the daily run works while the
    /// user is away).
    async fn access_token(&self, account: &EmailAccount) -> Result<String, MailError> {
        let stored = self
            .secrets
            .get(&account.secret_ref)
            .map_err(|_| MailError::Reauthorize)?
            .ok_or(MailError::Reauthorize)?;
        // An entry that cannot be read is the same as no entry: the user must
        // authorise again rather than send with half a credential.
        let secret = GmailSecret::from_json(&stored).map_err(|_| MailError::Reauthorize)?;

        if !secret
            .tokens
            .is_expired(oauth::now_unix(), REFRESH_SKEW_SECS)
        {
            return Ok(secret.tokens.access_token);
        }
        let refresh = secret
            .tokens
            .refresh_token
            .clone()
            .ok_or(MailError::Reauthorize)?;
        // Google takes its public installed-app secret here and nowhere else.
        let response = match &secret.client_secret {
            Some(client_secret) => {
                oauth::refresh_tokens_with_client_secret(
                    &self.http,
                    &self.endpoints,
                    &account.client_id,
                    client_secret,
                    &refresh,
                )
                .await
            }
            None => {
                oauth::refresh_tokens(&self.http, &self.endpoints, &account.client_id, &refresh)
                    .await
            }
        }
        .map_err(|_| MailError::Reauthorize)?;

        let renewed = GmailSecret {
            tokens: TokenSet::new(&response, &secret.tokens.scopes),
            client_secret: secret.client_secret,
        };
        self.secrets
            .put(&account.secret_ref, &renewed.to_json())
            .map_err(|_| MailError::Reauthorize)?;
        Ok(renewed.tokens.access_token)
    }

    /// The Gmail `users.messages.send` request for one message.
    fn send_request(&self, token: &str) -> reqwest::RequestBuilder {
        self.http
            .post(format!(
                "{}/users/me/messages/send",
                self.endpoints.api_base
            ))
            .bearer_auth(token)
    }
}

/// Renew this long before the token stops working, so a slow request cannot start
/// with a token that expires mid-flight.
const REFRESH_SKEW_SECS: u64 = 60;

impl MailPort for GmailMail {
    fn send<'a>(
        &'a self,
        account_id: &'a str,
        message: &'a OutgoingMessage,
    ) -> MailFuture<'a, Result<(), MailError>> {
        Box::pin(async move {
            let account = self
                .accounts
                .get_account(account_id)
                .map_err(|_| MailError::UnknownAccount)?
                .ok_or(MailError::UnknownAccount)?;
            if account.provider != MailProvider::Gmail {
                return Err(MailError::UnknownAccount);
            }

            let token = self.access_token(&account).await?;
            // The same RFC 5322 message the SMTP path sends (AC1: Gmail is
            // send-only, so nothing here reads).
            let raw = crate::smtp::rfc5322(&account.address, message)
                .map_err(|_| MailError::Rejected("the message could not be built".to_string()))?;

            let response = self
                .send_request(&token)
                .json(&Self::send_body(&raw))
                .send()
                .await
                .map_err(|_| {
                    MailError::Transport("the mail provider could not be reached".to_string())
                })?;

            // Gmail answers 200 with the message id; the error body is not
            // repeated, it can echo the message.
            if response.status().is_success() {
                Ok(())
            } else {
                Err(MailError::Rejected(format!(
                    "the provider answered HTTP {}",
                    response.status().as_u16()
                )))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use boba_app::ports::repo::email::{ClientIdKind, EmailMode, EmailRepoError};
    use boba_app::ports::secret::SecretError;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    const REF: &str = "mail:01GMAIL";
    const BYO_SECRET: &str = "GOCSPX-public-value";

    struct FakeSecrets(Mutex<HashMap<String, String>>);

    impl FakeSecrets {
        fn with(reference: &str, value: String) -> Arc<Self> {
            Arc::new(Self(Mutex::new(HashMap::from([(
                reference.to_string(),
                value,
            )]))))
        }

        fn value(&self, reference: &str) -> Option<String> {
            self.0.lock().expect("lock").get(reference).cloned()
        }
    }

    impl SecretPort for FakeSecrets {
        fn put(&self, reference: &str, value: &str) -> Result<(), SecretError> {
            self.0
                .lock()
                .expect("lock")
                .insert(reference.to_string(), value.to_string());
            Ok(())
        }

        fn get(&self, reference: &str) -> Result<Option<String>, SecretError> {
            Ok(self.value(reference))
        }

        fn delete(&self, reference: &str) -> Result<bool, SecretError> {
            Ok(self.0.lock().expect("lock").remove(reference).is_some())
        }
    }

    struct FakeAccounts(Option<EmailAccount>);

    impl EmailRepo for FakeAccounts {
        fn upsert_account(&self, _account: &EmailAccount) -> Result<(), EmailRepoError> {
            unimplemented!("not used by the send path")
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
            unimplemented!("not used by the send path")
        }

        fn delete_account(&self, _id: &str) -> Result<bool, EmailRepoError> {
            unimplemented!("not used by the send path")
        }
    }

    fn account() -> EmailAccount {
        EmailAccount {
            id: "01GMAIL".to_string(),
            provider: MailProvider::Gmail,
            address: "me@gmail.com".to_string(),
            // AC1: the bundled client only ever asks for `gmail.send`.
            mode: EmailMode::SendOnly,
            client_id: "client-1.apps.googleusercontent.com".to_string(),
            client_id_kind: ClientIdKind::Byo,
            scopes: vec![GMAIL_SEND_SCOPE.to_string()],
            secret_ref: REF.to_string(),
            connected_at: "2026-10-16T00:00:00Z".to_string(),
            updated_at: "2026-10-16T00:00:00Z".to_string(),
        }
    }

    /// A stored connection; `None` leaves the expiry unknown.
    fn stored(expires_in: Option<u64>, client_secret: Option<&str>) -> String {
        GmailSecret {
            tokens: TokenSet {
                access_token: "at-1".to_string(),
                refresh_token: Some("rt-1".to_string()),
                expires_at_unix: expires_in.map_or(0, |seconds| oauth::now_unix() + seconds),
                scopes: vec![GMAIL_SEND_SCOPE.to_string()],
            },
            client_secret: client_secret.map(str::to_string),
        }
        .to_json()
    }

    fn message() -> OutgoingMessage {
        OutgoingMessage {
            to: vec!["recruiter@example.com".to_string()],
            subject: "Application: Backend Engineer".to_string(),
            body_text: "Hello,\n\nPlease find my CV attached.".to_string(),
            attachments: vec![boba_app::ports::mail::Attachment {
                filename: "cv.pdf".to_string(),
                mime: "application/pdf".to_string(),
                bytes: b"%PDF-1.4 \x00\x01binary".to_vec(),
            }],
        }
    }

    fn adapter(
        server: &MockServer,
        secrets: Arc<dyn SecretPort>,
        accounts: Arc<dyn EmailRepo>,
    ) -> GmailMail {
        GmailMail::with_endpoints(
            reqwest::Client::new(),
            ProviderEndpoints {
                api_base: server.uri(),
                authorize: format!("{}/authorize", server.uri()),
                token: format!("{}/token", server.uri()),
                revoke: Some(format!("{}/revoke", server.uri())),
            },
            secrets,
            accounts,
        )
    }

    // --- AC1: the connection asks for `gmail.send` and nothing else ----------

    #[test]
    fn the_flow_asks_google_for_gmail_send_only() {
        let flow = oauth::PendingFlow::new(
            endpoints(),
            "client-1.apps.googleusercontent.com",
            "http://127.0.0.1:49152/cb",
            &[GMAIL_SEND_SCOPE],
        )
        .expect("flow");
        let url = url::Url::parse(&flow.authorize_url().expect("url")).expect("parse");

        assert_eq!(url.host_str(), Some(authorization_host()));
        assert_eq!(url.path(), "/o/oauth2/v2/auth");
        let pairs: HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(
            pairs.get("scope").map(String::as_str),
            Some(GMAIL_SEND_SCOPE)
        );
        assert_eq!(pairs.get("response_type").map(String::as_str), Some("code"));
        assert_eq!(
            pairs.get("code_challenge_method").map(String::as_str),
            Some("S256")
        );
        // No read access, and no confidential secret anywhere in the request.
        let scope = pairs.get("scope").cloned().unwrap_or_default();
        assert!(!scope.contains("readonly"), "scope: {scope}");
        assert!(!pairs.contains_key("client_secret"));
        assert!(!pairs.contains_key("access_type"));
    }

    #[test]
    fn gmail_uses_googles_endpoints_and_can_be_revoked() {
        let endpoints = endpoints();
        assert_eq!(endpoints.api_base, "https://gmail.googleapis.com/gmail/v1");
        assert_eq!(
            endpoints.token,
            "https://oauth2.googleapis.com/token".to_string()
        );
        assert!(endpoints.revoke.is_some(), "Google can revoke (AC4)");
    }

    #[test]
    fn the_connection_entry_round_trips_and_never_prints_the_tokens() {
        let secret = GmailSecret {
            tokens: TokenSet {
                access_token: "at-1".to_string(),
                refresh_token: Some("rt-1".to_string()),
                expires_at_unix: 0,
                scopes: vec![GMAIL_SEND_SCOPE.to_string()],
            },
            client_secret: Some(BYO_SECRET.to_string()),
        };
        let json = secret.to_json();
        assert_eq!(GmailSecret::from_json(&json).expect("read"), secret);
        assert!(!format!("{secret:?}").contains("at-1"));
        assert!(!format!("{secret:?}").contains("rt-1"));
        assert_eq!(
            GmailSecret::from_json("not json"),
            Err(OAuthError::InvalidTokens)
        );
    }

    // --- Sending (AC1) ------------------------------------------------------

    /// AC1: what leaves the app is the in-app message, base64url-encoded, on the
    /// send endpoint, with the connection's bearer token.
    #[tokio::test]
    async fn send_posts_the_message_as_raw_base64url() {
        use base64::engine::general_purpose::URL_SAFE;

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/users/me/messages/send"))
            .and(header("authorization", "Bearer at-1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "id": "18f0", "threadId": "18f0" })),
            )
            .expect(1)
            .mount(&server)
            .await;

        let secrets = FakeSecrets::with(REF, stored(Some(3600), Some(BYO_SECRET)));
        let mail = adapter(
            &server,
            secrets.clone(),
            Arc::new(FakeAccounts(Some(account()))),
        );
        mail.send("01GMAIL", &message()).await.expect("sent");

        let requests = server.received_requests().await.expect("requests");
        assert_eq!(requests.len(), 1);
        let sent: Value = serde_json::from_slice(&requests[0].body).expect("json");
        let raw = URL_SAFE
            .decode(sent["raw"].as_str().expect("raw"))
            .expect("decode");
        let wire = String::from_utf8_lossy(&raw);
        assert!(
            wire.contains("Subject: Application: Backend Engineer"),
            "{wire}"
        );
        assert!(wire.contains("filename=\"cv.pdf\""), "{wire}");
        assert!(wire.contains("recruiter@example.com"), "{wire}");

        // A usable token is used as-is: the stored set is not rewritten.
        assert_eq!(
            GmailSecret::from_json(&secrets.value(REF).expect("stored"))
                .expect("json")
                .tokens
                .access_token,
            "at-1"
        );
    }

    #[tokio::test]
    async fn an_expired_token_is_refreshed_with_the_public_client_secret() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("client_secret=GOCSPX-public-value"))
            .and(body_string_contains("grant_type=refresh_token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "at-2",
                "refresh_token": "rt-2",
                "expires_in": 3600
            })))
            .expect(1)
            .mount(&server)
            .await;

        let secrets = FakeSecrets::with(
            REF,
            GmailSecret {
                tokens: TokenSet {
                    access_token: "at-1".to_string(),
                    refresh_token: Some("rt-1".to_string()),
                    expires_at_unix: oauth::now_unix() - 1,
                    scopes: vec![GMAIL_SEND_SCOPE.to_string()],
                },
                client_secret: Some(BYO_SECRET.to_string()),
            }
            .to_json(),
        );
        let mail = adapter(
            &server,
            secrets.clone(),
            Arc::new(FakeAccounts(Some(account()))),
        );
        Mock::given(method("POST"))
            .and(path("/users/me/messages/send"))
            .and(header("authorization", "Bearer at-2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "18f1" })))
            .expect(1)
            .mount(&server)
            .await;

        mail.send("01GMAIL", &message()).await.expect("sent");
        let renewed = GmailSecret::from_json(&secrets.value(REF).expect("renewed")).expect("json");
        assert_eq!(renewed.tokens.access_token, "at-2");
        assert_eq!(renewed.tokens.refresh_token.as_deref(), Some("rt-2"));
        assert_eq!(
            renewed.client_secret.as_deref(),
            Some(BYO_SECRET),
            "the public client secret is kept for the next run"
        );
    }

    #[tokio::test]
    async fn a_rejected_message_reports_the_status_and_not_the_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/users/me/messages/send"))
            .respond_with(ResponseTemplate::new(403).set_body_string("insufficient scope"))
            .mount(&server)
            .await;

        let mail = adapter(
            &server,
            FakeSecrets::with(REF, stored(Some(3600), None)),
            Arc::new(FakeAccounts(Some(account()))),
        );
        let error = mail
            .send("01GMAIL", &message())
            .await
            .expect_err("rejected");
        assert_eq!(
            error,
            MailError::Rejected("the provider answered HTTP 403".to_string())
        );
        assert!(!format!("{error}").contains("at-1"));
    }

    #[tokio::test]
    async fn a_missing_or_unreadable_entry_asks_for_reauthorisation() {
        let server = MockServer::start().await;
        let accounts = Arc::new(FakeAccounts(Some(account())));

        // Nothing under the reference, an entry of another shape, unreadable JSON.
        for entry in [None, Some("{}"), Some("not json")] {
            let secrets: Arc<dyn SecretPort> = match entry {
                Some(value) => FakeSecrets::with(REF, value.to_string()),
                None => FakeSecrets::with("other", "{}".to_string()),
            };
            let mail = adapter(&server, secrets, accounts.clone());
            assert_eq!(
                mail.send("01GMAIL", &message()).await,
                Err(MailError::Reauthorize),
                "entry: {entry:?}"
            );
        }
    }

    #[tokio::test]
    async fn an_unknown_or_non_gmail_connection_is_not_sent_through() {
        let server = MockServer::start().await;
        let secrets = FakeSecrets::with(REF, stored(Some(3600), None));

        let mail = adapter(&server, secrets.clone(), Arc::new(FakeAccounts(None)));
        assert_eq!(
            mail.send("01GMAIL", &message()).await,
            Err(MailError::UnknownAccount)
        );

        let mut outlook = account();
        outlook.provider = MailProvider::MicrosoftGraph;
        let mail = adapter(&server, secrets, Arc::new(FakeAccounts(Some(outlook))));
        assert_eq!(
            mail.send("01GMAIL", &message()).await,
            Err(MailError::UnknownAccount)
        );
    }
}
