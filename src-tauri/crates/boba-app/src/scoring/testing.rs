//! In-memory `ScoringRepo` and a clock stuck at one instant, for use-case tests
//! (test-only).
//!
//! The fake mirrors the contract the SQLite adapter keeps — cards are keyed by
//! `(posting, profile version, prefs version, model version)` and a key that
//! already exists is refused rather than overwritten — so a use-case test that
//! passes here is about the rule, not about the storage.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use boba_domain::scoring::narration::ScoreNarration;
use boba_domain::scoring::rank::{CardVersions, ScoreCardRow};

use crate::ports::clock::ClockPort;
use crate::ports::repo::scoring::{ScoringRepo, ScoringRepoError};

/// The key a card is stored under, as the SQLite adapter's unique index spells
/// it.
fn key(posting_id: &str, versions: CardVersions) -> String {
    format!(
        "{posting_id}|{}|{}|{}",
        versions.profile_version, versions.prefs_version, versions.model_version
    )
}

/// Cards in memory, oldest first per Offer.
#[derive(Default)]
pub struct FakeScoringRepo {
    cards: Mutex<Vec<ScoreCardRow>>,
    narrations: Mutex<Vec<ScoreNarration>>,
    /// Set by [`FakeScoringRepo::break_storage`] to make every call fail.
    broken: AtomicBool,
}

impl FakeScoringRepo {
    /// Make every call fail, so a use-case test can see how storage errors are
    /// reported.
    pub fn break_storage(&self) {
        self.broken.store(true, Ordering::SeqCst);
    }

    fn check(&self) -> Result<(), ScoringRepoError> {
        if self.broken.load(Ordering::SeqCst) {
            return Err(ScoringRepoError::new("storage is broken"));
        }
        Ok(())
    }
}

impl ScoringRepo for FakeScoringRepo {
    fn insert_card(&self, row: &ScoreCardRow) -> Result<(), ScoringRepoError> {
        self.check()?;
        let mut cards = self.cards.lock().expect("not poisoned");
        let wanted = key(&row.posting_id, row.versions());
        if cards
            .iter()
            .any(|stored| key(&stored.posting_id, stored.versions()) == wanted)
        {
            // AC3: the same key is never written twice.
            return Err(ScoringRepoError::new(format!(
                "card {wanted} already exists"
            )));
        }
        cards.push(row.clone());
        Ok(())
    }

    fn card(
        &self,
        posting_id: &str,
        versions: CardVersions,
    ) -> Result<Option<ScoreCardRow>, ScoringRepoError> {
        self.check()?;
        let wanted = key(posting_id, versions);
        Ok(self
            .cards
            .lock()
            .expect("not poisoned")
            .iter()
            .find(|stored| key(&stored.posting_id, stored.versions()) == wanted)
            .cloned())
    }

    fn latest_card(&self, posting_id: &str) -> Result<Option<ScoreCardRow>, ScoringRepoError> {
        Ok(self.cards_for(posting_id)?.pop())
    }

    fn cards_for(&self, posting_id: &str) -> Result<Vec<ScoreCardRow>, ScoringRepoError> {
        self.check()?;
        Ok(self
            .cards
            .lock()
            .expect("not poisoned")
            .iter()
            .filter(|stored| stored.posting_id == posting_id)
            .cloned()
            .collect())
    }

    fn latest_narration(
        &self,
        posting_id: &str,
        score_card_id: &str,
        lang: &str,
    ) -> Result<Option<ScoreNarration>, ScoringRepoError> {
        self.check()?;
        Ok(self
            .narrations
            .lock()
            .expect("not poisoned")
            .iter()
            .filter(|stored| {
                stored.posting_id == posting_id
                    && stored.score_card_id == score_card_id
                    && stored.lang == lang
            })
            .max_by(|left, right| {
                left.created_at
                    .cmp(&right.created_at)
                    .then(left.id.cmp(&right.id))
            })
            .cloned())
    }

    fn insert_narration(&self, row: &ScoreNarration) -> Result<(), ScoringRepoError> {
        self.check()?;
        self.narrations
            .lock()
            .expect("not poisoned")
            .push(row.clone());
        Ok(())
    }
}

/// A clock that always says the same instant (AD-17: the use case injects it,
/// so `created_at` is a fixed string in tests).
pub struct FixedTestClock(String);

impl FixedTestClock {
    /// A clock stuck at `now`, UTC RFC 3339.
    pub fn at(now: &str) -> Self {
        Self(now.to_string())
    }
}

impl ClockPort for FixedTestClock {
    fn now_rfc3339(&self) -> String {
        self.0.clone()
    }
}
