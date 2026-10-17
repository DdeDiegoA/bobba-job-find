//! Studio commands (S7.7, FR-78/FR-79, EXPERIENCE §9.5). Thin wrappers over
//! [`StudioService`] and the agent runtime: resolve the ports, call the use case,
//! map the error (AD-3).
//!
//! The store arrives through [`wiring::port`] (ADR-012): a locked or unmounted
//! store is `errors.store.locked`, never a panic.
//!
//! Two commands start agent runs: `studio_generate` drafts (and has the drafts
//! reviewed and revised), `studio_regenerate` does the same and then copies one
//! section of the fresh draft into the version the user was reading. A run's
//! drafts reach the store through the `save_document_draft` tool
//! (`wiring::agent`), never through this layer: the agent proposes, the core
//! disposes (AD-5).
//!
//! The DOCX export is written to `<app data>/exports/` and its path returned:
//! the webview picks nothing, so no page can ask the core to write wherever it
//! likes.

use std::path::PathBuf;
use std::sync::Arc;

use boba_app::error::{AppError, AppResult, ErrorCode};
use boba_app::fitting::PageBudget;
use boba_app::ports::agent::{
    AgentEvent, AgentPort, AgentTask, AgentTaskKind, InputTrust, RunBudget,
};
use boba_app::ports::cli::CliInstallsPort;
use boba_app::ports::render::RenderPort;
use boba_app::ports::repo::apply::ApplyRepo;
use boba_app::ports::repo::documents::DocumentsRepo;
use boba_app::ports::repo::ingestion::IngestionRepo;
use boba_app::ports::repo::profile::ProfileRepo;
use boba_app::ports::verify::VerifyPort;
use boba_app::review::{self, Review, ReviewPass, Step};
use boba_app::studio::{
    self, EditInput, RegenerateInput, StudioError, StudioKind, StudioService, StudioView,
};
use boba_app::system::SystemClock;
use boba_app::tailoring::Drafts;
use boba_domain::application::Application;
use boba_domain::documents::{CoverLetterDocument, CvDocument, DocumentKind, Lang};
use boba_domain::posting::Offer;
use boba_domain::profile::ProfileFact;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::Manager;

use crate::wiring;

/// The command-level failures this slice raises. The core's own failures come
/// from `StudioError`; these are the ones only a command can hit (a missing row,
/// a run that died, a file that could not be written), and each carries its own
/// i18n key (CH7: `errors.studio.*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandError {
    Store,
    UnknownApplication,
    UnknownOffer,
    ExportWrite,
    ExportPath,
    Run,
    StoredDraft,
}

impl CommandError {
    /// The key the UI translates (the gate scans `message_key` fns, AC4).
    const fn message_key(self) -> &'static str {
        match self {
            CommandError::Store => "errors.studio.store",
            CommandError::UnknownApplication => "errors.studio.unknown_application",
            CommandError::UnknownOffer => "errors.studio.unknown_offer",
            CommandError::ExportWrite => "errors.studio.export_write",
            CommandError::ExportPath => "errors.studio.export_path",
            CommandError::Run => "errors.studio.run",
            CommandError::StoredDraft => "errors.studio.stored_draft",
        }
    }

    fn app_error(self) -> AppError {
        let code = match self {
            CommandError::UnknownApplication | CommandError::UnknownOffer => ErrorCode::NotFound,
            CommandError::Store
            | CommandError::ExportWrite
            | CommandError::ExportPath
            | CommandError::Run
            | CommandError::StoredDraft => ErrorCode::Internal,
        };
        AppError::new(code, self.message_key())
    }
}

/// The page budget the studio renders with. FR-76 allows 1–3 pages; the user
/// preference that sets it is not in the preferences shape yet, so a run uses
/// the default and the studio shows the number it actually used.
const PAGE_BUDGET: PageBudget = PageBudget::DEFAULT;

/// What one studio read needs from the store: the application, its Offer and the
/// Profile the drafts are checked against.
struct Context {
    application: Application,
    offer: Offer,
    facts: Vec<ProfileFact>,
}

fn service(app: &tauri::AppHandle) -> AppResult<StudioService> {
    Ok(StudioService::new(
        wiring::port::<dyn DocumentsRepo>(app)?,
        wiring::port::<dyn RenderPort>(app)?,
        wiring::port::<dyn VerifyPort>(app)?,
        Arc::new(SystemClock),
    ))
}

