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
use boba_app::apply::session::SessionFormPort;
use boba_app::ports::agent::AgentPort;
use boba_app::ports::browser::BrowserPort;
use boba_app::ports::cli::CliInstallsPort;
use boba_app::ports::mail::MailPort;
use boba_app::ports::render::RenderPort;
use boba_app::ports::repo::agent::AgentRepo;
use boba_app::ports::repo::apply::ApplyRepo;
use boba_app::ports::repo::auto_rules::AutoRulesRepo;
use boba_app::ports::repo::documents::DocumentsRepo;
use boba_app::ports::repo::email::EmailRepo;
use boba_app::ports::repo::ingestion::IngestionRepo;
use boba_app::ports::repo::outbox::OutboxRepo;
use boba_app::ports::repo::outbox_hold::OutboxHoldRepo;
use boba_app::ports::repo::profile::ProfileRepo;
use boba_app::ports::repo::scoring::ScoringRepo;
use boba_app::ports::repo::sources::SourcesRepo;
use boba_app::ports::repo::system::SystemRepo;
use boba_app::ports::repo::triage::TriageRepo;
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
    // Kept in hand: the Kill Switch (S9.8) writes its flag and its audit rows
    // through the same adapter.
    let system: Arc<dyn SystemRepo> =
        Arc::new(SqliteSystemRepo::new(store.clone(), keychain_service()));
    mount_port::<dyn SystemRepo>(app, Arc::clone(&system));
    // The on-device F3 encoder and its opt-in (S6.9, AD-8): the port exists
    // only in a build with the `embeddings` feature; the use case always does,
    // so Settings can say why F3 is unavailable (AC4).
    mount_embeddings(app, Arc::clone(&system));
    mount_port::<dyn ProfileRepo>(app, store.clone());
    // The ingestion owner writes `posting` + `posting_source` (S4.2).
    mount_port::<dyn IngestionRepo>(app, store.clone());
    // The agent runtime is built by this slice (CH5): it needs the app handle
    // for the event sink and the app-data paths, so it cannot be a plain
    // `store.clone()` line. Every run is wrapped so the Kill Switch (S9.8) can
    // cancel it in flight (FR-86).
    let runs = Arc::new(boba_app::apply::kill_switch::RunCancellations::default());
    let agent: Arc<dyn AgentPort> = Arc::new(boba_app::apply::kill_switch::CancellingAgent::new(
        Arc::new(agent::agent_runtime(app, store)),
        Arc::clone(&runs),
    ));
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
    // The apply policy engine owns `auto_rule` + `auto_rule_attempt` (S9.6): the
    // rules the user writes and the ledger the three caps count.
    mount_port::<dyn AutoRulesRepo>(app, store.clone());
    // The apply outbox owns `apply_attempt` (S9.3): the evidence an Email Apply
    // retry reads before it sends again.
    mount_port::<dyn OutboxRepo>(app, store.clone());
    // The Outbox hold owns `apply_outbox` and `apply_rule_dry_run` (S9.7): the
    // queued automatic sends and each rule's dry-run progress.
    mount_port::<dyn OutboxHoldRepo>(app, store.clone());
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
    // An item a crash left mid-release (S9.7, FR-85): the ledger entry exists,
    // the send's result does not, so the item goes to the review queue instead
    // of being released a second time.
    recover_outbox(app);
    // The ATS Form browser (S9.4, AD-10): the user's own Chrome over CDP, on
    // the Boba profile, launched only when an apply session starts. Kept in
    // hand: the Kill Switch closes it, which stops an in-flight fill (S9.8).
    let browser: Option<Arc<dyn BrowserPort>> = match app.path().app_data_dir() {
        Ok(data) => {
            let browser: Arc<dyn BrowserPort> =
                Arc::new(boba_browser::driver::ChromeDriver::new(data));
            mount_port::<dyn BrowserPort>(app, Arc::clone(&browser));
            Some(browser)
        }
        Err(error) => {
            tracing::error!(?error, "browser port left unmounted");
            None
        }
    };
    // The Session Board apply browser (S11.2, AD-10): the user's own Chrome
    // on that Source's session profile, headed, only when an apply starts.
    match app.path().app_data_dir() {
        Ok(data) => mount_port::<dyn SessionFormPort>(
            app,
            Arc::new(boba_browser::session_forms::SessionFormDriver::new(data)),
        ),
        Err(error) => tracing::error!(?error, "session form port left unmounted"),
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
    // The daily search (S5.2): the sources a run queries behind `SourcePort`,
    // and the scheduler's handler for the job it runs.
    if let Err(error) = crate::commands::daily_search::mount(app) {
        tracing::error!(%error, "daily search wiring left unmounted");
    }
    // The triage owner keeps the Inbox decisions (S6.6): why an Offer was
    // dismissed, the VETOs the user overruled and the answers to suggestions.
    mount_port::<dyn TriageRepo>(app, store.clone());
    // The Kill Switch (S9.8, FR-86): the one flag every automatic send
    // re-checks before it queues, releases and submits; the runs in flight it
    // cancels; the browser session it closes. Mounted twice on purpose — as the
    // `KillSwitchPort` the Outbox asks, and as the concrete use case the tray,
    // the shortcut and the header chip flip.
    let kill_switch = Arc::new(boba_app::apply::kill_switch::KillSwitchService::new(
        system,
        store.clone(),
        Arc::new(boba_app::system::SystemClock),
        browser,
        runs,
    ));
    mount_port::<dyn boba_app::ports::kill_switch::KillSwitchPort>(
        app,
        Arc::clone(&kill_switch) as Arc<dyn boba_app::ports::kill_switch::KillSwitchPort>,
    );
    assert!(
        app.manage(kill_switch),
        "duplicate kill switch state in port_list"
    );
    // Backup, restore and Reset everything (S10.2): the open store is the
    // adapter, because a restore swaps its pages and a reset erases it.
    mount_port::<dyn boba_app::ports::backup::BackupPort>(app, store.clone());
    // The matched-mail sink (S8.3): the email area writes `email_message`.
    mount_port::<dyn boba_mail::sync::MessageSink>(app, store.clone());
    // The scoped mail sync (S8.3): registers the scheduler's `email_sync`
    // handler, which reads the read+send connections into the sink above.
    crate::commands::email::mount(app);
    // The status-proposal owner keeps the reply verdicts and the proposals
    // (S8.4): `email_classification` is no other area's table.
    mount_port::<dyn boba_app::status_proposals::repo::StatusProposalRepo>(app, store.clone());
    // slices append ONE line above this comment; never reorder/remove
}

