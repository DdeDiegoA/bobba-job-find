//! The apply engine's Email Apply path (S9.3, FR-82/FR-88, AD-12, AD-14).
//!
//! Review mode is one click with three guarantees:
//!
//! * **AD-14** — the attachments come from the same attachable gate the Manual
//!   kit uses: only a document whose newest draft is Ready and verified, with a
//!   rendered PDF, can be attached. The check runs again at send time; the
//!   screen is never the gate.
//! * **AD-12** — an audit `intent` row is written *before* the message leaves
//!   and an `outcome` row *after* it, naming the message id. The row holds the
//!   payload's blake3, never the payload.
//! * **AC2** — never twice. The attempt row is written before the send and the
//!   next send reads it: a `sent` attempt refuses a resend, a `pending` attempt
//!   (the process died mid-send) asks for a check, and an `uncertain` (transport)
//!   attempt resends only when the user says so.
//!
//! The message id is Boba's: Microsoft Graph's `sendMail` answers 202 with no
//! body, so the id is assigned when the attempt is recorded and is what the
//! audit outcome and the attempt row name. When a provider starts returning its
//! own id, it replaces this value in [`SendReceipt::message_id`].

pub mod outbox;

use std::sync::Arc;

use boba_domain::application::{AppStatus, Application};
use boba_domain::audit::{AuditOutcome, AuditRow};
use boba_domain::posting::Offer;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::applications::{TrackerError, TrackerService, TransitionInput};
use crate::apply::email::{
    AttachmentView, Draft, draft_for, one_line, payload_bytes, pdf_attachment,
};
use crate::apply::manual::{KitAnswerView, ManualError, ManualService};
use crate::ports::clock::ClockPort;
use crate::ports::mail::{MailError, MailPort, OutgoingMessage};
use crate::ports::repo::apply::ApplyRepo;
use crate::ports::repo::documents::DocumentsRepo;
use crate::ports::repo::profile::ProfileRepo;
use crate::ports::repo::system::{NewIntent, SystemRepo, SystemRepoError};

pub use outbox::{ApplyAttempt, AttemptState, NewAttempt, OutboxRepo, OutboxRepoError};

/// The audit action every email application writes (AD-12).
pub const APPLY_ACTION: &str = "apply.submitted";

/// What the review screen shows before one click sends (AC1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct EmailApplyView {
    pub application_id: String,
    /// The posting's apply address (already checked to be one mailbox).
    pub recipient: String,
    pub subject: String,
    pub body_text: String,
    /// The verified PDFs, ready to attach (AD-14).
    pub attachments: Vec<AttachmentView>,
    /// The approved screening answers, ready to fold into the body (FR-82,
    /// S7.8): only answers the user approved are here.
    pub answers: Vec<KitAnswerView>,
    /// Kinds whose newest draft is not Ready and verified, so nothing may be
    /// sent until the Studio fixes them.
    pub pending: Vec<String>,
    /// Whether every draft in the package passed the attachable check.
    pub ready: bool,
    /// The newest attempt at sending this application, if any (AC2 evidence).
    pub last_attempt: Option<AttemptView>,
}

/// One attempt as the review screen shows it (AC2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct AttemptView {
    pub id: String,
    /// `pending`, `sent`, `failed` or `uncertain`.
    pub state: String,
    pub message_id: Option<String>,
    pub created_at: String,
    pub sent_at: Option<String>,
}

impl From<&ApplyAttempt> for AttemptView {
    fn from(attempt: &ApplyAttempt) -> Self {
        Self {
            id: attempt.id.clone(),
            state: attempt.state.as_str().to_string(),
            message_id: attempt.message_id.clone(),
            created_at: attempt.created_at.clone(),
            sent_at: attempt.sent_at.clone(),
        }
    }
}

/// What the user confirms on the review screen (AC1).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct SendRequest {
    /// The connected mailbox the message goes from.
    pub account_id: String,
    /// The subject, when the user edited it. Always forced onto one line.
    pub subject: Option<String>,
    /// The body, when the user edited it.
    pub body_text: Option<String>,
    /// The user's word that a resend after an unproven (transport) attempt is
    /// intended (AC2).
    #[serde(default)]
    pub confirm_resend: bool,
}

/// What a successful send did (AC1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct SendReceipt {
    pub attempt_id: String,
    /// The id the audit outcome names. Boba assigns it: the provider's
    /// `sendMail` returns none.
    pub message_id: String,
    /// The application's status after the send (`applied`).
    pub status: String,
}

