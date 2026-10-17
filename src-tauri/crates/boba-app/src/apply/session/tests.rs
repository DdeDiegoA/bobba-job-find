//! The Session Board apply use case (S11.2, FR-91, AD-10 `Review`).

use std::sync::{Arc, Mutex};

use boba_domain::application::{
    AnswerClass, AppStatus, Application, NewBankAnswer, PackageItem, question_key,
};
use boba_domain::documents::{DocumentKind, Lang, NewDocument, TemplateId};
use boba_domain::posting::{ApplyHints, Offer, WorkMode};
use boba_domain::source::{
    AccessClass, ApplyCapability, LegalTier, OptInRecord, RunStatus, SourceRecord, SourceRunRecord,
};

use super::*;
use crate::applications::testing::FakeApplyRepo;
use crate::ports::browser::{
    BrowserError, Confirmation, PageField, PageFieldKind, Screenshot, Upload,
};
use crate::ports::repo::documents::fake::FakeDocumentsRepo;
use crate::ports::repo::sources::{SourcesRepo, SourcesRepoError};
use crate::profile::testing::FakeProfileRepo;
use crate::system::{FixedClock, testing::FakeSystemRepo};

const NOW: &str = "2026-10-17T00:00:00Z";
const CV_PDF: &[u8] = b"%PDF-1.7 cv";
const JOB_URL: &str = "https://www.linkedin.com/jobs/view/123/?refId=abc";

// ---- fakes ----------------------------------------------------------------

/// A session browser the test scripts: what the page says and what it was asked.
#[derive(Default)]
struct FakeSessionBrowser {
    page: Mutex<SessionPage>,
    fills: Mutex<Vec<(String, String)>>,
    attached: Mutex<Vec<(String, Upload)>>,
    opened: Mutex<Vec<(String, String)>>,
    closes: Mutex<u32>,
    live: Mutex<bool>,
}

impl FakeSessionBrowser {
    fn with_page(page: SessionPage) -> Self {
        let browser = Self::default();
        *browser.page.lock().unwrap() = page;
        browser
    }

    fn set(&self, page: SessionPage) {
        *self.page.lock().unwrap() = page;
    }
}

impl SessionFormPort for FakeSessionBrowser {
    fn open(&self, source_id: &str, url: &str) -> Result<SessionPage, BrowserError> {
        self.opened
            .lock()
            .unwrap()
            .push((source_id.to_string(), url.to_string()));
        *self.live.lock().unwrap() = true;
        Ok(self.page.lock().unwrap().clone())
    }

    fn snapshot(&self) -> Result<SessionPage, BrowserError> {
        if !*self.live.lock().unwrap() {
            return Err(BrowserError::NotRunning);
        }
        Ok(self.page.lock().unwrap().clone())
    }

    fn fill_text(&self, handle: &str, value: &str) -> Result<(), BrowserError> {
        self.fills
            .lock()
            .unwrap()
            .push((handle.to_string(), value.to_string()));
        Ok(())
    }

    fn choose(&self, handle: &str) -> Result<(), BrowserError> {
        self.fills
            .lock()
            .unwrap()
            .push((handle.to_string(), "chosen".to_string()));
        Ok(())
    }

    fn attach(&self, handle: &str, file: &Upload) -> Result<(), BrowserError> {
        self.attached
            .lock()
            .unwrap()
            .push((handle.to_string(), file.clone()));
        Ok(())
    }

    fn screenshot(&self) -> Result<Screenshot, BrowserError> {
        if !*self.live.lock().unwrap() {
            return Err(BrowserError::NotRunning);
        }
        Ok(Screenshot {
            png_blake3: "hash-of-png".to_string(),
        })
    }

    fn close(&self) -> Result<(), BrowserError> {
        *self.closes.lock().unwrap() += 1;
        *self.live.lock().unwrap() = false;
        Ok(())
    }
}

