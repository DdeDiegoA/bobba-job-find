//! In-memory `StatusProposalRepo` and a scripted classifier (test-only).
//!
//! The fake mirrors the store's contract — one classification per message, one
//! proposal per message, a decision replaces the previous state — without
//! SQLite. The SQL adapter is tested in `boba-store`.

use std::sync::{Arc, Mutex};

use boba_domain::mail::{ProposalState, ReplyClass, RuleMatch, StatusProposal};

use super::ReplyClassifier;
use super::repo::{Classification, StatusProposalRepo, StatusProposalRepoError, StoredMessage};

#[derive(Default, Clone)]
pub struct FakeStatusProposalRepo {
    state: Arc<Mutex<State>>,
    /// One proposal id whose read fails, the way a broken row would (F1).
    fail_get: Arc<Mutex<Option<String>>>,
}

#[derive(Default)]
struct State {
    messages: Vec<StoredMessage>,
    classifications: Vec<Classification>,
    proposals: Vec<StatusProposal>,
}

impl FakeStatusProposalRepo {
    /// Store one matched message, as the email area's sink would have.
    pub fn seed_message(&self, message: StoredMessage) {
        self.state.lock().unwrap().messages.push(message);
    }

    /// Every classification written, oldest first.
    pub fn classifications(&self) -> Vec<Classification> {
        self.state.lock().unwrap().classifications.clone()
    }

    /// Every proposal written, oldest first (any state).
    pub fn proposals(&self) -> Vec<StatusProposal> {
        self.state.lock().unwrap().proposals.clone()
    }

    /// Make reading one proposal fail, as a store error would.
    pub fn fail_get_for(&self, id: &str) {
        *self.fail_get.lock().unwrap() = Some(id.to_string());
    }

    /// Force one proposal into a state, to simulate a card moved elsewhere.
    pub fn force_state(&self, id: &str, state: ProposalState) {
        if let Some(row) = self
            .state
            .lock()
            .unwrap()
            .proposals
            .iter_mut()
            .find(|row| row.id == id)
        {
            row.state = state;
        }
    }
}

impl StatusProposalRepo for FakeStatusProposalRepo {
    fn messages_unclassified(&self) -> Result<Vec<StoredMessage>, StatusProposalRepoError> {
        let state = self.state.lock().unwrap();
        let mut rows: Vec<StoredMessage> = state
            .messages
            .iter()
            .filter(|message| {
                !state
                    .classifications
                    .iter()
                    .any(|row| row.message_id == message.id)
            })
            .cloned()
            .collect();
        rows.sort_by(|left, right| left.received_at.cmp(&right.received_at));
        Ok(rows)
    }

    fn message_get(&self, id: &str) -> Result<Option<StoredMessage>, StatusProposalRepoError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .messages
            .iter()
            .find(|row| row.id == id)
            .cloned())
    }

    fn classification_put(
        &self,
        classification: &Classification,
    ) -> Result<(), StatusProposalRepoError> {
        let mut state = self.state.lock().unwrap();
        state
            .classifications
            .retain(|row| row.message_id != classification.message_id);
        state.classifications.push(classification.clone());
        Ok(())
    }

    fn proposal_insert(&self, proposal: &StatusProposal) -> Result<bool, StatusProposalRepoError> {
        let mut state = self.state.lock().unwrap();
        if state
            .proposals
            .iter()
            .any(|row| row.message_id == proposal.message_id)
        {
            return Ok(false);
        }
        state.proposals.push(proposal.clone());
        Ok(true)
    }

    fn proposals(
        &self,
        state: ProposalState,
    ) -> Result<Vec<StatusProposal>, StatusProposalRepoError> {
        let mut rows: Vec<StatusProposal> = self
            .state
            .lock()
            .unwrap()
            .proposals
            .iter()
            .filter(|row| row.state == state)
            .cloned()
            .collect();
        rows.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        Ok(rows)
    }

    fn proposal_get(&self, id: &str) -> Result<Option<StatusProposal>, StatusProposalRepoError> {
        if self.fail_get.lock().unwrap().as_deref() == Some(id) {
            return Err(StatusProposalRepoError("row unreadable".to_string()));
        }
        Ok(self
            .state
            .lock()
            .unwrap()
            .proposals
            .iter()
            .find(|row| row.id == id)
            .cloned())
    }

    fn proposal_decide(
        &self,
        id: &str,
        application_id: Option<&str>,
        from_status: Option<&str>,
        state: ProposalState,
        decided_at: &str,
    ) -> Result<bool, StatusProposalRepoError> {
        let mut guard = self.state.lock().unwrap();
        let Some(row) = guard.proposals.iter_mut().find(|row| row.id == id) else {
            return Ok(false);
        };
        if let Some(application_id) = application_id {
            row.application_id = Some(application_id.to_string());
        }
        if let Some(from_status) = from_status {
            row.from_status = Some(from_status.to_string());
        }
        row.state = state;
        row.decided_at = Some(decided_at.to_string());
        Ok(true)
    }
}

/// A classifier that answers from a script.
#[derive(Clone)]
pub struct FakeClassifier {
    /// One entry per call, in order; `None` leaves the message uncertain.
    pub answers: Arc<Mutex<Vec<Option<RuleMatch>>>>,
    pub calls: Arc<Mutex<usize>>,
}

impl Default for FakeClassifier {
    fn default() -> Self {
        Self {
            answers: Arc::new(Mutex::new(Vec::new())),
            calls: Arc::new(Mutex::new(0)),
        }
    }
}

impl FakeClassifier {
    /// Answer every message the same way.
    pub fn scripted(answers: Vec<Option<RuleMatch>>) -> Self {
        Self {
            answers: Arc::new(Mutex::new(answers)),
            calls: Arc::new(Mutex::new(0)),
        }
    }

    /// How many messages the deterministic stage read.
    pub fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

impl ReplyClassifier for FakeClassifier {
    fn classify(&self, _subject: &str, _body: &str) -> Option<RuleMatch> {
        *self.calls.lock().unwrap() += 1;
        let mut answers = self.answers.lock().unwrap();
        if answers.is_empty() {
            return None;
        }
        answers.remove(0)
    }
}

/// A rule match, as the classifier would return it.
pub fn matched(class: ReplyClass, confidence: f64, quote: &str) -> Option<RuleMatch> {
    Some(RuleMatch {
        class,
        confidence,
        quoted_line: quote.to_string(),
    })
}
