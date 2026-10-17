//! In-memory `ApplyRepo` for use-case tests (test-only).
//!
//! It mirrors the store's contract — a move lands only when the stored status is
//! still the one the caller saw, and its event is appended in the same step —
//! without SQLite. The real SQL and the adapter are tested against an encrypted
//! database in `boba-store`.

use std::sync::{Arc, Mutex};

use boba_domain::application::{
    AppStatus, Application, BankAnswer, NewBankAnswer, PackageItem, StatusEvent,
};

use crate::ports::repo::apply::{ApplyRepo, ApplyRepoError};

#[derive(Default, Clone)]
pub struct FakeApplyRepo {
    /// Shared, so a test keeps a handle after the service takes one.
    state: Arc<Mutex<State>>,
    /// When set, every write fails, the way a full disk would.
    pub fail: bool,
}

#[derive(Default)]
struct State {
    applications: Vec<Application>,
    events: Vec<StatusEvent>,
    package: Vec<(String, PackageItem)>,
    answers: Vec<BankAnswer>,
    /// How many per-Application history queries ran: the Audit Log must not
    /// ask once per row (CH12).
    status_event_queries: usize,
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

    /// How many per-Application history queries ran (CH12).
    pub fn status_event_queries(&self) -> usize {
        self.state.lock().unwrap().status_event_queries
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

    /// Every answer-bank row written, oldest first (S7.8).
    pub fn answers(&self) -> Vec<BankAnswer> {
        self.state.lock().unwrap().answers.clone()
    }

    /// A fake whose writes fail, the way a full disk would.
    pub fn failing() -> Self {
        Self {
            fail: true,
            ..Self::default()
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
        let mut state = self.state.lock().unwrap();
        state.status_event_queries += 1;
        Ok(state
            .events
            .iter()
            .filter(|event| event.application_id == application_id)
            .cloned()
            .collect())
    }

    fn all_status_events(&self) -> Result<Vec<StatusEvent>, ApplyRepoError> {
        let mut events = self.state.lock().unwrap().events.clone();
        events.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(events)
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

    fn application_create(
        &self,
        application: &Application,
        event: &StatusEvent,
    ) -> Result<(), ApplyRepoError> {
        if self.fail {
            return Err(ApplyRepoError::Store("disk gone".to_string()));
        }
        let mut state = self.state.lock().unwrap();
        if state
            .applications
            .iter()
            .any(|row| row.id == application.id)
        {
            return Err(ApplyRepoError::Conflict {
                expected: event
                    .from_status
                    .map(|s| s.as_str().to_string())
                    .unwrap_or_default(),
                found: application.status.as_str().to_string(),
            });
        }
        state.applications.push(application.clone());
        state.events.push(event.clone());
        Ok(())
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

    fn answer_bank_insert(&self, answer: &NewBankAnswer) -> Result<(), ApplyRepoError> {
        if self.fail {
            return Err(ApplyRepoError::Store("disk gone".to_string()));
        }
        self.state.lock().unwrap().answers.push(BankAnswer {
            id: answer.id.clone(),
            application_id: answer.application_id.clone(),
            question: answer.question.clone(),
            question_key: answer.question_key.clone(),
            answer: answer.answer.clone(),
            class: answer.class,
            fact_ids: answer.fact_ids.clone(),
            needs_review: answer.needs_review,
            untrusted_origin: answer.untrusted_origin,
            approved_at: None,
            created_at: answer.created_at.clone(),
            updated_at: answer.created_at.clone(),
        });
        Ok(())
    }

    fn answer_bank_for_application(
        &self,
        application_id: &str,
    ) -> Result<Vec<BankAnswer>, ApplyRepoError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .answers
            .iter()
            .filter(|row| row.application_id == application_id)
            .cloned()
            .collect())
    }

    fn answer_bank_reusable(&self, keys: &[String]) -> Result<Vec<BankAnswer>, ApplyRepoError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .answers
            .iter()
            .filter(|row| row.is_reusable() && keys.contains(&row.question_key))
            .cloned()
            .collect())
    }

    fn answer_bank_approve(
        &self,
        id: &str,
        answer: Option<&str>,
        approved_at: &str,
    ) -> Result<bool, ApplyRepoError> {
        if self.fail {
            return Err(ApplyRepoError::Store("disk gone".to_string()));
        }
        let mut state = self.state.lock().unwrap();
        let Some(row) = state.answers.iter_mut().find(|row| row.id == id) else {
            return Ok(false);
        };
        if let Some(text) = answer {
            row.answer = Some(text.to_string());
        }
        row.approved_at = Some(approved_at.to_string());
        row.updated_at = approved_at.to_string();
        // The user made this text theirs by confirming or editing it (AC3).
        row.untrusted_origin = false;
        Ok(true)
    }
}