/// An in-memory `SourcesRepo` with the `source` and `source_optin` parts this
/// use case reads (the store's SQL is tested in `boba-store`).
#[derive(Default)]
struct FakeSources {
    records: Mutex<Vec<SourceRecord>>,
    optins: Mutex<Vec<OptInRecord>>,
}

impl FakeSources {
    fn with_board(source_id: &str, opted_in: bool) -> Self {
        let sources = Self::default();
        sources.add_board(source_id, opted_in);
        sources
    }

    /// Add one configured board (and its opt-in when `opted_in`).
    fn add_board(&self, source_id: &str, opted_in: bool) {
        self.records.lock().unwrap().push(source_record(source_id));
        if opted_in {
            self.optins.lock().unwrap().push(OptInRecord {
                source_id: source_id.to_string(),
                warning_version: "v1".to_string(),
                accepted_at: NOW.to_string(),
            });
        }
    }
}

impl SourcesRepo for FakeSources {
    fn upsert_source(&self, record: &SourceRecord) -> Result<(), SourcesRepoError> {
        self.records.lock().unwrap().push(record.clone());
        Ok(())
    }
    fn get_source(&self, id: &str) -> Result<Option<SourceRecord>, SourcesRepoError> {
        Ok(self
            .records
            .lock()
            .unwrap()
            .iter()
            .find(|row| row.id == id)
            .cloned())
    }
    fn list_sources(&self) -> Result<Vec<SourceRecord>, SourcesRepoError> {
        Ok(self.records.lock().unwrap().clone())
    }
    fn set_source_enabled(
        &self,
        id: &str,
        enabled: bool,
        updated_at: &str,
    ) -> Result<(), SourcesRepoError> {
        let mut rows = self.records.lock().unwrap();
        let row = rows
            .iter_mut()
            .find(|row| row.id == id)
            .ok_or_else(|| SourcesRepoError::Invalid(id.to_string()))?;
        row.enabled = enabled;
        row.updated_at = updated_at.to_string();
        Ok(())
    }
    fn record_optin(&self, optin: &OptInRecord) -> Result<(), SourcesRepoError> {
        self.optins.lock().unwrap().push(optin.clone());
        Ok(())
    }
    fn get_optin(&self, source_id: &str) -> Result<Option<OptInRecord>, SourcesRepoError> {
        Ok(self
            .optins
            .lock()
            .unwrap()
            .iter()
            .find(|row| row.source_id == source_id)
            .cloned())
    }
    fn remove_optin(&self, source_id: &str) -> Result<(), SourcesRepoError> {
        self.optins
            .lock()
            .unwrap()
            .retain(|row| row.source_id != source_id);
        Ok(())
    }
    fn opted_in_ids(&self) -> Result<Vec<String>, SourcesRepoError> {
        Ok(self
            .optins
            .lock()
            .unwrap()
            .iter()
            .map(|row| row.source_id.clone())
            .collect())
    }
    fn start_run(&self, _run: &SourceRunRecord) -> Result<(), SourcesRepoError> {
        Ok(())
    }
    fn recent_runs(
        &self,
        _source_id: &str,
        _limit: usize,
    ) -> Result<Vec<SourceRunRecord>, SourcesRepoError> {
        Ok(vec![])
    }
    fn finish_run(
        &self,
        _id: &str,
        _finished_at: &str,
        _status: RunStatus,
        _postings_found: i64,
        _error: Option<&str>,
    ) -> Result<(), SourcesRepoError> {
        Ok(())
    }
}

struct FakeKillSwitch(bool);

impl crate::ports::kill_switch::KillSwitchPort for FakeKillSwitch {
    fn engaged(&self) -> bool {
        self.0
    }
}

// ---- fixtures -------------------------------------------------------------