/// Why an Email Apply was refused or failed.
#[derive(Debug, Clone, thiserror::Error)]
pub enum EngineError {
    /// No usable apply address on the posting: this is not an Email Apply
    /// channel.
    #[error("the posting carries no usable apply address")]
    NoEmailChannel,
    /// A previous attempt proves this application was already sent (AC2).
    #[error("this application was already sent as {0}")]
    AlreadySent(String),
    /// A previous attempt never recorded its result (the process died
    /// mid-send): the evidence must be checked before another send (AC2).
    #[error("an earlier send never reported back; check your sent mail first")]
    NeedsCheck,
    /// A previous attempt ended without an answer; resending needs the user's
    /// confirmation (AC2).
    #[error("an earlier send may have gone through; confirm to send again")]
    ConfirmResend,
    /// The user's subject was empty once flattened.
    #[error("the subject cannot be empty")]
    EmptySubject,
    /// The attachable gate refused the package (AD-14).
    #[error(transparent)]
    Manual(#[from] ManualError),
    #[error(transparent)]
    Outbox(#[from] OutboxRepoError),
    #[error(transparent)]
    Audit(#[from] SystemRepoError),
    #[error(transparent)]
    Mail(#[from] MailError),
    #[error(transparent)]
    Tracker(#[from] TrackerError),
}

impl EngineError {
    /// The translated message key the UI shows (CH7: `errors.<area>.*`).
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::NoEmailChannel => "errors.apply.no_email_channel",
            Self::AlreadySent(_) => "errors.apply.already_sent",
            Self::NeedsCheck => "errors.apply.needs_check",
            Self::ConfirmResend => "errors.apply.confirm_resend",
            Self::EmptySubject => "errors.apply.empty_subject",
            Self::Manual(error) => error.message_key(),
            Self::Outbox(_) | Self::Audit(_) => "errors.apply.store",
            Self::Mail(MailError::Transport(_)) => "errors.apply.send_unreachable",
            Self::Mail(MailError::Rejected(_)) => "errors.apply.send_rejected",
            Self::Mail(MailError::Reauthorize) => "errors.apply.reauthorize",
            Self::Mail(MailError::UnknownAccount) => "errors.apply.unknown_account",
            Self::Tracker(error) => error.message_key(),
        }
    }
}

/// Review and send an application by email (AC1/AC2/AC3).
pub struct ApplyEngine {
    apply: Arc<dyn ApplyRepo>,
    documents: Arc<dyn DocumentsRepo>,
    profile: Arc<dyn ProfileRepo>,
    outbox: Arc<dyn OutboxRepo>,
    system: Arc<dyn SystemRepo>,
    mail: Arc<dyn MailPort>,
    clock: Arc<dyn ClockPort>,
}

impl ApplyEngine {
    pub fn new(
        apply: Arc<dyn ApplyRepo>,
        documents: Arc<dyn DocumentsRepo>,
        profile: Arc<dyn ProfileRepo>,
        outbox: Arc<dyn OutboxRepo>,
        system: Arc<dyn SystemRepo>,
        mail: Arc<dyn MailPort>,
        clock: Arc<dyn ClockPort>,
    ) -> Self {
        Self {
            apply,
            documents,
            profile,
            outbox,
            system,
            mail,
            clock,
        }
    }

    /// The draft, the verified attachments and the evidence for one
    /// application (AC1/AC3).
    pub fn review(
        &self,
        application: &Application,
        offer: &Offer,
    ) -> Result<EmailApplyView, EngineError> {
        let draft = draft_for(offer).ok_or(EngineError::NoEmailChannel)?;
        let manual = self.manual();
        let gate = manual.view(application, offer)?;
        let attachments = manual
            .export_pdfs(application)?
            .iter()
            .map(|export| {
                AttachmentView::new(
                    &export.file_name,
                    &export.kind,
                    export.version,
                    export.bytes.len(),
                )
            })
            .collect();
        let attempts = self.outbox.attempts_for(&application.id)?;
        Ok(EmailApplyView {
            application_id: application.id.clone(),
            recipient: draft.recipient,
            subject: draft.subject,
            body_text: draft.body_text,
            attachments,
            answers: gate.answers,
            pending: gate.pending,
            ready: gate.ready,
            last_attempt: attempts.last().map(AttemptView::from),
        })
    }

    /// Send the reviewed application (AC1), with the evidence checks of AC2.
    pub async fn send(
        &self,
        application: &Application,
        offer: &Offer,
        request: &SendRequest,
    ) -> Result<SendReceipt, EngineError> {
        if request.account_id.trim().is_empty() {
            return Err(EngineError::Mail(MailError::UnknownAccount));
        }
        let draft = draft_for(offer).ok_or(EngineError::NoEmailChannel)?;

        // AD-14: the gate runs again here, never on the screen's word.
        let manual = self.manual();
        let gate = manual.view(application, offer)?;
        if !gate.pending.is_empty() {
            return Err(EngineError::Manual(ManualError::NotReady(gate.pending)));
        }

        // AC2: read the evidence before anything leaves.
        let attempts = self.outbox.attempts_for(&application.id)?;
        check_evidence(&attempts, request.confirm_resend)?;

        let message = compose(&draft, &manual, application, request)?;
        let now = self.clock.now_rfc3339();

        // AC2: the attempt is on disk before the message is.
        let attempt = self.outbox.attempt_insert(
            &NewAttempt {
                application_id: application.id.clone(),
                channel: "email".to_string(),
                account_id: request.account_id.trim().to_string(),
                recipient: message.to.first().cloned().unwrap_or_default(),
                subject: message.subject.clone(),
                payload: payload_bytes(&message),
            },
            &now,
        )?;
        // The id the attempt was recorded with is the one the outcome names.
        let message_id = attempt
            .message_id
            .clone()
            .unwrap_or_else(|| message_id_for(&attempt.id));

        // AD-12: intent before the outbound call. A log that cannot be written
        // stops the send, and the attempt is closed as failed so it never
        // pretends a message is in flight.
        let details = intent_detail(&attempt);
        let intent = match self.intent(&attempt, &details, &now) {
            Ok(intent) => intent,
            Err(error) => {
                self.close(&attempt.id, AttemptState::Failed, Some("audit"), &now)?;
                return Err(error);
            }
        };

        match self.mail.send(request.account_id.trim(), &message).await {
            Ok(()) => {
                let sent_at = self.clock.now_rfc3339();
                self.outbox
                    .attempt_set_state(&attempt.id, AttemptState::Sent, None, &sent_at)?;
                self.outcome(
                    &intent,
                    AuditOutcome::Ok,
                    &outcome_detail(&message_id),
                    &sent_at,
                )?;
                // The message already left: a card that refuses the move (it
                // was applied from the Tracker meanwhile) must not turn the send
                // into an error the user retries. The attempt is the evidence.
                let status = self
                    .mark_applied(application)
                    .unwrap_or_else(|_| application.status.as_str().to_string());
                Ok(SendReceipt {
                    attempt_id: attempt.id,
                    message_id,
                    status,
                })
            }
            Err(error) => {
                // A rejected message definitely did not leave; a transport
                // failure may have. The state is what a retry reads (AC2).
                let (state, class) = match &error {
                    MailError::Transport(_) => (AttemptState::Uncertain, "unreachable"),
                    MailError::Rejected(_) => (AttemptState::Failed, "rejected"),
                    MailError::Reauthorize => (AttemptState::Failed, "reauthorize"),
                    MailError::UnknownAccount => (AttemptState::Failed, "unknown_account"),
                };
                let failed_at = self.clock.now_rfc3339();
                self.close(&attempt.id, state, Some(class), &failed_at)?;
                self.outcome(
                    &intent,
                    AuditOutcome::Failed,
                    &format!("{{\"message_id\":\"{message_id}\",\"reason\":\"{class}\"}}"),
                    &failed_at,
                )?;
                Err(EngineError::Mail(error))
            }
        }
    }

    /// Mark the application Applied through the Tracker's one writer (AD-16),
    /// with `source = apply_outcome`: the history says the engine sent it.
    fn mark_applied(&self, application: &Application) -> Result<String, EngineError> {
        if application.status == AppStatus::Applied {
            // The user already marked it; the send is still recorded.
            return Ok(AppStatus::Applied.as_str().to_string());
        }
        let tracker = TrackerService::new(self.apply.clone(), self.clock.clone());
        let detail = tracker.transition(TransitionInput {
            application_id: application.id.clone(),
            from_status: application.status.as_str().to_string(),
            to_status: AppStatus::Applied.as_str().to_string(),
            source: "apply_outcome".to_string(),
            reason: None,
        })?;
        Ok(detail.status)
    }

    fn manual(&self) -> ManualService {
        ManualService::new(
            self.apply.clone(),
            self.documents.clone(),
            self.profile.clone(),
            self.clock.clone(),
        )
    }

    fn close(
        &self,
        attempt_id: &str,
        state: AttemptState,
        error: Option<&str>,
        now: &str,
    ) -> Result<(), EngineError> {
        Ok(self
            .outbox
            .attempt_set_state(attempt_id, state, error, now)?)
    }

    /// Append the `intent` row (AD-12).
    ///
    /// `AuditService` is generic over its repo and clock; this engine holds dyn
    /// ports, so the same one-row wrapper is spelled out here with the injected
    /// clock.
    fn intent(
        &self,
        attempt: &ApplyAttempt,
        detail_json: &str,
        now: &str,
    ) -> Result<AuditRow, EngineError> {
        Ok(self.system.audit_append_intent(&NewIntent {
            action: APPLY_ACTION.to_string(),
            correlation_id: attempt.id.clone(),
            actor: "user".to_string(),
            target_kind: "application".to_string(),
            target_id: Some(attempt.application_id.clone()),
            detail_json: detail_json.to_string(),
            payload_blake3: Some(attempt.payload_blake3.clone()),
            created_at: now.to_string(),
        })?)
    }

    fn outcome(
        &self,
        intent: &AuditRow,
        outcome: AuditOutcome,
        detail_json: &str,
        now: &str,
    ) -> Result<AuditRow, EngineError> {
        Ok(self
            .system
            .audit_append_outcome(intent, outcome, detail_json, now)?)
    }
}

/// AC2: what the earlier attempts allow.
fn check_evidence(attempts: &[ApplyAttempt], confirm_resend: bool) -> Result<(), EngineError> {
    if let Some(sent) = attempts
        .iter()
        .find(|attempt| attempt.state == AttemptState::Sent)
    {
        return Err(EngineError::AlreadySent(
            sent.message_id.clone().unwrap_or_else(|| sent.id.clone()),
        ));
    }
    // A pending row means the process stopped between the send and its result:
    // there is no evidence either way, and sending again may double the mail.
    if attempts
        .iter()
        .any(|attempt| attempt.state == AttemptState::Pending)
    {
        return Err(EngineError::NeedsCheck);
    }
    if !confirm_resend
        && attempts
            .iter()
            .any(|attempt| attempt.state == AttemptState::Uncertain)
    {
        return Err(EngineError::ConfirmResend);
    }
    Ok(())
}

/// The message the user reviewed, with their edits applied (AC1).
fn compose(
    draft: &Draft,
    manual: &ManualService,
    application: &Application,
    request: &SendRequest,
) -> Result<OutgoingMessage, EngineError> {
    let subject = match request.subject.as_deref() {
        Some(edited) => one_line(edited),
        None => draft.subject.clone(),
    };
    if subject.trim().is_empty() {
        return Err(EngineError::EmptySubject);
    }
    let attachments = manual
        .export_pdfs(application)?
        .into_iter()
        .map(|export| pdf_attachment(&export.file_name, export.bytes))
        .collect();
    Ok(OutgoingMessage {
        to: vec![draft.recipient.clone()],
        subject,
        body_text: request
            .body_text
            .clone()
            .unwrap_or_else(|| draft.body_text.clone()),
        attachments,
    })
}

/// Boba's message id for one attempt: stable, unique per attempt, and in the
/// shape a mail header uses.
fn message_id_for(attempt_id: &str) -> String {
    format!("<{attempt_id}@boba.local>")
}

/// Metadata for the `intent` row: ids and counts, never the body (AD-12).
fn intent_detail(attempt: &ApplyAttempt) -> String {
    let value = serde_json::json!({
        "attempt_id": attempt.id,
        "channel": attempt.channel,
        "payload_blake3": attempt.payload_blake3,
    });
    value.to_string()
}

/// The `outcome` row's detail: the message id the send is known by (AC1).
fn outcome_detail(message_id: &str) -> String {
    serde_json::json!({ "message_id": message_id }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use boba_domain::application::{PackageItem, new_application_id};
    use boba_domain::audit::{AuditPhase, ChainStatus};
    use boba_domain::documents::{DocumentKind, Lang, NewDocument, TemplateId};
    use boba_domain::posting::{ApplyHints, WorkMode};

    use crate::applications::testing::FakeApplyRepo;
    use crate::ports::mail::{Attachment, MailFuture};
    use crate::ports::repo::documents::fake::FakeDocumentsRepo;
    use crate::ports::repo::system::SettingRecord;
    use crate::profile::testing::FakeProfileRepo;
    use crate::system::FixedClock;

    /// Poll the engine's future to completion: the fake mail port answers
    /// immediately, so one poll suffices and no runtime is needed (a real
    /// executor lives in the Tauri shell, not in this crate).
    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        use std::task::{Context, Poll, Waker};

        let mut context = Context::from_waker(Waker::noop());
        let mut future = std::pin::pin!(future);
        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(value) => return value,
                Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    const NOW: &str = "2026-10-17T00:00:00Z";

    // ---- fakes ------------------------------------------------------------

    /// A `SystemRepo` that only records audit rows, so a test can read them.
    #[derive(Default)]
    struct RecordingSystem {
        rows: Mutex<Vec<AuditRow>>,
    }

    impl RecordingSystem {
        fn rows(&self) -> Vec<AuditRow> {
            self.rows.lock().unwrap().clone()
        }

        fn push(&self, mut row: AuditRow) -> AuditRow {
            let mut rows = self.rows.lock().unwrap();
            row.seq = rows.len() as i64 + 1;
            row.prev_hash = rows
                .last()
                .map(|last| last.hash.clone())
                .unwrap_or_else(|| boba_domain::audit::GENESIS_HASH.to_string());
            row.hash = format!("hash{}", row.seq);
            rows.push(row.clone());
            row
        }
    }

    impl SystemRepo for RecordingSystem {
        fn setting_get(&self, _key: &str) -> Result<Option<String>, SystemRepoError> {
            Ok(None)
        }

        fn setting_put(
            &self,
            _key: &str,
            _value: &str,
            _updated_at: &str,
        ) -> Result<(), SystemRepoError> {
            Err(SystemRepoError::new("not used in this test"))
        }

        fn setting_list(&self) -> Result<Vec<SettingRecord>, SystemRepoError> {
            Ok(Vec::new())
        }

        fn setting_delete(&self, _key: &str) -> Result<bool, SystemRepoError> {
            Ok(false)
        }

        fn audit_append_intent(&self, intent: &NewIntent) -> Result<AuditRow, SystemRepoError> {
            Ok(self.push(AuditRow {
                seq: 0,
                id: format!("audit_intent_{}", self.rows.lock().unwrap().len() + 1),
                created_at: intent.created_at.clone(),
                phase: AuditPhase::Intent,
                action: intent.action.clone(),
                outcome: None,
                correlation_id: intent.correlation_id.clone(),
                intent_id: None,
                actor: intent.actor.clone(),
                target_kind: intent.target_kind.clone(),
                target_id: intent.target_id.clone(),
                detail_json: intent.detail_json.clone(),
                payload_blake3: intent.payload_blake3.clone(),
                prev_hash: String::new(),
                hash: String::new(),
            }))
        }

        fn audit_append_outcome(
            &self,
            intent: &AuditRow,
            outcome: AuditOutcome,
            detail_json: &str,
            created_at: &str,
        ) -> Result<AuditRow, SystemRepoError> {
            Ok(self.push(AuditRow {
                seq: 0,
                id: format!("audit_outcome_{}", self.rows.lock().unwrap().len() + 1),
                created_at: created_at.to_string(),
                phase: AuditPhase::Outcome,
                action: intent.action.clone(),
                outcome: Some(outcome),
                correlation_id: intent.correlation_id.clone(),
                intent_id: Some(intent.id.clone()),
                actor: intent.actor.clone(),
                target_kind: intent.target_kind.clone(),
                target_id: intent.target_id.clone(),
                detail_json: detail_json.to_string(),
                payload_blake3: None,
                prev_hash: String::new(),
                hash: String::new(),
            }))
        }

        fn audit_head_hash(&self) -> Result<Option<String>, SystemRepoError> {
            Ok(self.rows.lock().unwrap().last().map(|row| row.hash.clone()))
        }

        fn audit_verify_chain(&self) -> Result<ChainStatus, SystemRepoError> {
            Ok(ChainStatus {
                rows: self.rows.lock().unwrap().len(),
                head: self.audit_head_hash()?,
                broken_at: None,
            })
        }

        fn audit_unresolved_intents(&self) -> Result<Vec<AuditRow>, SystemRepoError> {
            Ok(Vec::new())
        }

        fn audit_anchor_get(&self) -> Result<Option<String>, SystemRepoError> {
            Ok(None)
        }

        fn audit_anchor_is_known(&self, _hash: &str) -> Result<bool, SystemRepoError> {
            Ok(true)
        }

        fn audit_anchor_set(&self, _hash: &str) -> Result<(), SystemRepoError> {
            Ok(())
        }
    }

    /// A `MailPort` that records what it was handed and answers what the test
    /// says, in order: `Some(error)` fails that call, `None` succeeds; once the
    /// list is used up every call succeeds.
    #[derive(Default)]
    struct FakeMail {
        sent: Mutex<Vec<(String, OutgoingMessage)>>,
        answers: Mutex<Vec<Option<MailError>>>,
    }

    impl FakeMail {
        fn answers(answers: Vec<Option<MailError>>) -> Self {
            Self {
                sent: Mutex::new(Vec::new()),
                answers: Mutex::new(answers),
            }
        }

        fn failing(error: MailError) -> Self {
            Self::answers(vec![Some(error)])
        }

        fn sent(&self) -> Vec<(String, OutgoingMessage)> {
            self.sent.lock().unwrap().clone()
        }
    }

    impl MailPort for FakeMail {
        fn send<'a>(
            &'a self,
            account_id: &'a str,
            message: &'a OutgoingMessage,
        ) -> MailFuture<'a, Result<(), MailError>> {
            Box::pin(async move {
                self.sent
                    .lock()
                    .unwrap()
                    .push((account_id.to_string(), message.clone()));
                let mut answers = self.answers.lock().unwrap();
                if answers.is_empty() {
                    Ok(())
                } else {
                    answers.remove(0).map_or(Ok(()), Err)
                }
            })
        }
    }

    /// An `OutboxRepo` in memory, mirroring the adapter's contract.
    #[derive(Default)]
    struct FakeOutbox {
        attempts: Mutex<Vec<ApplyAttempt>>,
        next: Mutex<u64>,
    }

    impl FakeOutbox {
        fn attempts(&self) -> Vec<ApplyAttempt> {
            self.attempts.lock().unwrap().clone()
        }

        fn seed(&self, attempt: ApplyAttempt) {
            self.attempts.lock().unwrap().push(attempt);
        }
    }

    impl OutboxRepo for FakeOutbox {
        fn attempt_insert(
            &self,
            attempt: &NewAttempt,
            now: &str,
        ) -> Result<ApplyAttempt, OutboxRepoError> {
            let mut next = self.next.lock().unwrap();
            *next += 1;
            let id = format!("att_{next:04}");
            let stored = ApplyAttempt {
                id: id.clone(),
                application_id: attempt.application_id.clone(),
                channel: attempt.channel.clone(),
                account_id: attempt.account_id.clone(),
                recipient: attempt.recipient.clone(),
                subject: attempt.subject.clone(),
                payload_blake3: fnv(&attempt.payload),
                message_id: Some(format!("<{id}@boba.local>")),
                state: AttemptState::Pending,
                error: None,
                created_at: now.to_string(),
                updated_at: now.to_string(),
                sent_at: None,
            };
            self.attempts.lock().unwrap().push(stored.clone());
            Ok(stored)
        }

        fn attempt_set_state(
            &self,
            id: &str,
            state: AttemptState,
            error: Option<&str>,
            now: &str,
        ) -> Result<(), OutboxRepoError> {
            let mut attempts = self.attempts.lock().unwrap();
            let attempt = attempts
                .iter_mut()
                .find(|attempt| attempt.id == id)
                .ok_or_else(|| OutboxRepoError::new(format!("unknown attempt {id}")))?;
            attempt.state = state;
            attempt.error = error.map(str::to_string);
            attempt.updated_at = now.to_string();
            if state == AttemptState::Sent {
                attempt.sent_at = Some(now.to_string());
            }
            Ok(())
        }

        fn attempts_for(&self, application_id: &str) -> Result<Vec<ApplyAttempt>, OutboxRepoError> {
            Ok(self
                .attempts()
                .into_iter()
                .filter(|attempt| attempt.application_id == application_id)
                .collect())
        }
    }

    fn fnv(bytes: &[u8]) -> String {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        format!("{hash:016x}")
    }

    // ---- fixtures ---------------------------------------------------------

    fn ready_model(kind: DocumentKind) -> String {
        match kind {
            DocumentKind::Cv => serde_json::json!({
                "schema_version": 1,
                "lang": "en",
                "contact": { "full_name": "Ada Lovelace" },
                "sections": [],
            }),
            DocumentKind::CoverLetter => serde_json::json!({
                "schema_version": 1,
                "lang": "en",
                "contact": { "full_name": "Ada Lovelace" },
                "paragraphs": [],
            }),
        }
        .to_string()
    }

    fn unbacked_model(kind: DocumentKind) -> String {
        match kind {
            DocumentKind::Cv => serde_json::json!({
                "schema_version": 1,
                "lang": "en",
                "contact": { "full_name": "Ada Lovelace" },
                "sections": [{
                    "heading": "Experience",
                    "items": [{ "text": "Shipped a billing API.", "fact_ids": [] }],
                }],
            }),
            DocumentKind::CoverLetter => serde_json::json!({
                "schema_version": 1,
                "lang": "en",
                "contact": { "full_name": "Ada Lovelace" },
                "paragraphs": [{ "text": "I shipped a billing API.", "fact_ids": [] }],
            }),
        }
        .to_string()
    }

    fn offer(email: Option<&str>) -> Offer {
        Offer {
            id: "pst_01".into(),
            source_id: "src_01".into(),
            external_id: "ext-1".into(),
            title: "Backend Engineer".into(),
            company: "Nimbus".into(),
            locations: vec![],
            work_mode: WorkMode::Remote,
            money: None,
            posted_at: None,
            deadline: None,
            first_seen: NOW.into(),
            last_seen: NOW.into(),
            lang: "en".into(),
            description_html: "<p>Hiring.</p>".into(),
            description_text: "Hiring.".into(),
            requirements: vec![],
            apply: ApplyHints {
                url: "https://jobs.lever.co/nimbus/abc".into(),
                email: email.map(str::to_string),
            },
        }
    }

    fn application() -> Application {
        Application {
            id: new_application_id(),
            posting_id: "pst_01".into(),
            status: AppStatus::Drafted,
            offer_snapshot_json: "{}".into(),
            notes: None,
            next_action: None,
            created_at: NOW.into(),
            updated_at: NOW.into(),
        }
    }

    fn seed_document(
        documents: &FakeDocumentsRepo,
        id: &str,
        application_id: &str,
        kind: DocumentKind,
        model_json: String,
        verified: bool,
    ) -> i64 {
        let version = documents
            .insert(&NewDocument {
                id: id.into(),
                application_id: application_id.into(),
                kind,
                lang: Lang::En,
                template: TemplateId::new("cv-classic"),
                model_json,
                language_fallback_from: None,
                untrusted_origin: true,
                created_at: NOW.into(),
            })
            .unwrap();
        documents.set_pdf(id, b"%PDF-1.7 application", NOW).unwrap();
        if verified {
            documents.mark_verified(id, "{}", NOW).unwrap();
        }
        version
    }

    fn package_item(document_id: &str, kind: &str, version: i64) -> PackageItem {
        PackageItem {
            document_id: document_id.into(),
            kind: kind.into(),
            lang: "en".into(),
            version,
            verified_at: Some(NOW.to_string()),
        }
    }

    struct Harness {
        apply: Arc<FakeApplyRepo>,
        documents: Arc<FakeDocumentsRepo>,
        outbox: Arc<FakeOutbox>,
        system: Arc<RecordingSystem>,
        mail: Arc<FakeMail>,
    }

    fn harness(mail: FakeMail) -> Harness {
        Harness {
            apply: Arc::new(FakeApplyRepo::default()),
            documents: Arc::new(FakeDocumentsRepo::default()),
            outbox: Arc::new(FakeOutbox::default()),
            system: Arc::new(RecordingSystem::default()),
            mail: Arc::new(mail),
        }
    }

    fn engine(harness: &Harness) -> ApplyEngine {
        ApplyEngine::new(
            harness.apply.clone(),
            harness.documents.clone(),
            Arc::new(FakeProfileRepo::default()),
            harness.outbox.clone(),
            harness.system.clone(),
            harness.mail.clone(),
            Arc::new(FixedClock::new(NOW)),
        )
    }

    fn seed_verified_cv(harness: &Harness, application: &Application) {
        let version = seed_document(
            &harness.documents,
            "doc_cv_1",
            &application.id,
            DocumentKind::Cv,
            ready_model(DocumentKind::Cv),
            true,
        );
        harness
            .apply
            .seed_package(&application.id, package_item("doc_cv_1", "cv", version));
    }

    fn request() -> SendRequest {
        SendRequest {
            account_id: "eml_1".into(),
            subject: None,
            body_text: None,
            confirm_resend: false,
        }
    }

    // ---- review -----------------------------------------------------------

    #[test]
    fn the_review_shows_the_draft_and_the_verified_attachments() {
        let harness = harness(FakeMail::default());
        let application = application();
        seed_verified_cv(&harness, &application);

        let view = engine(&harness)
            .review(&application, &offer(Some("jobs@nimbus.example")))
            .unwrap();

        assert_eq!(view.recipient, "jobs@nimbus.example");
        assert_eq!(view.subject, "Application: Backend Engineer");
        assert!(view.body_text.contains("Backend Engineer"));
        assert_eq!(view.attachments.len(), 1);
        assert_eq!(view.attachments[0].file_name, "cv-v1.pdf");
        assert_eq!(view.attachments[0].kind, "cv");
        assert!(view.ready);
        assert!(view.pending.is_empty());
        assert_eq!(view.last_attempt, None);
    }

    #[test]
    fn the_review_shows_the_approved_answers_and_only_those() {
        let harness = harness(FakeMail::default());
        let application = application();
        harness
            .apply
            .answer_bank_insert(&boba_domain::application::NewBankAnswer {
                id: "ans_1".into(),
                application_id: application.id.clone(),
                question: "What are your salary expectations?".into(),
                question_key: "what are your salary expectations".into(),
                answer: Some("€65,000".into()),
                class: boba_domain::application::AnswerClass::Salary,
                fact_ids: vec![],
                needs_review: true,
                untrusted_origin: true,
                created_at: NOW.into(),
            })
            .unwrap();
        harness
            .apply
            .answer_bank_insert(&boba_domain::application::NewBankAnswer {
                id: "ans_2".into(),
                application_id: application.id.clone(),
                question: "When can you start?".into(),
                question_key: "when can you start".into(),
                answer: Some("Two weeks".into()),
                class: boba_domain::application::AnswerClass::Other,
                fact_ids: vec![],
                needs_review: false,
                untrusted_origin: true,
                created_at: NOW.into(),
            })
            .unwrap();
        harness
            .apply
            .answer_bank_approve("ans_1", None, NOW)
            .unwrap();

        let view = engine(&harness)
            .review(&application, &offer(Some("jobs@nimbus.example")))
            .unwrap();

        assert_eq!(view.answers.len(), 1, "an unapproved draft stays out");
        assert_eq!(
            view.answers[0].question,
            "What are your salary expectations?"
        );
        assert_eq!(view.answers[0].answer, "€65,000");
    }

    #[test]
    fn a_posting_without_an_apply_address_has_no_email_channel() {
        let harness = harness(FakeMail::default());
        let error = engine(&harness)
            .review(&application(), &offer(None))
            .unwrap_err();
        assert_eq!(error.message_key(), "errors.apply.no_email_channel");
    }

    #[test]
    fn an_unverified_draft_is_named_and_not_attached() {
        let harness = harness(FakeMail::default());
        let application = application();
        // Verified render of a draft the validator rejects: not attachable.
        let version = seed_document(
            &harness.documents,
            "doc_cv_1",
            &application.id,
            DocumentKind::Cv,
            unbacked_model(DocumentKind::Cv),
            true,
        );
        harness
            .apply
            .seed_package(&application.id, package_item("doc_cv_1", "cv", version));

        let view = engine(&harness)
            .review(&application, &offer(Some("jobs@nimbus.example")))
            .unwrap();

        assert!(view.attachments.is_empty(), "AD-14: never handed out");
        assert_eq!(view.pending, vec!["cv"]);
        assert!(!view.ready);
    }

    // ---- send -------------------------------------------------------------

    #[test]
    fn sending_writes_intent_then_sends_then_writes_outcome_and_moves_the_card() {
        block_on(async {
            let harness = harness(FakeMail::default());
            let application = application();
            harness.apply.seed_application(application.clone());
            seed_verified_cv(&harness, &application);

            let receipt = engine(&harness)
                .send(
                    &application,
                    &offer(Some("jobs@nimbus.example")),
                    &request(),
                )
                .await
                .unwrap();

            // The message that left.
            let sent = harness.mail.sent();
            assert_eq!(sent.len(), 1);
            assert_eq!(sent[0].0, "eml_1");
            assert_eq!(sent[0].1.to, vec!["jobs@nimbus.example"]);
            assert_eq!(sent[0].1.attachments[0].filename, "cv-v1.pdf");
            assert_eq!(sent[0].1.attachments[0].mime, "application/pdf");

            // AC1: intent before, outcome after, both naming the attempt.
            let rows = harness.system.rows();
            assert_eq!(rows.len(), 2, "one intent, one outcome");
            assert_eq!(rows[0].phase, AuditPhase::Intent);
            assert_eq!(rows[0].action, APPLY_ACTION);
            assert_eq!(rows[0].actor, "user");
            assert_eq!(rows[0].target_id.as_deref(), Some(application.id.as_str()));
            assert_eq!(rows[0].correlation_id, receipt.attempt_id);
            let blake3 = rows[0]
                .payload_blake3
                .clone()
                .expect("payload hash recorded");
            assert!(!blake3.is_empty());
            assert_eq!(rows[1].phase, AuditPhase::Outcome);
            assert_eq!(rows[1].outcome, Some(AuditOutcome::Ok));
            assert!(
                rows[1].detail_json.contains(&receipt.message_id),
                "the outcome names the message id: {}",
                rows[1].detail_json
            );

            // The attempt is the evidence, and it says sent.
            let attempts = harness.outbox.attempts();
            assert_eq!(attempts.len(), 1);
            assert_eq!(attempts[0].state, AttemptState::Sent);
            assert_eq!(attempts[0].sent_at.as_deref(), Some(NOW));
            assert_eq!(attempts[0].payload_blake3, blake3);

            // The card moved through the Tracker's one writer (AD-16).
            assert_eq!(
                harness.apply.stored_status(&application.id),
                Some(AppStatus::Applied)
            );
            let events = harness.apply.events();
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].source.as_str(), "apply_outcome");
            assert_eq!(receipt.status, "applied");
        });
    }

