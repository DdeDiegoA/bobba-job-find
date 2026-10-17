//! Session Board apply, Review mode (S11.2, FR-91, AD-10, AD-11).
//!
//! A Session Board Offer (LinkedIn Easy Apply, Indeed) is applied to in **the
//! user's own signed-in session**: a headed browser opens on that Source's
//! dedicated profile (S11.1a), Boba fills the form it can read, and the **user**
//! presses submit — exactly the Applies rule of the ATS Form channel (S9.4),
//! on the profile that already holds the Source's cookies.
//!
//! The core decides, never the adapter: which Sources may be driven
//! ([`SessionBoard`]), whether the user opted in, whether a challenge paused
//! the Source, and whether the page is even on the board's own host. The
//! adapter (a closed list of boards) owns only the DOM.
//!
//! A form that moves in steps (LinkedIn Easy Apply) is filled one step at a
//! time: [`SessionApplyService::start`] fills the step on screen, `fill` again
//! after the user presses *Next*. The user submits; [`SessionApplyService::confirm`]
//! reads the page and records the evidence — the confirmation text and the
//! screenshot's blake3 — or marks the attempt failed with `needs_check: true`,
//! exactly as the ATS channel does (FR-88, AD-12).

use std::collections::BTreeMap;
use std::sync::Arc;

use boba_domain::application::{AppStatus, Application, BankAnswer};
use boba_domain::audit::{AuditOutcome, AuditRow};
use boba_domain::posting::Offer;
use boba_domain::source::{AccessClass, ApplyCapability};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::applications::{TrackerError, TrackerService, TransitionInput};
use crate::apply::ats_form::{
    FieldPlan, FilledField, HandoffKind, Identity, UnansweredField, filled_field, intent_url,
    plan_field, same_host,
};
use crate::apply::manual::{ManualError, ManualService, PdfExport};
use crate::ports::browser::{BrowserError, Upload};
use crate::ports::clock::ClockPort;
use crate::ports::kill_switch::KillSwitchPort;
use crate::ports::repo::apply::{ApplyRepo, ApplyRepoError};
use crate::ports::repo::documents::{DocumentsRepo, DocumentsRepoError};
use crate::ports::repo::profile::{ProfileRepo, ProfileRepoError};
use crate::ports::repo::sources::{SourcesRepo, SourcesRepoError};
use crate::ports::repo::system::{NewIntent, SystemRepo, SystemRepoError};

/// The audit action an attempt on a Session Board writes (AD-12).
pub const AUDIT_ACTION: &str = "apply.session_submitted";

/// The audit action a challenge seen while filling writes (AD-10). It is the
/// same action the session search run uses (S11.1b), so the log reads one way.
pub const CHALLENGE_ACTION: &str = "session.challenge_paused";

/// The `setting` key a Source's challenge pause lives in (S11.1a/AD-10).
///
/// It mirrors `boba_sources::session::sdk::pause_key` word for word: the
/// search side writes it and this apply side reads it, and the crate graph
/// (sources -> app) means the constant cannot be shared. A change to either
/// spelling must change both; the S11.1a pause tests keep the search side true.
pub const PAUSE_KEY_PREFIX: &str = "session.pause.";

/// The Session Board a Source is driven as (FR-30).
///
/// A closed list: the core says which boards Boba knows how to fill, so an
/// adapter can never be reached for a site policy did not name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SessionBoard {
    LinkedIn,
    Indeed,
}

impl SessionBoard {
    /// The board a Source id is, if it is one Boba can apply on.
    pub fn of_source(source_id: &str) -> Option<Self> {
        match source_id {
            "linkedin" => Some(SessionBoard::LinkedIn),
            "indeed" => Some(SessionBoard::Indeed),
            _ => None,
        }
    }