fn source_record(source_id: &str) -> SourceRecord {
    SourceRecord {
        id: source_id.to_string(),
        name: source_id.to_string(),
        access_class: AccessClass::SessionBrowser,
        legal_tier: LegalTier::ProhibitedAutomation,
        apply_capability: ApplyCapability::SessionForm,
        attribution: None,
        min_interval_ms: 30_000,
        daily_cap: Some(60),
        enabled: true,
        added_at: NOW.to_string(),
        updated_at: NOW.to_string(),
    }
}

fn field(handle: &str, label: &str, kind: PageFieldKind, required: bool) -> PageField {
    PageField {
        handle: handle.into(),
        label: label.into(),
        kind,
        required,
        options: vec![],
    }
}

fn page(fields: Vec<PageField>) -> SessionPage {
    SessionPage {
        url: JOB_URL.to_string(),
        title: "Nimbus | LinkedIn".into(),
        text: "Easy Apply".into(),
        fields,
        challenge: None,
        embedded_frame: false,
        confirmation: None,
    }
}

fn offer() -> Offer {
    Offer {
        id: "pst_01".into(),
        source_id: "linkedin".into(),
        external_id: "123".into(),
        title: "Backend Engineer".into(),
        company: "Nimbus".into(),
        locations: vec!["Remote (EU)".into()],
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
            url: JOB_URL.into(),
            email: None,
        },
    }
}

fn application() -> Application {
    Application {
        id: "app_01".into(),
        posting_id: "pst_01".into(),
        status: AppStatus::Drafted,
        offer_snapshot_json: "{}".into(),
        notes: None,
        next_action: None,
        created_at: NOW.into(),
        updated_at: NOW.into(),
    }
}

fn ready_model() -> String {
    serde_json::json!({
        "schema_version": 1,
        "lang": "en",
        "contact": {
            "full_name": "Ada Lovelace",
            "email": "ada@example.com",
            "phone": "+34 600 000 000",
            "location": "Madrid",
        },
        "sections": [],
    })
    .to_string()
}

fn seed_cv(apply: &FakeApplyRepo, documents: &FakeDocumentsRepo, application_id: &str) {
    let version = documents
        .insert(&NewDocument {
            id: "doc_cv".into(),
            application_id: application_id.into(),
            kind: DocumentKind::Cv,
            lang: Lang::En,
            template: TemplateId::new("cv-classic"),
            model_json: ready_model(),
            language_fallback_from: None,
            untrusted_origin: true,
            created_at: NOW.into(),
        })
        .unwrap();
    documents.set_pdf("doc_cv", CV_PDF, NOW).unwrap();
    documents.mark_verified("doc_cv", "{}", NOW).unwrap();
    apply.seed_package(
        application_id,
        PackageItem {
            document_id: "doc_cv".into(),
            kind: "cv".into(),
            lang: "en".into(),
            version,
            verified_at: Some(NOW.into()),
        },
    );
}

fn seed_answer(
    apply: &FakeApplyRepo,
    application_id: &str,
    id: &str,
    question: &str,
    answer: &str,
    class: AnswerClass,
    approved: bool,
) {
    apply
        .answer_bank_insert(&NewBankAnswer {
            id: id.into(),
            application_id: application_id.into(),
            question: question.into(),
            question_key: question_key(question),
            answer: Some(answer.into()),
            class,
            fact_ids: vec![],
            needs_review: false,
            untrusted_origin: true,
            created_at: NOW.into(),
        })
        .unwrap();
    if approved {
        apply.answer_bank_approve(id, None, NOW).unwrap();
    }
}

struct Harness {
    apply: Arc<FakeApplyRepo>,
    documents: Arc<FakeDocumentsRepo>,
    system: Arc<FakeSystemRepo>,
    browser: Arc<FakeSessionBrowser>,
    sources: Arc<FakeSources>,
    service: SessionApplyService,
}

fn harness(fields: Vec<PageField>) -> Harness {
    build_harness(fields, true, None)
}

