//! Loopback OAuth 2.0 (RFC 8252) with PKCE S256 (S8.1, AC1/AC2, architecture §11).
//!
//! One connection flow, shared by every provider (Graph here, Gmail in S8.2):
//!
//! 1. [`PendingFlow::new`] mints a PKCE `S256` pair and a 256-bit `state`.
//! 2. [`PendingFlow::authorize_url`] is opened in the user's browser; the
//!    redirect target is a loopback URI on an ephemeral port ([`loopback::bind`]).
//! 3. [`loopback::accept_callback`] takes exactly one callback and closes.
//! 4. [`PendingFlow::take_callback`] checks the `state` against the pending flow
//!    — single use, so a replay always fails — and hands back the code.
//! 5. [`exchange_code`] swaps the code for tokens at the provider's token
//!    endpoint; [`refresh_tokens`] renews them later.
//!
//! There is no custom-scheme deep-link fallback anywhere: if the loopback port
//! cannot be bound, the flow fails and the UI points at the troubleshooting
//! steps or IMAP/BYO (§17.1, AC2).

pub mod loopback;

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

use boba_app::ports::repo::email::MailProvider;

/// How long a pending authorisation stays valid (architecture §11: 10 minutes).
pub const STATE_TTL: std::time::Duration = std::time::Duration::from_secs(600);

/// The bundled public desktop client id for Microsoft Graph (architecture §11).
///
/// It is public by definition — a desktop client has no secret and security
/// never relies on it — and it is set at build time (`BOBA_GRAPH_CLIENT_ID`),
/// because a client id belongs to a registration, not to the source tree. When
/// it is absent the bundled path is simply unavailable and the UI offers the
/// user's own client id (BYO) instead of failing at the provider.
pub const BUNDLED_GRAPH_CLIENT_ID: Option<&str> = option_env!("BOBA_GRAPH_CLIENT_ID");

/// Timeout for one provider call.
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// The one client every provider call goes through: an honest user agent and a
/// bounded timeout. Never shared with the source fetchers (adapters are
/// independent), never pointed at anything but the provider's own endpoints.
pub fn http_client() -> Result<reqwest::Client, OAuthError> {
    reqwest::Client::builder()
        .user_agent(user_agent())
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|_| OAuthError::Transport)
}

/// Honest identity for the provider's HTTP endpoints (`BobaJobFind/<ver>`).
pub fn user_agent() -> String {
    format!(
        "BobaJobFind/{} (+{})",
        env!("CARGO_PKG_VERSION"),
        "https://github.com/diegoarenas/boba-job-find"
    )
}

/// Failures of the authorisation flow. Every variant is safe to show: none
/// carries a token, a code or a state value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OAuthError {
    /// The user never finished (or cancelled) the browser step.
    #[error("the sign-in window was not completed in time")]
    Timeout,
    /// The loopback port could not be bound or the listener failed.
    #[error("the local sign-in listener could not be opened")]
    Listen,
    /// The provider returned `error=…` on the redirect.
    #[error("the provider refused the request: {0}")]
    Provider(String),
    /// The redirect's `state` did not match this attempt.
    #[error("the sign-in response did not match this attempt")]
    StateMismatch,
    /// The `state` was already used (or the flow already completed).
    #[error("this sign-in attempt is no longer valid")]
    StateExpired,
    /// The redirect carried no authorisation code.
    #[error("the provider returned no authorisation code")]
    MissingCode,
    /// The token endpoint could not be reached.
    #[error("the provider could not be reached")]
    Transport,
    /// The token endpoint answered with an error (bad client, bad code).
    #[error("the provider rejected the sign-in: {0}")]
    TokenRejected(String),
    /// The stored tokens could not be parsed.
    #[error("the stored connection tokens are unreadable")]
    InvalidTokens,
    /// The OS random source failed while minting the PKCE verifier or state.
    #[error("the system random source failed")]
    Rng,
    /// An authorisation or redirect URL could not be built.
    #[error("the sign-in URL could not be built")]
    BadUrl,
}

/// The endpoints of one provider (architecture §11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderEndpoints {
    /// Base URL of the API the connection talks to, e.g.
    /// `https://graph.microsoft.com/v1.0`.
    pub api_base: String,
    /// The OAuth 2.0 authorisation endpoint.
    pub authorize: String,
    /// The OAuth 2.0 token endpoint.
    pub token: String,
    /// The token revocation endpoint, when the provider has one. Microsoft
    /// Graph has none for delegated tokens (`/logout` only ends a browser
    /// session), so "revoked where possible" means: delete the tokens (`AC4`).
    pub revoke: Option<String>,
}

