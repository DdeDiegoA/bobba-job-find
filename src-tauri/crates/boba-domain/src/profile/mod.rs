//! Profile facts: immutable, versioned and bilingual (AD-15, AD-17, FR-11..13).
//!
//! Pure model. The rules that make documents trustworthy live here:
//!
//! * a fact is a **set of revisions**: the item keeps one stable ULID
//!   (`item_id`), every confirmed change appends a revision row that
//!   `supersedes` the previous one — nothing is ever rewritten (AD-15);
//! * a free-text fact holds up to two variants, `en` and `es`, and a missing
//!   one is *flagged*, never invented (AC2);
//! * a fact records whether the user typed it or confirmed an agent proposal
//!   (ADR-010) — a confirmed fact is the user's own claim and carries no
//!   `untrusted_origin`, only provenance.
//!
//! ULIDs, clocks and storage live in the caller; this module only decides what
//! the data means.

pub mod improve;

use serde::{Deserialize, Serialize};

/// A fresh ULID, as text (AD-17). Kept here so the app layer needs no id crate
/// and every id a fact gets comes from one place.
pub fn new_ulid() -> String {
    ulid::Ulid::generate().to_string()
}

/// A content language variant (AD-17: BCP-47, reduced to the two shipped
/// languages; the UI locale is a separate setting).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lang {
    En,
    Es,
}

impl Lang {
    /// Both variants, `en` first (the default content language).
    pub const ALL: [Lang; 2] = [Lang::En, Lang::Es];

    /// The stored form.
    pub fn as_str(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Es => "es",
        }
    }

    /// Parse the stored form, `None` for an unknown language.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.as_str() == value)
    }
}

/// The Profile sections the editor edits (AC1, EXPERIENCE §9.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactKind {
    Identity,
    Experience,
    Skill,
    Education,
    Certification,
    Language,
    Eligibility,
}

impl FactKind {
    /// Every section, in editor order.
    pub const ALL: [FactKind; 7] = [
        FactKind::Identity,
        FactKind::Experience,
        FactKind::Skill,
        FactKind::Education,
        FactKind::Certification,
        FactKind::Language,
        FactKind::Eligibility,
    ];

    /// The stored form.
    pub fn as_str(self) -> &'static str {
        match self {
            FactKind::Identity => "identity",
            FactKind::Experience => "experience",
            FactKind::Skill => "skill",
            FactKind::Education => "education",
            FactKind::Certification => "certification",
            FactKind::Language => "language",
            FactKind::Eligibility => "eligibility",
        }
    }

    /// Parse the stored form, `None` for an unknown section.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == value)
    }

    /// Whether this section's items carry an evidence quote (AC1: skills).
    pub fn takes_evidence(self) -> bool {
        matches!(self, FactKind::Skill)
    }
}

/// Where a fact's text came from (ADR-010).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    /// The user typed or edited it.
    User,
    /// The user confirmed a proposal an agent run made.
    AgentProposal,
}

impl Provenance {
    pub fn as_str(self) -> &'static str {
        match self {
            Provenance::User => "user",
            Provenance::AgentProposal => "agent_proposal",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "user" => Some(Provenance::User),
            "agent_proposal" => Some(Provenance::AgentProposal),
            _ => None,
        }
    }
}

/// A fact's editable text, before it becomes a revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactDraft {
    pub kind: FactKind,
    /// English variant, `None` when the fact has no English text yet.
    pub text_en: Option<String>,
    /// Spanish variant, `None` when the fact has no Spanish text yet.
    pub text_es: Option<String>,
    /// The evidence quote that backs a skill (AC1).
    pub evidence: Option<String>,
    /// Kind-specific fields (organisation, dates, CEFR level, …) as JSON.
    pub detail_json: String,
}

impl FactDraft {
    /// An empty draft for a section, ready for the editor.
    pub fn new(kind: FactKind) -> Self {
        Self {
            kind,
            text_en: None,
            text_es: None,
            evidence: None,
            detail_json: "{}".to_string(),
        }
    }