fn build_harness(fields: Vec<PageField>, opted_in: bool, kill_switch: Option<bool>) -> Harness {
    let apply = Arc::new(FakeApplyRepo::default());
    let documents = Arc::new(FakeDocumentsRepo::default());
    let system = Arc::new(FakeSystemRepo::default());
    let sources = Arc::new(FakeSources::with_board("linkedin", opted_in));
    let browser = Arc::new(FakeSessionBrowser::with_page(page(fields)));
    let service = SessionApplyService::new(
        apply.clone(),
        documents.clone(),
        Arc::new(FakeProfileRepo::default()),
        sources.clone(),
        system.clone(),
        browser.clone(),
        Arc::new(FixedClock::new(NOW)),
    )
    .with_kill_switch(
        kill_switch.map(|engaged| Arc::new(FakeKillSwitch(engaged)) as Arc<dyn KillSwitchPort>),
    );
    Harness {
        apply,
        documents,
        system,
        browser,
        sources,
        service,
    }
}

// ---- gates ----------------------------------------------------------------

#[test]
fn starting_without_an_opt_in_is_refused() {
    let h = build_harness(vec![], false, None);
    let error = h.service.start(&application(), &offer()).unwrap_err();
    assert_eq!(error.message_key(), "errors.apply.session_opt_in");
    assert!(
        h.browser.opened.lock().unwrap().is_empty(),
        "no browser opens"
    );
    assert!(h.apply.events().is_empty(), "nothing is written");
}

#[test]
fn a_source_that_is_not_a_session_board_is_refused() {
    let h = harness(vec![]);
    let mut offer = offer();
    offer.source_id = "greenhouse".into();
    let error = h.service.start(&application(), &offer).unwrap_err();
    assert_eq!(error.message_key(), "errors.apply.session_not_board");
    assert!(h.browser.opened.lock().unwrap().is_empty());
}

#[test]
fn a_disabled_source_is_refused() {
    let h = harness(vec![]);
    h.sources.records.lock().unwrap()[0].enabled = false;
    let error = h.service.start(&application(), &offer()).unwrap_err();
    assert_eq!(error.message_key(), "errors.apply.session_opt_in");
    assert!(h.browser.opened.lock().unwrap().is_empty());
}

#[test]
fn a_paused_source_refuses_the_fill() {
    let h = harness(vec![]);
    h.system
        .setting_put(
            "session.pause.linkedin",
            r#"{"kind":"captcha","since":"2026-10-17T00:00:00Z"}"#,
            NOW,
        )
        .unwrap();
    let error = h.service.start(&application(), &offer()).unwrap_err();
    assert_eq!(error.message_key(), "errors.apply.session_paused");
    assert!(h.browser.opened.lock().unwrap().is_empty());
}

#[test]
fn the_kill_switch_stops_a_session_apply() {
    let h = build_harness(vec![], true, Some(true));
    let error = h.service.start(&application(), &offer()).unwrap_err();
    assert_eq!(error.message_key(), "errors.apply.session_kill_switch");
    assert!(
        h.browser.opened.lock().unwrap().is_empty(),
        "the switch is checked before any browser opens"
    );
}

#[test]
fn an_apply_url_off_the_board_is_refused() {
    for url in [
        "https://evil.test/jobs/view/123",
        "http://www.linkedin.com/jobs/view/123",
        "https://linkedin.com.evil.test/jobs/view/123",
    ] {
        let h = harness(vec![]);
        let mut offer = offer();
        offer.apply.url = url.into();
        let error = h.service.start(&application(), &offer).unwrap_err();
        assert_eq!(
            error.message_key(),
            "errors.apply.session_off_board",
            "{url}"
        );
        assert!(
            h.browser.opened.lock().unwrap().is_empty(),
            "the signed-in profile never visits another host ({url})"
        );
    }
}

