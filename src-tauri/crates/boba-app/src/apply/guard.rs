//! The duplicate-application guard, wired to the store (S9.5, FR-92).
//!
//! The decision itself is pure ([`boba_domain::apply::guard`]); this adapter
//! hands it the board. Every apply path calls [`duplicate_for`] on its own
//! application: the manual, email, ATS and session views show the warning, and
//! `PolicyCandidate.duplicate` is what blocks Auto on the outbox.

use boba_domain::apply::guard::{
    DuplicateCandidate, DuplicateMatch, PriorApplication, find_duplicate,
};
use serde::{Deserialize, Serialize};
use specta::Type;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::applications::OfferSnapshot;
use crate::ports::repo::apply::{ApplyRepo, ApplyRepoError};

/// The warning an apply screen shows before it sends (FR-92).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct DuplicateGuardView {
    /// The role of the application already sent, as it was stored.
    pub role: String,
    /// Whole days since that application was created.
    pub days_ago: u32,
}

impl From<DuplicateMatch> for DuplicateGuardView {
    fn from(matched: DuplicateMatch) -> Self {
        Self {
            role: matched.role,
            // Whole local days, never negative; a clock that says otherwise
            // reads as the day it happened.
            days_ago: u32::try_from(matched.days_ago).unwrap_or(0),
        }
    }
}

/// The prior applications, read from their frozen Offer snapshots. A snapshot
/// that no longer parses is skipped: a broken card must not hide a real
/// duplicate, and it must not invent one either.
fn prior_applications(
    apply: &dyn ApplyRepo,
) -> Result<Vec<(String, PriorApplication)>, ApplyRepoError> {
    Ok(apply
        .applications()?
        .into_iter()
        .map(|row| {
            let snapshot: OfferSnapshot =
                serde_json::from_str(&row.offer_snapshot_json).unwrap_or_default();
            let prior = PriorApplication {
                company: snapshot.company,
                role: snapshot.title,
                status: row.status,
                created_at: row.created_at.clone(),
            };
            (row.id, prior)
        })
        .collect())
}

/// Whether this application duplicates one already on the board (FR-92).
///
/// The application itself is excluded: it is on the board too, and matching it
/// would make every send a duplicate of itself.
pub fn duplicate_for(
    apply: &dyn ApplyRepo,
    application: &boba_domain::application::Application,
    now: &str,
) -> Result<Option<DuplicateGuardView>, ApplyRepoError> {
    let Ok(now) = OffsetDateTime::parse(now, &Rfc3339) else {
        // The clock is this app's own RFC 3339 writer; a value it cannot read
        // says nothing about duplicates.
        return Ok(None);
    };
    let snapshot: OfferSnapshot =
        serde_json::from_str(&application.offer_snapshot_json).unwrap_or_default();
    let candidate = DuplicateCandidate {
        company: snapshot.company,
        role: snapshot.title,
    };
    let prior: Vec<PriorApplication> = prior_applications(apply)?
        .into_iter()
        .filter(|(id, _)| id != &application.id)
        .map(|(_, prior)| prior)
        .collect();
    Ok(find_duplicate(&candidate, &prior, now).map(DuplicateGuardView::from))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::applications::testing::FakeApplyRepo;
    use boba_domain::application::{AppStatus, Application};

    const NOW: &str = "2026-10-17T12:00:00Z";

    fn days_ago(days: i64) -> String {
        let now = OffsetDateTime::parse(NOW, &Rfc3339).expect("valid now");
        (now - time::Duration::days(days))
            .format(&Rfc3339)
            .expect("format")
    }

    fn application(
        id: &str,
        company: &str,
        title: &str,
        status: AppStatus,
        at: String,
    ) -> Application {
        Application {
            id: id.to_string(),
            posting_id: format!("pst_{id}"),
            status,
            offer_snapshot_json: OfferSnapshot {
                title: title.to_string(),
                company: company.to_string(),
                apply_url: None,
                lang: Some("en".to_string()),
                deadline: None,
            }
            .to_json(),
            notes: None,
            next_action: None,
            created_at: at.clone(),
            updated_at: at,
        }
    }

    #[test]
    fn the_application_never_duplicates_itself() {
        let repo = FakeApplyRepo::default();
        let row = application(
            "app_1",
            "Acme",
            "Software Engineer",
            AppStatus::Drafted,
            days_ago(2),
        );
        repo.seed_application(row.clone());
        assert_eq!(duplicate_for(&repo, &row, NOW).unwrap(), None);
    }

    #[test]
    fn finds_a_recent_application_to_the_same_company_and_role() {
        let repo = FakeApplyRepo::default();
        let prior = application(
            "app_1",
            "Acme, Inc.",
            "Software Engineer",
            AppStatus::Applied,
            days_ago(9),
        );
        let target = application(
            "app_2",
            "Acme",
            "Software Developer",
            AppStatus::Drafted,
            days_ago(1),
        );
        repo.seed_application(prior);
        repo.seed_application(target.clone());
        assert_eq!(
            duplicate_for(&repo, &target, NOW).unwrap(),
            Some(DuplicateGuardView {
                role: "Software Engineer".to_string(),
                days_ago: 9,
            })
        );
    }

    #[test]
    fn ignores_a_prior_application_outside_the_window() {
        let repo = FakeApplyRepo::default();
        repo.seed_application(application(
            "app_1",
            "Acme",
            "Software Engineer",
            AppStatus::Applied,
            days_ago(45),
        ));
        let target = application(
            "app_2",
            "Acme",
            "Software Engineer",
            AppStatus::Drafted,
            days_ago(1),
        );
        repo.seed_application(target.clone());
        assert_eq!(duplicate_for(&repo, &target, NOW).unwrap(), None);
    }
}
