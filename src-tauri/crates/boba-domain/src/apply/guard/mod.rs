//! Duplicate-application guard (S9.5, FR-92).
//!
//! Sending two applications to the same company for the same job wastes the
//! user's time and reads as noise to the employer. Before a send, the core asks
//! this module whether an Application to the same company and a similar role
//! already exists inside the 30-day window; a yes is the `duplicate` fact the
//! Auto policy refuses (`boba_domain::apply::policy::Reason::Duplicate`) and the
//! warning the review screen shows (FR-92).
//!
//! Everything here is pure (AD-1): no IO, no store, no clock. The caller passes
//! the prior applications and `now`.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::application::AppStatus;

/// How far back a prior application still counts (FR-92).
pub const DUPLICATE_WINDOW_DAYS: i64 = 30;

/// The application the user is about to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateCandidate {
    pub company: String,
    pub role: String,
}

/// One existing application the guard compares the candidate against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriorApplication {
    pub company: String,
    pub role: String,
    pub status: AppStatus,
    /// When the application was created (RFC 3339, UTC).
    pub created_at: String,
}

/// The application the candidate duplicates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DuplicateMatch {
    /// The role of the matching application, as it was stored.
    pub role: String,
    /// When the matching application was created (RFC 3339, UTC).
    pub created_at: String,
    /// Whole days between that application and `now`.
    pub days_ago: i64,
}

/// Terms that name the same role differently: boards write "Developer" where
/// others write "Engineer".
fn canonical_term(term: &str) -> &str {
    match term {
        "dev" | "developer" | "programmer" | "engineer" => "engineer",
        "sr" => "senior",
        "jr" => "junior",
        _ => term,
    }
}

/// Words that carry no role meaning ("Ingeniero **de** Software").
const FILLER: &[&str] = &["a", "and", "de", "del", "e", "en", "of", "the", "y"];

fn role_terms(role: &str) -> std::collections::BTreeSet<String> {
    role.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty() && !FILLER.contains(word))
        .map(|word| canonical_term(word).to_string())
        .collect()
}

/// Whether two role titles name the same job (FR-92 "a similar role").
///
/// Two titles are the same role when one's terms are all in the other
/// (`Senior Rust Engineer` ⊇ `Rust Engineer`) or when their term overlap is at
/// least 60% (`Backend Engineer` ≠ `Frontend Engineer`).
pub fn roles_similar(a: &str, b: &str) -> bool {
    let a = role_terms(a);
    let b = role_terms(b);
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let (small, big) = if a.len() <= b.len() {
        (&a, &b)
    } else {
        (&b, &a)
    };
    if small.iter().all(|term| big.contains(term)) {
        return true;
    }
    let shared = a.intersection(&b).count();
    let union = a.union(&b).count();
    (shared as f32) / (union as f32) >= 0.6
}

/// The company in the form two spellings agree on: `Acme, Inc.` → `acme`.
fn company_key(company: &str) -> String {
    crate::apply::policy::company_key(company)
}