#[test]
fn a_package_that_is_not_ready_is_refused_before_the_browser_opens() {
    let h = harness(vec![]);
    let application = application();
    // A draft in the package that never passed verification (AD-14).
    h.apply.seed_package(
        &application.id,
        PackageItem {
            document_id: "doc_cv".into(),
            kind: "cv".into(),
            lang: "en".into(),
            version: 1,
            verified_at: None,
        },
    );
    let error = h.service.start(&application, &offer()).unwrap_err();
    assert_eq!(error.message_key(), "errors.apply.not_ready");
    assert!(h.browser.opened.lock().unwrap().is_empty());
    assert!(h.apply.events().is_empty());
}

// ---- filling --------------------------------------------------------------

fn ready_harness(fields: Vec<PageField>) -> Harness {
    let h = harness(fields);
    let application = application();
    seed_cv(&h.apply, &h.documents, &application.id);
    h
}

#[test]
fn the_form_is_filled_from_the_cv_the_answers_and_the_verified_documents() {
    let h = ready_harness(vec![
        field("boba-1", "Email address", PageFieldKind::Email, true),
        field("boba-2", "Mobile phone number", PageFieldKind::Tel, true),
        field("boba-3", "Resume", PageFieldKind::File, true),
        field(
            "boba-4",
            "How many years of Rust?",
            PageFieldKind::Number,
            false,
        ),
        field("boba-5", "Cover letter", PageFieldKind::TextArea, false),
    ]);
    let application = application();
    seed_answer(
        &h.apply,
        &application.id,
        "ans_1",
        "How many years of Rust?",
        "7",
        AnswerClass::Other,
        true,
    );

    let view = h.service.start(&application, &offer()).unwrap();

    assert_eq!(view.source_id, "linkedin");
    assert_eq!(view.board, "linkedin");
    let fills = h.browser.fills.lock().unwrap().clone();
    assert!(fills.contains(&("boba-1".into(), "ada@example.com".into())));
    assert!(fills.contains(&("boba-2".into(), "+34 600 000 000".into())));
    assert!(fills.contains(&("boba-4".into(), "7".into())));
    assert_eq!(
        view.filled.len(),
        4,
        "three values typed and the resume attached"
    );
    assert_eq!(
        h.browser.attached.lock().unwrap().len(),
        1,
        "the verified CV is attached"
    );
    assert!(view.attached.contains(&"cv".to_string()));
    assert_eq!(
        view.unanswered.len(),
        1,
        "a cover-letter box with no verified document is left alone"
    );
    assert_eq!(view.unanswered[0].label, "Cover letter");
    assert!(view.handoff.is_none());
}

#[test]
fn an_unapproved_answer_is_never_typed() {
    let h = ready_harness(vec![field(
        "boba-1",
        "How many years of Rust?",
        PageFieldKind::Text,
        true,
    )]);
    let application = application();
    seed_answer(
        &h.apply,
        &application.id,
        "ans_1",
        "How many years of Rust?",
        "7",
        AnswerClass::Other,
        false,
    );

    let view = h.service.start(&application, &offer()).unwrap();

    assert!(
        h.browser.fills.lock().unwrap().is_empty(),
        "a draft stays a draft"
    );
    assert_eq!(
        view.handoff.expect("hands back").kind,
        SessionHandoffKind::UnknownRequired
    );
}

#[test]
fn a_login_wall_hands_control_back_and_fills_nothing() {
    let h = ready_harness(vec![field("boba-1", "Email", PageFieldKind::Email, true)]);
    let mut wall = page(vec![]);
    wall.challenge = Some(SessionChallenge::LoginWall);
    wall.text = "Sign in to continue".into();
    h.browser.set(wall);

    let view = h.service.start(&application(), &offer()).unwrap();

    assert!(h.browser.fills.lock().unwrap().is_empty());
    assert_eq!(
        view.handoff.expect("hands back").kind,
        SessionHandoffKind::LoginWall
    );
    assert_eq!(
        h.system.audit_unresolved_intents().unwrap().len(),
        1,
        "the attempt is still recorded before the user takes over"
    );
}

