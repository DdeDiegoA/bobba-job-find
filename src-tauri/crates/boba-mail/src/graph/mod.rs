//! Microsoft Graph mail adapter (S8.1, FR-35, architecture §11).
//!
//! `MailPort` over the delegated `Mail.Send` scope of a connected Outlook.com /
//! M365 mailbox. The bearer token is resolved from the keychain per send and
//! renewed from the refresh token when it is near expiry (AC1: the tokens never
//! leave the keychain). Reading stays with the sync owner (S8.3): this slice
//! sends.

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use boba_app::ports::mail::{Attachment, MailError, MailFuture, MailPort, OutgoingMessage};
use boba_app::ports::repo::email::{EmailAccount, EmailMode, EmailRepo, MailProvider};
use boba_app::ports::secret::SecretPort;
use mail_parser::DateTime;
use serde_json::{Value, json};

use crate::eml;
use crate::oauth::{self, OAuthError, ProviderEndpoints, TokenSet};
use crate::sync::{RawMessage, normalize_message_id};

/// Renew this long before the token stops working, so a slow request cannot
/// start with a token that expires mid-flight.
const REFRESH_SKEW_SECS: u64 = 60;

/// Microsoft Graph's mail adapter.
pub struct GraphMail {
    http: reqwest::Client,
    secrets: Arc<dyn SecretPort>,
    accounts: Arc<dyn EmailRepo>,
    endpoints: ProviderEndpoints,
}

impl GraphMail {
    /// The adapter as the app wires it: Graph's endpoints and a client that
    /// identifies Boba honestly. Fails only when no HTTP client can be built,
    /// which is a startup problem the composition root reports.
    pub fn new(
        secrets: Arc<dyn SecretPort>,
        accounts: Arc<dyn EmailRepo>,
    ) -> Result<Self, OAuthError> {
        Ok(Self::with_endpoints(
            oauth::http_client()?,
            secrets,
            accounts,
            ProviderEndpoints::microsoft_graph(),
        ))
    }

    /// The adapter against explicit endpoints (tests, or a wiremock server).
    pub fn with_endpoints(
        http: reqwest::Client,
        secrets: Arc<dyn SecretPort>,
        accounts: Arc<dyn EmailRepo>,
        endpoints: ProviderEndpoints,
    ) -> Self {
        Self {
            http,
            secrets,
            accounts,
            endpoints,
        }
    }

    /// A usable access token for `account`, renewing it when it is near expiry.
    async fn access_token(&self, account: &EmailAccount) -> Result<String, MailError> {
        let stored = self
            .secrets
            .get(&account.secret_ref)
            .map_err(|_| MailError::Reauthorize)?
            .ok_or(MailError::Reauthorize)?;
        // A token set that cannot be read is the same as no token: the user must
        // authorise again rather than send with half a credential.
        let tokens = TokenSet::from_json(&stored).map_err(|_| MailError::Reauthorize)?;

        if !tokens.is_expired(oauth::now_unix(), REFRESH_SKEW_SECS) {
            return Ok(tokens.access_token);
        }
        let refresh = tokens.refresh_token.clone().ok_or(MailError::Reauthorize)?;
        let response = oauth::refresh_tokens(
            &self.http,
            &self.endpoints,
            &account.client_id,
            refresh.as_str(),
        )
        .await
        .map_err(|_| MailError::Reauthorize)?;

        // A refresh that omits the refresh token keeps the stored one (AD-13).
        let renewed = tokens.renewed(&response);
        self.secrets
            .put(&account.secret_ref, &renewed.to_json())
            .map_err(|_| MailError::Reauthorize)?;
        Ok(renewed.access_token)
    }