    /// The stable wire name (`linkedin`, `indeed`).
    pub fn as_str(self) -> &'static str {
        match self {
            SessionBoard::LinkedIn => "linkedin",
            SessionBoard::Indeed => "indeed",
        }
    }

    /// The registrable domains the board lives on. The apply URL must be
    /// `https` and on one of these: the session profile is signed in, so Boba
    /// never drives it to a host the board did not name.
    pub fn domains(self) -> &'static [&'static str] {
        match self {
            SessionBoard::LinkedIn => &["linkedin.com"],
            SessionBoard::Indeed => &["indeed.com"],
        }
    }

    /// Whether `url` is an `https` page on this board (`www.` and subdomains
    /// included, a lookalike host excluded).
    ///
    /// The authority is whitelisted character by character, the way
    /// `apply::channel::host_of` is (S4.5): a backslash is a path separator to
    /// a browser, and a userinfo (`@`) can make the browser visit a host this
    /// check did not name, so neither is ever part of an accepted URL.
    pub fn allows(self, url: &str) -> bool {
        let Some((scheme, rest)) = url.split_once("://") else {
            return false;
        };
        if !scheme.eq_ignore_ascii_case("https") {
            return false;
        }
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
        if authority.is_empty()
            || !authority
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'))
        {
            return false;
        }
        let Some(host) = authority.split(':').next() else {
            return false;
        };
        let host = host.trim().to_ascii_lowercase();
        self.domains()
            .iter()
            .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
    }
}

pub use crate::ports::session::{SessionChallenge, SessionFormPort, SessionPage};