#[test]
fn an_apply_form_inside_a_frame_hands_control_back() {
    let h = ready_harness(vec![]);
    let mut framed = page(vec![]);
    framed.url = "https://www.indeed.com/viewjob?jk=abc".into();
    framed.embedded_frame = true;
    h.browser.set(framed);

    let mut offer = offer();
    offer.source_id = "indeed".into();
    offer.apply.url = "https://www.indeed.com/viewjob?jk=abc".into();
    h.sources.add_board("indeed", true);

    let view = h.service.start(&application(), &offer).unwrap();
    assert_eq!(
        view.handoff.expect("hands back").kind,
        SessionHandoffKind::EmbeddedFrame
    );
}

#[test]
fn the_audit_names_the_page_without_its_query() {
    let h = ready_harness(vec![]);
    let view = h.service.start(&application(), &offer()).unwrap();
    let rows = h.system.audit_unresolved_intents().unwrap();
    let row = rows
        .iter()
        .find(|row| row.action == AUDIT_ACTION)
        .expect("the attempt is audited");
    assert_eq!(row.correlation_id, view.attempt_id);
    let detail: serde_json::Value = serde_json::from_str(&row.detail_json).unwrap();
    assert_eq!(
        detail["url"], "https://www.linkedin.com/jobs/view/123/",
        "the query never reaches the audit log (S11.1b R-b)"
    );
    assert_eq!(detail["board"], "linkedin");
    assert!(
        !row.detail_json.contains("ada@example.com"),
        "no typed value is ever audited"
    );
}

// ---- confirming -----------------------------------------------------------

fn started(h: &Harness, application: &Application) -> String {
    h.service.start(application, &offer()).unwrap().attempt_id
}

#[test]
fn a_confirmed_submission_records_the_evidence_and_applies_the_card() {
    let h = ready_harness(vec![]);
    let application = application();
    h.apply.seed_application(application.clone());
    let attempt = started(&h, &application);

    let mut confirmed = page(vec![]);
    confirmed.url = "https://www.linkedin.com/jobs/view/123/".into();
    confirmed.text = "Your application was sent to Nimbus.".into();
    confirmed.confirmation = Some(Confirmation {
        text: "Your application was sent to Nimbus.".into(),
    });
    h.browser.set(confirmed);

    let outcome = h.service.confirm(&attempt, &application).unwrap();

    match outcome {
        SessionOutcome::Applied { evidence } => {
            assert_eq!(
                evidence.confirmation_text,
                "Your application was sent to Nimbus."
            );
            assert_eq!(evidence.screenshot_blake3, "hash-of-png");
        }
        other => panic!("expected Applied, got {other:?}"),
    }
    assert_eq!(*h.browser.closes.lock().unwrap(), 1, "the window closes");
    assert!(
        h.system.audit_unresolved_intents().unwrap().is_empty(),
        "the evidence resolves the intent"
    );
    assert_eq!(
        h.system.audit_verify_chain().unwrap().rows,
        2,
        "the intent/outcome pair is chained"
    );
    let events = h.apply.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].to_status, AppStatus::Applied);
    assert_eq!(events[0].source.as_str(), "apply_outcome");
}

#[test]
fn a_page_without_confirmation_marks_needs_check_and_the_card_stays() {
    let h = ready_harness(vec![]);
    let application = application();
    let attempt = started(&h, &application);

    let outcome = h.service.confirm(&attempt, &application).unwrap();

    assert!(matches!(outcome, SessionOutcome::NeedsCheck { .. }));
    assert!(
        h.system.audit_unresolved_intents().unwrap().is_empty(),
        "the attempt is resolved as a failed outcome"
    );
    assert_eq!(h.system.audit_verify_chain().unwrap().rows, 2);
    assert!(
        h.apply.events().is_empty(),
        "FR-88: no evidence means Needs check, and the card does not move"
    );
}