/// Map a use-case failure onto the typed IPC error (AD-3).
fn to_app(error: StudioError) -> AppError {
    let code = match &error {
        StudioError::NoDraft { .. } | StudioError::NoEngine | StudioError::AmbiguousEngine => {
            ErrorCode::InvalidInput
        }
        StudioError::InvalidEdit | StudioError::Model(_) => ErrorCode::InvalidInput,
        StudioError::Repo(_) | StudioError::Render(_) | StudioError::Verify(_) => {
            ErrorCode::Internal
        }
        StudioError::Fit(_) => ErrorCode::InvalidInput,
    };
    AppError::new(code, error.message_key())
}

fn required_id(id: String) -> AppResult<String> {
    if id.trim().is_empty() {
        return Err(AppError::invalid_input("errors.studio.missing_id"));
    }
    Ok(id)
}

fn context(app: &tauri::AppHandle, application_id: &str) -> AppResult<Context> {
    let application = wiring::port::<dyn ApplyRepo>(app)?
        .application_get(application_id)
        .map_err(|_| CommandError::Store.app_error())?
        .ok_or_else(|| CommandError::UnknownApplication.app_error())?;
    let offer = wiring::port::<dyn IngestionRepo>(app)?
        .get_offer(&application.posting_id)
        .map_err(|_| CommandError::Store.app_error())?
        .ok_or_else(|| CommandError::UnknownOffer.app_error())?;
    let facts = wiring::port::<dyn ProfileRepo>(app)?
        .facts_current()
        .map_err(|_| CommandError::Store.app_error())?;
    Ok(Context {
        application,
        offer,
        facts,
    })
}

/// The studio: both drafts with their trace, coverage, findings and checks.
#[tauri::command]
#[specta::specta]
pub fn studio_view(app: tauri::AppHandle, application_id: String) -> AppResult<StudioView> {
    let application_id = required_id(application_id)?;
    let context = context(&app, &application_id)?;
    service(&app)?
        .view(
            &context.application,
            &context.offer,
            &context.facts,
            PAGE_BUDGET,
        )
        .map_err(to_app)
}

/// Save one inline edit as a new draft version and re-run the checks (AC1).
/// Off the UI thread: the re-check renders a PDF.
#[tauri::command]
#[specta::specta]
pub async fn studio_edit(app: tauri::AppHandle, input: EditInput) -> AppResult<StudioView> {
    let input = EditInput {
        application_id: required_id(input.application_id)?,
        ..input
    };
    tauri::async_runtime::spawn_blocking(move || {
        let context = context(&app, &input.application_id)?;
        service(&app)?
            .edit(
                &context.application,
                &context.offer,
                &context.facts,
                PAGE_BUDGET,
                &input,
            )
            .map_err(to_app)
    })
    .await
    .map_err(|_| CommandError::Run.app_error())?
}

/// Render both drafts: fit, render, verify, and store the verdict (AC1). Off
/// the UI thread: Typst compiles two documents.
#[tauri::command]
#[specta::specta]
pub async fn studio_render(app: tauri::AppHandle, application_id: String) -> AppResult<StudioView> {
    let application_id = required_id(application_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let context = context(&app, &application_id)?;
        service(&app)?
            .render(
                &context.application,
                &context.offer,
                &context.facts,
                PAGE_BUDGET,
            )
            .map_err(to_app)
    })
    .await
    .map_err(|_| CommandError::Run.app_error())?
}

/// A DOCX exported to `<app data>/exports/` (FR-78).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ExportedFile {
    /// Where the file is, so the UI can open or reveal it.
    pub path: String,
    pub file_name: String,
    pub version: u32,
}

/// Export the newest draft of one kind as a DOCX (AC2).
#[tauri::command]
#[specta::specta]
pub async fn studio_export_docx(
    app: tauri::AppHandle,
    application_id: String,
    kind: StudioKind,
) -> AppResult<ExportedFile> {
    let application_id = required_id(application_id)?;
    let service = service(&app)?;
    let application = wiring::port::<dyn ApplyRepo>(&app)?
        .application_get(&application_id)
        .map_err(|_| CommandError::Store.app_error())?
        .ok_or_else(|| CommandError::UnknownApplication.app_error())?;
    let export = service.export_docx(&application, kind).map_err(to_app)?;
    let dir = exports_dir(&app)?;
    let path = dir.join(&export.file_name);
    std::fs::write(&path, &export.bytes).map_err(|_| CommandError::ExportWrite.app_error())?;
    Ok(ExportedFile {
        path: path.to_string_lossy().to_string(),
        file_name: export.file_name,
        version: export.version,
    })
}