    #[test]
    fn a_package_that_is_not_ready_is_refused_and_nothing_leaves() {
        block_on(async {
            let harness = harness(FakeMail::default());
            let application = application();
            let version = seed_document(
                &harness.documents,
                "doc_cv_1",
                &application.id,
                DocumentKind::Cv,
                unbacked_model(DocumentKind::Cv),
                true,
            );
            harness
                .apply
                .seed_package(&application.id, package_item("doc_cv_1", "cv", version));

            let error = engine(&harness)
                .send(
                    &application,
                    &offer(Some("jobs@nimbus.example")),
                    &request(),
                )
                .await
                .unwrap_err();

            assert_eq!(error.message_key(), "errors.apply.not_ready");
            assert!(harness.mail.sent().is_empty(), "AD-14: nothing is sent");
            assert!(harness.outbox.attempts().is_empty());
            assert!(harness.system.rows().is_empty());
        });
    }

    #[test]
    fn a_second_send_is_refused_by_the_evidence_of_the_first() {
        block_on(async {
            let harness = harness(FakeMail::default());
            let application = application();
            harness.apply.seed_application(application.clone());
            seed_verified_cv(&harness, &application);
            let posting = offer(Some("jobs@nimbus.example"));

            engine(&harness)
                .send(&application, &posting, &request())
                .await
                .unwrap();
            let mut applied = application.clone();
            applied.status = AppStatus::Applied;
            let error = engine(&harness)
                .send(&applied, &posting, &request())
                .await
                .unwrap_err();

            assert_eq!(error.message_key(), "errors.apply.already_sent");
            assert_eq!(harness.mail.sent().len(), 1, "AC2: never twice");
            assert_eq!(harness.outbox.attempts().len(), 1);
        });
    }