#[test]
fn a_confirmation_on_another_host_is_not_evidence() {
    let h = ready_harness(vec![]);
    let application = application();
    let attempt = started(&h, &application);

    let mut elsewhere = page(vec![]);
    elsewhere.url = "https://evil.test/thank-you".into();
    elsewhere.confirmation = Some(Confirmation {
        text: "Thank you for applying".into(),
    });
    h.browser.set(elsewhere);

    let outcome = h.service.confirm(&attempt, &application).unwrap();
    assert!(matches!(outcome, SessionOutcome::NeedsCheck { .. }));
}

#[test]
fn confirming_an_unknown_attempt_is_refused() {
    let h = ready_harness(vec![]);
    let error = h
        .service
        .confirm("session_nope", &application())
        .unwrap_err();
    assert_eq!(error.message_key(), "errors.apply.session_unknown_attempt");
}

#[test]
fn revoking_the_opt_in_stops_even_the_confirmation_read() {
    let h = ready_harness(vec![]);
    let application = application();
    let attempt = started(&h, &application);
    h.sources.optins.lock().unwrap().clear();

    let error = h.service.confirm(&attempt, &application).unwrap_err();
    assert_eq!(error.message_key(), "errors.apply.session_opt_in");
    assert_eq!(
        *h.browser.closes.lock().unwrap(),
        0,
        "the window is left alone"
    );
}

#[test]
fn a_later_step_is_filled_under_the_same_attempt() {
    let h = ready_harness(vec![field(
        "boba-1",
        "Email address",
        PageFieldKind::Email,
        true,
    )]);
    let application = application();
    let attempt = started(&h, &application);

    let mut next = page(vec![field(
        "boba-9",
        "How many years of Rust?",
        PageFieldKind::Text,
        false,
    )]);
    next.text = "Additional questions".into();
    h.browser.set(next);
    seed_answer(
        &h.apply,
        &application.id,
        "ans_1",
        "How many years of Rust?",
        "7",
        AnswerClass::Other,
        true,
    );

    let view = h.service.fill(&attempt, &application).unwrap();

    assert_eq!(view.attempt_id, attempt);
    assert!(
        h.browser
            .fills
            .lock()
            .unwrap()
            .contains(&("boba-9".into(), "7".into())),
        "the next step is filled on the same attempt"
    );
    assert_eq!(
        h.system.audit_unresolved_intents().unwrap().len(),
        1,
        "a step never opens a second attempt"
    );
}

#[test]
fn filling_again_checks_the_kill_switch_and_the_pause() {
    let h = ready_harness(vec![]);
    let application = application();
    let attempt = started(&h, &application);
    h.system
        .setting_put(
            "session.pause.linkedin",
            r#"{"kind":"rate_limited","since":"2026-10-17T00:00:00Z"}"#,
            NOW,
        )
        .unwrap();

    let error = h.service.fill(&attempt, &application).unwrap_err();
    assert_eq!(error.message_key(), "errors.apply.session_paused");
}

#[test]
fn the_board_decides_which_hosts_are_allowed() {
    assert!(SessionBoard::LinkedIn.allows("https://www.linkedin.com/jobs/view/1"));
    assert!(SessionBoard::LinkedIn.allows("https://es.linkedin.com/jobs/view/1"));
    assert!(!SessionBoard::LinkedIn.allows("https://linkedin.com.evil.test/x"));
    assert!(!SessionBoard::LinkedIn.allows("http://www.linkedin.com/jobs/view/1"));
    assert!(SessionBoard::Indeed.allows("https://smartapply.indeed.com/apply"));
    assert!(!SessionBoard::Indeed.allows("https://www.indeed.com.evil.test/x"));
    assert_eq!(
        SessionBoard::of_source("linkedin"),
        Some(SessionBoard::LinkedIn)
    );
    assert_eq!(
        SessionBoard::of_source("indeed"),
        Some(SessionBoard::Indeed)
    );
    assert_eq!(SessionBoard::of_source("greenhouse"), None);
}