    /// The Graph `sendMail` request body for one message.
    pub fn send_body(message: &OutgoingMessage) -> Value {
        json!({
            "message": {
                "subject": message.subject,
                "body": { "contentType": "Text", "content": message.body_text },
                "toRecipients": message
                    .to
                    .iter()
                    .map(|address| json!({ "emailAddress": { "address": address } }))
                    .collect::<Vec<_>>(),
                "attachments": message
                    .attachments
                    .iter()
                    .map(attachment_body)
                    .collect::<Vec<_>>(),
            },
            // The message is already in the user's mailbox view once sent; keep
            // the draft out of it.
            "saveToSentItems": true,
        })
    }

    /// The Graph file-attachment shape: `contentBytes` is base64.
    fn send_request(&self, token: &str) -> reqwest::RequestBuilder {
        self.http
            .post(format!("{}/me/sendMail", self.endpoints.api_base))
            .bearer_auth(token)
    }
}

fn attachment_body(attachment: &Attachment) -> Value {
    json!({
        "@odata.type": "#microsoft.graph.fileAttachment",
        "name": attachment.filename,
        "contentType": attachment.mime,
        "contentBytes": BASE64.encode(&attachment.bytes),
    })
}

impl MailPort for GraphMail {
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
            if account.provider != MailProvider::MicrosoftGraph {
                return Err(MailError::UnknownAccount);
            }

            let token = self.access_token(&account).await?;
            let response = self
                .send_request(&token)
                .json(&Self::send_body(message))
                .send()
                .await
                .map_err(|_| {
                    MailError::Transport("the mail provider could not be reached".to_string())
                })?;

            // Graph answers 202 Accepted on success; the body carries nothing.
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

/// How many messages one read pass may return. Graph caps a page at 1000;
/// reading a recruiter reply needs a handful, so the smaller cap is the one
/// that keeps a run bounded.
pub const READ_PAGE_SIZE: usize = 100;

/// The largest inbox answer that is accepted. The page is buffered before it
/// is mapped, so a provider sending an oversized body is refused before a run
/// can hold it all in memory.
const MAX_READ_BYTES: u64 = 25 * 1024 * 1024;

impl GraphMail {
    /// Read the inbox messages received since `since` (Unix seconds), for an
    /// account the user connected in **read+send** mode (FR-38). The account
    /// already holds `Mail.Read`; no scope is requested here.
    ///
    /// Only headers and a bounded body are read: no attachment is fetched, and
    /// nothing is marked, moved or sent. A token near expiry is renewed through
    /// the S8.1 store and never appears in an error (AD-13).
    pub async fn fetch_inbox(
        &self,
        account_id: &str,
        since: i64,
        now: i64,
        limit: usize,
    ) -> Result<Vec<RawMessage>, MailError> {
        let account = self
            .accounts
            .get_account(account_id)
            .map_err(|_| MailError::UnknownAccount)?
            .ok_or(MailError::UnknownAccount)?;
        if account.provider != MailProvider::MicrosoftGraph {
            return Err(MailError::UnknownAccount);
        }
        // Reading is a read+send permission: a send-only connection never had
        // `Mail.Read` granted, and asking would be a scope change.
        if !account.covers(EmailMode::ReadSend) {
            return Err(MailError::Reauthorize);
        }

        let token = self.access_token(&account).await?;
        let since = DateTime::from_timestamp(since).to_rfc3339();
        // `reqwest`'s `query` feature is off in this workspace, so the URL is
        // built with `url`, which percent-encodes the OData parameters.
        let mut url = url::Url::parse(&format!(
            "{}/me/mailFolders/inbox/messages",
            self.endpoints.api_base
        ))
        .map_err(|_| MailError::Transport("the provider URL is not valid".to_string()))?;
        url.query_pairs_mut()
            .append_pair(
                "$select",
                "id,internetMessageId,internetMessageHeaders,subject,from,toRecipients,\
                 receivedDateTime,body,hasAttachments",
            )
            .append_pair("$filter", &format!("receivedDateTime ge {since}"))
            .append_pair("$orderby", "receivedDateTime asc")
            .append_pair("$top", &limit.min(READ_PAGE_SIZE).to_string());
        let response = self
            .http
            .get(url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|_| {
                MailError::Transport("the mail provider could not be reached".to_string())
            })?;
        if !response.status().is_success() {
            return Err(MailError::Rejected(format!(
                "the provider answered HTTP {}",
                response.status().as_u16()
            )));
        }
        // A provider may omit (or lie about) Content-Length, so the body is
        // read in chunks under a running cap instead of buffered blindly.
        if response
            .content_length()
            .is_some_and(|length| length > MAX_READ_BYTES)
        {
            return Err(MailError::Rejected(
                "the inbox answer is too large".to_string(),
            ));
        }
        let mut response = response;
        let mut body: Vec<u8> = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| MailError::Transport("the inbox answer was cut off".to_string()))?
        {
            if body.len() as u64 + chunk.len() as u64 > MAX_READ_BYTES {
                return Err(MailError::Rejected(
                    "the inbox answer is too large".to_string(),
                ));
            }
            body.extend_from_slice(&chunk);
        }
        let body: Value = serde_json::from_slice(&body)
            .map_err(|_| MailError::Rejected("the inbox answer could not be read".to_string()))?;
        Ok(read_messages(&body, now, limit.min(READ_PAGE_SIZE)))
    }
}