/// Why a Session Board could not be filled or read.
#[derive(Debug, thiserror::Error)]
pub enum SessionApplyError {
    /// The Offer's Source is not a Session Board Boba may apply on.
    #[error("source '{0}' is not a Session Board Boba can fill")]
    NotSessionBoard(String),
    /// The Source has no live opt-in (AD-9).
    #[error("source '{0}' has no recorded opt-in")]
    OptInRequired(String),
    /// A challenge paused the Source (AD-10).
    #[error("source '{0}' is paused after a challenge")]
    Paused(String),
    /// The apply URL is not an `https` page on the board's own domain.
    #[error("'{0}' is not a page on the board")]
    OffBoard(String),
    /// Today's page budget for the Source is spent (FR-30, S11.2 M4 ruling).
    #[error("source '{source_id}' already used its {cap} page loads today")]
    CapReached { source_id: String, cap: u32 },
    /// A package draft is not Ready and verified (AD-14).
    #[error("not Ready and verified: {0:?}")]
    NotReady(Vec<String>),
    /// The attempt id names no unresolved Session Board attempt.
    #[error("unknown session attempt: {0}")]
    UnknownAttempt(String),
    /// The Kill Switch is engaged: nothing automatic runs (S9.8, FR-86).
    #[error("the kill switch is engaged")]
    KillSwitch,
    #[error(transparent)]
    Manual(#[from] ManualError),
    #[error(transparent)]
    Tracker(#[from] TrackerError),
    #[error(transparent)]
    Sources(#[from] SourcesRepoError),
    #[error(transparent)]
    System(#[from] SystemRepoError),
    #[error(transparent)]
    Repo(#[from] ApplyRepoError),
    #[error(transparent)]
    Documents(#[from] DocumentsRepoError),
    #[error(transparent)]
    Profile(#[from] ProfileRepoError),
    #[error(transparent)]
    Browser(#[from] BrowserError),
}

impl SessionApplyError {
    /// The translated message key the UI shows (CH7: `errors.apply.*`).
    pub fn message_key(&self) -> &'static str {
        match self {
            SessionApplyError::NotSessionBoard(_) => "errors.apply.session_not_board",
            SessionApplyError::OptInRequired(_) => "errors.apply.session_opt_in",
            SessionApplyError::Paused(_) => "errors.apply.session_paused",
            SessionApplyError::OffBoard(_) => "errors.apply.session_off_board",
            SessionApplyError::CapReached { .. } => "errors.apply.session_cap_reached",
            SessionApplyError::NotReady(_) => "errors.apply.not_ready",
            SessionApplyError::UnknownAttempt(_) => "errors.apply.session_unknown_attempt",
            SessionApplyError::KillSwitch => "errors.apply.session_kill_switch",
            SessionApplyError::Browser(_) => "errors.apply.session_browser",
            SessionApplyError::Manual(error) => error.message_key(),
            SessionApplyError::Tracker(error) => error.message_key(),
            SessionApplyError::Sources(_)
            | SessionApplyError::System(_)
            | SessionApplyError::Repo(_)
            | SessionApplyError::Documents(_)
            | SessionApplyError::Profile(_) => "errors.apply.store",
        }
    }
}

/// Why automation stopped and the user took over (AC1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SessionHandoffKind {
    Captcha,
    LoginWall,
    UnusualActivity,
    RateLimited,
    /// The form is inside a cross-origin frame Boba cannot type into.
    EmbeddedFrame,
    UnknownRequired,
    MissingDocument,
}

impl From<SessionChallenge> for SessionHandoffKind {
    fn from(challenge: SessionChallenge) -> Self {
        match challenge {
            SessionChallenge::Captcha => SessionHandoffKind::Captcha,
            SessionChallenge::LoginWall => SessionHandoffKind::LoginWall,
            SessionChallenge::UnusualActivity => SessionHandoffKind::UnusualActivity,
            SessionChallenge::RateLimited => SessionHandoffKind::RateLimited,
        }
    }
}

/// One handoff, with the field that caused it when a field did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct SessionHandoff {
    pub kind: SessionHandoffKind,
    pub label: Option<String>,
}

/// The filled form handed to the user, and the window it lives in (AC1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct SessionApplyView {
    pub application_id: String,
    /// The attempt's correlation id, which `fill` and `confirm` take back.
    pub attempt_id: String,
    pub source_id: String,
    /// The board the Source is driven as (`linkedin`, `indeed`).
    pub board: String,
    pub url: String,
    pub filled: Vec<FilledField>,
    /// Document kinds attached from the Package (`cv`, `cover_letter`).
    pub attached: Vec<String>,
    pub unanswered: Vec<UnansweredField>,
    pub handoff: Option<SessionHandoff>,
}

/// The evidence a confirmed submission keeps (FR-88).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct SessionEvidence {
    pub confirmation_text: String,
    pub screenshot_blake3: String,
}

/// What a submission turned out to be (FR-88).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionOutcome {
    /// The page confirmed it: evidence stored, the card is Applied.
    Applied { evidence: SessionEvidence },
    /// No confirmation was found: the attempt is recorded and the card stays,
    /// marked Needs check from that outcome.
    NeedsCheck { screenshot_blake3: String },
}

/// What one fill of the visible step produced.
struct FilledStep {
    filled: Vec<FilledField>,
    attached: Vec<String>,
    unanswered: Vec<UnansweredField>,
    handoff: Option<SessionHandoff>,
    /// The challenge that made Boba stop, when the page was one (AD-10).
    paused: Option<SessionChallenge>,
}

impl FilledStep {
    /// Nothing was filled because the page handed control back (AC1).
    fn handed_back(handoff: SessionHandoff) -> Self {
        Self {
            filled: vec![],
            attached: vec![],
            unanswered: vec![],
            handoff: Some(handoff),
            paused: None,
        }
    }
}

/// The Session Board apply use cases (AC1, FR-88, AD-10).
pub struct SessionApplyService {
    apply: Arc<dyn ApplyRepo>,
    documents: Arc<dyn DocumentsRepo>,
    profile: Arc<dyn ProfileRepo>,
    sources: Arc<dyn SourcesRepo>,
    system: Arc<dyn SystemRepo>,
    browser: Arc<dyn SessionFormPort>,
    clock: Arc<dyn ClockPort>,
    /// The Kill Switch (S9.8). Unmounted means not engaged, the same seam
    /// S9.7 uses, so the feature ships before S9.8's adapter is mounted.
    kill_switch: Option<Arc<dyn KillSwitchPort>>,
}

impl SessionApplyService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        apply: Arc<dyn ApplyRepo>,
        documents: Arc<dyn DocumentsRepo>,
        profile: Arc<dyn ProfileRepo>,
        sources: Arc<dyn SourcesRepo>,
        system: Arc<dyn SystemRepo>,
        browser: Arc<dyn SessionFormPort>,
        clock: Arc<dyn ClockPort>,
    ) -> Self {
        Self {
            apply,
            documents,
            profile,
            sources,
            system,
            browser,
            clock,
            kill_switch: None,
        }
    }