/// Mount the on-device F3 encoder and the opt-in use case over it (S6.9).
///
/// The model directory is app data `models/`: the opt-in download (AC1) writes
/// there, and the network-activity view lists that same destination. A build
/// without the `embeddings` feature mounts no port, so the use case answers
/// `available: false` and F3 drops out of scoring unchanged (AC4).
fn mount_embeddings<R: Runtime>(app: &AppHandle<R>, system: Arc<dyn SystemRepo>) {
    #[cfg(feature = "embeddings")]
    let port: Option<Arc<dyn boba_app::ports::embedding::EmbeddingPort>> =
        match app.path().app_data_dir() {
            Ok(data) => {
                let port: Arc<dyn boba_app::ports::embedding::EmbeddingPort> =
                    Arc::new(boba_embed::FastembedEncoder::new(data.join("models")));
                mount_port::<dyn boba_app::ports::embedding::EmbeddingPort>(app, Arc::clone(&port));
                Some(port)
            }
            Err(error) => {
                tracing::error!(?error, "embedding port left unmounted");
                None
            }
        };
    #[cfg(not(feature = "embeddings"))]
    let port: Option<Arc<dyn boba_app::ports::embedding::EmbeddingPort>> = {
        let _ = app;
        None
    };

    let service = Arc::new(boba_app::embeddings::EmbeddingsService::new(
        port,
        system,
        Arc::new(boba_app::system::SystemClock),
    ));
    assert!(
        app.manage(Arc::clone(&service)),
        "duplicate embeddings state in port_list"
    );
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

/// Close the Outbox items a crash left mid-release (S9.7, FR-85).
fn recover_outbox<R: Runtime>(app: &AppHandle<R>) {
    let Ok(items) = super::port::<dyn OutboxHoldRepo>(app) else {
        return;
    };
    let now = boba_app::ports::clock::ClockPort::now_rfc3339(&boba_app::system::SystemClock);
    match boba_app::apply::outbox::reconcile_stuck(items.as_ref(), &now) {
        Ok(0) => {}
        Ok(recovered) => tracing::info!(recovered, "outbox: interrupted releases reconciled"),
        Err(error) => tracing::warn!(%error, "outbox: reconcile failed"),
    }
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