/// Generate (or regenerate) the application's drafts: tailor, review, revise
/// (AC1, architecture §5.5). Runs off the UI thread: it waits for the CLI.
#[tauri::command]
#[specta::specta]
pub async fn studio_generate(
    app: tauri::AppHandle,
    application_id: String,
) -> AppResult<StudioView> {
    let application_id = required_id(application_id)?;
    tauri::async_runtime::spawn_blocking(move || generate(&app, &application_id))
        .await
        .map_err(|_| CommandError::Run.app_error())?
}

/// Regenerate one section: run the generation flow, then copy that section of
/// the fresh draft into the version the user was reading (AC1).
#[tauri::command]
#[specta::specta]
pub async fn studio_regenerate(
    app: tauri::AppHandle,
    application_id: String,
    kind: StudioKind,
    section: Option<u32>,
    into: String,
) -> AppResult<StudioView> {
    let application_id = required_id(application_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        generate(&app, &application_id)?;
        let context = context(&app, &application_id)?;
        service(&app)?
            .replace_section(
                &context.application,
                &context.offer,
                &context.facts,
                PAGE_BUDGET,
                &RegenerateInput {
                    application_id,
                    kind,
                    section,
                    into,
                },
            )
            .map_err(to_app)
    })
    .await
    .map_err(|_| CommandError::Run.app_error())?
}

/// `<app data>/exports`, created on first use.
fn exports_dir(app: &tauri::AppHandle) -> AppResult<PathBuf> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|_| CommandError::ExportPath.app_error())?;
    let dir = base.join("exports");
    std::fs::create_dir_all(&dir).map_err(|_| CommandError::ExportPath.app_error())?;
    Ok(dir)
}

/// The generation flow: draft, review from a fresh context, revise once
/// (architecture §5.5 steps 2–4), then read the studio back.
fn generate(app: &tauri::AppHandle, application_id: &str) -> AppResult<StudioView> {
    let context = context(app, application_id)?;
    let service = service(app)?;
    let agents = wiring::port::<dyn AgentPort>(app)?;
    let installed = wiring::port::<dyn CliInstallsPort>(app)?
        .installed()
        .map_err(|_| CommandError::Store.app_error())?;
    let cli = studio::pick_cli(&installed).map_err(to_app)?;
    let documents = wiring::port::<dyn DocumentsRepo>(app)?;
    let language = draft_language(&documents, &context);

    // 1. draft both documents.
    run(
        &agents,
        &cli,
        AgentTaskKind::TailorApplication,
        tailor_input(&context),
        &language,
    )?;
    // 2. a fresh-context review of what the run drafted.
    if let Some(drafts) = current_drafts(&documents, &context.application.id)? {
        run(
            &agents,
            &cli,
            AgentTaskKind::ReviewApplication,
            review::review_input(&context.application.id, &context.offer.id, &drafts),
            &language,
        )?;
        // 3. one revision when the critique asks for one (ReviewPass caps it).
        if let Some(review) = newest_review(&documents, &context.application.id)? {
            let mut pass = ReviewPass::new();
            if pass.step(&review) == Step::Revise {
                run(
                    &agents,
                    &cli,
                    AgentTaskKind::TailorApplication,
                    review::revision_input(
                        &context.application.id,
                        &context.offer.id,
                        &drafts,
                        &review,
                    ),
                    &language,
                )?;
                pass.revised();
            }
        }
    }
    service
        .view(
            &context.application,
            &context.offer,
            &context.facts,
            PAGE_BUDGET,
        )
        .map_err(to_app)
}

/// The language the run writes in (FR-70/AD-17): the drafts' own language once
/// there is one, otherwise the Offer's.
fn draft_language(documents: &Arc<dyn DocumentsRepo>, context: &Context) -> String {
    documents
        .latest(&context.application.id, DocumentKind::Cv)
        .ok()
        .flatten()
        .map(|record| record.lang.code().to_string())
        .unwrap_or_else(|| {
            Lang::for_offer(&context.offer.lang, None)
                .lang
                .code()
                .to_string()
        })
}

/// The run's own job: which application, which Offer (the skill reads the rest
/// through its tools).
fn tailor_input(context: &Context) -> String {
    serde_json::json!({
        "app_id": context.application.id,
        "posting_id": context.offer.id,
    })
    .to_string()
}

/// The newest draft of each kind, as the review and revision runs read them.
fn current_drafts(
    documents: &Arc<dyn DocumentsRepo>,
    application_id: &str,
) -> AppResult<Option<Drafts>> {
    let records = documents
        .list_for_application(application_id)
        .map_err(|_| CommandError::Store.app_error())?;
    let newest = |kind: DocumentKind| {
        records
            .iter()
            .filter(|record| record.kind == kind)
            .max_by_key(|record| record.version)
    };
    let (Some(cv), Some(cover)) = (newest(DocumentKind::Cv), newest(DocumentKind::CoverLetter))
    else {
        return Ok(None);
    };
    let cv: CvDocument =
        serde_json::from_str(&cv.model_json).map_err(|_| CommandError::StoredDraft.app_error())?;
    let cover: CoverLetterDocument = serde_json::from_str(&cover.model_json)
        .map_err(|_| CommandError::StoredDraft.app_error())?;
    Ok(Some(Drafts { cv, cover }))
}