    /// Attach the Kill Switch once S9.8's adapter is mounted.
    pub fn with_kill_switch(mut self, kill_switch: Option<Arc<dyn KillSwitchPort>>) -> Self {
        self.kill_switch = kill_switch;
        self
    }

    /// Open the board's apply page in that Source's session and fill the step
    /// on screen (AC1). The user presses submit; Boba never does.
    pub fn start(
        &self,
        application: &Application,
        offer: &Offer,
    ) -> Result<SessionApplyView, SessionApplyError> {
        let (board, cap) = self.gate(&offer.source_id)?;
        let url = offer.apply.url.trim();
        // The session profile is signed in: never drive it off the board.
        if !board.allows(url) {
            return Err(SessionApplyError::OffBoard(url.to_string()));
        }
        // AD-14: the same gate the manual kit uses, before the browser opens.
        let view = self.manual().view(application, offer)?;
        if !view.pending.is_empty() {
            return Err(SessionApplyError::NotReady(view.pending));
        }
        let documents = self.documents(application)?;
        let answers = self.reusable_answers(&application.id)?;
        let identity = self.identity(application)?;

        // M4 ruling: the click still spends one page from today's budget.
        self.spend_page_load(&offer.source_id, url, cap)?;

        let page = self.browser.open(&offer.source_id, url)?;
        // H1 (review): the board's own apply button may lead anywhere. Boba
        // types nothing into a page the board did not name — not even the CV's
        // contact block, and never a verified PDF.
        if !board.allows(&page.url) {
            let _ = self.browser.close();
            return Err(SessionApplyError::OffBoard(page.url.clone()));
        }

        // M2 (review): AD-12 wants the attempt on record **before** the
        // browser types anything. Page text is not part of it — only ids, the
        // URL (no query, S11.1b R-b) and the document kinds.
        let stamp = self.clock.now_rfc3339().replace([':', '+', '.'], "-");
        let attempt_id = format!("session_{}_{stamp}", application.id);
        let intent = self.system.audit_append_intent(&NewIntent {
            action: AUDIT_ACTION.to_string(),
            correlation_id: attempt_id.clone(),
            // The user presses submit in their own browser.
            actor: "user".to_string(),
            target_kind: "application".to_string(),
            target_id: Some(application.id.clone()),
            detail_json: serde_json::json!({
                "source_id": offer.source_id,
                "board": board.as_str(),
                "url": page_url(url),
                "documents": documents.keys().collect::<Vec<_>>(),
            })
            .to_string(),
            payload_blake3: None,
            created_at: self.clock.now_rfc3339(),
        });
        let intent = match intent {
            Ok(intent) => intent,
            Err(error) => {
                // Nothing was typed yet: close the window rather than leave
                // one open for an attempt the log never heard of.
                let _ = self.browser.close();
                return Err(error.into());
            }
        };
        let _ = intent;

        let step = self.fill_page_step(&identity, &answers, &documents, &offer.source_id, page)?;

        Ok(SessionApplyView {
            application_id: application.id.clone(),
            attempt_id,
            source_id: offer.source_id.clone(),
            board: board.as_str().to_string(),
            url: url.to_string(),
            filled: step.filled,
            attached: step.attached,
            unanswered: step.unanswered,
            handoff: step.handoff,
        })
    }

