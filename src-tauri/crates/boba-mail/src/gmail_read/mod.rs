//! Gmail read over the Gmail API (S8.5, FR-34, architecture §11).
//!
//! Reading Gmail needs a scope the bundled client does not ask for, so this
//! reader only ever works for a connection the user authorised with **their own
//! Google client** (BYO) and a read+send mode. Boba's bundled client stays
//! send-only until CASA (AC1): a bundled connection is refused here even if its
//! stored row claims otherwise, so the gap in the console review can never be
//! papered over by a stale row.
//!
//! The reader is a *reader*: it lists the inbox since a cursor, fetches each
//! message's headers and a bounded text body, and returns [`RawMessage`]s. It
//! never marks, moves, deletes or sends anything, and it fetches no attachment
//! bytes. The scoped sync (S8.3) decides what is stored.

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use boba_app::ports::mail::MailError;
use boba_app::ports::repo::email::{
    ClientIdKind, EmailAccount, EmailMode, EmailRepo, MailProvider,
};
use boba_app::ports::secret::SecretPort;
use mail_parser::DateTime;
use serde_json::Value;

use crate::eml;
use crate::gmail::{GMAIL_SEND_SCOPE, GmailSecret};
use crate::oauth::{self, OAuthError, ProviderEndpoints};
use crate::sync::{RawMessage, normalize_message_id};

/// The scope that lets the reader list and get messages (AC1: only with the
/// user's own client, until CASA).
pub const GMAIL_READ_SCOPE: &str = "https://www.googleapis.com/auth/gmail.readonly";

/// How many messages one pass may return.
pub const READ_PAGE_SIZE: usize = 100;

/// How many ids one listing call asks for while the reader walks down to the
/// oldest end of the window.
const SCAN_CHUNK: usize = 100;

/// How far down the reader will walk to find the bottom of a window. Each round
/// asks for the chunk below the previous one, so a busy mailbox settles in a
/// couple of rounds instead of paging through thousands of unrelated messages.
const MAX_DESCENT_ROUNDS: usize = 12;

/// The largest answer accepted for one message. Gmail returns bodies inline with
/// `format=full`, so a single oversized message is refused before a run holds it
/// all in memory.
const MAX_MESSAGE_BYTES: u64 = 8 * 1024 * 1024;

/// The largest answer accepted for one listing page.
const MAX_LIST_BYTES: u64 = 2 * 1024 * 1024;

/// Renew this long before the token stops working, so a slow request cannot
/// start with a token that expires mid-flight.
const REFRESH_SKEW_SECS: u64 = 60;

/// The Google scopes a Gmail connection asks for, per mode (AC1, NFR-5).
///
/// Send-only asks for `gmail.send` and nothing else. Read+send asks for the read
/// scope **in addition**: it is only ever requested for the user's own client.
pub fn requested_scopes(mode: EmailMode) -> Vec<&'static str> {
    match mode {
        EmailMode::SendOnly => vec![GMAIL_SEND_SCOPE],
        EmailMode::ReadSend => vec![GMAIL_SEND_SCOPE, GMAIL_READ_SCOPE],
    }
}

/// Whether `account`'s grant covers `mode` (AC1, NFR-5).
///
/// Gmail's scopes are Google's, not Graph's: `Mail.Read` never appears in a
/// Gmail grant and `gmail.readonly` never appears in an Outlook one, so the core
/// asks this instead of [`EmailAccount::covers`] for a Gmail row. Reading also
/// needs the user's own client (the bundled one stays send-only until CASA), so
/// it is part of what read+send coverage means here.
pub fn covers(account: &EmailAccount, mode: EmailMode) -> bool {
    if account.provider != MailProvider::Gmail {
        return false;
    }
    let has = |scope: &str| account.scopes.iter().any(|granted| granted == scope);
    match mode {
        EmailMode::SendOnly => has(GMAIL_SEND_SCOPE),
        EmailMode::ReadSend => {
            has(GMAIL_SEND_SCOPE)
                && has(GMAIL_READ_SCOPE)
                && account.client_id_kind == ClientIdKind::Byo
        }
    }
}

/// Whether this connection may read its mailbox through the Gmail API (AC1).
///
/// It must be connected read+send **and** its grant must cover that mode. The
/// bundled client is refused outright until CASA.
pub fn read_capable(account: &EmailAccount) -> bool {
    account.mode == EmailMode::ReadSend && covers(account, EmailMode::ReadSend)
}

/// The Gmail API reader. Holds no tokens: they are read from the keychain per
/// pass and never leave it (AD-13).
pub struct GmailReader {
    http: reqwest::Client,
    endpoints: ProviderEndpoints,
    secrets: Arc<dyn SecretPort>,
    accounts: Arc<dyn EmailRepo>,
}

impl GmailReader {
    /// The reader as the app wires it: Google's endpoints and a client that
    /// identifies Boba honestly.
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

    /// The reader against explicit endpoints (tests, or a wiremock server).
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