/// The critique the review run stored, as the revision decision reads it.
fn newest_review(
    documents: &Arc<dyn DocumentsRepo>,
    application_id: &str,
) -> AppResult<Option<Review>> {
    let records = documents
        .list_for_application(application_id)
        .map_err(|_| CommandError::Store.app_error())?;
    for record in records.iter().rev() {
        let Some(json) = &record.review_json else {
            continue;
        };
        let findings =
            serde_json::from_str(json).map_err(|_| CommandError::StoredDraft.app_error())?;
        return Ok(Some(Review {
            application_id: application_id.to_string(),
            findings,
        }));
    }
    Ok(None)
}

/// Start one run and wait for it, mapping a failed run onto the typed error.
fn run(
    agents: &Arc<dyn AgentPort>,
    cli: &str,
    kind: AgentTaskKind,
    prompt: String,
    target_language: &str,
) -> AppResult<()> {
    let handle = agents.run(AgentTask {
        kind,
        // Both runs read the posting (and the drafts), which nobody in this app
        // vouched for: the lockdown gate decides whether the CLI may (ADR-010).
        input_trust: InputTrust::Untrusted,
        cli: cli.to_string(),
        model: None,
        prompt,
        target_language: target_language.to_string(),
        budget: RunBudget::default(),
        env: Vec::new(),
    });
    let events: Vec<AgentEvent> = handle.events.iter().collect();
    for event in events {
        if let AgentEvent::Failed { message_key, .. } = event {
            return Err(AppError::new(ErrorCode::Internal, message_key));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_map_to_codes_and_i18n_keys() {
        let mapped = to_app(StudioError::NoDraft {
            application_id: "app_1".to_string(),
            kind: "cv",
        });
        assert_eq!(mapped.code, ErrorCode::InvalidInput);
        assert_eq!(mapped.message_key, "errors.studio.no_draft");

        let mapped = to_app(StudioError::InvalidEdit);
        assert_eq!(mapped.code, ErrorCode::InvalidInput);
        assert_eq!(mapped.message_key, "errors.studio.invalid_edit");

        let mapped = to_app(StudioError::AmbiguousEngine);
        assert_eq!(mapped.code, ErrorCode::InvalidInput);
        assert_eq!(mapped.message_key, "errors.studio.ambiguous_engine");

        let mapped = to_app(StudioError::Repo(
            boba_app::ports::repo::documents::DocumentsRepoError::new("disk"),
        ));
        assert_eq!(mapped.code, ErrorCode::Internal);
        assert_eq!(mapped.message_key, "errors.studio.store");
    }

    #[test]
    fn a_blank_application_id_is_rejected_before_touching_the_store() {
        assert_eq!(
            required_id("  ".to_string()).unwrap_err().message_key,
            "errors.studio.missing_id"
        );
        assert_eq!(required_id("app_1".to_string()).unwrap(), "app_1");
    }

    #[test]
    fn the_run_input_names_the_application_and_the_offer() {
        let context = Context {
            application: Application {
                id: "app_1".into(),
                posting_id: "pst_1".into(),
                status: boba_domain::application::AppStatus::Saved,
                offer_snapshot_json: "{}".into(),
                notes: None,
                next_action: None,
                created_at: "t".into(),
                updated_at: "t".into(),
            },
            offer: Offer {
                id: "pst_1".into(),
                source_id: "src_1".into(),
                external_id: "ext_1".into(),
                title: "Backend engineer".into(),
                company: "Acme".into(),
                locations: vec![],
                work_mode: boba_domain::posting::WorkMode::Remote,
                money: None,
                posted_at: None,
                deadline: None,
                first_seen: "t".into(),
                last_seen: "t".into(),
                lang: "es-MX".into(),
                description_html: String::new(),
                description_text: String::new(),
                requirements: vec![],
                apply: boba_domain::posting::ApplyHints {
                    url: "https://acme.example/1".into(),
                    email: None,
                },
            },
            facts: vec![],
        };
        let input: serde_json::Value =
            serde_json::from_str(&tailor_input(&context)).expect("the job is JSON");
        assert_eq!(input["app_id"], "app_1");
        assert_eq!(input["posting_id"], "pst_1");
    }
}