    /// Fill the step the user advanced to, under the same attempt (AC1).
    ///
    /// Nothing is opened here: if the window is gone the user must start again
    /// (the attempt stays unresolved), exactly as the ATS channel behaves.
    pub fn fill(
        &self,
        attempt_id: &str,
        application: &Application,
    ) -> Result<SessionApplyView, SessionApplyError> {
        let intent = self.intent(attempt_id, application)?;
        let source_id = intent_source(&intent)
            .ok_or_else(|| SessionApplyError::UnknownAttempt(attempt_id.to_string()))?;
        let (board, _cap) = self.gate(&source_id)?;
        let documents = self.documents(application)?;
        let answers = self.reusable_answers(&application.id)?;
        let identity = self.identity(application)?;

        // M4 (review): the package can change between steps; a draft that is
        // no longer Ready and verified must stop the fill.
        let pending = self.manual().pending(application)?;
        if !pending.is_empty() {
            return Err(SessionApplyError::NotReady(pending));
        }
        let page = self.browser.snapshot()?;
        // H1 (review): the user may have navigated in the window. Nothing is
        // typed into a page the board did not name.
        if !board.allows(&page.url) {
            return Err(SessionApplyError::OffBoard(page.url.clone()));
        }
        let url = page.url.clone();
        let step = self.fill_page_step(&identity, &answers, &documents, &source_id, page)?;
        Ok(SessionApplyView {
            application_id: application.id.clone(),
            attempt_id: attempt_id.to_string(),
            source_id,
            board: board.as_str().to_string(),
            url,
            filled: step.filled,
            attached: step.attached,
            unanswered: step.unanswered,
            handoff: step.handoff,
        })
    }

    /// Take the page's answer after the user submitted (FR-88).
    pub fn confirm(
        &self,
        attempt_id: &str,
        application: &Application,
    ) -> Result<SessionOutcome, SessionApplyError> {
        let intent = self.intent(attempt_id, application)?;
        let source_id = intent_source(&intent)
            .ok_or_else(|| SessionApplyError::UnknownAttempt(attempt_id.to_string()))?;
        // Revoking the opt-in stops even a read of the open page (S9.4 F3).
        // A kill switch or a pause never hides evidence: both only stop
        // automation, which confirming does not perform.
        if self.sources.get_optin(&source_id)?.is_none() {
            return Err(SessionApplyError::OptInRequired(source_id));
        }
        let page = self.browser.snapshot()?;
        let screenshot = self.browser.screenshot()?;
        // The session is over: leave no browser running (§6.4). A failure to
        // close must not cost the evidence, so it is not fatal.
        let _ = self.browser.close();
        let now = self.clock.now_rfc3339();

        // A confirmation only counts on the page's own host: a redirect to
        // another site's "thank you" page is not evidence of this submission.
        let on_form_host = intent_url(&intent)
            .as_deref()
            .is_none_or(|url| same_host(url, &page.url));
        let confirmation = page.confirmation.filter(|_| on_form_host);

        let Some(confirmation) = confirmation else {
            self.system.audit_append_outcome(
                &intent,
                AuditOutcome::Failed,
                &serde_json::json!({
                    "needs_check": true,
                    "url": page_url(&page.url),
                    "screenshot_blake3": screenshot.png_blake3,
                })
                .to_string(),
                &now,
            )?;
            return Ok(SessionOutcome::NeedsCheck {
                screenshot_blake3: screenshot.png_blake3,
            });
        };

        // Evidence first: the Application moves to Applied only because it
        // exists (FR-88). The screenshot is kept as its hash (AD-12).
        self.system.audit_append_outcome(
            &intent,
            AuditOutcome::Ok,
            &serde_json::json!({
                "confirmation_text": confirmation.text,
                "url": page_url(&page.url),
                "screenshot_blake3": screenshot.png_blake3,
            })
            .to_string(),
            &now,
        )?;
        if application.status != AppStatus::Applied {
            TrackerService::new(self.apply.clone(), self.clock.clone()).transition(
                TransitionInput {
                    application_id: application.id.clone(),
                    from_status: application.status.as_str().to_string(),
                    to_status: AppStatus::Applied.as_str().to_string(),
                    source: "apply_outcome".to_string(),
                    reason: Some("session_form_confirmed".to_string()),
                },
            )?;
        }
        Ok(SessionOutcome::Applied {
            evidence: SessionEvidence {
                confirmation_text: confirmation.text,
                screenshot_blake3: screenshot.png_blake3,
            },
        })
    }

