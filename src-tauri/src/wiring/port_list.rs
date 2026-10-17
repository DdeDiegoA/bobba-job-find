//! Port registration list (ADR-012 §3, `.gitattributes` `merge=union`).
//!
//! Append **one** `mount_port::<dyn Port>(app, store.clone());` line per port,
//! above the marker comment. Never reorder or remove a line: a duplicate panics
//! at startup (and in the app-start smoke test), which is the point.
//!
//! The store implements its repo traits directly, so a port line is only the
//! type name plus `store.clone()`. A port whose adapter needs a constructor
//! passes the built adapter instead, e.g. `Arc::new(SqliteSystemRepo::new(…))`.

use std::sync::Arc;

use boba_app::apply::engine::ApplyEngine;
use boba_app::ports::agent::AgentPort;
use boba_app::ports::browser::BrowserPort;
use boba_app::ports::cli::CliInstallsPort;
use boba_app::ports::mail::MailPort;
use boba_app::ports::render::RenderPort;
use boba_app::ports::repo::agent::AgentRepo;
use boba_app::ports::repo::apply::ApplyRepo;
use boba_app::ports::repo::documents::DocumentsRepo;
use boba_app::ports::repo::email::EmailRepo;
use boba_app::ports::repo::ingestion::IngestionRepo;
use boba_app::ports::repo::outbox::OutboxRepo;
use boba_app::ports::repo::profile::ProfileRepo;
use boba_app::ports::repo::scoring::ScoringRepo;
use boba_app::ports::repo::sources::SourcesRepo;
use boba_app::ports::repo::system::SystemRepo;
use boba_app::ports::secret::SecretPort;
use boba_app::ports::session::SessionStore;
use boba_app::ports::verify::VerifyPort;
use boba_docs::TypstRenderer;
use boba_mail::graph::GraphMail;
use boba_store::db::Store;
use boba_store::keys::SecretStore;
use boba_store::repos::audit::SqliteSystemRepo;
use tauri::{AppHandle, Manager, Runtime};

use super::{agent, keychain_service, mount_port};