    #[test]
    fn a_transport_failure_is_uncertain_and_needs_the_users_word() {
        block_on(async {
            let harness = harness(FakeMail::failing(MailError::Transport(
                "offline".to_string(),
            )));
            let application = application();
            harness.apply.seed_application(application.clone());
            seed_verified_cv(&harness, &application);
            let posting = offer(Some("jobs@nimbus.example"));

            let error = engine(&harness)
                .send(&application, &posting, &request())
                .await
                .unwrap_err();
            assert_eq!(error.message_key(), "errors.apply.send_unreachable");
            assert_eq!(harness.mail.sent().len(), 1);

            // The attempt is ambiguous, not failed: the outcome says so.
            let attempts = harness.outbox.attempts();
            assert_eq!(attempts[0].state, AttemptState::Uncertain);
            let rows = harness.system.rows();
            assert_eq!(rows[1].outcome, Some(AuditOutcome::Failed));
            assert!(
                rows[1].detail_json.contains("uncertain")
                    || rows[1].detail_json.contains("unreachable")
            );
            assert_eq!(
                harness.apply.stored_status(&application.id),
                Some(AppStatus::Drafted),
                "a failed send does not move the card"
            );

            // A plain retry is refused: the mail may already have gone.
            let error = engine(&harness)
                .send(&application, &posting, &request())
                .await
                .unwrap_err();
            assert_eq!(error.message_key(), "errors.apply.confirm_resend");
            assert_eq!(harness.mail.sent().len(), 1);

            // With the user's word it goes again, and the second attempt is the
            // one that proves the send.
            let confirmed = SendRequest {
                confirm_resend: true,
                ..request()
            };
            engine(&harness)
                .send(&application, &posting, &confirmed)
                .await
                .unwrap();
            assert_eq!(harness.mail.sent().len(), 2);
            let attempts = harness.outbox.attempts();
            assert_eq!(attempts[1].state, AttemptState::Sent);
        });
    }