    /// The gate every automation step passes (AD-9, AD-10, FR-86).
    ///
    /// Fail closed on anything but a live opt-in on an enabled Session Board:
    /// no browser opens for a Source the user did not opt in, a Source a
    /// challenge paused, or a Source whose row names another capability.
    fn gate(&self, source_id: &str) -> Result<(SessionBoard, Option<u32>), SessionApplyError> {
        if self
            .kill_switch
            .as_ref()
            .is_some_and(|switch| switch.engaged())
        {
            return Err(SessionApplyError::KillSwitch);
        }
        let record = self
            .sources
            .get_source(source_id)?
            .ok_or_else(|| SessionApplyError::NotSessionBoard(source_id.to_string()))?;
        let Some(board) = SessionBoard::of_source(source_id) else {
            return Err(SessionApplyError::NotSessionBoard(source_id.to_string()));
        };
        if record.access_class != AccessClass::SessionBrowser
            || record.apply_capability != ApplyCapability::SessionForm
        {
            return Err(SessionApplyError::NotSessionBoard(source_id.to_string()));
        }
        if !record.enabled || self.sources.get_optin(source_id)?.is_none() {
            return Err(SessionApplyError::OptInRequired(source_id.to_string()));
        }
        // The same key the session search run writes (S11.1a/AD-10): one
        // pause stops both reading and filling that Source.
        if self
            .system
            .setting_get(&format!("{PAUSE_KEY_PREFIX}{source_id}"))?
            .is_some()
        {
            return Err(SessionApplyError::Paused(source_id.to_string()));
        }
        Ok((board, record.daily_cap))
    }

    /// Spend one page from the Source's daily budget for a review-mode start.
    ///
    /// S11.2 M4 ruling (Diego standing OK): the ban risk is per account, so a
    /// user-clicked Apply spends the same `session.loads.<source>.<day>` total
    /// the search run spends. Pacing is exempt — the click is the pace.
    fn spend_page_load(
        &self,
        source_id: &str,
        url: &str,
        cap: Option<u32>,
    ) -> Result<(), SessionApplyError> {
        // A Source without a daily cap has no budget to spend.
        let Some(cap) = cap else {
            return Ok(());
        };
        if cap == 0 {
            return Err(SessionApplyError::CapReached {
                source_id: source_id.to_string(),
                cap,
            });
        }
        let now = self.clock.now_rfc3339();
        let key = boba_domain::session::loads_key(source_id, &now);
        // The same audit action the search run writes, so one log reads the
        // day's budget in one place (AD-12).
        let intent = self.system.audit_append_intent(&NewIntent {
            action: boba_domain::session::PAGE_LOAD_ACTION.to_string(),
            correlation_id: format!(
                "{}_{source_id}_{}",
                boba_domain::session::PAGE_LOAD_ACTION,
                now.replace([':', '+', '.'], "-")
            ),
            actor: "user".to_string(),
            target_kind: "source".to_string(),
            target_id: Some(source_id.to_string()),
            detail_json: serde_json::json!({
                "source_id": source_id,
                // Never the query or fragment (S11.1b R-b).
                "url": page_url(url),
                "cap": cap,
                "while": "apply",
            })
            .to_string(),
            payload_blake3: None,
            created_at: now.clone(),
        })?;
        match self.system.setting_spend_below(&key, cap, &now)? {
            Some(loads_today) => {
                self.system.audit_append_outcome(
                    &intent,
                    AuditOutcome::Ok,
                    &serde_json::json!({ "loads_today": loads_today }).to_string(),
                    &now,
                )?;
                Ok(())
            }
            None => {
                self.system.audit_append_outcome(
                    &intent,
                    AuditOutcome::Failed,
                    &serde_json::json!({ "cap_reached": cap }).to_string(),
                    &now,
                )?;
                Err(SessionApplyError::CapReached {
                    source_id: source_id.to_string(),
                    cap,
                })
            }
        }
    }