    /// Read the inbox messages received since `since` (Unix seconds) up to
    /// `limit` (FR-38). See the module docs for what is and is not read.
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
        // AC1: only the user's own client may read; the bundled client stays
        // send-only until CASA, whatever the stored row says.
        if !read_capable(&account) {
            return Err(MailError::Reauthorize);
        }
        let limit = limit.min(READ_PAGE_SIZE);
        if limit == 0 {
            return Ok(Vec::new());
        }

        let token = self.access_token(&account).await?;
        let ids = self.list_ids(&token, since, limit).await?;
        let mut messages = Vec::with_capacity(ids.len());
        for id in ids.into_iter().take(limit) {
            let body = self.get_message(&token, &id).await?;
            if let Some(message) = read_message(&body, now) {
                messages.push(message);
            }
        }
        Ok(messages)
    }

    /// A usable access token for `account`, renewed when it is near expiry
    /// (AD-13: the tokens never leave the keychain).
    async fn access_token(&self, account: &EmailAccount) -> Result<String, MailError> {
        let stored = self
            .secrets
            .get(&account.secret_ref)
            .map_err(|_| MailError::Reauthorize)?
            .ok_or(MailError::Reauthorize)?;
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
            // A refresh that omits the refresh token keeps the stored one (AD-13).
            tokens: secret.tokens.renewed(&response),
            client_secret: secret.client_secret,
        };
        self.secrets
            .put(&account.secret_ref, &renewed.to_json())
            .map_err(|_| MailError::Reauthorize)?;
        Ok(renewed.tokens.access_token)
    }

    /// The ids a pass reads: the **oldest** messages of the window, oldest
    /// first, at most `limit`.
    ///
    /// Gmail lists newest-first and has no ascending order, so the bottom of the
    /// window is found by walking down in chunks: list a chunk from the top of
    /// the remaining window; while it is full, there may be older mail, so move
    /// the window's top below its oldest message and list again. The chunk that
    /// is not full (or one the budget stops at) holds the oldest messages the
    /// pass can see, and its tail is what is read.
    ///
    /// Reading the old end is what keeps the sync safe: the pass cursor moves to
    /// the newest message *returned*, so anything skipped stays newer than the
    /// cursor and is looked at again on the next pass. Reading the newest page
    /// instead would push the cursor past unread mail.
    async fn list_ids(
        &self,
        token: &str,
        since: i64,
        limit: usize,
    ) -> Result<Vec<String>, MailError> {
        let mut before: Option<i64> = None;
        let mut page = self.list_page(token, since, before, SCAN_CHUNK).await?;
        let mut rounds = 0;
        while page.len() == SCAN_CHUNK && rounds < MAX_DESCENT_ROUNDS {
            let Some(oldest) = page.last() else { break };
            let Some(date) = self.received_at(token, oldest).await? else {
                break;
            };
            // Gmail's `before:` is second-granular but dates can disagree with
            // the receipt time; a round that would not move is the bottom.
            if before.is_some_and(|previous| date >= previous) {
                break;
            }
            before = Some(date);
            rounds += 1;
            let deeper = self.list_page(token, since, before, SCAN_CHUNK).await?;
            if deeper.is_empty() {
                // The previous chunk *was* the bottom of the window.
                break;
            }
            page = deeper;
        }
        // The page is newest-first: its tail is the oldest end, reversed to the
        // order the sync reads.
        let take = limit.min(page.len());
        let mut oldest: Vec<String> = page.split_off(page.len() - take);
        oldest.reverse();
        Ok(oldest)
    }

    /// One listing call: at most `limit` ids of the inbox window, newest first.
    async fn list_page(
        &self,
        token: &str,
        since: i64,
        before: Option<i64>,
        limit: usize,
    ) -> Result<Vec<String>, MailError> {
        let mut url = url::Url::parse(&format!("{}/users/me/messages", self.endpoints.api_base))
            .map_err(|_| MailError::Transport("the provider URL is not valid".to_string()))?;
        // Gmail's search takes epoch-second bounds; the sync filters by the
        // cursor and the message date again, so a coarse bound is safe.
        let query = match before {
            Some(before) => format!("after:{since} before:{before}"),
            None => format!("after:{since}"),
        };
        url.query_pairs_mut()
            .append_pair("labelIds", "INBOX")
            .append_pair("q", &query)
            .append_pair("maxResults", &limit.to_string());
        let body = self.request(url.as_str(), token, MAX_LIST_BYTES).await?;
        Ok(body
            .get("messages")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.get("id").and_then(Value::as_str))
                    .map(str::to_string)
                    .take(limit)
                    .collect()
            })
            .unwrap_or_default())
    }

    /// One message's receipt date (Unix seconds), read from a metadata answer
    /// (`internalDate`), without fetching its body.
    async fn received_at(&self, token: &str, id: &str) -> Result<Option<i64>, MailError> {
        let mut url = url::Url::parse(&format!("{}/", self.endpoints.api_base))
            .map_err(|_| MailError::Transport("the provider URL is not valid".to_string()))?;
        url.path_segments_mut()
            .map_err(|_| MailError::Transport("the provider URL is not valid".to_string()))?
            .extend(["users", "me", "messages", id]);
        url.query_pairs_mut().append_pair("format", "metadata");
        let body = self.request(url.as_str(), token, MAX_LIST_BYTES).await?;
        Ok(internal_date(&body)
            .map(|milliseconds| milliseconds / 1000)
            .filter(|timestamp| *timestamp > 0))
    }

    /// One message's headers and a bounded text body (`format=full`).
    async fn get_message(&self, token: &str, id: &str) -> Result<Value, MailError> {
        let mut url = url::Url::parse(&format!("{}/", self.endpoints.api_base))
            .map_err(|_| MailError::Transport("the provider URL is not valid".to_string()))?;
        url.path_segments_mut()
            .map_err(|_| MailError::Transport("the provider URL is not valid".to_string()))?
            .extend(["users", "me", "messages", id]);
        url.query_pairs_mut().append_pair("format", "full");
        self.request(url.as_str(), token, MAX_MESSAGE_BYTES).await
    }

    /// A GET bounded in bytes and in error detail: no body is echoed (it can
    /// repeat the message), and no token ever appears (AD-13).
    async fn request(&self, url: &str, token: &str, cap: u64) -> Result<Value, MailError> {
        let response = self
            .http
            .get(url)
            .bearer_auth(token)
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
        let too_large = || MailError::Rejected("the inbox answer is too large".to_string());
        if response.content_length().is_some_and(|length| length > cap) {
            return Err(too_large());
        }
        let mut response = response;
        let mut body: Vec<u8> = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| MailError::Transport("the inbox answer was cut off".to_string()))?
        {
            if body.len() as u64 + chunk.len() as u64 > cap {
                return Err(too_large());
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body)
            .map_err(|_| MailError::Rejected("the inbox answer could not be read".to_string()))
    }
}