impl ProviderEndpoints {
    /// Microsoft Graph, tenant `common` (work, school and personal accounts),
    /// no admin consent (FR-35).
    pub fn microsoft_graph() -> Self {
        Self {
            api_base: "https://graph.microsoft.com/v1.0".to_string(),
            authorize: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize".to_string(),
            token: "https://login.microsoftonline.com/common/oauth2/v2.0/token".to_string(),
            revoke: None,
        }
    }

    /// Google's endpoints for a public installed-app client (S8.2).
    ///
    /// A desktop client is a public client: the flow is PKCE plus loopback, never
    /// a confidential secret. Google does insist on the `client_secret` its
    /// console issues for a "Desktop app" client at the token endpoint — its own
    /// docs call that value "obviously not treated as a secret" — so it travels
    /// with the token request only ([`refresh_tokens_with_client_secret`]); it
    /// never reaches the authorisation URL or the API.
    pub fn gmail() -> Self {
        Self {
            api_base: "https://gmail.googleapis.com/gmail/v1".to_string(),
            authorize: "https://accounts.google.com/o/oauth2/v2/auth".to_string(),
            token: "https://oauth2.googleapis.com/token".to_string(),
            revoke: Some("https://oauth2.googleapis.com/revoke".to_string()),
        }
    }

    /// The endpoints for a provider; `None` for a provider without OAuth (S8.3
    /// syncs IMAP through `boba_mail::imap` instead).
    pub fn for_provider(provider: MailProvider) -> Option<Self> {
        match provider {
            MailProvider::MicrosoftGraph => Some(Self::microsoft_graph()),
            MailProvider::Gmail => Some(Self::gmail()),
            MailProvider::ImapSmtp => None,
        }
    }
}

/// Seconds since the Unix epoch.
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The PKCE pair (RFC 7636). The verifier never leaves the process; only its
/// `S256` challenge travels in the authorisation URL.
#[derive(Clone)]
pub struct Pkce {
    verifier: String,
    challenge: String,
}

impl Pkce {
    /// Mint a fresh verifier: 32 random bytes, base64url (43 characters).
    pub fn generate() -> Result<Self, OAuthError> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|_| OAuthError::Rng)?;
        Self::from_verifier(URL_SAFE_NO_PAD.encode(bytes))
    }

    /// Build the pair from a known verifier (RFC 7636 §4.1: 43–128 characters
    /// from the unreserved set).
    pub fn from_verifier(verifier: String) -> Result<Self, OAuthError> {
        let ok = (43..=128).contains(&verifier.len())
            && verifier
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'));
        if !ok {
            return Err(OAuthError::BadUrl);
        }
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        Ok(Self {
            verifier,
            challenge,
        })
    }

    /// The verifier, sent only to the token endpoint.
    pub fn verifier(&self) -> &str {
        &self.verifier
    }

    /// The `S256` challenge, sent in the authorisation URL.
    pub fn challenge(&self) -> &str {
        &self.challenge
    }
}

impl std::fmt::Debug for Pkce {
    /// The verifier is a secret and never reaches a log; the challenge is public
    /// by construction (it travels in the authorisation URL).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pkce")
            .field("verifier", &"<redacted>")
            .field("challenge", &self.challenge)
            .finish()
    }
}

/// The single-use, expiring `state` of one pending flow (AC2).
#[derive(Debug)]
struct FlowState {
    token: String,
    issued_at: Instant,
    consumed: bool,
}

impl FlowState {
    fn generate() -> Result<Self, OAuthError> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|_| OAuthError::Rng)?;
        Ok(Self {
            token: URL_SAFE_NO_PAD.encode(bytes),
            issued_at: Instant::now(),
            consumed: false,
        })
    }
}

/// One pending authorisation: everything the redirect is checked against.
#[derive(Debug)]
pub struct PendingFlow {
    endpoints: ProviderEndpoints,
    client_id: String,
    /// Google's public installed-app client secret, when the provider requires
    /// it at the token endpoint (S8.2). Never sent in the authorisation URL.
    client_secret: Option<String>,
    redirect_uri: String,
    scopes: Vec<String>,
    pkce: Pkce,
    state: FlowState,
}

impl PendingFlow {
    /// Start a flow for `scopes` — exactly the scopes the chosen mode needs
    /// (AC1/NFR-5).
    pub fn new(
        endpoints: ProviderEndpoints,
        client_id: impl Into<String>,
        redirect_uri: impl Into<String>,
        scopes: &[&str],
    ) -> Result<Self, OAuthError> {
        let client_id = client_id.into();
        if client_id.trim().is_empty() || scopes.is_empty() {
            return Err(OAuthError::BadUrl);
        }
        Ok(Self {
            endpoints,
            client_id,
            client_secret: None,
            redirect_uri: redirect_uri.into(),
            scopes: scopes.iter().map(|s| (*s).to_string()).collect(),
            pkce: Pkce::generate()?,
            state: FlowState::generate()?,
        })
    }