    /// The unresolved attempt `attempt_id` names, if it is this application's.
    fn intent(
        &self,
        attempt_id: &str,
        application: &Application,
    ) -> Result<AuditRow, SessionApplyError> {
        let intent = self
            .system
            .audit_unresolved_intents()?
            .into_iter()
            .find(|row| row.action == AUDIT_ACTION && row.correlation_id == attempt_id)
            .ok_or_else(|| SessionApplyError::UnknownAttempt(attempt_id.to_string()))?;
        // An attempt id is bound to the application it was opened for: one
        // application can never resolve another's submission.
        if intent.target_id.as_deref() != Some(application.id.as_str()) {
            return Err(SessionApplyError::UnknownAttempt(attempt_id.to_string()));
        }
        Ok(intent)
    }

    /// Pause the Source after a challenge (AD-10, M3 of the S11.2 review).
    ///
    /// The record is the one `boba_sources`' search run reads — same key, same
    /// JSON shape, same action — so a challenge met while filling stops the
    /// next search and the next fill alike. User-initiated or not, a wall is
    /// never retried around.
    fn pause_source(
        &self,
        source_id: &str,
        challenge: SessionChallenge,
    ) -> Result<(), SessionApplyError> {
        let now = self.clock.now_rfc3339();
        let intent = self.system.audit_append_intent(&NewIntent {
            action: CHALLENGE_ACTION.to_string(),
            correlation_id: format!(
                "challenge_{source_id}_{}",
                now.replace([':', '+', '.'], "-")
            ),
            actor: "user".to_string(),
            target_kind: "source".to_string(),
            target_id: Some(source_id.to_string()),
            detail_json: serde_json::json!({
                "kind": challenge.as_str(),
                "while": "apply",
            })
            .to_string(),
            payload_blake3: None,
            created_at: now.clone(),
        })?;
        self.system.setting_put(
            &format!("{PAUSE_KEY_PREFIX}{source_id}"),
            &serde_json::json!({ "kind": challenge.as_str(), "since": now }).to_string(),
            &now,
        )?;
        self.system
            .audit_append_outcome(&intent, AuditOutcome::Ok, "{}", &now)?;
        Ok(())
    }

    /// Fill the visible step; a wall or an unreachable form hands back (AC1),
    /// and a challenge also closes the Source's gate for the next attempt
    /// (AD-10, M3 of the S11.2 review).
    fn fill_page_step(
        &self,
        identity: &Identity,
        answers: &[BankAnswer],
        documents: &BTreeMap<String, PdfExport>,
        source_id: &str,
        page: SessionPage,
    ) -> Result<FilledStep, SessionApplyError> {
        let step = self.fill_page(identity, answers, documents, page)?;
        if let Some(kind) = step.paused {
            self.pause_source(source_id, kind)?;
        }
        Ok(step)
    }