    #[test]
    fn a_rejected_message_is_failed_and_may_be_retried_at_once() {
        block_on(async {
            let harness = harness(FakeMail::answers(vec![
                Some(MailError::Rejected("mailbox full".to_string())),
                Some(MailError::Rejected("mailbox full".to_string())),
            ]));
            let application = application();
            harness.apply.seed_application(application.clone());
            seed_verified_cv(&harness, &application);
            let posting = offer(Some("jobs@nimbus.example"));

            let error = engine(&harness)
                .send(&application, &posting, &request())
                .await
                .unwrap_err();
            assert_eq!(error.message_key(), "errors.apply.send_rejected");
            assert_eq!(harness.outbox.attempts()[0].state, AttemptState::Failed);

            // A rejection proves nothing left the account, so a retry is free.
            // The fake answers every call with the same rejection.
            let error = engine(&harness)
                .send(&application, &posting, &request())
                .await
                .unwrap_err();
            assert_eq!(error.message_key(), "errors.apply.send_rejected");
            assert_eq!(harness.mail.sent().len(), 2);
        });
    }

    #[test]
    fn an_attempt_that_never_reported_back_asks_for_a_check() {
        block_on(async {
            let harness = harness(FakeMail::default());
            let application = application();
            seed_verified_cv(&harness, &application);
            harness.outbox.seed(ApplyAttempt {
                id: "att_crash".into(),
                application_id: application.id.clone(),
                channel: "email".into(),
                account_id: "eml_1".into(),
                recipient: "jobs@nimbus.example".into(),
                subject: "Application: Backend Engineer".into(),
                payload_blake3: "deadbeef".into(),
                message_id: Some("<att_crash@boba.local>".into()),
                state: AttemptState::Pending,
                error: None,
                created_at: NOW.into(),
                updated_at: NOW.into(),
                sent_at: None,
            });

            let error = engine(&harness)
                .send(
                    &application,
                    &offer(Some("jobs@nimbus.example")),
                    &request(),
                )
                .await
                .unwrap_err();

            assert_eq!(error.message_key(), "errors.apply.needs_check");
            assert!(harness.mail.sent().is_empty(), "AC2: evidence first");
        });
    }

