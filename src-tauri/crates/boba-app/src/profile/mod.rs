//! Profile use cases (FR-11..13, AD-15): the editor's snapshot, saving a fact as
//! a new immutable revision, History and restore.
//!
//! All storage goes through [`ProfileRepo`]; all time comes from the injected
//! clock, so every rule here is testable with an in-memory repo (see
//! [`testing`](crate::profile::testing)).
//!
//! Two rules shape the API:
//!
//! * a **new** fact and an **edit** travel the same path — both append a
//!   revision and bump `profile_version` (AC3), the only difference being that
//!   an edit keeps the item's ULID and points at what it supersedes;
//! * a **restore** is also an edit: it appends the earlier text as the newest
//!   revision, so nothing a document cited is ever rewritten.

use std::sync::Arc;

use boba_domain::profile::{
    FactDraft, FactKind, Lang, NewRevision, ProfileFact, ProfileRuleError, Provenance, new_ulid,
};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ports::clock::ClockPort;
use crate::ports::repo::profile::{ProfileRepo, ProfileRepoError};

/// A use-case failure.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ProfileError {
    #[error(transparent)]
    Repo(#[from] ProfileRepoError),
    /// The draft breaks a profile rule (no variant, blank variant).
    #[error(transparent)]
    Rule(#[from] ProfileRuleError),
    /// The editor sent a section that does not exist.
    #[error("unknown fact section: {0}")]
    UnknownKind(String),
    /// The item was deleted in another window / never existed.
    #[error("unknown fact item: {0}")]
    UnknownItem(String),
    /// History was asked to restore a revision of another item.
    #[error("unknown revision: {0}")]
    UnknownRevision(String),
}

impl ProfileError {
    /// The i18n key the UI shows (AD-3: keys cross IPC, never raw text).
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::Repo(_) => "errors.profile.store",
            Self::Rule(ProfileRuleError::NoVariant) => "errors.profile.no_variant",
            Self::Rule(ProfileRuleError::BlankVariant { .. }) => "errors.profile.blank_variant",
            Self::UnknownKind(_) => "errors.profile.unknown_kind",
            Self::UnknownItem(_) => "errors.profile.unknown_item",
            Self::UnknownRevision(_) => "errors.profile.unknown_revision",
        }
    }
}

/// One fact item as the editor shows it: the current text, the gap it still has
/// and where the text came from (AC1/AC2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct FactView {
    /// Stable ULID of the item, kept by every revision (AC1).
    pub item_id: String,
    /// ULID of the revision currently in force.
    pub revision_id: String,
    /// Section id: `identity`, `experience`, `skill`, `education`,
    /// `certification`, `language` or `eligibility`.
    pub kind: String,
    pub text_en: Option<String>,
    pub text_es: Option<String>,
    /// Evidence quote (skills carry one, AC1).
    pub evidence: Option<String>,
    /// Kind-specific fields (organisation, dates, CEFR level, …) as JSON.
    pub detail_json: String,
    /// `en` or `es` when the fact has only one variant, `None` when it has both.
    /// The UI flags it; it is never filled in automatically (AC2).
    pub missing_variant: Option<String>,
    /// The profile version this revision created (AC3).
    pub profile_version: u32,
    /// `user` or `agent_proposal`.
    pub provenance: String,
    /// Whether an agent run proposed this text (its run is audited in the DB).
    pub from_agent_run: bool,
}

impl From<&ProfileFact> for FactView {
    fn from(fact: &ProfileFact) -> Self {
        Self {
            item_id: fact.item_id.clone(),
            revision_id: fact.id.clone(),
            kind: fact.kind.as_str().to_string(),
            text_en: fact.text(Lang::En).map(str::to_string),
            text_es: fact.text(Lang::Es).map(str::to_string),
            evidence: fact.evidence.clone().filter(|e| !e.trim().is_empty()),
            detail_json: fact.detail_json.clone(),
            missing_variant: fact.missing_variant().map(|l| l.as_str().to_string()),
            profile_version: fact.profile_version,
            provenance: fact.provenance.as_str().to_string(),
            from_agent_run: fact.provenance == Provenance::AgentProposal,
        }
    }
}

/// The whole Profile screen in one read (AC1/AC2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ProfileSnapshot {
    /// The current profile version (AC3).
    pub profile_version: u32,
    /// The current revision of every fact, in editor order.
    pub facts: Vec<FactView>,
    /// Facts an agent proposed that the user has not confirmed yet (S3.5).
    pub pending_proposals: u32,
}