    /// Attach the public installed-app client secret the provider requires at its
    /// token endpoint (Google; see [`ProviderEndpoints::gmail`]).
    pub fn with_client_secret(mut self, secret: impl Into<String>) -> Self {
        self.client_secret = Some(secret.into());
        self
    }

    /// The endpoints this flow talks to.
    pub fn endpoints(&self) -> &ProviderEndpoints {
        &self.endpoints
    }

    /// The scopes the user is asked to consent to, in request order.
    pub fn scopes(&self) -> &[String] {
        &self.scopes
    }

    /// The URL to open in the browser: a public-client authorisation request
    /// with a loopback redirect, PKCE `S256`, and the single-use `state`.
    pub fn authorize_url(&self) -> Result<String, OAuthError> {
        let mut url = url::Url::parse(&self.endpoints.authorize).map_err(|_| OAuthError::BadUrl)?;
        url.query_pairs_mut()
            .append_pair("client_id", &self.client_id)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", &self.redirect_uri)
            .append_pair("response_mode", "query")
            .append_pair("scope", &self.scopes.join(" "))
            .append_pair("state", &self.state.token)
            .append_pair("code_challenge", self.pkce.challenge())
            .append_pair("code_challenge_method", "S256");
        Ok(url.into())
    }

    /// Exchange the code from [`Self::take_callback`] for tokens. The PKCE
    /// verifier never leaves this module: callers only ever hold the challenge.
    pub async fn exchange(
        &self,
        http: &reqwest::Client,
        code: &str,
    ) -> Result<TokenResponse, OAuthError> {
        exchange_code(
            http,
            &self.endpoints,
            &self.client_id,
            self.client_secret.as_deref(),
            &self.redirect_uri,
            code,
            self.pkce.verifier(),
        )
        .await
    }

    /// Check the redirect against this flow and return the authorisation code.
    ///
    /// The `state` is consumed here whatever the outcome, so a replayed or
    /// duplicated callback can never authorise twice (AC2).
    pub fn take_callback(
        &mut self,
        query: &CallbackQuery,
        now: Instant,
    ) -> Result<String, OAuthError> {
        let already_used = std::mem::replace(&mut self.state.consumed, true);
        let stale = now.saturating_duration_since(self.state.issued_at) >= STATE_TTL;
        if already_used || stale {
            return Err(OAuthError::StateExpired);
        }
        // Not constant-time on purpose: the state is 256 random bits, single-use
        // and local, so a timing side channel has nothing to work with.
        if query.state.as_deref() != Some(self.state.token.as_str()) {
            return Err(OAuthError::StateMismatch);
        }
        if let Some(error) = &query.error {
            return Err(OAuthError::Provider(error.clone()));
        }
        query.code.clone().ok_or(OAuthError::MissingCode)
    }
}

/// The query string of the loopback redirect.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallbackQuery {
    /// `code` — the authorisation code.
    pub code: Option<String>,
    /// `state` — echoed back by the provider.
    pub state: Option<String>,
    /// `error` — set when the provider refused.
    pub error: Option<String>,
}

impl CallbackQuery {
    /// Parse the request target of the callback, e.g. `/cb?code=…&state=…`.
    /// Percent-encoding is decoded by `form_urlencoded`.
    pub fn parse(target: &str) -> Self {
        let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
        let mut parsed = Self::default();
        for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
            match key.as_ref() {
                "code" => parsed.code = Some(value.into_owned()),
                "state" => parsed.state = Some(value.into_owned()),
                "error" => parsed.error = Some(value.into_owned()),
                _ => {}
            }
        }
        parsed
    }
}

/// The wire shape of a token response (`grant_type=authorization_code` or
/// `refresh_token`).
#[derive(Clone, serde::Deserialize)]
pub struct TokenResponse {
    /// The bearer token for API calls.
    pub access_token: String,
    /// The refresh token, when the provider issues one.
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Lifetime of the access token in seconds.
    #[serde(default)]
    pub expires_in: Option<u64>,
    /// The scopes actually granted, space-separated. Absent means "as
    /// requested".
    #[serde(default)]
    pub scope: Option<String>,
}

impl std::fmt::Debug for TokenResponse {
    /// Tokens are secrets: no `{:?}` may print one (AD-13, FR-40).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenResponse")
            .field("access_token", &"<redacted>")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "<redacted>"),
            )
            .field("expires_in", &self.expires_in)
            .field("scope", &self.scope)
            .finish()
    }
}

