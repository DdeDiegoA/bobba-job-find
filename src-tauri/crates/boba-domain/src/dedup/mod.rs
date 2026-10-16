//! Deduplication of Offers into clusters (S4.3, FR-49, AD-19).
//!
//! One job posted on three boards is one Offer in the inbox with three
//! provenance rows. Three signals say "same job", in the order they are trusted:
//! the **canonical apply URL**, the **`job_key`** (company + title + location
//! bucket, slugged, with a hash disambiguator), and the **description simhash**
//! of two near-identical descriptions (≥ 0.9 similarity).
//!
//! Everything here is pure: no IO, no store, no clock (AD-1). The app layer
//! decides what to do with a match; this module only says whether two Offers are
//! the same job and why.

use serde::{Deserialize, Serialize};

use crate::posting::Offer;

mod key;
mod simhash;
mod text;
mod url;

pub use key::job_key_of;
pub use simhash::{SIMHASH_SIMILARITY, description_simhash, similarity};
pub use url::canonical_url;

/// What the store is asked for when it looks for the posting an Offer may
/// already be (AD-19). The store answers with candidate postings; this module
/// decides whether any of them is the same job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DedupProbe {
    /// The Offer's `job_key`.
    pub dedup_key: String,
    /// Its canonical apply URL.
    pub canonical_url: String,
    /// Its company slug — the pool the description simhash is compared within,
    /// because two boards rarely agree on the title of the same job.
    pub company: String,
}

impl DedupProbe {
    pub fn of(offer: &Offer) -> Self {
        Self {
            dedup_key: job_key_of(offer),
            canonical_url: canonical_url(&offer.apply.url),
            company: key::company_key(&offer.company),
        }
    }
}

/// Whether a posting already in the store is worth comparing with an Offer
/// (AD-19): it carries the same `job_key`, it links to the same canonical apply
/// URL, or it is from the same company.
///
/// This is the store's filter, not the verdict — a candidate still has to pass
/// [`duplicate_of`]. The company is in the pool because two boards retitle the
/// same job often enough that the simhash needs somewhere to be compared at all;
/// everything else is a stranger and is never fetched.
pub fn probe_matches(probe: &DedupProbe, candidate: &Offer) -> bool {
    (!probe.dedup_key.is_empty() && job_key_of(candidate) == probe.dedup_key)
        || (!probe.canonical_url.is_empty()
            && canonical_url(&candidate.apply.url) == probe.canonical_url)
        || (!probe.company.is_empty() && key::company_key(&candidate.company) == probe.company)
}

/// The strongest reason these two Offers are the same job, or `None` (AD-19).
///
/// The order is the order of trust. An apply page is the same page; a slugged
/// company + title + location bucket is the same job posted twice; a
/// near-identical description is the weakest of the three, which is why it is
/// only believed when both descriptions are long enough to compare at all.
pub fn duplicate_of(offer: &Offer, existing: &Offer) -> Option<DuplicateReason> {
    let url = canonical_url(&offer.apply.url);
    if !url.is_empty() && url == canonical_url(&existing.apply.url) {
        return Some(DuplicateReason::CanonicalUrl);
    }

    let key = job_key_of(offer);
    if !key.is_empty() && key == job_key_of(existing) {
        return Some(DuplicateReason::JobKey);
    }

    if !same_place(offer, existing) {
        // One employer, one title, one description, two cities: two postings.
        // A similar description cannot outvote the location the key is built
        // from, or every repost across cities would collapse into one.
        return None;
    }

    match (
        description_simhash(&offer.description_text),
        description_simhash(&existing.description_text),
    ) {
        (Some(left), Some(right)) if similarity(left, right) >= SIMHASH_SIMILARITY => {
            Some(DuplicateReason::Description)
        }
        _ => None,
    }
}

/// Whether two Offers agree on where the job is, so far as either says. A board
/// that sends no location is not disagreeing — it is saying nothing.
fn same_place(offer: &Offer, existing: &Offer) -> bool {
    let (left, right) = (key::bucket_of(offer), key::bucket_of(existing));
    left.is_empty() || right.is_empty() || left == right
}

