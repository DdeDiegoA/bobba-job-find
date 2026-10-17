//! In-memory `TriageRepo` for use-case tests (test-only).
//!
//! It mirrors the store's contract — one reason per posting, one override per
//! posting, one decision per suggestion, later writes replace earlier ones —
//! without SQLite. The adapter and its SQL are tested in `boba-store`.

use std::sync::{Arc, Mutex};

use boba_domain::triage::{DismissalRecord, SuggestionDecision, VetoOverride};

use crate::ports::repo::triage::{TriageRepo, TriageRepoError};

#[derive(Default, Clone)]
pub struct FakeTriageRepo {
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    dismissals: Vec<DismissalRecord>,
    overrides: Vec<VetoOverride>,
    decisions: Vec<SuggestionDecision>,
}

impl FakeTriageRepo {
    /// The reasons on record, newest first (the order the store reads them).
    pub fn dismissals(&self) -> Vec<DismissalRecord> {
        let mut rows = self.state.lock().unwrap().dismissals.clone();
        rows.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        rows
    }

    /// Every override written, oldest first.
    pub fn overrides(&self) -> Vec<VetoOverride> {
        self.state.lock().unwrap().overrides.clone()
    }

    /// Every suggestion the user answered.
    pub fn decisions(&self) -> Vec<SuggestionDecision> {
        self.state.lock().unwrap().decisions.clone()
    }
}

impl TriageRepo for FakeTriageRepo {
    fn dismiss(&self, record: &DismissalRecord) -> Result<(), TriageRepoError> {
        let mut state = self.state.lock().unwrap();
        state
            .dismissals
            .retain(|row| row.posting_id != record.posting_id);
        state.dismissals.push(record.clone());
        Ok(())
    }

    fn undo_dismiss(&self, posting_id: &str) -> Result<bool, TriageRepoError> {
        let mut state = self.state.lock().unwrap();
        let before = state.dismissals.len();
        state.dismissals.retain(|row| row.posting_id != posting_id);
        Ok(state.dismissals.len() != before)
    }

    fn dismissals(&self) -> Result<Vec<DismissalRecord>, TriageRepoError> {
        Ok(FakeTriageRepo::dismissals(self))
    }

    fn override_veto(&self, row: &VetoOverride) -> Result<(), TriageRepoError> {
        let mut state = self.state.lock().unwrap();
        state
            .overrides
            .retain(|stored| stored.posting_id != row.posting_id);
        state.overrides.push(row.clone());
        Ok(())
    }

    fn overrides(&self) -> Result<Vec<VetoOverride>, TriageRepoError> {
        Ok(FakeTriageRepo::overrides(self))
    }

    fn suggestion_decision(&self, id: &str) -> Result<Option<SuggestionDecision>, TriageRepoError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .decisions
            .iter()
            .find(|row| row.id == id)
            .cloned())
    }

    fn save_suggestion_decision(&self, row: &SuggestionDecision) -> Result<(), TriageRepoError> {
        let mut state = self.state.lock().unwrap();
        state.decisions.retain(|stored| stored.id != row.id);
        state.decisions.push(row.clone());
        Ok(())
    }
}