    #[test]
    fn the_users_subject_edit_is_kept_on_one_line() {
        block_on(async {
            let harness = harness(FakeMail::default());
            let application = application();
            harness.apply.seed_application(application.clone());
            seed_verified_cv(&harness, &application);
            let edited = SendRequest {
                subject: Some("Hello\nBcc: evil@attacker.example".into()),
                body_text: Some("My body".into()),
                ..request()
            };

            engine(&harness)
                .send(&application, &offer(Some("jobs@nimbus.example")), &edited)
                .await
                .unwrap();

            let sent = harness.mail.sent();
            assert_eq!(sent[0].1.subject, "Hello Bcc: evil@attacker.example");
            assert_eq!(sent[0].1.body_text, "My body");
            assert_eq!(
                sent[0].1.to,
                vec!["jobs@nimbus.example"],
                "no extra recipient"
            );
        });
    }

    #[test]
    fn an_empty_subject_is_refused() {
        block_on(async {
            let harness = harness(FakeMail::default());
            let application = application();
            seed_verified_cv(&harness, &application);
            let edited = SendRequest {
                subject: Some("   \n ".into()),
                ..request()
            };

            let error = engine(&harness)
                .send(&application, &offer(Some("jobs@nimbus.example")), &edited)
                .await
                .unwrap_err();

            assert_eq!(error.message_key(), "errors.apply.empty_subject");
            assert!(harness.mail.sent().is_empty());
            assert!(harness.outbox.attempts().is_empty());
        });
    }