/// Map Graph's message list onto the pipeline's shape. A message without an id
/// or sender is skipped rather than failing the whole read.
fn read_messages(body: &Value, now: i64, limit: usize) -> Vec<RawMessage> {
    body.get("value")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| read_message(item, now))
                .take(limit)
                .collect()
        })
        .unwrap_or_default()
}

fn read_message(item: &Value, now: i64) -> Option<RawMessage> {
    let provider_id = item.get("id")?.as_str()?.to_string();
    let sender = item.get("from").and_then(|from| from.get("emailAddress"));
    let from_address = sender
        .and_then(|address| address.get("address"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let from_name = sender
        .and_then(|address| address.get("name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string);

    let message_id = item
        .get("internetMessageId")
        .and_then(Value::as_str)
        .map(normalize_message_id)
        .filter(|id| !id.is_empty());
    let body = item.get("body");
    let content_type = body
        .and_then(|body| body.get("contentType"))
        .and_then(Value::as_str)
        .unwrap_or("text");
    let content = body
        .and_then(|body| body.get("content"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let received_at = item
        .get("receivedDateTime")
        .and_then(Value::as_str)
        .and_then(DateTime::parse_rfc3339)
        .map(|date| date.to_timestamp())
        .filter(|timestamp| *timestamp > 0)
        .unwrap_or(now);

    Some(RawMessage {
        provider_id,
        message_id,
        in_reply_to: header_ids(item, "In-Reply-To"),
        references: header_ids(item, "References"),
        from_address,
        from_name,
        to_addresses: item
            .get("toRecipients")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|to| to.get("emailAddress"))
                    .filter_map(|address| address.get("address"))
                    .filter_map(Value::as_str)
                    .map(|address| address.trim().to_string())
                    .collect()
            })
            .unwrap_or_default(),
        subject: item
            .get("subject")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string(),
        received_at,
        body_text: eml::body_from(content_type, content),
        has_attachments: item
            .get("hasAttachments")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

/// `internetMessageHeaders` is only returned when selected; a missing header
/// list simply means no thread evidence.
fn header_ids(item: &Value, name: &str) -> Vec<String> {
    item.get("internetMessageHeaders")
        .and_then(Value::as_array)
        .map(|headers| {
            headers
                .iter()
                .filter(|header| {
                    header
                        .get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|value| value.eq_ignore_ascii_case(name))
                })
                .filter_map(|header| header.get("value").and_then(Value::as_str))
                .flat_map(str::split_whitespace)
                .map(normalize_message_id)
                .filter(|id| !id.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use boba_app::ports::repo::email::{ClientIdKind, EmailMode, EmailRepoError};
    use boba_app::ports::secret::SecretError;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

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

    const REF: &str = "mail:01ACC";

    fn account() -> EmailAccount {
        EmailAccount {
            id: "01ACC".into(),
            provider: MailProvider::MicrosoftGraph,
            address: "me@example.com".into(),
            mode: EmailMode::ReadSend,
            client_id: "client-1".into(),
            client_id_kind: ClientIdKind::Bundled,
            scopes: vec!["Mail.Read".into(), "Mail.Send".into()],
            secret_ref: REF.into(),
            connected_at: "2026-10-16T00:00:00Z".into(),
            updated_at: "2026-10-16T00:00:00Z".into(),
        }
    }

    /// A stored token set; `None` leaves the expiry unknown.
    fn tokens(expires_in: Option<u64>) -> String {
        TokenSet {
            access_token: "at-1".into(),
            refresh_token: Some("rt-1".into()),
            expires_at_unix: expires_in.map_or(0, |seconds| oauth::now_unix() + seconds),
            scopes: vec!["Mail.Send".into()],
        }
        .to_json()
    }

    fn message() -> OutgoingMessage {
        OutgoingMessage {
            to: vec!["recruiter@example.com".into()],
            subject: "Application: Backend Engineer".into(),
            body_text: "Hello,\n\nPlease find my CV attached.".into(),
            attachments: vec![Attachment {
                filename: "cv.pdf".into(),
                mime: "application/pdf".into(),
                bytes: b"%PDF-1.4".to_vec(),
            }],
        }
    }

    fn adapter(
        server: &MockServer,
        secrets: Arc<dyn SecretPort>,
        accounts: Arc<dyn EmailRepo>,
    ) -> GraphMail {
        GraphMail::with_endpoints(
            reqwest::Client::new(),
            secrets,
            accounts,
            ProviderEndpoints {
                api_base: server.uri(),
                authorize: format!("{}/authorize", server.uri()),
                token: format!("{}/token", server.uri()),
                revoke: None,
            },
        )
    }

    #[test]
    fn send_body_matches_the_graph_shape() {
        let body = GraphMail::send_body(&message());
        assert_eq!(body["message"]["subject"], "Application: Backend Engineer");
        assert_eq!(body["message"]["body"]["contentType"], "Text");
        assert_eq!(
            body["message"]["toRecipients"][0]["emailAddress"]["address"],
            "recruiter@example.com"
        );
        let file = &body["message"]["attachments"][0];
        assert_eq!(file["@odata.type"], "#microsoft.graph.fileAttachment");
        assert_eq!(file["name"], "cv.pdf");
        assert_eq!(file["contentType"], "application/pdf");
        assert_eq!(file["contentBytes"], BASE64.encode(b"%PDF-1.4"));
        assert_eq!(body["saveToSentItems"], true);
    }

    #[tokio::test]
    async fn send_posts_to_graph_with_the_stored_bearer_token() {
        let server = MockServer::start().await;
        let expected = GraphMail::send_body(&message());
        // One token set for both the seed and the comparison: `TokenSet::new`
        // dates the expiry from `now_unix()`, so building it twice can straddle
        // a second boundary and fail a loaded machine.
        let stored = tokens(Some(3600));
        Mock::given(method("POST"))
            .and(path("/me/sendMail"))
            .and(header("authorization", "Bearer at-1"))
            .and(body_json(&expected))
            .respond_with(ResponseTemplate::new(202))
            .expect(1)
            .mount(&server)
            .await;

        let secrets = FakeSecrets::with(REF, stored.clone());
        let mail = adapter(
            &server,
            secrets.clone(),
            Arc::new(FakeAccounts(Some(account()))),
        );
        mail.send("01ACC", &message()).await.expect("sent");
        // A usable token is used as-is: the stored set is not rewritten.
        assert_eq!(
            TokenSet::from_json(&secrets.value(REF).expect("stored")).expect("json"),
            TokenSet::from_json(&stored).expect("json")
        );
    }

    #[tokio::test]
    async fn an_expired_token_is_refreshed_before_sending() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "at-2",
                "refresh_token": "rt-2",
                "expires_in": 3600
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/me/sendMail"))
            .and(header("authorization", "Bearer at-2"))
            .respond_with(ResponseTemplate::new(202))
            .expect(1)
            .mount(&server)
            .await;

        let secrets = FakeSecrets::with(
            REF,
            TokenSet {
                access_token: "at-1".into(),
                refresh_token: Some("rt-1".into()),
                expires_at_unix: oauth::now_unix() - 1,
                scopes: vec!["Mail.Send".into()],
            }
            .to_json(),
        );
        let mail = adapter(
            &server,
            secrets.clone(),
            Arc::new(FakeAccounts(Some(account()))),
        );
        mail.send("01ACC", &message()).await.expect("sent");

        let stored = TokenSet::from_json(&secrets.value(REF).expect("renewed")).expect("json");
        assert_eq!(stored.access_token, "at-2");
        assert_eq!(stored.refresh_token.as_deref(), Some("rt-2"));
        assert_eq!(stored.scopes, vec!["Mail.Send"], "scopes are kept");
    }

    #[tokio::test]
    async fn a_rejected_message_reports_the_status_and_not_the_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/me/sendMail"))
            .respond_with(ResponseTemplate::new(403).set_body_string("Forbidden"))
            .mount(&server)
            .await;

        let mail = adapter(
            &server,
            FakeSecrets::with(REF, tokens(Some(3600))),
            Arc::new(FakeAccounts(Some(account()))),
        );
        let error = mail.send("01ACC", &message()).await.expect_err("rejected");
        assert_eq!(
            error,
            MailError::Rejected("the provider answered HTTP 403".to_string())
        );
        assert!(!format!("{error}").contains("at-1"));
    }

    #[tokio::test]
    async fn a_missing_or_unreadable_token_asks_for_reauthorisation() {
        let server = MockServer::start().await;
        let accounts = Arc::new(FakeAccounts(Some(account())));

        // Nothing stored.
        let mail = adapter(
            &server,
            FakeSecrets::with("other", "{}".into()),
            accounts.clone(),
        );
        assert_eq!(
            mail.send("01ACC", &message()).await,
            Err(MailError::Reauthorize)
        );

        // Stored but not a token set.
        let mail = adapter(
            &server,
            FakeSecrets::with(REF, "not-json".into()),
            accounts.clone(),
        );
        assert_eq!(
            mail.send("01ACC", &message()).await,
            Err(MailError::Reauthorize)
        );

        // Expired with no refresh token to renew it.
        let mail = adapter(
            &server,
            FakeSecrets::with(
                REF,
                TokenSet {
                    access_token: "at-1".into(),
                    refresh_token: None,
                    expires_at_unix: oauth::now_unix() - 1,
                    scopes: vec!["Mail.Send".into()],
                }
                .to_json(),
            ),
            accounts,
        );
        assert_eq!(
            mail.send("01ACC", &message()).await,
            Err(MailError::Reauthorize)
        );
    }

    #[tokio::test]
    async fn an_unknown_connection_is_not_sent_through() {
        let server = MockServer::start().await;
        let mail = adapter(
            &server,
            FakeSecrets::with(REF, tokens(Some(3600))),
            Arc::new(FakeAccounts(None)),
        );
        assert_eq!(
            mail.send("01ACC", &message()).await,
            Err(MailError::UnknownAccount)
        );

        // A connection of another provider is not Graph's to send.
        let mut other = account();
        other.provider = MailProvider::ImapSmtp;
        let mail = adapter(
            &server,
            FakeSecrets::with(REF, tokens(Some(3600))),
            Arc::new(FakeAccounts(Some(other))),
        );
        assert_eq!(
            mail.send("01ACC", &message()).await,
            Err(MailError::UnknownAccount)
        );
    }

    // --- Scoped read (S8.3, FR-38) ---

    #[tokio::test]
    async fn fetch_inbox_maps_a_graph_page_into_the_pipeline_shape() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/me/mailFolders/inbox/messages"))
            .and(header("authorization", "Bearer at-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "value": [{
                    "id": "AAMk-1",
                    "internetMessageId": "<ABC@acme.example>",
                    "internetMessageHeaders": [
                        {"name": "In-Reply-To", "value": "<sent-1@boba.local>"},
                        {"name": "References", "value": "<x@y.example> <sent-1@boba.local>"}
                    ],
                    "subject": "Your application",
                    "from": {"emailAddress": {"name": "Ana", "address": "ana@acme.example"}},
                    "toRecipients": [{"emailAddress": {"address": "me@example.com"}}],
                    "receivedDateTime": "2024-01-01T10:00:00Z",
                    "body": {"contentType": "html", "content": "<p>Hi &amp; welcome</p><script>x()</script>"},
                    "hasAttachments": true
                }]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let mail = adapter(
            &server,
            FakeSecrets::with(REF, tokens(Some(3600))),
            Arc::new(FakeAccounts(Some(account()))),
        );
        let messages = mail
            .fetch_inbox("01ACC", 1_704_000_000, 1_760_000_000, 50)
            .await
            .expect("read");
        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.provider_id, "AAMk-1");
        assert_eq!(message.message_id.as_deref(), Some("abc@acme.example"));
        assert_eq!(message.in_reply_to, vec!["sent-1@boba.local".to_string()]);
        assert_eq!(
            message.references,
            vec!["x@y.example".to_string(), "sent-1@boba.local".to_string()]
        );
        assert_eq!(message.from_address, "ana@acme.example");
        assert_eq!(message.from_name.as_deref(), Some("Ana"));
        assert_eq!(message.to_addresses, vec!["me@example.com".to_string()]);
        assert_eq!(message.subject, "Your application");
        assert_eq!(message.received_at, 1_704_103_200);
        assert!(message.has_attachments);
        assert!(!message.body_text.contains('<'), "{}", message.body_text);
        assert!(
            message.body_text.contains("Hi & welcome"),
            "{}",
            message.body_text
        );
        assert!(!message.body_text.contains("x()"), "{}", message.body_text);
    }

    #[tokio::test]
    async fn a_page_never_exceeds_the_read_cap() {
        let server = MockServer::start().await;
        let item = |id: &str| {
            json!({
                "id": id,
                "internetMessageId": format!("<{id}@acme.example>"),
                "subject": "reply",
                "from": {"emailAddress": {"address": "ana@acme.example"}},
                "receivedDateTime": "2024-01-01T10:00:00Z",
                "body": {"contentType": "text", "content": "hello"},
                "hasAttachments": false
            })
        };
        Mock::given(method("GET"))
            .and(path("/me/mailFolders/inbox/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "value": [item("m1"), item("m2"), item("m3")]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let mail = adapter(
            &server,
            FakeSecrets::with(REF, tokens(Some(3600))),
            Arc::new(FakeAccounts(Some(account()))),
        );
        let messages = mail
            .fetch_inbox("01ACC", 0, 1_760_000_000, 2)
            .await
            .expect("read");
        assert_eq!(messages.len(), 2, "the cap is enforced on the mapped page");
        assert_eq!(messages[0].provider_id, "m1");
        assert_eq!(messages[1].provider_id, "m2");
    }

    #[tokio::test]
    async fn a_send_only_connection_is_never_read() {
        let server = MockServer::start().await;
        let mut send_only = account();
        send_only.mode = EmailMode::SendOnly;
        send_only.scopes = vec!["Mail.Send".into()];
        let mail = adapter(
            &server,
            FakeSecrets::with(REF, tokens(Some(3600))),
            Arc::new(FakeAccounts(Some(send_only))),
        );
        assert_eq!(
            mail.fetch_inbox("01ACC", 0, 1_760_000_000, 10).await,
            Err(MailError::Reauthorize),
            "no Mail.Read was granted, so no request is made"
        );
    }
}