impl TokenResponse {
    /// The granted scopes, falling back to what was requested.
    pub fn granted_scopes(&self, requested: &[String]) -> Vec<String> {
        match &self.scope {
            Some(scope) => scope.split_whitespace().map(str::to_string).collect(),
            None => requested.to_vec(),
        }
    }
}

/// What the keychain holds for one connection (AD-13, FR-40). Serialized as
/// JSON so one keychain entry carries the whole token set.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TokenSet {
    /// The current bearer token.
    pub access_token: String,
    /// The refresh token, when the provider issued one.
    pub refresh_token: Option<String>,
    /// Unix seconds when the access token stops being usable; `0` when the
    /// provider did not say.
    pub expires_at_unix: u64,
    /// The scopes this token set was granted.
    pub scopes: Vec<String>,
}

impl std::fmt::Debug for TokenSet {
    /// Tokens are secrets: no `{:?}` may print one (AD-13, FR-40).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenSet")
            .field("access_token", &"<redacted>")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "<redacted>"),
            )
            .field("expires_at_unix", &self.expires_at_unix)
            .field("scopes", &self.scopes)
            .finish()
    }
}

impl TokenSet {
    /// Build the set from a token response, dating the expiry from now.
    pub fn new(response: &TokenResponse, requested_scopes: &[String]) -> Self {
        Self {
            access_token: response.access_token.clone(),
            refresh_token: response.refresh_token.clone(),
            expires_at_unix: match response.expires_in {
                Some(seconds) if seconds > 0 => now_unix().saturating_add(seconds),
                _ => 0,
            },
            scopes: response.granted_scopes(requested_scopes),
        }
    }

    /// The set after a refresh: the new access token, the provider's refresh
    /// token when it rotated one, and **the stored refresh token otherwise**.
    ///
    /// A refresh response may omit the refresh token (RFC 6749 §6 makes it
    /// optional, and Google never re-issues it), and dropping it would leave the
    /// connection unrenewable at the next expiry (AD-13).
    pub fn renewed(&self, response: &TokenResponse) -> Self {
        let mut renewed = Self::new(response, &self.scopes);
        if renewed.refresh_token.is_none() {
            renewed.refresh_token.clone_from(&self.refresh_token);
        }
        renewed
    }

    /// The keychain representation.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Parse the keychain representation.
    pub fn from_json(json: &str) -> Result<Self, OAuthError> {
        serde_json::from_str(json).map_err(|_| OAuthError::InvalidTokens)
    }

    /// Whether the access token must be renewed before use. `skew_secs` renews
    /// slightly early; an unknown expiry (`0`) is used as-is and refreshed only
    /// when the provider rejects it.
    pub fn is_expired(&self, now_unix: u64, skew_secs: u64) -> bool {
        self.expires_at_unix != 0 && now_unix.saturating_add(skew_secs) >= self.expires_at_unix
    }
}

/// Exchange an authorisation code for tokens (RFC 6749 §4.1.3, PKCE §4.5).
///
/// `client_secret` is the public installed-app value Google requires at its token
/// endpoint; `None` is the Microsoft shape (a public client with no secret at
/// all).
pub async fn exchange_code(
    http: &reqwest::Client,
    endpoints: &ProviderEndpoints,
    client_id: &str,
    client_secret: Option<&str>,
    redirect_uri: &str,
    code: &str,
    verifier: &str,
) -> Result<TokenResponse, OAuthError> {
    let mut form = vec![
        ("client_id", client_id),
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("code_verifier", verifier),
    ];
    if let Some(secret) = client_secret {
        form.push(("client_secret", secret));
    }
    token_request(http, &endpoints.token, &form).await
}

/// Renew an access token. Graph rotates refresh tokens, so the whole response
/// replaces the stored set.
pub async fn refresh_tokens(
    http: &reqwest::Client,
    endpoints: &ProviderEndpoints,
    client_id: &str,
    refresh_token: &str,
) -> Result<TokenResponse, OAuthError> {
    let form = [
        ("client_id", client_id),
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
    ];
    token_request(http, &endpoints.token, &form).await
}