/// Map one `users.messages.get` answer onto the pipeline's shape. A message
/// without an id is skipped rather than failing the whole read.
fn read_message(item: &Value, now: i64) -> Option<RawMessage> {
    let provider_id = item.get("id")?.as_str()?.to_string();
    let payload = item.get("payload");
    let headers = payload
        .and_then(|payload| payload.get("headers"))
        .and_then(Value::as_array);

    let from = header(headers, "From")
        .map(|value| parse_address(&value))
        .unwrap_or_default();
    let to_addresses = header(headers, "To")
        .map(|value| {
            split_addresses(&value)
                .into_iter()
                .map(|(address, _)| address)
                .collect()
        })
        .unwrap_or_default();
    let subject = header(headers, "Subject")
        .unwrap_or_default()
        .trim()
        .to_string();
    let message_id = header(headers, "Message-ID")
        .map(|value| normalize_message_id(&value))
        .filter(|id| !id.is_empty());
    let received_at = internal_date(item)
        .map(|milliseconds| milliseconds / 1000)
        .filter(|timestamp| *timestamp > 0)
        .or_else(|| {
            header(headers, "Date")
                .and_then(|value| DateTime::parse_rfc822(&value))
                .map(|date| date.to_timestamp())
                .filter(|timestamp| *timestamp > 0)
        })
        .unwrap_or(now);

    let (body_text, has_attachments) = payload.map(read_body).unwrap_or_default();

    Some(RawMessage {
        provider_id,
        message_id,
        in_reply_to: header_ids(headers, "In-Reply-To"),
        references: header_ids(headers, "References"),
        from_address: from.0,
        from_name: from.1,
        to_addresses,
        subject,
        received_at,
        body_text,
        has_attachments,
    })
}

/// The plain text of a message payload (the first `text/plain` part, else the
/// HTML stripped), and whether any part is an attachment.
fn read_body(payload: &Value) -> (String, bool) {
    let mut text: Option<String> = None;
    let mut html: Option<String> = None;
    let mut attachments = false;
    collect_parts(payload, &mut text, &mut html, &mut attachments);
    let body = match (text, html) {
        (Some(text), _) => eml::body_from("text", &text),
        (None, Some(html)) => eml::body_from("html", &html),
        (None, None) => String::new(),
    };
    (body, attachments)
}

fn collect_parts(
    part: &Value,
    text: &mut Option<String>,
    html: &mut Option<String>,
    attachments: &mut bool,
) {
    let mime = part.get("mimeType").and_then(Value::as_str).unwrap_or("");
    let filename = part.get("filename").and_then(Value::as_str).unwrap_or("");
    let is_attachment = !filename.trim().is_empty()
        || part
            .get("body")
            .and_then(|body| body.get("attachmentId"))
            .is_some_and(|id| !id.is_null());
    *attachments |= is_attachment;

    // Attachment bytes are never read, even when the API inlined them.
    if !is_attachment {
        let data = part
            .get("body")
            .and_then(|body| body.get("data"))
            .and_then(Value::as_str)
            .and_then(decode_part);
        if let Some(bytes) = data {
            let content = String::from_utf8_lossy(&bytes).to_string();
            if mime.eq_ignore_ascii_case("text/plain") && text.is_none() {
                *text = Some(content);
            } else if mime.eq_ignore_ascii_case("text/html") && html.is_none() {
                *html = Some(content);
            }
        }
    }

    if let Some(parts) = part.get("parts").and_then(Value::as_array) {
        for child in parts {
            collect_parts(child, text, html, attachments);
        }
    }
}