/// Why two Offers count as the same job (AD-19). The strongest signal found, in
/// the order they are checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateReason {
    /// Both apply pages are the same canonical URL.
    CanonicalUrl,
    /// Both carry the same `job_key`.
    JobKey,
    /// Both descriptions are ≥ 0.9 similar.
    Description,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::posting::{ApplyHints, WorkMode};

    fn offer(company: &str, title: &str, locations: &[&str]) -> Offer {
        Offer {
            id: "pst_1".into(),
            source_id: "remotive".into(),
            external_id: "abc-1".into(),
            title: title.into(),
            company: company.into(),
            locations: locations.iter().map(|l| l.to_string()).collect(),
            work_mode: WorkMode::Remote,
            money: None,
            posted_at: None,
            deadline: None,
            first_seen: "2026-10-16T00:00:00Z".into(),
            last_seen: "2026-10-16T00:00:00Z".into(),
            lang: "en".into(),
            description_html: String::new(),
            description_text: String::new(),
            requirements: Vec::new(),
            apply: ApplyHints {
                url: "https://jobs.example.com/abc-1".into(),
                email: None,
            },
        }
    }

    #[test]
    fn the_same_job_keeps_its_key_across_company_formatting() {
        let key = job_key_of(&offer("Acme Corp", "Backend Engineer", &["Remote"]));

        assert_eq!(
            key,
            job_key_of(&offer("  ACME  ", "Backend Engineer", &["Remote"])),
            "case, spacing and a legal suffix are the same company"
        );
        assert_eq!(
            key,
            job_key_of(&offer("Acmé, Inc.", "Backend Engineer", &["Remote"])),
            "accents fold, so an ES board and a US board still agree"
        );
        assert!(key.starts_with("acme-backend-engineer-remote-"), "{key}");
    }

    #[test]
    fn two_different_jobs_never_share_a_key() {
        let backend = job_key_of(&offer("Acme", "Backend Engineer", &["Remote"]));
        let frontend = job_key_of(&offer("Acme", "Frontend Engineer", &["Remote"]));
        assert_ne!(backend, frontend, "different titles, different jobs");

        let other_company = job_key_of(&offer("Globex", "Backend Engineer", &["Remote"]));
        assert_ne!(
            backend, other_company,
            "different companies, different jobs"
        );

        // The slug of `C++ Developer` and of `C Developer` is `c-developer`; the
        // disambiguator is what keeps them apart (AD-19).
        let cpp = job_key_of(&offer("Acme", "C++ Developer", &["Remote"]));
        let c = job_key_of(&offer("Acme", "C Developer", &["Remote"]));
        assert!(cpp.starts_with("acme-c-developer-"), "{cpp}");
        assert_ne!(cpp, c, "a slug collision must not merge two jobs");
    }

    #[test]
    fn the_location_bucket_groups_the_words_boards_use_for_one_place() {
        let remote = job_key_of(&offer("Acme", "Backend Engineer", &["Remote"]));
        assert_eq!(
            remote,
            job_key_of(&offer("Acme", "Backend Engineer", &["Anywhere"])),
            "'Remote' and 'Anywhere' are the same bucket"
        );
        assert_eq!(
            remote,
            job_key_of(&offer("Acme", "Backend Engineer", &["Work from home"])),
            "a remote phrase is a remote bucket"
        );
        assert_eq!(
            job_key_of(&offer("Acme", "Backend Engineer", &["Bogotá, CO"])),
            job_key_of(&offer("Acme", "Backend Engineer", &["Bogota"])),
            "the city before the comma, accent-folded"
        );
        assert_ne!(
            job_key_of(&offer("Acme", "Backend Engineer", &["New York, NY"])),
            job_key_of(&offer("Acme", "Backend Engineer", &["Bogotá, CO"])),
            "two onsite cities are two different jobs"
        );
        assert_ne!(
            job_key_of(&offer("Acme", "Backend Engineer", &[])),
            remote,
            "no location is not the remote bucket"
        );
    }

    #[test]
    fn tracking_noise_and_a_fragment_are_not_a_new_apply_page() {
        assert_eq!(
            canonical_url(
                "HTTPS://Jobs.Example.com:443/apply/abc-1/?utm_source=linkedin&utm_campaign=daily#apply"
            ),
            canonical_url("https://jobs.example.com/apply/abc-1"),
            "case, a default port, a tracking query, a fragment and a trailing slash are noise"
        );
        assert_eq!(
            canonical_url("https://www.jobs.example.com/apply/abc-1"),
            canonical_url("https://jobs.example.com/apply/abc-1"),
            "www is the same host"
        );
        assert_eq!(canonical_url("   "), "", "nothing to canonicalise");
    }

    #[test]
    fn a_real_query_parameter_still_tells_two_apply_pages_apart() {
        assert_ne!(
            canonical_url("https://boards.example.com/jobs?gh_jid=1"),
            canonical_url("https://boards.example.com/jobs?gh_jid=2"),
            "the ATS job id is the posting, not noise"
        );
        assert_eq!(
            canonical_url("https://boards.example.com/jobs?gh_jid=1&b=2"),
            canonical_url("https://boards.example.com/jobs?b=2&gh_jid=1"),
            "the order of the parameters is noise"
        );
    }

    /// A realistic posting: real ones run 150-400 words, and the simhash is only
    /// asked to survive the edits a board makes to a description this long.
    const BACKEND_JOB: &str = "About the role. We are looking for a Backend Engineer to join \
        our platform team in a remote first company. You will design and build APIs in Rust \
        and PostgreSQL, own services end to end, and work with product managers to ship \
        features that our customers use every day. You will review code, write tests, and \
        keep our deployment pipeline boring and fast. About you. You have at least four years \
        building production backends, you are comfortable with SQL, and you care about \
        observability, queues and clean interfaces. You have worked on systems that handle \
        real traffic and you know when to reach for a simpler design. About us. We are a \
        distributed team across Europe and Latin America, we pair on hard problems, and we \
        keep meetings short. We offer a remote first culture, a learning budget, twenty five \
        days of holiday and a competitive salary reviewed every year.";

    const MARKETING_JOB: &str = "About the role. We are looking for a Growth Marketing Manager \
        to lead paid acquisition across search and social. You will own the media budget, run \
        experiments, report on conversion, and partner with design on campaign creative for \
        our brand. About you. You have five years of performance marketing experience, you \
        read a dashboard before you trust a hunch, and you are fluent in attribution. You \
        have run campaigns at scale and you know how to brief a copywriter. About us. We are a \
        small brand team inside a larger consumer company, we move fast, and we measure \
        everything. We offer health cover, a wellness allowance, flexible hours and a bonus \
        tied to growth targets that we set together at the start of each quarter.";

    #[test]
    fn a_lightly_reworded_republish_still_reads_as_the_same_description() {
        let original = description_simhash(BACKEND_JOB).expect("long enough to compare");

        assert_eq!(
            similarity(original, description_simhash(BACKEND_JOB).unwrap()),
            1.0,
            "the same text is the most similar text"
        );

        let reworded = BACKEND_JOB.replace("a competitive salary", "competitive pay");
        let reworded = description_simhash(&reworded).expect("long enough to compare");
        assert!(
            similarity(original, reworded) >= SIMHASH_SIMILARITY,
            "a reworded sentence must not hide the duplicate ({})",
            similarity(original, reworded)
        );

        // What a board actually does to a copied description: drop a paragraph
        // and add a call to action of its own.
        let shortened = format!(
            "{} Apply now through our careers page.",
            BACKEND_JOB.replace(
                " We offer a remote first culture, a learning budget, twenty five \
                 days of holiday and a competitive salary reviewed every year.",
                ""
            )
        );
        let shortened = description_simhash(&shortened).expect("still long enough");
        assert!(
            similarity(original, shortened) >= SIMHASH_SIMILARITY,
            "a dropped paragraph must not hide the duplicate ({})",
            similarity(original, shortened)
        );
    }

    #[test]
    fn two_different_jobs_do_not_read_as_the_same_description() {
        let backend = description_simhash(BACKEND_JOB).unwrap();
        let marketing = description_simhash(MARKETING_JOB).unwrap();

        assert!(
            similarity(backend, marketing) < SIMHASH_SIMILARITY,
            "unrelated postings must stay apart ({})",
            similarity(backend, marketing)
        );
    }

    #[test]
    fn the_strongest_signal_decides_why_two_offers_are_the_same_job() {
        let mut posted = offer("Acme Corp", "Backend Engineer", &["Remote"]);
        posted.description_text = BACKEND_JOB.into();

        // The same apply page, re-published under a different title.
        let mut relabelled = posted.clone();
        relabelled.title = "Backend Engineer (Platform)".into();
        relabelled.apply.url = "https://jobs.example.com/abc-1?utm_source=linkedin".into();
        assert_eq!(
            duplicate_of(&posted, &relabelled),
            Some(DuplicateReason::CanonicalUrl)
        );

        // Another board, another URL, same company + title + bucket.
        let mut elsewhere = posted.clone();
        elsewhere.apply.url = "https://other.example.com/jobs/99".into();
        assert_eq!(
            duplicate_of(&posted, &elsewhere),
            Some(DuplicateReason::JobKey)
        );

        // A third board that reworded the text and retitled the job.
        let mut reworded = elsewhere.clone();
        reworded.title = "Senior Backend Engineer".into();
        reworded.apply.url = "https://third.example.com/x".into();
        reworded.description_text = format!("{BACKEND_JOB} Apply now through our careers page.");
        assert_eq!(
            duplicate_of(&posted, &reworded),
            Some(DuplicateReason::Description)
        );

        // A different job at the same company.
        let mut other_job = posted.clone();
        other_job.title = "Growth Marketing Manager".into();
        other_job.apply.url = "https://jobs.example.com/abc-2".into();
        other_job.description_text = MARKETING_JOB.into();
        assert_eq!(duplicate_of(&posted, &other_job), None);
    }

    #[test]
    fn a_similar_description_never_overrules_the_location() {
        let mut remote = offer("Acme", "Backend Engineer", &["Remote"]);
        let mut madrid = offer("Acme", "Backend Engineer", &["Madrid, ES"]);
        madrid.apply.url = "https://other.example.com/madrid".into();
        madrid.title = "Backend Engineer".into();
        // The employer re-used the same text in both cities.
        remote.description_text = BACKEND_JOB.into();
        madrid.description_text = BACKEND_JOB.into();

        assert!(
            similarity(
                description_simhash(&remote.description_text).unwrap(),
                description_simhash(&madrid.description_text).unwrap()
            ) >= SIMHASH_SIMILARITY,
            "the descriptions really are near-identical — that is the trap"
        );
        assert_eq!(
            duplicate_of(&remote, &madrid),
            None,
            "two cities are two postings, however identical the text"
        );

        // But a board that sends no location at all cannot be told apart, so a
        // near-identical description still counts there.
        let mut unknown = offer("Acme", "Backend Engineer", &[]);
        unknown.apply.url = "https://third.example.com/x".into();
        unknown.description_text = format!("{BACKEND_JOB} Apply now through our careers page.");
        assert_eq!(
            duplicate_of(&remote, &unknown),
            Some(DuplicateReason::Description)
        );
    }

    #[test]
    fn offers_with_nothing_to_compare_are_never_merged() {
        let mut bare = offer("", "", &[]);
        let mut also_bare = bare.clone();
        also_bare.id = "pst_2".into();

        assert_eq!(job_key_of(&bare), "", "nothing to key on");
        bare.apply.url = String::new();
        also_bare.apply.url = String::new();
        assert_eq!(
            duplicate_of(&bare, &also_bare),
            None,
            "two empty offers are not the same job"
        );

        // Two offers whose descriptions say too little to compare.
        let mut thin = offer("Acme", "Backend Engineer", &["Remote"]);
        let mut also_thin = offer("Globex", "Backend Engineer II", &["Remote"]);
        also_thin.apply.url = "https://other.example.com/x".into();
        thin.description_text = "Senior Rust engineer wanted.".into();
        also_thin.description_text = "Senior Rust engineer wanted.".into();
        assert_eq!(duplicate_of(&thin, &also_thin), None);
    }

    #[test]
    fn the_probe_carries_what_the_store_must_look_up() {
        let probe = DedupProbe::of(&offer("Acme Corp", "Backend Engineer", &["Remote"]));

        assert_eq!(
            probe.dedup_key,
            job_key_of(&offer("Acme Corp", "Backend Engineer", &["Remote"]))
        );
        assert_eq!(
            probe.company, "acme",
            "the company pool the simhash compares within"
        );
        assert_eq!(
            probe.canonical_url, "https://jobs.example.com/abc-1",
            "the URL as two boards would both link to it"
        );
    }

    #[test]
    fn the_probe_finds_the_stored_postings_worth_comparing() {
        let mut posted = offer("Acme Corp", "Backend Engineer", &["Remote"]);
        posted.description_text = BACKEND_JOB.into();
        let probe = DedupProbe::of(&posted);

        let mut same_key = posted.clone();
        same_key.id = "pst_2".into();
        same_key.apply.url = "https://another.example.com/jobs/9".into();
        assert!(probe_matches(&probe, &same_key), "same job_key");

        let mut same_url = posted.clone();
        same_url.id = "pst_3".into();
        same_url.title = "Backend Engineer, Platform".into();
        assert!(probe_matches(&probe, &same_url), "same canonical apply URL");

        let mut same_company = posted.clone();
        same_company.id = "pst_4".into();
        same_company.title = "Data Engineer".into();
        same_company.apply.url = "https://another.example.com/jobs/11".into();
        assert!(
            probe_matches(&probe, &same_company),
            "same company is the pool the simhash is compared within"
        );

        let mut stranger = posted.clone();
        stranger.id = "pst_5".into();
        stranger.company = "Globex".into();
        stranger.title = "Data Engineer".into();
        stranger.apply.url = "https://another.example.com/jobs/12".into();
        assert!(!probe_matches(&probe, &stranger), "nothing in common");
    }

    #[test]
    fn an_empty_probe_matches_nothing() {
        let mut empty = offer("", "", &[]);
        empty.apply.url = String::new();
        let probe = DedupProbe::of(&empty);

        assert!(
            !probe_matches(&probe, &empty),
            "an Offer with nothing on it is not its own duplicate"
        );
    }

    #[test]
    fn a_description_too_short_to_be_evidence_has_no_simhash() {
        assert_eq!(description_simhash(""), None);
        assert_eq!(description_simhash("Apply now!"), None);
        assert_eq!(
            description_simhash("Senior Rust engineer wanted. Remote. Apply today."),
            None,
            "a sentence is not evidence of a duplicate"
        );
        assert!(description_simhash(BACKEND_JOB).is_some());
    }
}