/// Find the application the candidate duplicates: same company, similar role,
/// created inside the window. Returns the closest one when there are several.
///
/// A `saved` card is only a bookmark, not an application the user pursued, and
/// a row whose date cannot be read is skipped rather than guessed at.
pub fn find_duplicate(
    candidate: &DuplicateCandidate,
    prior: &[PriorApplication],
    now: OffsetDateTime,
) -> Option<DuplicateMatch> {
    let company = company_key(&candidate.company);
    if company.is_empty() {
        return None;
    }
    let window = time::Duration::days(DUPLICATE_WINDOW_DAYS);
    let mut best: Option<(i64, &PriorApplication)> = None;
    for application in prior {
        if application.status == AppStatus::Saved {
            continue;
        }
        if company_key(&application.company) != company {
            continue;
        }
        if !roles_similar(&candidate.role, &application.role) {
            continue;
        }
        let Ok(created) = OffsetDateTime::parse(&application.created_at, &Rfc3339) else {
            continue;
        };
        let age = now - created;
        if age > window {
            continue;
        }
        let days_ago = age.whole_days().max(0);
        if best.is_none_or(|(days, _)| days_ago < days) {
            best = Some((days_ago, application));
        }
    }
    best.map(|(days_ago, application)| DuplicateMatch {
        role: application.role.clone(),
        created_at: application.created_at.clone(),
        days_ago,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: &str = "2026-10-17T12:00:00Z";

    fn now() -> OffsetDateTime {
        OffsetDateTime::parse(NOW, &Rfc3339).expect("valid now")
    }

    fn days_before(days: i64) -> String {
        (now() - time::Duration::days(days))
            .format(&Rfc3339)
            .expect("format")
    }

    fn prior(company: &str, role: &str, status: AppStatus, days_ago: i64) -> PriorApplication {
        PriorApplication {
            company: company.to_string(),
            role: role.to_string(),
            status,
            created_at: days_before(days_ago),
        }
    }

    fn candidate(company: &str, role: &str) -> DuplicateCandidate {
        DuplicateCandidate {
            company: company.to_string(),
            role: role.to_string(),
        }
    }

    #[test]
    fn roles_similar_ignores_case_punctuation_and_legal_noise() {
        assert!(roles_similar(
            "Senior Rust Engineer",
            "senior rust engineer"
        ));
        assert!(roles_similar(
            "Software Engineer, Backend",
            "Software Engineer Backend"
        ));
    }

    #[test]
    fn roles_similar_when_one_title_widens_the_other() {
        assert!(roles_similar("Senior Rust Engineer", "Rust Engineer"));
    }

    #[test]
    fn roles_similar_when_boards_use_developer_and_engineer() {
        assert!(roles_similar("Software Developer", "Software Engineer"));
    }

    #[test]
    fn roles_differ_when_the_technology_differs() {
        assert!(!roles_similar("Backend Engineer", "Frontend Engineer"));
        assert!(!roles_similar("Marketing Manager", "Product Manager"));
    }

    #[test]
    fn empty_title_never_matches() {
        assert!(!roles_similar("", "Software Engineer"));
    }

    #[test]
    fn finds_a_duplicate_within_the_window_despite_company_spelling() {
        let prior = [prior(
            "Acme, Inc.",
            "Software Engineer",
            AppStatus::Applied,
            10,
        )];
        let found = find_duplicate(&candidate("Acme", "Software Developer"), &prior, now());
        assert_eq!(
            found.map(|m| (m.role, m.days_ago)),
            Some(("Software Engineer".to_string(), 10))
        );
    }

    #[test]
    fn ignores_applications_older_than_the_window() {
        let prior = [prior("Acme", "Software Engineer", AppStatus::Applied, 31)];
        assert_eq!(
            find_duplicate(&candidate("Acme", "Software Engineer"), &prior, now()),
            None
        );
    }

    #[test]
    fn ignores_a_different_company() {
        let prior = [prior("Globex", "Software Engineer", AppStatus::Applied, 5)];
        assert_eq!(
            find_duplicate(&candidate("Acme", "Software Engineer"), &prior, now()),
            None
        );
    }

    #[test]
    fn ignores_a_different_role() {
        let prior = [prior("Acme", "Marketing Manager", AppStatus::Applied, 5)];
        assert_eq!(
            find_duplicate(&candidate("Acme", "Software Engineer"), &prior, now()),
            None
        );
    }

    #[test]
    fn ignores_a_saved_card() {
        let prior = [prior("Acme", "Software Engineer", AppStatus::Saved, 5)];
        assert_eq!(
            find_duplicate(&candidate("Acme", "Software Engineer"), &prior, now()),
            None
        );
    }

    #[test]
    fn reports_the_most_recent_match() {
        let prior = [
            prior("Acme", "Software Engineer", AppStatus::Applied, 20),
            prior("Acme Inc", "Software Developer", AppStatus::Interview, 3),
        ];
        let found = find_duplicate(&candidate("Acme", "Software Engineer"), &prior, now());
        assert_eq!(found.map(|m| m.days_ago), Some(3));
    }

    #[test]
    fn skips_rows_with_an_unreadable_date() {
        let prior = [PriorApplication {
            company: "Acme".into(),
            role: "Software Engineer".into(),
            status: AppStatus::Applied,
            created_at: "not a date".into(),
        }];
        assert_eq!(
            find_duplicate(&candidate("Acme", "Software Engineer"), &prior, now()),
            None
        );
    }
}