    /// Fill the visible step; a wall or an unreachable form hands back (AC1).
    fn fill_page(
        &self,
        identity: &Identity,
        answers: &[BankAnswer],
        documents: &BTreeMap<String, PdfExport>,
        page: SessionPage,
    ) -> Result<FilledStep, SessionApplyError> {
        if let Some(challenge) = page.challenge {
            let mut step = FilledStep::handed_back(SessionHandoff {
                kind: challenge.into(),
                label: None,
            });
            step.paused = Some(challenge);
            return Ok(step);
        }
        if page.embedded_frame {
            // Indeed's smart-apply form lives in a cross-origin frame: Boba
            // cannot type into it, so the user fills that one by hand.
            return Ok(FilledStep::handed_back(SessionHandoff {
                kind: SessionHandoffKind::EmbeddedFrame,
                label: None,
            }));
        }
        let mut filled = Vec::new();
        let mut attached: Vec<String> = Vec::new();
        let mut unanswered = Vec::new();
        let mut handoff = None;
        for field in &page.fields {
            match plan_field(field, identity, answers, documents) {
                FieldPlan::Fill(value) => {
                    self.browser.fill_text(&field.handle, value.text())?;
                    filled.push(filled_field(field));
                }
                FieldPlan::Attach(kind) => {
                    let Some(export) = documents.get(kind) else {
                        handoff = Some(SessionHandoff {
                            kind: SessionHandoffKind::MissingDocument,
                            label: Some(field.label.clone()),
                        });
                        break;
                    };
                    self.browser.attach(
                        &field.handle,
                        &Upload {
                            file_name: export.file_name.clone(),
                            bytes: export.bytes.clone(),
                        },
                    )?;
                    if !attached.iter().any(|seen| seen == kind) {
                        attached.push(kind.to_string());
                    }
                    filled.push(filled_field(field));
                }
                FieldPlan::Leave => unanswered.push(UnansweredField {
                    label: field.label.clone(),
                    kind: field.kind.as_str().to_string(),
                    required: field.required,
                }),
                FieldPlan::Handoff { kind, label } => {
                    // AC1: stop here; the user takes over from this point.
                    handoff = Some(SessionHandoff {
                        kind: handoff_kind(kind),
                        label,
                    });
                    break;
                }
            }
        }
        Ok(FilledStep {
            filled,
            attached,
            unanswered,
            handoff,
            paused: None,
        })
    }

    /// The verified PDFs of the package, by kind (AD-14).
    fn documents(
        &self,
        application: &Application,
    ) -> Result<BTreeMap<String, PdfExport>, SessionApplyError> {
        let mut documents = BTreeMap::new();
        for export in self.manual().export_pdfs(application)? {
            documents.insert(export.kind.clone(), export);
        }
        Ok(documents)
    }

    /// The approved answers only: a draft is never typed into a form (S7.8).
    fn reusable_answers(&self, application_id: &str) -> Result<Vec<BankAnswer>, SessionApplyError> {
        Ok(self
            .apply
            .answer_bank_for_application(application_id)?
            .into_iter()
            .filter(BankAnswer::is_reusable)
            .collect())
    }

    /// The CV's contact block, when the package holds a verified one.
    fn identity(&self, application: &Application) -> Result<Identity, SessionApplyError> {
        let newest = self
            .apply
            .package(&application.id)?
            .into_iter()
            .filter(|item| item.kind == "cv" && item.verified_at.is_some())
            .max_by_key(|item| item.version);
        let Some(item) = newest else {
            return Ok(Identity::default());
        };
        let Some(record) = self.documents.get(&item.document_id)? else {
            return Ok(Identity::default());
        };
        let Ok(cv) = serde_json::from_str::<boba_domain::documents::CvDocument>(&record.model_json)
        else {
            return Ok(Identity::default());
        };
        Ok(Identity::of(&cv.contact))
    }

    /// The Manual use cases' gate and kit, reused: the attachable check
    /// (AD-14) is the same for every channel.
    fn manual(&self) -> ManualService {
        ManualService::new(
            self.apply.clone(),
            self.documents.clone(),
            self.profile.clone(),
            self.clock.clone(),
        )
    }
}

/// Map the shared planner's handoff onto the session one.
fn handoff_kind(kind: HandoffKind) -> SessionHandoffKind {
    match kind {
        HandoffKind::Captcha => SessionHandoffKind::Captcha,
        HandoffKind::LoginWall => SessionHandoffKind::LoginWall,
        HandoffKind::UnknownRequired => SessionHandoffKind::UnknownRequired,
        HandoffKind::MissingDocument => SessionHandoffKind::MissingDocument,
    }
}

/// The Source an attempt was opened for, as its intent recorded it.
fn intent_source(intent: &AuditRow) -> Option<String> {
    let detail: serde_json::Value = serde_json::from_str(&intent.detail_json).ok()?;
    detail.get("source_id")?.as_str().map(str::to_string)
}

/// A page without its query or fragment: the audit row names the page, never
/// the user's search terms or tracking parameters (S11.1b R-b).
fn page_url(url: &str) -> &str {
    url.split(['?', '#']).next().unwrap_or(url)
}

#[cfg(test)]
mod tests;