    #[test]
    fn sending_from_no_account_is_refused_before_anything_is_written() {
        block_on(async {
            let harness = harness(FakeMail::default());
            let application = application();
            seed_verified_cv(&harness, &application);
            let no_account = SendRequest {
                account_id: "  ".into(),
                ..request()
            };

            let error = engine(&harness)
                .send(
                    &application,
                    &offer(Some("jobs@nimbus.example")),
                    &no_account,
                )
                .await
                .unwrap_err();

            assert_eq!(error.message_key(), "errors.apply.unknown_account");
            assert!(harness.outbox.attempts().is_empty());
            assert!(harness.system.rows().is_empty());
        });
    }

    #[test]
    fn every_error_maps_to_a_translated_key() {
        for (error, key) in [
            (EngineError::NoEmailChannel, "errors.apply.no_email_channel"),
            (
                EngineError::AlreadySent("m".into()),
                "errors.apply.already_sent",
            ),
            (EngineError::NeedsCheck, "errors.apply.needs_check"),
            (EngineError::ConfirmResend, "errors.apply.confirm_resend"),
            (EngineError::EmptySubject, "errors.apply.empty_subject"),
            (
                EngineError::Mail(MailError::Transport("x".into())),
                "errors.apply.send_unreachable",
            ),
            (
                EngineError::Mail(MailError::Rejected("x".into())),
                "errors.apply.send_rejected",
            ),
            (
                EngineError::Mail(MailError::Reauthorize),
                "errors.apply.reauthorize",
            ),
            (
                EngineError::Mail(MailError::UnknownAccount),
                "errors.apply.unknown_account",
            ),
        ] {
            assert_eq!(error.message_key(), key);
        }
    }

    #[test]
    fn the_payload_hash_covers_the_attachment_bytes() {
        let message = OutgoingMessage {
            to: vec!["jobs@nimbus.example".into()],
            subject: "Application: Backend Engineer".into(),
            body_text: "Hello".into(),
            attachments: vec![Attachment {
                filename: "cv-v1.pdf".into(),
                mime: "application/pdf".into(),
                bytes: b"%PDF-1.7".to_vec(),
            }],
        };
        let first = payload_bytes(&message);
        let mut second = message;
        second.attachments[0].bytes = b"%PDF-1.8".to_vec();
        assert_ne!(payload_bytes(&second), first);
    }
}
