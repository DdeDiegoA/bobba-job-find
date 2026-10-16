//! In-memory `ApplyRepo` for use-case tests (test-only).
//!
//! It mirrors the store's contract — a move lands only when the stored status is
//! still the one the caller saw, and its event is appended in the same step —
//! without SQLite. The real SQL and the adapter are tested against an encrypted
//! database in `boba-store`.

use std::sync::{Arc, Mutex};

use boba_domain::application::{AppStatus, Application, PackageItem, StatusEvent};

use crate::ports::repo::apply::{ApplyRepo, ApplyRepoError};

#[derive(Default, Clone)]
pub struct FakeApplyRepo {
    /// Shared, so a test keeps a handle after the service takes one.
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    applications: Vec<Application>,
    events: Vec<StatusEvent>,
    package: Vec<(String, PackageItem)>,
}

impl FakeApplyRepo {
    /// Store one application, as the area that creates it would.
    pub fn seed_application(&self, application: Application) {
        self.state.lock().unwrap().applications.push(application);
    }

    /// Store one document of an application's package (AC3).
    pub fn seed_package(&self, application_id: &str, item: PackageItem) {
        self.state
            .lock()
            .unwrap()
            .package
            .push((application_id.to_string(), item));
    }

    /// Every event written, oldest first, for assertions on the history.
    pub fn events(&self) -> Vec<StatusEvent> {
        self.state.lock().unwrap().events.clone()
    }

    /// The stored status, or `None` when the application is unknown.
    pub fn stored_status(&self, id: &str) -> Option<AppStatus> {
        self.state
            .lock()
            .unwrap()
            .applications
            .iter()
            .find(|a| a.id == id)
            .map(|a| a.status)
    }

    /// Force the stored row, to simulate another window moving the card first.
    pub fn set_status(&self, id: &str, status: AppStatus) {
        let mut state = self.state.lock().unwrap();
        if let Some(row) = state.applications.iter_mut().find(|a| a.id == id) {
            row.status = status;
        }
    }
}

impl ApplyRepo for FakeApplyRepo {
    fn applications(&self) -> Result<Vec<Application>, ApplyRepoError> {
        Ok(self.state.lock().unwrap().applications.clone())
    }

    fn application_get(&self, id: &str) -> Result<Option<Application>, ApplyRepoError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .applications
            .iter()
            .find(|a| a.id == id)
            .cloned())
    }

    fn status_events(&self, application_id: &str) -> Result<Vec<StatusEvent>, ApplyRepoError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .events
            .iter()
            .filter(|event| event.application_id == application_id)
            .cloned()
            .collect())
    }

    fn package(&self, application_id: &str) -> Result<Vec<PackageItem>, ApplyRepoError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .package
            .iter()
            .filter(|(id, _)| id == application_id)
            .map(|(_, item)| item.clone())
            .collect())
    }

    fn append_status_event(&self, event: &StatusEvent) -> Result<(), ApplyRepoError> {
        let mut state = self.state.lock().unwrap();
        let Some(row) = state
            .applications
            .iter_mut()
            .find(|a| a.id == event.application_id)
        else {
            return Err(ApplyRepoError::UnknownApplication(
                event.application_id.clone(),
            ));
        };
        if let Some(expected) = event.from_status
            && row.status != expected
        {
            return Err(ApplyRepoError::Conflict {
                expected: expected.as_str().to_string(),
                found: row.status.as_str().to_string(),
            });
        }
        row.status = event.to_status;
        row.updated_at = event.created_at.clone();
        state.events.push(event.clone());
        Ok(())
    }
}