/// What the editor sends when the user saves a fact (AC1/AC3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct FactRevisionInput {
    /// `None` creates a new item; `Some(item_id)` edits that item (AC3).
    pub item_id: Option<String>,
    /// Section id, as in [`FactView::kind`].
    pub kind: String,
    pub text_en: Option<String>,
    pub text_es: Option<String>,
    pub evidence: Option<String>,
    /// Kind-specific fields as JSON; `None` means `{}`.
    pub detail_json: Option<String>,
}

/// Read and edit the profile.
///
/// The ports are erased (`Arc<dyn …>`) so the composition root mounts one handle
/// in Tauri state and every command reuses it (spine: ports are `Send + Sync`
/// exactly so they can live as `Arc<dyn …>`).
pub struct ProfileService {
    repo: Arc<dyn ProfileRepo>,
    clock: Arc<dyn ClockPort>,
}

impl ProfileService {
    pub fn new(repo: Arc<dyn ProfileRepo>, clock: Arc<dyn ClockPort>) -> Self {
        Self { repo, clock }
    }

    /// The current facts, the profile version and the pending proposal count.
    pub fn snapshot(&self) -> Result<ProfileSnapshot, ProfileError> {
        Ok(ProfileSnapshot {
            profile_version: self.repo.profile_version()?,
            facts: self
                .repo
                .facts_current()?
                .iter()
                .map(FactView::from)
                .collect(),
            pending_proposals: self.repo.proposals_pending()?.len() as u32,
        })
    }

    /// Save a new or edited fact as a new immutable revision (AC1/AC2/AC3).
    pub fn save(&self, input: FactRevisionInput) -> Result<ProfileSnapshot, ProfileError> {
        let kind = FactKind::parse(&input.kind)
            .ok_or_else(|| ProfileError::UnknownKind(input.kind.clone()))?;
        let draft = FactDraft {
            kind,
            text_en: clean(input.text_en),
            text_es: clean(input.text_es),
            evidence: clean(input.evidence),
            detail_json: input.detail_json.unwrap_or_else(|| "{}".to_string()),
        };
        draft.validate()?;

        // An edit keeps the item's ULID, a new item starts one. What the
        // revision supersedes and the version it creates are resolved by the
        // repo inside one transaction (AC3), never here.
        let is_new = input.item_id.is_none();
        let item_id = input.item_id.clone().unwrap_or_else(new_ulid);
        if !is_new {
            // Friendly error: the atomic append is the authority, this only
            // turns a missing item into NotFound instead of a store failure.
            self.repo
                .fact_latest(&item_id)?
                .ok_or_else(|| ProfileError::UnknownItem(item_id.clone()))?;
        }

        self.append_revision(&item_id, is_new, draft, Provenance::User, None)?;
        self.snapshot()
    }

    /// Every revision of one item, newest first (AC3 History).
    pub fn history(&self, item_id: &str) -> Result<Vec<FactView>, ProfileError> {
        Ok(self
            .repo
            .fact_history(item_id)?
            .iter()
            .map(FactView::from)
            .collect())
    }

    /// Restore an earlier revision by appending its text as the newest revision
    /// (AC3). The restored revision keeps the provenance of the one restored.
    pub fn restore(
        &self,
        item_id: &str,
        revision_id: &str,
    ) -> Result<ProfileSnapshot, ProfileError> {
        let source = self
            .repo
            .fact_get(revision_id)?
            .filter(|fact| fact.item_id == item_id)
            .ok_or_else(|| ProfileError::UnknownRevision(revision_id.to_string()))?;
        if self.repo.fact_latest(item_id)?.is_none() {
            return Err(ProfileError::UnknownItem(item_id.to_string()));
        }

        self.append_revision(
            item_id,
            false,
            source.draft(),
            source.provenance,
            source.agent_run_id,
        )?;
        self.snapshot()
    }

    /// Append one revision; the repo resolves what it supersedes and the
    /// version, in one transaction (AC3).
    fn append_revision(
        &self,
        item_id: &str,
        is_new: bool,
        draft: FactDraft,
        provenance: Provenance,
        agent_run_id: Option<String>,
    ) -> Result<(), ProfileError> {
        let revision = NewRevision {
            id: new_ulid(),
            item_id: item_id.to_string(),
            is_new,
            draft,
            provenance,
            agent_run_id,
            created_at: self.clock.now_rfc3339(),
        };
        let fact = self.repo.fact_append_revision(&revision)?;
        if fact.item_id != revision.item_id {
            return Err(ProfileError::UnknownItem(revision.item_id));
        }
        Ok(())
    }
}

