//! Deduplication of ingested Offers into clusters (S4.3, FR-49, AD-19).
//!
//! One job posted on three boards is one posting in the inbox with three
//! provenance rows. This module owns the *decision* — `boba_domain::dedup` says
//! whether two Offers are the same job, and this use case asks the store for the
//! postings that could already be it, merges when one is, and inserts when none
//! is (AD-19: dedup only merges, it never deletes provenance).
//!
//! Nothing here re-writes an existing posting's fields. The posting the user
//! reads keeps what it was first seen with; a later source contributes its own
//! provenance row and refreshes `last_seen`, which is what the repost and
//! staleness signals of S4.4 read.

use boba_domain::dedup::{DedupProbe, DuplicateReason, duplicate_of};
use boba_domain::posting::Provenance;
use boba_domain::source::RawPosting;

use crate::ingest::normalize;
use crate::ports::repo::ingestion::{IngestionRepo, IngestionRepoError};

#[cfg(test)]
mod golden;

/// What ingestion did with one Offer (AD-19).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing was the same job: one new posting, one provenance row.
    New {
        /// The posting the Offer became.
        posting_id: String,
    },
    /// An existing posting was the same job: one more provenance row against
    /// it, nothing deleted.
    Merged {
        /// The posting the Offer was merged into.
        posting_id: String,
        /// The signal that said so.
        reason: DuplicateReason,
    },
}

impl Outcome {
    /// The posting to read — the one the Offer became, or the one it merged
    /// into. Never the id of an Offer that was thrown away.
    pub fn posting_id(&self) -> &str {
        match self {
            Self::New { posting_id } | Self::Merged { posting_id, .. } => posting_id,
        }
    }
}

/// Normalise one `RawPosting` and merge it into the posting it duplicates, if
/// there is one (FR-49, AD-19).
///
/// The normalized Offer is thrown away when it merges: the posting it merged
/// into is the one the user sees, and the Offer only ever contributed a
/// provenance row. Its id is therefore never a posting id — the caller gets the
/// id of the posting to read.
pub fn ingest_deduped(
    repo: &dyn IngestionRepo,
    raw: &RawPosting,
    source_id: &str,
    now: &str,
) -> Result<Outcome, IngestionRepoError> {
    let (offer, provenance) = normalize::normalize(raw, source_id, now);
    let probe = DedupProbe::of(&offer);

    let mut best: Option<(DuplicateReason, String)> = None;
    for candidate in repo.dedup_candidates(&probe)? {
        let Some(reason) = duplicate_of(&offer, &candidate) else {
            continue;
        };
        if best
            .as_ref()
            .is_none_or(|(best, _)| rank(reason) < rank(*best))
        {
            best = Some((reason, candidate.id));
        }
    }

    let Some((reason, posting_id)) = best else {
        repo.insert_offer(&offer, &provenance)?;
        return Ok(Outcome::New {
            posting_id: offer.id,
        });
    };

    repo.merge_provenance(
        &posting_id,
        &Provenance {
            posting_id: posting_id.clone(),
            ..provenance
        },
        now,
    )?;
    Ok(Outcome::Merged { posting_id, reason })
}

/// How strongly a signal identifies a job; lower is stronger, so the strongest
/// reason is the one reported when several candidates match.
fn rank(reason: DuplicateReason) -> u8 {
    match reason {
        DuplicateReason::CanonicalUrl => 0,
        DuplicateReason::JobKey => 1,
        DuplicateReason::Description => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::normalize::testing::FakeIngestionRepo;

    const NOW: &str = "2026-10-16T00:00:00Z";

    fn raw(external_id: &str, url: &str, title: &str) -> RawPosting {
        RawPosting {
            external_id: external_id.into(),
            title: title.into(),
            company: "Acme Corp".into(),
            url: url.into(),
            locations: vec!["Remote".into()],
            remote: Some(true),
            description_html: Some("<p>We build APIs in Rust and PostgreSQL.</p>".into()),
            posted_at: Some("2026-10-01T00:00:00Z".into()),
            salary_text: None,
            raw: format!(r#"{{"id":"{external_id}"}}"#),
        }
    }

    #[test]
    fn a_new_offer_makes_one_posting_with_one_provenance_row() {
        let repo = FakeIngestionRepo::default();

        let outcome = ingest_deduped(
            &repo,
            &raw("a1", "https://remotive.com/a1", "Backend Engineer"),
            "remotive",
            NOW,
        )
        .expect("ingested");

        let Outcome::New { posting_id } = outcome else {
            panic!("nothing was there to merge into: {outcome:?}");
        };
        assert_eq!(repo.postings().len(), 1);
        assert_eq!(repo.provenance(&posting_id).unwrap().len(), 1);
    }

    #[test]
    fn the_same_job_from_three_sources_is_one_posting_with_three_provenance_rows() {
        let repo = FakeIngestionRepo::default();

        let first = ingest_deduped(
            &repo,
            &raw("a1", "https://remotive.com/a1", "Backend Engineer"),
            "remotive",
            NOW,
        )
        .unwrap();
        let Outcome::New { posting_id } = first else {
            panic!("the first source makes the posting: {first:?}");
        };

        // The same job on two other boards: another URL each, and one of them
        // carries the usual tracking noise.
        let second = ingest_deduped(
            &repo,
            &raw("x9", "https://himalayas.app/jobs/x9", "Backend Engineer"),
            "himalayas",
            NOW,
        )
        .unwrap();
        let third = ingest_deduped(
            &repo,
            &raw(
                "77",
                "https://jobs.example.com/77?utm_source=linkedin",
                "Backend Engineer",
            ),
            "linkedin",
            "2026-10-17T00:00:00Z",
        )
        .unwrap();

        assert_eq!(
            second,
            Outcome::Merged {
                posting_id: posting_id.clone(),
                reason: DuplicateReason::JobKey
            },
            "same company + title + location bucket"
        );
        assert_eq!(
            third,
            Outcome::Merged {
                posting_id: posting_id.clone(),
                reason: DuplicateReason::JobKey
            }
        );

        assert_eq!(repo.postings().len(), 1, "one job is one posting");
        let rows = repo.provenance(&posting_id).unwrap();
        assert_eq!(rows.len(), 3, "every source is kept (AD-19)");
        assert_eq!(
            rows.iter()
                .map(|row| row.source_id.as_str())
                .collect::<Vec<_>>(),
            ["remotive", "himalayas", "linkedin"],
            "nothing deleted, in the order they were seen"
        );
        assert!(
            rows.iter().all(|row| row.posting_id == posting_id),
            "every row points at the one posting"
        );
        assert!(
            rows.iter()
                .any(|row| row.source_id == "linkedin" && row.raw == r#"{"id":"77"}"#)
        );
    }

    #[test]
    fn a_merged_offer_is_not_a_posting_of_its_own() {
        let repo = FakeIngestionRepo::default();
        let Outcome::New { posting_id } = ingest_deduped(
            &repo,
            &raw("a1", "https://remotive.com/a1", "Backend Engineer"),
            "remotive",
            NOW,
        )
        .unwrap() else {
            panic!("first source makes the posting");
        };
        ingest_deduped(
            &repo,
            &raw("x9", "https://himalayas.app/x9", "Backend Engineer"),
            "himalayas",
            NOW,
        )
        .unwrap();

        let postings = repo.postings();
        assert_eq!(postings.len(), 1);
        assert_eq!(postings[0].id, posting_id);
    }
}
