//! In-memory `ProfileRepo` for use-case tests (test-only).
//!
//! It mirrors the store's contract — latest revision wins, history newest
//! first — without SQLite. The real SQL, its immutability triggers and the
//! adapter are tested against an encrypted database in `boba-store`.

use std::sync::{Arc, Mutex};

use boba_domain::profile::{FactProposal, NewRevision, ProfileFact};

use crate::ports::repo::profile::{ProfileRepo, ProfileRepoError};

#[derive(Default, Clone)]
pub struct FakeProfileRepo {
    /// Shared, so a test can keep a handle after the service takes one.
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    version: u32,
    updated_at: String,
    facts: Vec<ProfileFact>,
    proposals: Vec<FactProposal>,
}

impl FakeProfileRepo {
    /// Force the stored version, to exercise the overflow guard.
    pub fn set_version(&self, version: u32) {
        self.state.lock().unwrap().version = version;
    }

    /// The stored `updated_at`, for assertions on the clock wiring.
    pub fn updated_at(&self) -> String {
        self.state.lock().unwrap().updated_at.clone()
    }

    /// Every revision, oldest first (the store keeps them all; so does this).
    pub fn revisions(&self) -> Vec<ProfileFact> {
        self.state.lock().unwrap().facts.clone()
    }

    /// Seed a proposal, as an agent run would (AC4).
    pub fn seed_proposal(&self, proposal: FactProposal) {
        self.state.lock().unwrap().proposals.push(proposal);
    }
}

impl ProfileRepo for FakeProfileRepo {
    fn profile_version(&self) -> Result<u32, ProfileRepoError> {
        Ok(self.state.lock().unwrap().version)
    }

    fn facts_current(&self) -> Result<Vec<ProfileFact>, ProfileRepoError> {
        let state = self.state.lock().unwrap();
        Ok(state
            .facts
            .iter()
            .filter(|fact| {
                !state
                    .facts
                    .iter()
                    .any(|next| next.supersedes.as_deref() == Some(fact.id.as_str()))
            })
            .cloned()
            .collect())
    }

    fn fact_latest(&self, item_id: &str) -> Result<Option<ProfileFact>, ProfileRepoError> {
        Ok(self
            .facts_current()?
            .into_iter()
            .find(|fact| fact.item_id == item_id))
    }

    fn fact_get(&self, id: &str) -> Result<Option<ProfileFact>, ProfileRepoError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .facts
            .iter()
            .find(|fact| fact.id == id)
            .cloned())
    }

    fn fact_history(&self, item_id: &str) -> Result<Vec<ProfileFact>, ProfileRepoError> {
        let mut history: Vec<ProfileFact> = self
            .state
            .lock()
            .unwrap()
            .facts
            .iter()
            .filter(|fact| fact.item_id == item_id)
            .cloned()
            .collect();
        history.sort_by_key(|fact| std::cmp::Reverse(fact.profile_version));
        Ok(history)
    }

    /// Same contract as the store: resolve `supersedes` and the version under
    /// the lock, so a fork or a version drift cannot be reached by a test leak.
    fn fact_append_revision(
        &self,
        revision: &NewRevision,
    ) -> Result<ProfileFact, ProfileRepoError> {
        let mut state = self.state.lock().unwrap();
        let current = state
            .facts
            .iter()
            .find(|fact| {
                fact.item_id == revision.item_id
                    && !state
                        .facts
                        .iter()
                        .any(|next| next.supersedes.as_deref() == Some(fact.id.as_str()))
            })
            .cloned();
        let supersedes = match (current, revision.is_new) {
            (Some(existing), false) => Some(existing.id),
            (None, true) => None,
            (Some(_), true) => {
                return Err(ProfileRepoError::new(format!(
                    "profile item {} already exists",
                    revision.item_id
                )));
            }
            (None, false) => {
                return Err(ProfileRepoError::new(format!(
                    "profile item {} has no current revision",
                    revision.item_id
                )));
            }
        };
        let version = state
            .version
            .checked_add(1)
            .ok_or_else(|| ProfileRepoError::new("the profile version overflowed"))?;
        let fact = revision.clone().into_fact(supersedes, version);
        state.version = version;
        state.updated_at = fact.created_at.clone();
        state.facts.push(fact.clone());
        Ok(fact)
    }

    fn proposals_pending(&self) -> Result<Vec<FactProposal>, ProfileRepoError> {
        let mut proposals = self.state.lock().unwrap().proposals.clone();
        proposals.reverse();
        Ok(proposals)
    }

    fn proposal_append(&self, proposal: &FactProposal) -> Result<(), ProfileRepoError> {
        self.seed_proposal(proposal.clone());
        Ok(())
    }

    fn proposal_delete(&self, id: &str) -> Result<bool, ProfileRepoError> {
        let mut state = self.state.lock().unwrap();
        let before = state.proposals.len();
        state.proposals.retain(|proposal| proposal.id != id);
        Ok(state.proposals.len() != before)
    }
}