/// Drop an empty or whitespace-only variant, so a blank field means "missing".
fn clean(text: Option<String>) -> Option<String> {
    text.map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

#[cfg(test)]
pub mod testing;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::testing::FakeProfileRepo;
    use crate::system::FixedClock;
    use boba_domain::profile::FactProposal;

    fn service(repo: FakeProfileRepo) -> ProfileService {
        ProfileService::new(
            Arc::new(repo),
            Arc::new(FixedClock::new("2026-10-15T12:00:00.000Z")),
        )
    }

    fn input(kind: &str, en: Option<&str>, es: Option<&str>) -> FactRevisionInput {
        FactRevisionInput {
            item_id: None,
            kind: kind.to_string(),
            text_en: en.map(str::to_string),
            text_es: es.map(str::to_string),
            evidence: None,
            detail_json: None,
        }
    }

    // AC1: every section the editor offers becomes a fact with its own ULID.
    #[test]
    fn every_section_saves_as_a_fact_with_a_stable_ulid() {
        let svc = service(FakeProfileRepo::default());
        for kind in FactKind::ALL {
            let mut draft = input(kind.as_str(), Some("text"), None);
            draft.evidence = Some("quoted evidence".to_string());
            let snapshot = svc.save(draft).unwrap();
            let fact = snapshot
                .facts
                .iter()
                .find(|fact| fact.kind == kind.as_str())
                .expect("the fact is in the snapshot");
            assert_eq!(fact.item_id.len(), 26, "ULID text");
            assert_eq!(fact.revision_id.len(), 26);
            assert_ne!(fact.item_id, fact.revision_id);
        }
        assert_eq!(svc.snapshot().unwrap().facts.len(), FactKind::ALL.len());
    }

    // AC3: an edit appends a revision; the old text stays as documents cited it.
    #[test]
    fn an_edit_supersedes_the_previous_revision_and_bumps_the_version() {
        let repo = FakeProfileRepo::default();
        let svc = service(repo);
        svc.save(input("experience", Some("Senior Backend Eng"), None))
            .unwrap();
        let first = svc.snapshot().unwrap();
        let item_id = first.facts[0].item_id.clone();
        assert_eq!(first.profile_version, 1);

        let mut edit = input(
            "experience",
            Some("Senior Backend Engineer"),
            Some("Ing. Backend"),
        );
        edit.item_id = Some(item_id.clone());
        let second = svc.save(edit).unwrap();

        assert_eq!(
            second.profile_version, 2,
            "a confirmed change bumps the version"
        );
        assert_eq!(second.facts.len(), 1, "still one item");
        assert_eq!(second.facts[0].item_id, item_id, "the item ULID is stable");
        assert_ne!(second.facts[0].revision_id, first.facts[0].revision_id);

        let history = svc.history(&item_id).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(
            history[0].text_en.as_deref(),
            Some("Senior Backend Engineer")
        );
        assert_eq!(
            history[1].text_en.as_deref(),
            Some("Senior Backend Eng"),
            "the revision a document cited is untouched (AD-15)"
        );
        assert_eq!(history[1].profile_version, 1);
    }

    // AC2: one variant is allowed and flagged; both are preferred.
    #[test]
    fn a_single_variant_fact_is_flagged_not_invented() {
        let svc = service(FakeProfileRepo::default());
        let snapshot = svc
            .save(input(
                "experience",
                Some("Led the dbt/Airflow migration"),
                None,
            ))
            .unwrap();
        assert_eq!(snapshot.facts[0].missing_variant.as_deref(), Some("es"));

        let mut both = input("experience", Some("Shipped it"), Some("Lo lancé"));
        both.item_id = Some(snapshot.facts[0].item_id.clone());
        let snapshot = svc.save(both).unwrap();
        assert_eq!(
            snapshot.facts[0].missing_variant, None,
            "no gap once both exist"
        );
    }

    // AC2: a fact with no text at all is rejected, and a blank field means
    // "missing" rather than an empty string.
    #[test]
    fn a_fact_without_any_variant_is_rejected() {
        let svc = service(FakeProfileRepo::default());
        let error = svc.save(input("skill", None, None)).unwrap_err();
        assert_eq!(error.message_key(), "errors.profile.no_variant");

        let error = svc.save(input("skill", Some("   "), Some(""))).unwrap_err();
        assert_eq!(error.message_key(), "errors.profile.no_variant");

        let snapshot = svc.save(input("skill", Some("Rust"), Some("  "))).unwrap();
        assert_eq!(snapshot.facts[0].text_es, None);
        assert_eq!(snapshot.facts[0].missing_variant.as_deref(), Some("es"));
    }

    #[test]
    fn an_unknown_section_or_item_is_rejected() {
        let svc = service(FakeProfileRepo::default());
        assert_eq!(
            svc.save(input("hobby", Some("Tea"), None))
                .unwrap_err()
                .message_key(),
            "errors.profile.unknown_kind"
        );

        let mut orphan = input("skill", Some("Rust"), None);
        orphan.item_id = Some("01NOPE".to_string());
        assert_eq!(
            svc.save(orphan).unwrap_err().message_key(),
            "errors.profile.unknown_item"
        );
        assert_eq!(
            svc.snapshot().unwrap().profile_version,
            0,
            "nothing was written"
        );
    }

    // AC3: history shows every version and restore appends one.
    #[test]
    fn history_lists_every_version_and_restore_appends_the_earlier_text() {
        let svc = service(FakeProfileRepo::default());
        svc.save(input("experience", Some("v1"), None)).unwrap();
        let item_id = svc.snapshot().unwrap().facts[0].item_id.clone();
        for text in ["v2", "v3"] {
            let mut edit = input("experience", Some(text), None);
            edit.item_id = Some(item_id.clone());
            svc.save(edit).unwrap();
        }

        let history = svc.history(&item_id).unwrap();
        assert_eq!(
            history
                .iter()
                .map(|f| f.text_en.clone().unwrap())
                .collect::<Vec<_>>(),
            vec!["v3", "v2", "v1"],
            "newest first"
        );

        let restored = svc.restore(&item_id, &history[2].revision_id).unwrap();
        assert_eq!(
            restored.profile_version, 4,
            "a restore is a confirmed change"
        );
        assert_eq!(restored.facts[0].text_en.as_deref(), Some("v1"));
        assert_eq!(
            svc.history(&item_id).unwrap().len(),
            4,
            "nothing is deleted"
        );
    }

    #[test]
    fn restoring_a_revision_of_another_item_is_rejected() {
        let svc = service(FakeProfileRepo::default());
        svc.save(input("experience", Some("a"), None)).unwrap();
        let first = svc.snapshot().unwrap().facts[0].clone();
        svc.save(input("education", Some("b"), None)).unwrap();

        assert_eq!(
            svc.restore(&first.item_id, "01NOPE")
                .unwrap_err()
                .message_key(),
            "errors.profile.unknown_revision"
        );
        let other = svc
            .snapshot()
            .unwrap()
            .facts
            .iter()
            .find(|fact| fact.kind == "education")
            .unwrap()
            .revision_id
            .clone();
        assert_eq!(
            svc.restore(&first.item_id, &other)
                .unwrap_err()
                .message_key(),
            "errors.profile.unknown_revision"
        );
    }

    // AC4: a fact confirmed from an agent proposal keeps its provenance.
    #[test]
    fn the_snapshot_reports_pending_proposals() {
        let repo = FakeProfileRepo::default();
        repo.seed_proposal(FactProposal {
            id: "01P1".to_string(),
            item_id: None,
            draft: FactDraft::new(FactKind::Skill),
            source_excerpt: Some("Rust".to_string()),
            untrusted_origin: false,
            agent_run_id: "run_01".to_string(),
            created_at: "2026-10-15T11:00:00.000Z".to_string(),
        });
        assert_eq!(service(repo).snapshot().unwrap().pending_proposals, 1);
    }

    #[test]
    fn the_version_overflow_guard_stops_instead_of_wrapping() {
        let repo = FakeProfileRepo::default();
        repo.set_version(u32::MAX);
        let svc = service(repo);
        assert_eq!(
            svc.save(input("skill", Some("Rust"), None))
                .unwrap_err()
                .message_key(),
            "errors.profile.store"
        );
    }

    #[test]
    fn evidence_and_detail_json_round_trip_through_the_snapshot() {
        let svc = service(FakeProfileRepo::default());
        let mut draft = input("skill", Some("Airflow"), None);
        draft.evidence = Some("role “Nimbus”".to_string());
        draft.detail_json = Some("{\"level\":\"advanced\"}".to_string());
        let snapshot = svc.save(draft).unwrap();
        assert_eq!(snapshot.facts[0].evidence.as_deref(), Some("role “Nimbus”"));
        assert_eq!(snapshot.facts[0].detail_json, "{\"level\":\"advanced\"}");
    }

    #[test]
    fn the_change_is_stamped_with_the_injected_clock() {
        let repo = FakeProfileRepo::default();
        let svc = service(repo.clone());
        svc.save(input("skill", Some("Rust"), None)).unwrap();
        assert_eq!(repo.updated_at(), "2026-10-15T12:00:00.000Z");
    }
}