    /// Reject a draft that cannot be stored: no variant at all, or a blank one.
    pub fn validate(&self) -> Result<(), ProfileRuleError> {
        for (lang, raw) in [(Lang::En, &self.text_en), (Lang::Es, &self.text_es)] {
            if raw.as_ref().is_some_and(|text| text.trim().is_empty()) {
                return Err(ProfileRuleError::BlankVariant { lang });
            }
        }
        if self.text(Lang::En).is_none() && self.text(Lang::Es).is_none() {
            return Err(ProfileRuleError::NoVariant);
        }
        Ok(())
    }

    /// The variant's text, treating whitespace-only text as absent.
    pub fn text(&self, lang: Lang) -> Option<&str> {
        let text = match lang {
            Lang::En => self.text_en.as_deref(),
            Lang::Es => self.text_es.as_deref(),
        };
        text.filter(|t| !t.trim().is_empty())
    }

    /// The variant the fact is missing, when it has only one (AC2: flagged in
    /// the UI, never filled in here).
    pub fn missing_variant(&self) -> Option<Lang> {
        Lang::ALL
            .into_iter()
            .find(|lang| self.text(*lang).is_none())
    }
}

/// One immutable revision of a fact (a `profile_fact` row).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileFact {
    /// ULID of this revision.
    pub id: String,
    /// Stable ULID of the item, kept by every revision (AC1).
    pub item_id: String,
    pub kind: FactKind,
    pub text_en: Option<String>,
    pub text_es: Option<String>,
    pub evidence: Option<String>,
    pub detail_json: String,
    /// `id` of the revision this one replaces, `None` for the first one.
    pub supersedes: Option<String>,
    /// The profile version this revision created (AD-15).
    pub profile_version: u32,
    pub provenance: Provenance,
    /// The run that proposed the text, set when `provenance` is `AgentProposal`.
    pub agent_run_id: Option<String>,
    /// UTC RFC 3339, from the injected clock (AD-17).
    pub created_at: String,
}

impl ProfileFact {
    /// The draft this revision stores, for editing or restoring.
    pub fn draft(&self) -> FactDraft {
        FactDraft {
            kind: self.kind,
            text_en: self.text_en.clone(),
            text_es: self.text_es.clone(),
            evidence: self.evidence.clone(),
            detail_json: self.detail_json.clone(),
        }
    }

    /// The variant's text, treating whitespace-only text as absent.
    pub fn text(&self, lang: Lang) -> Option<&str> {
        let text = match lang {
            Lang::En => self.text_en.as_deref(),
            Lang::Es => self.text_es.as_deref(),
        };
        text.filter(|t| !t.trim().is_empty())
    }

    /// AC2: the missing variant, `None` when both are present.
    pub fn missing_variant(&self) -> Option<Lang> {
        Lang::ALL
            .into_iter()
            .find(|lang| self.text(*lang).is_none())
    }
}

/// A revision to append, before the store resolves what it supersedes and which
/// version it creates (AD-15).
///
/// The adapter resolves both **inside one transaction**, so two concurrent edits
/// cannot fork an item and a crash cannot leave `profile.version` behind the
/// facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewRevision {
    /// ULID of the new revision row.
    pub id: String,
    /// The item this revision belongs to (a fresh ULID for a new item).
    pub item_id: String,
    /// `true` when the item does not exist yet.
    pub is_new: bool,
    pub draft: FactDraft,
    pub provenance: Provenance,
    pub agent_run_id: Option<String>,
    /// UTC RFC 3339, from the injected clock (AD-17).
    pub created_at: String,
}

impl NewRevision {
    /// Materialise the row once the store has resolved what it supersedes and
    /// the profile version it created.
    pub fn into_fact(self, supersedes: Option<String>, profile_version: u32) -> ProfileFact {
        ProfileFact {
            id: self.id,
            item_id: self.item_id,
            kind: self.draft.kind,
            text_en: self.draft.text_en,
            text_es: self.draft.text_es,
            evidence: self.draft.evidence,
            detail_json: self.draft.detail_json,
            supersedes,
            profile_version,
            provenance: self.provenance,
            agent_run_id: self.agent_run_id,
            created_at: self.created_at,
        }
    }
}

