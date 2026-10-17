//! The Session Board run side (S11.1b, FR-30, AD-10, AD-12).
//!
//! A daily search runs LinkedIn and Indeed like any other Source. What those
//! adapters may not skip lives here: the core gate they ask before every page
//! load — the board (opt-in, pause), today's page budget, the `session.run`
//! audit row and the challenge pause — and the user's own Chrome they read
//! through. The adapters get these as ports, never as direct access to the
//! store, the notifier or the browser.

use std::sync::Arc;

use boba_app::error::{AppError, AppResult, ErrorCode};
use boba_app::ports::clock::ClockPort;
use boba_app::ports::notifier::NotifierPort;
use boba_app::ports::repo::sources::SourcesRepo;
use boba_app::ports::repo::system::SystemRepo;
use boba_app::ports::session::{SessionPages, SessionStore};
use boba_app::system::SystemClock;
use boba_browser::session;
use boba_sources::session::sdk::{
    BoardView, ChallengeKind, RunControl, SessionBoardService, loads_key,
};
use tauri::Manager;

use crate::scheduler::notify::TauriNotifier;
use crate::wiring;

/// The session service the run control talks to.
type Service = SessionBoardService<SystemClock>;

/// [`RunControl`] over the session core: every page load is gated, audited and
/// counted, and every challenge pauses the Source and notifies the user.
pub struct SessionRunControl {
    service: Service,
    system: Arc<dyn SystemRepo>,
}

impl SessionRunControl {
    pub fn new(service: Service, system: Arc<dyn SystemRepo>) -> Arc<Self> {
        Arc::new(Self { service, system })
    }

    /// Today's page loads already spent on `source_id` (the local day's key).
    fn used_today(&self, source_id: &str) -> Result<u32, String> {
        let now = SystemClock.now_rfc3339();
        Ok(self
            .system
            .setting_get(&loads_key(source_id, &now))
            .map_err(|error| error.to_string())?
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0))
    }
}

impl RunControl for SessionRunControl {
    fn board(&self, source_id: &str) -> Result<(BoardView, u32), String> {
        let snapshot = self.service.snapshot().map_err(|error| error.to_string())?;
        let board = snapshot
            .boards
            .into_iter()
            .find(|board| board.source_id == source_id)
            .ok_or_else(|| format!("'{source_id}' is not a Session Board Source"))?;
        let used = self.used_today(source_id)?;
        Ok((board, used))
    }

    fn page_loaded(&self, source_id: &str, url: &str, loads_today: u32) -> Result<(), String> {
        // The audit row and the counter are written by the service (AD-12).
        self.service
            .record_page_load(source_id, url, loads_today)
            .map_err(|error| error.to_string())
    }

    fn pause(&self, source_id: &str, kind: ChallengeKind) -> Result<(), String> {
        self.service
            .report_challenge(source_id, kind)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

/// The two run-side handles a daily search hands to the Session Board adapters
/// (S11.1b): the user's Chrome, and the core gate/audit/pause.
pub fn run_side(app: &tauri::AppHandle) -> AppResult<(Arc<dyn SessionPages>, Arc<dyn RunControl>)> {
    let base = app.path().app_data_dir().map_err(|error| {
        AppError::new(ErrorCode::Internal, "errors.session.store")
            .with_detail("reason", error.to_string())
    })?;
    let pages: Arc<dyn SessionPages> = Arc::new(session::ChromeSessions::new(base));
    let sessions = wiring::port::<dyn SessionStore>(app)?;
    let notifier: Arc<dyn NotifierPort> = Arc::new(TauriNotifier::new(app.clone()));
    let service = SessionBoardService::new(
        wiring::port::<dyn SourcesRepo>(app)?,
        wiring::port::<dyn SystemRepo>(app)?,
        SystemClock,
        sessions,
        session::detect_cookie_protection(),
        Some(notifier),
    );
    let system = wiring::port::<dyn SystemRepo>(app)?;
    let control: Arc<dyn RunControl> = SessionRunControl::new(service, system);
    Ok((pages, control))
}