/// Renew an access token at a provider that requires its public installed-app
/// client secret in the request (Google, see [`ProviderEndpoints::gmail`]).
///
/// The secret is the one printed for the client id at registration time; Google
/// states it is not confidential. It is sent to the token endpoint and nowhere
/// else.
pub async fn refresh_tokens_with_client_secret(
    http: &reqwest::Client,
    endpoints: &ProviderEndpoints,
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> Result<TokenResponse, OAuthError> {
    let form = [
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
    ];
    token_request(http, &endpoints.token, &form).await
}

/// Revoke one token at the provider, where the provider supports it (AC4).
/// `Ok(false)` means the provider has no revocation endpoint, so only the local
/// copy can be deleted.
pub async fn revoke(
    http: &reqwest::Client,
    endpoints: &ProviderEndpoints,
    client_id: &str,
    token: &str,
) -> Result<bool, OAuthError> {
    let Some(url) = &endpoints.revoke else {
        return Ok(false);
    };
    // The network-activity view (S10.4, NFR-1): host and time only.
    boba_app::network_log::record(boba_app::network_log::Purpose::Email, url);
    let form = [("client_id", client_id), ("token", token)];
    let response = http
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, FORM_CONTENT_TYPE)
        .body(encode_form(&form))
        .send()
        .await
        .map_err(|_| OAuthError::Transport)?;
    if response.status().is_success() {
        Ok(true)
    } else {
        Err(OAuthError::TokenRejected(format!(
            "revocation returned {}",
            response.status().as_u16()
        )))
    }
}

/// `application/x-www-form-urlencoded`, the only body a token endpoint takes.
const FORM_CONTENT_TYPE: &str = "application/x-www-form-urlencoded";

/// Percent-encode a form body. Done here rather than with `reqwest`'s `form`, so
/// this crate needs no extra HTTP feature (and the tests read the exact bytes).
fn encode_form(form: &[(&str, &str)]) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    serializer.extend_pairs(form.iter().copied());
    serializer.finish()
}