/// Mount every port that has a store adapter (ADR-012 §3).
pub fn mount_ports<R: Runtime>(app: &AppHandle<R>, store: &Arc<Store>) {
    mount_port::<dyn SourcesRepo>(app, store.clone());
    // The job-run owner (S5.1, AD-18): the scheduler records and reads `job_run`.
    mount_port::<dyn boba_app::jobs::JobsRepo>(app, store.clone());
    mount_port::<dyn AgentRepo>(app, store.clone());
    // The system repo also needs the keychain service for the audit anchor.
    mount_port::<dyn SystemRepo>(
        app,
        Arc::new(SqliteSystemRepo::new(store.clone(), keychain_service())),
    );
    mount_port::<dyn ProfileRepo>(app, store.clone());
    // The ingestion owner writes `posting` + `posting_source` (S4.2).
    mount_port::<dyn IngestionRepo>(app, store.clone());
    // The agent runtime is built by this slice (CH5): it needs the app handle
    // for the event sink and the app-data paths, so it cannot be a plain
    // `store.clone()` line.
    let agent: Arc<dyn AgentPort> = Arc::new(agent::agent_runtime(app, store));
    mount_port::<dyn AgentPort>(app, agent);
    // Email connections (S8.1): the row owner, the keychain scope a slice may
    // address secrets through, and the Graph sender.
    mount_port::<dyn EmailRepo>(app, store.clone());
    let secrets: Arc<dyn SecretPort> = Arc::new(SecretStore::new(keychain_service()));
    mount_port::<dyn SecretPort>(app, secrets.clone());
    let accounts: Arc<dyn EmailRepo> = store.clone();
    // A client that cannot be built leaves the port unmounted instead of
    // panicking: a send then reports the typed error (ADR-012 §4).
    match GraphMail::new(secrets, accounts) {
        Ok(mail) => mount_port::<dyn MailPort>(app, Arc::new(mail)),
        Err(error) => tracing::error!(?error, "mail port left unmounted"),
    }
    // The apply area owns `application` + `status_event` (S9.1).
    mount_port::<dyn ApplyRepo>(app, store.clone());
    // The apply outbox owns `apply_attempt` (S9.3): the evidence an Email Apply
    // retry reads before it sends again.
    mount_port::<dyn OutboxRepo>(app, store.clone());
    // The scoring owner reads `score_card` (S6.4 is its first reader: the Inbox
    // shows the newest card per Offer; the run slices write them).
    mount_port::<dyn ScoringRepo>(app, store.clone());
    // The one outbound HTTP client (AD-10): the manual intake fetch (S4.7) is
    // its first caller, and the source adapters share this instance. Mounted as
    // the concrete type: there is no trait to abstract (one client, one
    // implementation), and a second one would be a second bucket and a second
    // robots cache.
    match boba_sources::http::HttpClient::honest() {
        Ok(http) => mount_port(app, Arc::new(http)),
        Err(error) => tracing::error!(?error, "http client left unmounted"),
    }
    // The studio (S7.7): the document owner, the Typst renderer for both export
    // formats, the layout/ATS verification S7.5/S7.6 read, and the detected
    // CLIs a generation run picks its engine from.
    mount_port::<dyn DocumentsRepo>(app, store.clone());
    let render: Arc<dyn RenderPort> = Arc::new(TypstRenderer);
    mount_port::<dyn RenderPort>(app, render);
    let verify: Arc<dyn VerifyPort> = Arc::new(TypstRenderer);
    mount_port::<dyn VerifyPort>(app, verify);
    mount_port::<dyn CliInstallsPort>(app, store.clone());
    // The detection seam (S3.2): the engine step's probe refreshes the
    // `cli_install` evidence ADR-010's gate reads (the step owns the choice).
    mount_port(app, Arc::new(agent::EngineProbe::new(store.clone())));
    // A send that died mid-flight leaves `apply_attempt` pending (S9.3, AC2):
    // nothing else resolves it, and the next send would refuse forever. This
    // startup pass turns it `uncertain` — which the user's confirmation can
    // clear — and closes its audit intent as AD-12 prescribes.
    recover_interrupted_sends(app);
    // The ATS Form browser (S9.4, AD-10): the user's own Chrome over CDP, on
    // the Boba profile, launched only when an apply session starts.
    match app.path().app_data_dir() {
        Ok(data) => mount_port::<dyn BrowserPort>(
            app,
            Arc::new(boba_browser::driver::ChromeDriver::new(data)),
        ),
        Err(error) => tracing::error!(?error, "browser port left unmounted"),
    }
    // The Session Board profiles (S11.1a, AD-10): one Chrome profile per
    // source, deleted by "Sign out / clear session".
    match app.path().app_data_dir() {
        Ok(data) => mount_port::<dyn SessionStore>(
            app,
            Arc::new(boba_browser::session::ProfileSessions::new(data)),
        ),
        Err(error) => tracing::error!(?error, "session store left unmounted"),
    }
    // slices append ONE line above this comment; never reorder/remove
}

/// The Email Apply engine, built from the ports just mounted (startup pass only).
fn recovery_engine<R: Runtime>(app: &AppHandle<R>) -> boba_app::error::AppResult<ApplyEngine> {
    Ok(ApplyEngine::new(
        super::port::<dyn ApplyRepo>(app)?,
        super::port::<dyn DocumentsRepo>(app)?,
        super::port::<dyn ProfileRepo>(app)?,
        super::port::<dyn OutboxRepo>(app)?,
        super::port::<dyn SystemRepo>(app)?,
        super::port::<dyn MailPort>(app)?,
        Arc::new(boba_app::system::SystemClock),
    ))
}

/// Reconcile the sends a crash interrupted; a launch that cannot do it leaves
/// the rows for the next one rather than failing to start.
fn recover_interrupted_sends<R: Runtime>(app: &AppHandle<R>) {
    let Ok(engine) = recovery_engine(app) else {
        return;
    };
    match engine.recover() {
        Ok(0) => {}
        Ok(recovered) => tracing::info!(recovered, "apply: interrupted sends reconciled"),
        Err(error) => tracing::warn!(key = error.message_key(), "apply: reconcile failed"),
    }
}