/// Gmail hands part bodies as URL-safe base64, padded or not.
fn decode_part(data: &str) -> Option<Vec<u8>> {
    let compact: String = data.chars().filter(|c| !c.is_whitespace()).collect();
    URL_SAFE_NO_PAD
        .decode(&compact)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(&compact))
        .or_else(|_| base64::engine::general_purpose::STANDARD.decode(&compact))
        .ok()
}

/// `internalDate` in milliseconds. The API documents a string; some responses
/// carry a number, so both are read.
fn internal_date(item: &Value) -> Option<i64> {
    match item.get("internalDate") {
        Some(Value::String(text)) => text.trim().parse::<i64>().ok(),
        Some(Value::Number(number)) => number.as_i64(),
        _ => None,
    }
}

/// One header's value, case-insensitively.
fn header(headers: Option<&Vec<Value>>, name: &str) -> Option<String> {
    headers
        .and_then(|list| {
            list.iter()
                .find(|header| {
                    header
                        .get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|value| value.eq_ignore_ascii_case(name))
                })
                .and_then(|header| header.get("value"))
                .and_then(Value::as_str)
        })
        .map(str::to_string)
}

/// `In-Reply-To`/`References` as bare message ids: the header may carry a list,
/// and transporters re-wrap and re-case the ids in practice.
fn header_ids(headers: Option<&Vec<Value>>, name: &str) -> Vec<String> {
    header(headers, name)
        .map(|value| {
            value
                .split_whitespace()
                .map(normalize_message_id)
                .filter(|id| !id.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Split an address-list header on the commas that separate addresses, not
/// the ones inside a quoted display name or an angle-bracketed address.
fn split_addresses(value: &str) -> Vec<(String, Option<String>)> {
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();
    let (mut quotes, mut angles) = (false, false);
    for character in value.chars() {
        match character {
            '"' => {
                quotes = !quotes;
                current.push(character);
            }
            '<' if !quotes => {
                angles = true;
                current.push(character);
            }
            '>' if !quotes => {
                angles = false;
                current.push(character);
            }
            ',' | ';' if !quotes && !angles => {
                parts.push(std::mem::take(&mut current));
            }
            _ => current.push(character),
        }
    }
    parts.push(current);
    parts
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .map(|part| parse_address(&part))
        .filter(|(address, _)| !address.is_empty())
        .collect()
}

/// `Display Name <address>` (or a bare address) as `(address, name)`.
fn parse_address(value: &str) -> (String, Option<String>) {
    let value = value.trim();
    if let Some((before, rest)) = value.rsplit_once('<') {
        let address = rest
            .split('>')
            .next()
            .unwrap_or_default()
            .trim()
            .to_string();
        let name = before.trim().trim_matches('"').trim().to_string();
        return (address, (!name.is_empty()).then_some(name));
    }
    (value.trim_matches('"').trim().to_string(), None)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use boba_app::ports::repo::email::EmailRepoError;
    use boba_app::ports::secret::SecretError;
    use serde_json::json;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::oauth::TokenSet;

    const NOW: i64 = 1_760_000_000;
    const DAY: i64 = 24 * 60 * 60;
    const ACC: &str = "01GMAIL";
    const REF: &str = "mail:01GMAIL";

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
            unimplemented!("not used by the read path")
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
            unimplemented!("not used by the read path")
        }

        fn delete_account(&self, _id: &str) -> Result<bool, EmailRepoError> {
            unimplemented!("not used by the read path")
        }
    }

    /// A connection the user authorised with their own client, read+send (AC1).
    fn account() -> EmailAccount {
        EmailAccount {
            id: "01GMAIL".to_string(),
            provider: MailProvider::Gmail,
            address: "me@gmail.com".to_string(),
            mode: EmailMode::ReadSend,
            client_id: "me.apps.googleusercontent.com".to_string(),
            client_id_kind: ClientIdKind::Byo,
            scopes: vec![GMAIL_SEND_SCOPE.to_string(), GMAIL_READ_SCOPE.to_string()],
            secret_ref: REF.to_string(),
            connected_at: "2026-10-17T00:00:00Z".to_string(),
            updated_at: "2026-10-17T00:00:00Z".to_string(),
        }
    }

    fn stored() -> String {
        GmailSecret {
            tokens: TokenSet {
                access_token: "at-1".to_string(),
                refresh_token: Some("rt-1".to_string()),
                expires_at_unix: oauth::now_unix() + 3600,
                scopes: vec![GMAIL_SEND_SCOPE.to_string(), GMAIL_READ_SCOPE.to_string()],
            },
            client_secret: Some("GOCSPX-public-value".to_string()),
        }
        .to_json()
    }

    fn reader(server: &MockServer, secrets: Arc<dyn SecretPort>) -> GmailReader {
        reader_for(server, secrets, account())
    }

    fn reader_for(
        server: &MockServer,
        secrets: Arc<dyn SecretPort>,
        account: EmailAccount,
    ) -> GmailReader {
        GmailReader::with_endpoints(
            reqwest::Client::new(),
            ProviderEndpoints {
                api_base: server.uri(),
                authorize: format!("{}/authorize", server.uri()),
                token: format!("{}/token", server.uri()),
                revoke: Some(format!("{}/revoke", server.uri())),
            },
            secrets,
            Arc::new(FakeAccounts(Some(account))),
        )
    }

    fn encoded(text: &str) -> String {
        URL_SAFE_NO_PAD.encode(text.as_bytes())
    }

    /// One `users.messages.get` answer: a multipart/alternative message.
    fn message_body() -> Value {
        json!({
            "id": "m1",
            "threadId": "t1",
            "labelIds": ["INBOX"],
            "internalDate": ((NOW - DAY) * 1000).to_string(),
            "snippet": "Hello there",
            "payload": {
                "partId": "",
                "mimeType": "multipart/alternative",
                "filename": "",
                "headers": [
                    {"name": "From", "value": "Acme Talent <talent@acme.example>"},
                    {"name": "To", "value": "me@gmail.com"},
                    {"name": "Subject", "value": "Application update"},
                    {"name": "Message-ID", "value": "<reply-1@acme.example>"},
                    {"name": "In-Reply-To", "value": "<sent-1@boba.local>"},
                    {"name": "References", "value": "<older@acme.example>\r\n <sent-1@boba.local>"},
                    {"name": "Date", "value": "Mon, 16 Oct 2026 10:00:00 +0000"}
                ],
                "body": {"size": 0},
                "parts": [
                    {
                        "partId": "0",
                        "mimeType": "text/plain",
                        "filename": "",
                        "headers": [],
                        "body": {"size": 11, "data": encoded("Hello there")}
                    },
                    {
                        "partId": "1",
                        "mimeType": "text/html",
                        "filename": "",
                        "headers": [],
                        "body": {"size": 20, "data": encoded("<p>Hello there</p>")}
                    }
                ]
            },
            "sizeEstimate": 2048
        })
    }

    // --- AC1: the BYO read+send connection reads through the Gmail API --------

    #[tokio::test]
    async fn a_byo_read_send_connection_reads_the_inbox_through_the_gmail_api() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/me/messages"))
            .and(query_param("labelIds", "INBOX"))
            .and(header("authorization", "Bearer at-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "messages": [{"id": "m1", "threadId": "t1"}],
                "resultSizeEstimate": 1
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/users/me/messages/m1"))
            .and(query_param("format", "full"))
            .and(header("authorization", "Bearer at-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(message_body()))
            .expect(1)
            .mount(&server)
            .await;

        let reader = reader(
            &server,
            FakeSecrets::with(REF, stored()) as Arc<dyn SecretPort>,
        );
        let messages = reader
            .fetch_inbox(ACC, NOW - DAY, NOW, 10)
            .await
            .expect("read");

        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.provider_id, "m1");
        assert_eq!(message.message_id.as_deref(), Some("reply-1@acme.example"));
        assert_eq!(message.in_reply_to, vec!["sent-1@boba.local"]);
        assert_eq!(
            message.references,
            vec!["older@acme.example", "sent-1@boba.local"]
        );
        assert_eq!(message.from_address, "talent@acme.example");
        assert_eq!(message.from_name.as_deref(), Some("Acme Talent"));
        assert_eq!(message.to_addresses, vec!["me@gmail.com"]);
        assert_eq!(message.subject, "Application update");
        assert_eq!(message.received_at, NOW - DAY);
        assert_eq!(message.body_text, "Hello there");
        assert!(!message.has_attachments);

        // The listing asks for the inbox only, since the cursor.
        let requests = server.received_requests().await.expect("requests");
        let list = requests
            .iter()
            .find(|request| request.url.path() == "/users/me/messages")
            .expect("list request");
        let query: HashMap<_, _> = list.url.query_pairs().into_owned().collect();
        assert_eq!(query.get("q").map(String::as_str), Some("after:1759913600"));
    }

    // --- AC1: only the user's own client with a read grant may read ----------

    #[test]
    fn only_a_byo_read_send_connection_with_the_read_scope_is_read_capable() {
        assert!(read_capable(&account()));

        // The bundled client stays send-only until CASA (AC1).
        let mut bundled = account();
        bundled.client_id_kind = ClientIdKind::Bundled;
        assert!(!read_capable(&bundled), "bundled client must not read");

        let mut send_only = account();
        send_only.mode = EmailMode::SendOnly;
        assert!(!read_capable(&send_only));

        // A mode switch without re-consent never granted the read scope.
        let mut without_scope = account();
        without_scope.scopes = vec![GMAIL_SEND_SCOPE.to_string()];
        assert!(!read_capable(&without_scope));

        let mut outlook = account();
        outlook.provider = MailProvider::MicrosoftGraph;
        assert!(!read_capable(&outlook));
    }

    /// R3: Gmail coverage is Google's scopes, not Graph's `Mail.Read`. The core
    /// asks this before switching a connection's mode.
    #[test]
    fn gmail_coverage_is_googles_scopes_not_graphs() {
        let mut send_only = account();
        send_only.mode = EmailMode::SendOnly;
        send_only.scopes = vec![GMAIL_SEND_SCOPE.to_string()];
        assert!(covers(&send_only, EmailMode::SendOnly));
        assert!(!covers(&send_only, EmailMode::ReadSend));

        let read = account();
        assert!(covers(&read, EmailMode::ReadSend));
        assert!(
            covers(&read, EmailMode::SendOnly),
            "a read+send grant covers sending too"
        );

        let mut bundled = account();
        bundled.client_id_kind = ClientIdKind::Bundled;
        assert!(
            !covers(&bundled, EmailMode::ReadSend),
            "the bundled client never covers reading"
        );

        // An Outlook row carries Graph scopes; Gmail coverage says nothing about it.
        let mut outlook = account();
        outlook.provider = MailProvider::MicrosoftGraph;
        outlook.scopes = vec!["Mail.Send".to_string()];
        assert!(!covers(&outlook, EmailMode::SendOnly));
    }

    #[test]
    fn the_mode_decides_the_scopes_a_gmail_connection_asks_for() {
        assert_eq!(
            requested_scopes(EmailMode::SendOnly),
            vec![GMAIL_SEND_SCOPE]
        );
        assert_eq!(
            requested_scopes(EmailMode::ReadSend),
            vec![GMAIL_SEND_SCOPE, GMAIL_READ_SCOPE],
            "read is added next to send, never instead of it"
        );
        assert_eq!(
            GMAIL_READ_SCOPE,
            "https://www.googleapis.com/auth/gmail.readonly"
        );
    }

    /// The refusal happens before anything is asked of Google: no request, no
    /// token, no read.
    #[tokio::test]
    async fn a_connection_that_may_not_read_never_reaches_the_api() {
        let server = MockServer::start().await;
        let mut bundled = account();
        bundled.client_id_kind = ClientIdKind::Bundled;

        for refused in [
            bundled,
            {
                let mut send_only = account();
                send_only.mode = EmailMode::SendOnly;
                send_only
            },
            {
                let mut without_scope = account();
                without_scope.scopes = vec![GMAIL_SEND_SCOPE.to_string()];
                without_scope
            },
            {
                let mut outlook = account();
                outlook.provider = MailProvider::MicrosoftGraph;
                outlook
            },
        ] {
            let reader = reader_for(
                &server,
                FakeSecrets::with(REF, stored()) as Arc<dyn SecretPort>,
                refused.clone(),
            );
            assert_eq!(
                reader.fetch_inbox(ACC, NOW - DAY, NOW, 10).await,
                Err(MailError::Reauthorize),
                "{refused:?}"
            );
        }
        assert!(
            server
                .received_requests()
                .await
                .expect("requests")
                .is_empty()
        );
    }

    // --- The token (AD-13) --------------------------------------------------

    #[tokio::test]
    async fn an_expired_token_is_refreshed_with_the_public_client_secret() {
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
        Mock::given(method("GET"))
            .and(path("/users/me/messages"))
            .and(header("authorization", "Bearer at-2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "messages": [] })))
            .expect(1)
            .mount(&server)
            .await;

        let expired = GmailSecret {
            tokens: TokenSet {
                access_token: "at-1".to_string(),
                refresh_token: Some("rt-1".to_string()),
                expires_at_unix: oauth::now_unix() - 1,
                scopes: vec![GMAIL_SEND_SCOPE.to_string(), GMAIL_READ_SCOPE.to_string()],
            },
            client_secret: Some("GOCSPX-public-value".to_string()),
        }
        .to_json();
        let secrets = FakeSecrets::with(REF, expired);
        let reader = reader(&server, secrets.clone() as Arc<dyn SecretPort>);
        reader
            .fetch_inbox(ACC, NOW - DAY, NOW, 10)
            .await
            .expect("read");

        let renewed = GmailSecret::from_json(&secrets.value(REF).expect("renewed")).expect("json");
        assert_eq!(renewed.tokens.access_token, "at-2");
        assert_eq!(renewed.tokens.refresh_token.as_deref(), Some("rt-2"));
        assert_eq!(
            renewed.client_secret.as_deref(),
            Some("GOCSPX-public-value"),
            "the public client secret is kept for the next pass"
        );
    }

    /// R1: Google does not re-issue the refresh token on a refresh, so the one
    /// in the keychain must survive — otherwise the next expiry asks the user to
    /// authorise again, every hour.
    #[tokio::test]
    async fn a_refresh_that_omits_the_refresh_token_keeps_the_stored_one() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "at-2",
                "expires_in": 1
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/users/me/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "messages": [] })))
            .mount(&server)
            .await;

        let expired = GmailSecret {
            tokens: TokenSet {
                access_token: "at-1".to_string(),
                refresh_token: Some("rt-1".to_string()),
                expires_at_unix: oauth::now_unix() - 1,
                scopes: vec![GMAIL_SEND_SCOPE.to_string(), GMAIL_READ_SCOPE.to_string()],
            },
            client_secret: Some("GOCSPX-public-value".to_string()),
        }
        .to_json();
        let secrets = FakeSecrets::with(REF, expired);
        let reader = reader(&server, secrets.clone() as Arc<dyn SecretPort>);

        // Two passes: the second one has to renew again, from the stored token.
        for _ in 0..2 {
            reader
                .fetch_inbox(ACC, NOW - DAY, NOW, 10)
                .await
                .expect("read");
            assert_eq!(
                GmailSecret::from_json(&secrets.value(REF).expect("stored"))
                    .expect("json")
                    .tokens
                    .refresh_token
                    .as_deref(),
                Some("rt-1"),
                "the refresh token must survive a refresh that omits it"
            );
        }

        let requests = server.received_requests().await.expect("requests");
        let refreshes = requests
            .iter()
            .filter(|request| request.url.path() == "/token")
            .count();
        assert_eq!(refreshes, 2, "the second pass renewed on its own");
        assert!(
            String::from_utf8_lossy(&requests[0].body).contains("refresh_token=rt-1"),
            "the stored token is what is presented"
        );
    }

    #[tokio::test]
    async fn a_missing_or_unreadable_entry_asks_for_reauthorisation() {
        let server = MockServer::start().await;
        for entry in [None, Some("{}"), Some("not json")] {
            let secrets: Arc<dyn SecretPort> = match entry {
                Some(value) => FakeSecrets::with(REF, value.to_string()),
                None => FakeSecrets::with("other", stored()),
            };
            let reader = reader(&server, secrets);
            assert_eq!(
                reader.fetch_inbox(ACC, NOW - DAY, NOW, 10).await,
                Err(MailError::Reauthorize),
                "entry: {entry:?}"
            );
        }
    }

    // --- Mapping a message --------------------------------------------------

    #[tokio::test]
    async fn an_html_only_message_is_reduced_to_text_and_never_yields_attachments() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/me/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "messages": [{"id": "m1"}]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/users/me/messages/m1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "m1",
                "payload": {
                    "mimeType": "multipart/mixed",
                    "headers": [
                        {"name": "from", "value": "no-reply@greenhouse.io"},
                        {"name": "subject", "value": "  Update  "}
                    ],
                    "parts": [
                        {
                            "mimeType": "text/html",
                            "filename": "",
                            "body": {"data": encoded("<p>Hi <b>there</b></p>")}
                        },
                        {
                            "mimeType": "application/pdf",
                            "filename": "cv.pdf",
                            "body": {"attachmentId": "a1", "size": 4096}
                        }
                    ]
                }
            })))
            .mount(&server)
            .await;

        let messages = reader(
            &server,
            FakeSecrets::with(REF, stored()) as Arc<dyn SecretPort>,
        )
        .fetch_inbox(ACC, NOW - DAY, NOW, 10)
        .await
        .expect("read");

        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.body_text, "Hi there");
        assert!(message.has_attachments, "an attachment part is reported");
        assert!(!message.body_text.contains("PDF"), "{}", message.body_text);
        assert_eq!(message.from_address, "no-reply@greenhouse.io");
        assert_eq!(message.from_name, None);
        assert_eq!(message.subject, "Update");
        assert_eq!(message.message_id, None);
        // No internalDate: the Date header is not there either, so the pass clock
        // is the safe fallback.
        assert_eq!(message.received_at, NOW);
    }

    #[test]
    fn a_quoted_display_name_with_a_comma_is_one_address() {
        let (address, name) = parse_address("\"Doe, Jane\" <jane@acme.example>");
        assert_eq!(address, "jane@acme.example");
        assert_eq!(name.as_deref(), Some("Doe, Jane"));

        let (address, name) = parse_address("talent@acme.example");
        assert_eq!(address, "talent@acme.example");
        assert_eq!(name, None);

        let list = split_addresses("\"Doe, Jane\" <jane@acme.example>, me@gmail.com");
        assert_eq!(
            list,
            vec![
                (
                    "jane@acme.example".to_string(),
                    Some("Doe, Jane".to_string())
                ),
                ("me@gmail.com".to_string(), None),
            ]
        );
    }

    // --- Bounds and failures ------------------------------------------------

    #[tokio::test]
    async fn a_pass_never_reads_more_than_the_limit_it_was_given() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/me/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "messages": [
                    {"id": "m1"}, {"id": "m2"}, {"id": "m3"}, {"id": "m4"}
                ]
            })))
            .mount(&server)
            .await;
        // The list is newest-first, so the two oldest are m4 and m3.
        for id in ["m4", "m3"] {
            Mock::given(method("GET"))
                .and(path(format!("/users/me/messages/{id}")))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "id": id,
                    "internalDate": (NOW - DAY).to_string(),
                    "payload": {"mimeType": "text/plain", "body": {"data": encoded("hi")}}
                })))
                .expect(1)
                .mount(&server)
                .await;
        }

        let messages = reader(
            &server,
            FakeSecrets::with(REF, stored()) as Arc<dyn SecretPort>,
        )
        .fetch_inbox(ACC, NOW - DAY, NOW, 2)
        .await
        .expect("read");
        assert_eq!(
            messages
                .iter()
                .map(|m| m.provider_id.as_str())
                .collect::<Vec<_>>(),
            vec!["m4", "m3"],
            "the oldest two of the window, oldest first"
        );
        let requests = server.received_requests().await.expect("requests");
        let gets = requests
            .iter()
            .filter(|request| request.url.path() != "/users/me/messages")
            .count();
        assert_eq!(gets, 2, "a third message is never fetched");
    }

    /// R2: Gmail lists newest-first and offers no ascending order. A pass reads
    /// the OLDEST messages of the window — otherwise the cursor jumps to the
    /// newest one it saw and every message older than the page it read is
    /// skipped for good.
    #[tokio::test]
    async fn a_pass_walks_to_the_oldest_messages_of_the_window() {
        let server = MockServer::start().await;
        let since = NOW - DAY;
        let boundary = NOW - 5 * DAY;
        let page: Vec<Value> = (0..SCAN_CHUNK)
            .map(|index| json!({ "id": format!("n{index:03}") }))
            .collect();
        // A full page: there may be older mail, so the window's top moves below
        // its oldest message (n099).
        Mock::given(method("GET"))
            .and(path("/users/me/messages"))
            .and(query_param("q", format!("after:{since}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "messages": page })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/users/me/messages/n099"))
            .and(query_param("format", "metadata"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "n099",
                "internalDate": (boundary * 1000).to_string()
            })))
            .expect(1)
            .mount(&server)
            .await;
        // The bottom of the window: two messages, newest first.
        Mock::given(method("GET"))
            .and(path("/users/me/messages"))
            .and(query_param("q", format!("after:{since} before:{boundary}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "messages": [{"id": "b1"}, {"id": "b0"}]
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/users/me/messages/b0"))
            .and(query_param("format", "full"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "b0",
                "internalDate": ((NOW - 40 * DAY) * 1000).to_string(),
                "payload": {"mimeType": "text/plain", "body": {"data": encoded("oldest")}}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let messages = reader(
            &server,
            FakeSecrets::with(REF, stored()) as Arc<dyn SecretPort>,
        )
        .fetch_inbox(ACC, since, NOW, 1)
        .await
        .expect("read");

        assert_eq!(
            messages
                .iter()
                .map(|m| m.provider_id.as_str())
                .collect::<Vec<_>>(),
            vec!["b0"],
            "the oldest message of the window is the one read"
        );
    }

    /// A window that fits exactly in one page is its own bottom: the extra walk
    /// finds nothing older and the page's tail is what is read — never a loop.
    #[tokio::test]
    async fn a_window_that_ends_at_the_page_boundary_reads_its_tail() {
        let server = MockServer::start().await;
        let since = NOW - DAY;
        let boundary = NOW - 5 * DAY;
        let page: Vec<Value> = (0..SCAN_CHUNK)
            .map(|index| json!({ "id": format!("n{index:03}") }))
            .collect();
        Mock::given(method("GET"))
            .and(path("/users/me/messages"))
            .and(query_param("q", format!("after:{since}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "messages": page })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/users/me/messages/n099"))
            .and(query_param("format", "metadata"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "n099",
                "internalDate": (boundary * 1000).to_string()
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/users/me/messages"))
            .and(query_param("q", format!("after:{since} before:{boundary}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "messages": [] })))
            .mount(&server)
            .await;
        for id in ["n098", "n099"] {
            Mock::given(method("GET"))
                .and(path(format!("/users/me/messages/{id}")))
                .and(query_param("format", "full"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "id": id,
                    "internalDate": ((NOW - 3 * DAY) * 1000).to_string(),
                    "payload": {"mimeType": "text/plain", "body": {"data": encoded(id)}}
                })))
                .mount(&server)
                .await;
        }

        let messages = reader(
            &server,
            FakeSecrets::with(REF, stored()) as Arc<dyn SecretPort>,
        )
        .fetch_inbox(ACC, since, NOW, 2)
        .await
        .expect("read");

        assert_eq!(
            messages
                .iter()
                .map(|m| m.provider_id.as_str())
                .collect::<Vec<_>>(),
            vec!["n099", "n098"],
            "the tail of the page, oldest first"
        );
    }

    #[tokio::test]
    async fn a_rejected_answer_reports_the_status_and_not_the_token() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/me/messages"))
            .respond_with(ResponseTemplate::new(403).set_body_string("insufficient scope"))
            .mount(&server)
            .await;

        let error = reader(
            &server,
            FakeSecrets::with(REF, stored()) as Arc<dyn SecretPort>,
        )
        .fetch_inbox(ACC, NOW - DAY, NOW, 10)
        .await
        .expect_err("rejected");
        assert_eq!(
            error,
            MailError::Rejected("the provider answered HTTP 403".to_string())
        );
        assert!(!format!("{error}").contains("at-1"));
    }

    #[tokio::test]
    async fn an_unknown_connection_is_not_read_through() {
        let server = MockServer::start().await;
        let reader = GmailReader::with_endpoints(
            reqwest::Client::new(),
            ProviderEndpoints::gmail(),
            FakeSecrets::with(REF, stored()),
            Arc::new(FakeAccounts(None)),
        );
        assert_eq!(
            reader.fetch_inbox(ACC, NOW - DAY, NOW, 10).await,
            Err(MailError::UnknownAccount)
        );
        assert!(
            server
                .received_requests()
                .await
                .expect("requests")
                .is_empty()
        );
    }
}
