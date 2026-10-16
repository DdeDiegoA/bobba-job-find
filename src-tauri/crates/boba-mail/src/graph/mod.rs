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
use boba_app::ports::repo::email::{EmailAccount, EmailRepo, MailProvider};
use boba_app::ports::secret::SecretPort;
use serde_json::{Value, json};

use crate::oauth::{self, OAuthError, ProviderEndpoints, TokenSet};

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
        let refresh = tokens.refresh_token.ok_or(MailError::Reauthorize)?;
        let response = oauth::refresh_tokens(
            &self.http,
            &self.endpoints,
            &account.client_id,
            refresh.as_str(),
        )
        .await
        .map_err(|_| MailError::Reauthorize)?;

        let renewed = TokenSet::new(&response, &tokens.scopes);
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
        Mock::given(method("POST"))
            .and(path("/me/sendMail"))
            .and(header("authorization", "Bearer at-1"))
            .and(body_json(&expected))
            .respond_with(ResponseTemplate::new(202))
            .expect(1)
            .mount(&server)
            .await;

        let secrets = FakeSecrets::with(REF, tokens(Some(3600)));
        let mail = adapter(
            &server,
            secrets.clone(),
            Arc::new(FakeAccounts(Some(account()))),
        );
        mail.send("01ACC", &message()).await.expect("sent");
        // A usable token is used as-is: the stored set is not rewritten.
        assert_eq!(
            TokenSet::from_json(&secrets.value(REF).expect("stored")).expect("json"),
            TokenSet::from_json(&tokens(Some(3600))).expect("json")
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
}