async fn token_request(
    http: &reqwest::Client,
    url: &str,
    form: &[(&str, &str)],
) -> Result<TokenResponse, OAuthError> {
    // The network-activity view (S10.4, NFR-1): host and time only.
    boba_app::network_log::record(boba_app::network_log::Purpose::Email, url);
    let response = http
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, FORM_CONTENT_TYPE)
        .body(encode_form(form))
        .send()
        .await
        .map_err(|_| OAuthError::Transport)?;
    let status = response.status();
    if !status.is_success() {
        // The body names the OAuth error code (invalid_grant, invalid_client…)
        // and never a token; it is the one thing worth surfacing.
        let body = response.text().await.unwrap_or_default();
        let code = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
            .unwrap_or_else(|| format!("HTTP {}", status.as_u16()));
        return Err(OAuthError::TokenRejected(code));
    }
    response.json().await.map_err(|_| OAuthError::InvalidTokens)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    /// RFC 7636 appendix B: the challenge of [`VERIFIER`].
    const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

    fn flow() -> PendingFlow {
        PendingFlow::new(
            ProviderEndpoints::microsoft_graph(),
            "00000000-0000-0000-0000-0000000000b0",
            "http://127.0.0.1:49152/cb",
            &["Mail.Send", "Mail.Read"],
        )
        .expect("flow")
    }

    /// The pending `state` of a flow (the flow keeps it private).
    fn flow_state(flow: &PendingFlow) -> String {
        flow.state.token.clone()
    }

    fn callback(state: &str) -> CallbackQuery {
        CallbackQuery {
            code: Some("the-code".to_string()),
            state: Some(state.to_string()),
            error: None,
        }
    }

    fn endpoints(uri: &str) -> ProviderEndpoints {
        ProviderEndpoints {
            api_base: uri.to_string(),
            authorize: format!("{uri}/authorize"),
            token: format!("{uri}/token"),
            revoke: Some(format!("{uri}/revoke")),
        }
    }

    // --- Token renewal (AD-13) ----------------------------------------------

    /// Google does not re-issue the refresh token on a refresh (RFC 6749 §6: it
    /// is optional), so a renewal that omits it must keep the stored one —
    /// otherwise the connection dies at the next expiry.
    #[test]
    fn a_renewal_that_omits_the_refresh_token_keeps_the_stored_one() {
        let stored = TokenSet {
            access_token: "at-1".to_string(),
            refresh_token: Some("rt-1".to_string()),
            expires_at_unix: 0,
            scopes: vec!["Mail.Send".to_string()],
        };
        let omitted = TokenResponse {
            access_token: "at-2".to_string(),
            refresh_token: None,
            expires_in: Some(3600),
            scope: None,
        };
        let renewed = stored.renewed(&omitted);
        assert_eq!(renewed.access_token, "at-2");
        assert_eq!(renewed.refresh_token.as_deref(), Some("rt-1"));

        // A provider that rotates the token replaces the stored one.
        let rotated = TokenResponse {
            access_token: "at-3".to_string(),
            refresh_token: Some("rt-2".to_string()),
            expires_in: Some(3600),
            scope: None,
        };
        assert_eq!(
            stored.renewed(&rotated).refresh_token.as_deref(),
            Some("rt-2")
        );
    }

    // --- PKCE (AC1) ---------------------------------------------------------

    #[test]
    fn pkce_challenge_is_the_rfc7636_answer() {
        let pkce = Pkce::from_verifier(VERIFIER.to_string()).expect("valid verifier");
        assert_eq!(pkce.challenge(), CHALLENGE);
        assert_eq!(pkce.verifier(), VERIFIER);
    }

    #[test]
    fn pkce_generated_verifier_is_usable_and_random() {
        let a = Pkce::generate().expect("generate");
        let b = Pkce::generate().expect("generate");
        assert_eq!(a.verifier().len(), 43);
        assert_ne!(a.verifier(), b.verifier());
        // The challenge is base64url of a SHA-256 digest: 32 bytes, no padding.
        assert_eq!(a.challenge().len(), 43);
        assert!(!a.challenge().contains('='));
    }

    #[test]
    fn pkce_rejects_a_verifier_outside_the_rfc_lengths() {
        assert!(Pkce::from_verifier("short".to_string()).is_err());
        assert!(Pkce::from_verifier("x".repeat(129)).is_err());
        assert!(Pkce::from_verifier(format!("{}!", "x".repeat(42))).is_err());
    }

    // --- Authorisation URL (AC1/AC2) ---------------------------------------

    #[test]
    fn authorize_url_requests_least_privilege_on_loopback() {
        let flow = flow();
        let url = url::Url::parse(&flow.authorize_url().expect("url")).expect("parse");
        assert_eq!(url.host_str(), Some("login.microsoftonline.com"));
        // AC1: tenant `common` for work, school and personal accounts.
        assert!(url.path().starts_with("/common/oauth2/v2.0/authorize"));

        let pairs: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(
            pairs.get("redirect_uri").map(String::as_str),
            Some("http://127.0.0.1:49152/cb")
        );
        assert_eq!(pairs.get("response_type").map(String::as_str), Some("code"));
        assert_eq!(
            pairs.get("code_challenge_method").map(String::as_str),
            Some("S256")
        );
        assert_eq!(
            pairs.get("code_challenge").map(String::as_str),
            Some(flow.pkce.challenge())
        );
        assert_eq!(
            pairs.get("scope").map(String::as_str),
            Some("Mail.Send Mail.Read")
        );
        assert!(!flow.state.token.is_empty());
        // No implicit/fragment flow, and no secret: a public client only.
        assert_eq!(
            pairs.get("response_mode").map(String::as_str),
            Some("query")
        );
        assert!(!pairs.contains_key("client_secret"));
    }

    #[test]
    fn send_only_requests_only_mail_send() {
        let flow = PendingFlow::new(
            ProviderEndpoints::microsoft_graph(),
            "client",
            "http://127.0.0.1:1/cb",
            &["Mail.Send"],
        )
        .expect("flow");
        let url = url::Url::parse(&flow.authorize_url().unwrap()).unwrap();
        let scope = url
            .query_pairs()
            .find(|(k, _)| k == "scope")
            .map(|(_, v)| v.into_owned());
        assert_eq!(scope.as_deref(), Some("Mail.Send"));
        assert!(!scope.unwrap().contains("Mail.Read"));
    }

    #[test]
    fn a_flow_needs_a_client_id_and_at_least_one_scope() {
        let endpoints = ProviderEndpoints::microsoft_graph();
        assert!(
            PendingFlow::new(
                endpoints.clone(),
                "  ",
                "http://127.0.0.1:1/cb",
                &["Mail.Send"]
            )
            .is_err()
        );
        assert!(PendingFlow::new(endpoints, "client", "http://127.0.0.1:1/cb", &[]).is_err());
    }

    // --- Callback handling (AC2) -------------------------------------------

    #[test]
    fn a_matching_state_returns_the_code_and_the_state_is_single_use() {
        let mut flow = flow();
        let state = flow.state.token.clone();
        let now = Instant::now();

        assert_eq!(
            flow.take_callback(&callback(&state), now).unwrap(),
            "the-code"
        );
        // AC2: the state is discarded, so a replay never authorises again.
        assert_eq!(
            flow.take_callback(&callback(&state), now),
            Err(OAuthError::StateExpired)
        );
    }

    #[test]
    fn a_wrong_state_aborts_and_still_burns_the_flow() {
        let mut flow = flow();
        let now = Instant::now();
        assert_eq!(
            flow.take_callback(&callback("not-our-state"), now),
            Err(OAuthError::StateMismatch)
        );
        let state = flow.state.token.clone();
        assert_eq!(
            flow.take_callback(&callback(&state), now),
            Err(OAuthError::StateExpired),
            "a failed callback must not leave the flow re-usable"
        );
    }

    #[test]
    fn a_stale_state_aborts() {
        let mut flow = flow();
        let state = flow.state.token.clone();
        let late = Instant::now() + STATE_TTL + std::time::Duration::from_secs(1);
        assert_eq!(
            flow.take_callback(&callback(&state), late),
            Err(OAuthError::StateExpired)
        );
    }

    #[test]
    fn a_provider_error_or_a_missing_code_aborts() {
        let now = Instant::now();

        let mut refused_flow = flow();
        let state = refused_flow.state.token.clone();
        let refused = CallbackQuery {
            code: None,
            state: Some(state),
            error: Some("access_denied".to_string()),
        };
        assert_eq!(
            refused_flow.take_callback(&refused, now),
            Err(OAuthError::Provider("access_denied".to_string()))
        );

        let mut empty_flow = flow();
        let state = empty_flow.state.token.clone();
        let empty = CallbackQuery {
            code: None,
            state: Some(state),
            error: None,
        };
        assert_eq!(
            empty_flow.take_callback(&empty, now),
            Err(OAuthError::MissingCode)
        );
    }

    #[test]
    fn callback_query_decodes_percent_encoding_and_ignores_extras() {
        let query = CallbackQuery::parse("/cb?code=a%2Fb&state=s%2Bt&extra=1&error=");
        assert_eq!(query.code.as_deref(), Some("a/b"));
        assert_eq!(query.state.as_deref(), Some("s+t"));
        assert_eq!(query.error.as_deref(), Some(""));
        assert_eq!(CallbackQuery::parse("/cb"), CallbackQuery::default());
    }

    // --- Token set (AC1/AC4) ------------------------------------------------

    #[test]
    fn token_set_round_trips_through_the_keychain_encoding() {
        let response = TokenResponse {
            access_token: "at".into(),
            refresh_token: Some("rt".into()),
            expires_in: Some(3600),
            scope: Some("Mail.Send Mail.Read".into()),
        };
        let set = TokenSet::new(&response, &[]);
        assert_eq!(set.scopes, vec!["Mail.Send", "Mail.Read"]);
        assert_eq!(TokenSet::from_json(&set.to_json()).unwrap(), set);
        assert!(set.expires_at_unix > now_unix());
        assert_eq!(
            TokenSet::from_json("not json"),
            Err(OAuthError::InvalidTokens)
        );
    }

    #[test]
    fn expiry_is_known_unknown_or_near() {
        let response = TokenResponse {
            access_token: "at".into(),
            refresh_token: None,
            expires_in: Some(3600),
            scope: None,
        };
        let set = TokenSet::new(&response, &["Mail.Send".to_string()]);
        assert_eq!(set.scopes, vec!["Mail.Send"]);
        assert!(!set.is_expired(now_unix(), 60));
        assert!(
            set.is_expired(set.expires_at_unix - 1, 60),
            "renews within the skew"
        );
        assert!(set.is_expired(set.expires_at_unix, 0));

        // A provider that sends no `expires_in` leaves the expiry unknown: the
        // token is used and refreshed only when the provider rejects it.
        let unknown = TokenSet::new(
            &TokenResponse {
                access_token: "at".into(),
                refresh_token: None,
                expires_in: None,
                scope: None,
            },
            &[],
        );
        assert_eq!(unknown.expires_at_unix, 0);
        assert!(!unknown.is_expired(u64::MAX - 1, 60));
    }

    #[test]
    fn granted_scopes_fall_back_to_the_requested_ones() {
        let response = TokenResponse {
            access_token: "at".into(),
            refresh_token: None,
            expires_in: None,
            scope: None,
        };
        assert_eq!(
            response.granted_scopes(&["Mail.Read".to_string()]),
            vec!["Mail.Read"]
        );
    }

    #[test]
    fn every_oauth_provider_has_endpoints_and_imap_has_none() {
        assert!(ProviderEndpoints::for_provider(MailProvider::MicrosoftGraph).is_some());
        // S8.2: the Gmail send flow shares this flow (loopback, PKCE S256,
        // single-use state) with Google's endpoints.
        assert_eq!(
            ProviderEndpoints::for_provider(MailProvider::Gmail),
            Some(ProviderEndpoints::gmail())
        );
        assert!(ProviderEndpoints::for_provider(MailProvider::ImapSmtp).is_none());
        // AC4: Graph has no revocation endpoint, so revocation is local only.
        assert_eq!(ProviderEndpoints::microsoft_graph().revoke, None);
        // Google has one, so Disconnect revokes there too.
        assert_eq!(
            ProviderEndpoints::gmail().revoke.as_deref(),
            Some("https://oauth2.googleapis.com/revoke")
        );
    }

    /// The Gmail shape differs from Graph's in exactly one place: Google insists
    /// on its public installed-app secret at the token endpoint (S8.2).
    #[tokio::test]
    async fn the_installed_app_secret_is_sent_only_to_the_token_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("client_secret=public-app-value"))
            .and(body_string_contains("grant_type=authorization_code"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "at",
                "refresh_token": "rt",
                "expires_in": 3600
            })))
            .expect(1)
            .mount(&server)
            .await;

        let mut flow = PendingFlow::new(
            endpoints(&server.uri()),
            "client-1",
            "http://127.0.0.1:49152/cb",
            &["https://www.googleapis.com/auth/gmail.send"],
        )
        .expect("flow")
        .with_client_secret("public-app-value");
        // Nothing about the secret leaks into the authorisation URL...
        assert!(
            !flow
                .authorize_url()
                .expect("url")
                .contains("public-app-value")
        );
        // ...and the tokens still carry only what the provider granted.
        let code = {
            let state = flow_state(&flow);
            flow.take_callback(&callback(&state), Instant::now())
                .expect("code")
        };
        let tokens = flow
            .exchange(&reqwest::Client::new(), &code)
            .await
            .expect("exchange");
        assert_eq!(tokens.access_token, "at");
    }

    // --- Token endpoint (AC1/AC4) -------------------------------------------

    #[tokio::test]
    async fn exchange_code_sends_the_pkce_verifier_and_reads_the_tokens() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=authorization_code"))
            .and(body_string_contains("code_verifier=the-verifier"))
            .and(body_string_contains(
                "redirect_uri=http%3A%2F%2F127.0.0.1%3A49152%2Fcb",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "at",
                "refresh_token": "rt",
                "expires_in": 3600,
                "scope": "Mail.Send Mail.Read"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let tokens = exchange_code(
            &reqwest::Client::new(),
            &endpoints(&server.uri()),
            "client-1",
            None,
            "http://127.0.0.1:49152/cb",
            "the-code",
            "the-verifier",
        )
        .await
        .expect("exchange");

        assert_eq!(tokens.access_token, "at");
        assert_eq!(tokens.refresh_token.as_deref(), Some("rt"));
        assert_eq!(
            tokens.granted_scopes(&[]),
            vec!["Mail.Send".to_string(), "Mail.Read".to_string()]
        );
    }

    #[tokio::test]
    async fn a_rejected_token_request_surfaces_the_oauth_error_code() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "error": "invalid_grant",
                "error_description": "the code expired"
            })))
            .mount(&server)
            .await;

        let error = exchange_code(
            &reqwest::Client::new(),
            &endpoints(&server.uri()),
            "client-1",
            None,
            "http://127.0.0.1:49152/cb",
            "stale-code",
            "the-verifier",
        )
        .await
        .expect_err("must fail");
        assert_eq!(
            error,
            OAuthError::TokenRejected("invalid_grant".to_string())
        );
        assert!(!format!("{error}").contains("stale-code"));
    }

    #[tokio::test]
    async fn refresh_returns_the_rotated_set() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "at-2",
                "refresh_token": "rt-2",
                "expires_in": 3600
            })))
            .mount(&server)
            .await;

        let tokens = refresh_tokens(
            &reqwest::Client::new(),
            &endpoints(&server.uri()),
            "client-1",
            "rt-1",
        )
        .await
        .expect("refresh");
        assert_eq!(tokens.access_token, "at-2");
        assert_eq!(tokens.refresh_token.as_deref(), Some("rt-2"));
    }

    #[tokio::test]
    async fn revoke_reports_whether_the_provider_was_asked() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/revoke"))
            .and(body_string_contains("token=rt-1"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let endpoints = endpoints(&server.uri());
        assert!(
            revoke(&reqwest::Client::new(), &endpoints, "client-1", "rt-1")
                .await
                .expect("revoke")
        );

        // A provider without a revocation endpoint (Graph): local deletion only.
        let mut no_revoke = endpoints;
        no_revoke.revoke = None;
        assert!(
            !revoke(&reqwest::Client::new(), &no_revoke, "client-1", "rt-1")
                .await
                .expect("no endpoint")
        );
    }

    #[tokio::test]
    async fn transport_failures_do_not_leak_the_url_or_secrets() {
        let endpoints = endpoints("http://127.0.0.1:1");
        let error = exchange_code(
            &reqwest::Client::new(),
            &endpoints,
            "client-1",
            None,
            "http://127.0.0.1:49152/cb",
            "the-code",
            "the-verifier",
        )
        .await
        .expect_err("unreachable");
        assert_eq!(error, OAuthError::Transport);
        assert!(!format!("{error}").contains("the-code"));
    }
}