/// A fact an agent proposed and the user has not confirmed (S3.5, S3.6).
///
/// `untrusted_origin` is the producing run's `input_trust` (ADR-010): `true` for
/// untrusted input such as a posting, `false` for the user's own input such as
/// their CV.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactProposal {
    pub id: String,
    /// Set when the proposal revises an existing item (translation draft, S3.6).
    pub item_id: Option<String>,
    pub draft: FactDraft,
    /// Excerpt the proposal was read from, shown with Confirm/Edit/Delete (S3.5).
    pub source_excerpt: Option<String>,
    /// The revision an improvement suggestion was written against (FX10a). A
    /// confirm is refused as stale once the item has moved past it; `None` for
    /// proposals that do not revise existing text (a CV import).
    pub base_revision: Option<String>,
    pub untrusted_origin: bool,
    pub agent_run_id: String,
    pub created_at: String,
}

impl FactProposal {
    /// The stored `untrusted_origin` flag (AC4).
    pub fn untrusted_origin_flag(&self) -> i64 {
        i64::from(self.untrusted_origin)
    }
}

/// A profile rule violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProfileRuleError {
    #[error("a fact needs at least one language variant")]
    NoVariant,
    #[error("the {lang:?} variant is blank")]
    BlankVariant { lang: Lang },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(en: Option<&str>, es: Option<&str>) -> FactDraft {
        FactDraft {
            text_en: en.map(str::to_string),
            text_es: es.map(str::to_string),
            ..FactDraft::new(FactKind::Experience)
        }
    }

    // AC1: every editable section is a known fact kind, and it round-trips.
    #[test]
    fn every_section_kind_round_trips_through_storage() {
        assert_eq!(FactKind::ALL.len(), 7);
        for kind in FactKind::ALL {
            assert_eq!(FactKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(FactKind::parse("hobby"), None);
        assert!(FactKind::Skill.takes_evidence());
        assert!(!FactKind::Eligibility.takes_evidence());
    }

    // AC2: one fact holds both variants; the missing one is flagged.
    #[test]
    fn missing_variant_is_flagged_and_never_invented() {
        assert_eq!(
            draft(Some("Led the migration"), None).missing_variant(),
            Some(Lang::Es)
        );
        assert_eq!(
            draft(None, Some("Lideré la migración")).missing_variant(),
            Some(Lang::En)
        );
        assert_eq!(draft(Some("a"), Some("b")).missing_variant(), None);
        // Whitespace is not a variant.
        assert_eq!(draft(Some("  "), None).missing_variant(), Some(Lang::En));
        assert_eq!(draft(Some("  "), None).text(Lang::En), None);
    }

    #[test]
    fn a_blank_or_empty_draft_is_rejected() {
        assert_eq!(
            draft(None, None).validate(),
            Err(ProfileRuleError::NoVariant)
        );
        assert_eq!(
            draft(Some(" "), None).validate(),
            Err(ProfileRuleError::BlankVariant { lang: Lang::En })
        );
        assert!(
            draft(Some("Senior Backend Engineer"), None)
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn lang_round_trips_and_orders_en_first() {
        assert_eq!(Lang::ALL, [Lang::En, Lang::Es]);
        for lang in Lang::ALL {
            assert_eq!(Lang::parse(lang.as_str()), Some(lang));
        }
        assert_eq!(Lang::parse("fr"), None);
    }

    #[test]
    fn provenance_round_trips() {
        assert_eq!(Provenance::parse("user"), Some(Provenance::User));
        assert_eq!(
            Provenance::parse("agent_proposal"),
            Some(Provenance::AgentProposal)
        );
        assert_eq!(Provenance::parse("llm"), None);
    }

    // AC4: the proposal carries the run's trust level as 0/1.
    #[test]
    fn proposal_untrusted_origin_is_a_flag() {
        let mut proposal = FactProposal {
            id: "01J".to_string(),
            item_id: None,
            draft: draft(Some("Rust"), None),
            source_excerpt: Some("Rust, 5 years".to_string()),
            base_revision: None,
            untrusted_origin: true,
            agent_run_id: "run_01".to_string(),
            created_at: "2026-10-15T12:00:00.000Z".to_string(),
        };
        assert_eq!(proposal.untrusted_origin_flag(), 1);
        proposal.untrusted_origin = false;
        assert_eq!(proposal.untrusted_origin_flag(), 0);
    }
}
