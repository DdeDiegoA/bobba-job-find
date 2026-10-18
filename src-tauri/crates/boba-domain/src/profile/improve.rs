//! Fact improvement suggestions (FX10a, S3.6): an agent run rewrites the text
//! of an existing fact as a **proposal**, never as a fact.
//!
//! Pure model. The core, not the run, decides what is safe to keep:
//!
//! * a suggestion names the revision it was written against; once the item
//!   moves past it the suggestion is **stale** and is refused on confirm, with
//!   nothing written (a newer edit always wins);
//! * an improvement rewrites the variants the fact already has — filling a
//!   missing one is [`Lang`]'s translate path (S3.6), not this one;
//! * the profile's own rules still apply: a suggestion stores at least one
//!   non-blank variant.

use super::{FactDraft, Lang, ProfileFact, ProfileRuleError};

/// One improvement an agent suggested for an existing fact item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactImprovement {
    /// Stable ULID of the fact item the suggestion revises.
    pub item_id: String,
    /// The revision the run read and wrote against (AC3).
    pub base_revision: String,
    pub draft: FactDraft,
}

impl FactImprovement {
    /// Whether the item has moved past the revision this suggestion was written
    /// against, or no longer exists (AC3: a stale confirm writes nothing).
    pub fn is_stale(&self, current: Option<&ProfileFact>) -> bool {
        match current {
            Some(fact) => fact.item_id != self.item_id || fact.id != self.base_revision,
            None => true,
        }
    }

    /// Whether the suggestion keeps the fact's variant set: it may rewrite an
    /// existing variant or drop one, but it never fills a missing one (that is
    /// the translate task's job, S3.6).
    pub fn keeps_variants(&self, current: &ProfileFact) -> bool {
        Lang::ALL
            .into_iter()
            .all(|lang| current.text(lang).is_some() || self.draft.text(lang).is_none())
    }

    /// The profile rule a suggestion must satisfy before it can be stored.
    pub fn validate(&self) -> Result<(), ProfileRuleError> {
        self.draft.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{FactDraft, FactKind, Lang};

    fn draft(en: Option<&str>, es: Option<&str>) -> FactDraft {
        FactDraft {
            kind: FactKind::Experience,
            text_en: en.map(str::to_string),
            text_es: es.map(str::to_string),
            evidence: None,
            detail_json: "{}".to_string(),
        }
    }

    fn fact(id: &str, item_id: &str, en: Option<&str>, es: Option<&str>) -> ProfileFact {
        ProfileFact {
            id: id.to_string(),
            item_id: item_id.to_string(),
            kind: FactKind::Experience,
            text_en: en.map(str::to_string),
            text_es: es.map(str::to_string),
            evidence: None,
            detail_json: "{}".to_string(),
            supersedes: None,
            profile_version: 1,
            provenance: crate::profile::Provenance::User,
            agent_run_id: None,
            created_at: "2026-10-18T09:00:00.000Z".to_string(),
        }
    }

    // FX10a AC3: a suggestion is stale when the item has moved past the
    // revision the run read — or vanished altogether.
    #[test]
    fn a_suggestion_is_stale_when_the_fact_moved_on() {
        let suggestion = FactImprovement {
            item_id: "01ITEM".to_string(),
            base_revision: "01REV1".to_string(),
            draft: draft(Some("Rust and Go"), None),
        };
        let unchanged = fact("01REV1", "01ITEM", Some("Rust"), None);
        assert!(!suggestion.is_stale(Some(&unchanged)));

        let edited = fact("01REV2", "01ITEM", Some("Rust, Go and SQL"), None);
        assert!(suggestion.is_stale(Some(&edited)));

        let other_item = fact("01REV1", "01OTHER", Some("Rust"), None);
        assert!(suggestion.is_stale(Some(&other_item)));

        assert!(suggestion.is_stale(None));
    }

    // FX10a: an improvement rewrites the variants the fact already has; filling
    // a missing one is the translate task's job (S3.6), so the core drops a
    // suggestion that invents a language.
    #[test]
    fn a_suggestion_may_not_add_the_missing_variant() {
        let current = fact("01REV1", "01ITEM", Some("Rust"), None);
        let rewrite = FactImprovement {
            item_id: "01ITEM".to_string(),
            base_revision: "01REV1".to_string(),
            draft: draft(Some("Rust and Go"), None),
        };
        assert!(rewrite.keeps_variants(&current));

        let invents_es = FactImprovement {
            item_id: "01ITEM".to_string(),
            base_revision: "01REV1".to_string(),
            draft: draft(Some("Rust and Go"), Some("Rust y Go")),
        };
        assert!(!invents_es.keeps_variants(&current));

        let both = fact("01REV1", "01ITEM", Some("Rust"), Some("Rust"));
        assert!(invents_es.keeps_variants(&both));

        let drops_es = FactImprovement {
            item_id: "01ITEM".to_string(),
            base_revision: "01REV1".to_string(),
            draft: draft(Some("Rust and Go"), None),
        };
        assert!(
            drops_es.keeps_variants(&both),
            "removing a variant is an edit the user reviews, not an invention"
        );
    }

    // The profile's own rule still applies: a suggestion with no text at all
    // can never be stored.
    #[test]
    fn a_suggestion_without_text_is_refused() {
        let blank = FactImprovement {
            item_id: "01ITEM".to_string(),
            base_revision: "01REV1".to_string(),
            draft: draft(None, Some("  ")),
        };
        assert_eq!(
            blank.validate(),
            Err(crate::profile::ProfileRuleError::BlankVariant { lang: Lang::Es })
        );
        let empty = FactImprovement {
            item_id: "01ITEM".to_string(),
            base_revision: "01REV1".to_string(),
            draft: draft(None, None),
        };
        assert_eq!(
            empty.validate(),
            Err(crate::profile::ProfileRuleError::NoVariant)
        );
    }
}
