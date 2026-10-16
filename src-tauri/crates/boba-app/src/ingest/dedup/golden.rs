//! The golden dedup fixture set (S4.3 AC-2, FR-49).
//!
//! Twelve real jobs, twenty-nine sightings of them, each labeled with the job it
//! is: every sighting inside a cluster is a duplicate of the others, and no
//! sighting in one cluster is the same job as anything in another. The set is
//! ingested through the real use case, in order, against the in-memory repo, and
//! then measured pairwise: how many same-job pairs ended up on one posting
//! (recall) and how many different-job pairs did (false merges).
//!
//! The clusters are written to be awkward rather than convenient: the same
//! employer hiring for two different roles, two boards that retitle the same
//! job, a board that cuts a paragraph, an ATS link with tracking noise, a
//! Spanish board that spells the city without its accent, and the same role
//! posted in two cities. Nothing here is tuned to the algorithm — the variations
//! are the edits boards actually make.

use boba_domain::source::RawPosting;

use super::ingest_deduped;
use crate::ingest::normalize::testing::FakeIngestionRepo;

const NOW: &str = "2026-10-16T00:00:00Z";

/// What a board does to the description it copied. Applied to the cluster's base
/// text, so a variant is never written with the algorithm in mind.
#[derive(Clone, Copy)]
enum Variant {
    /// Copied whole.
    Whole,
    /// The closing paragraph about the company is cut.
    NoTail,
    /// A call to action is appended.
    WithCta,
    /// The sentences are put in a different order.
    Reordered,
    /// The employer's own boilerplate is published with it, as it is with every
    /// posting that employer makes.
    WithEmployerBoilerplate,
}

/// One board's sighting of one job.
struct Sighting {
    source: &'static str,
    external_id: &'static str,
    company: &'static str,
    title: &'static str,
    locations: &'static [&'static str],
    url: String,
    description: String,
}

/// One job, as the boards that carry it saw it. Every sighting in one cluster is
/// the same job; nothing in one cluster is the same job as anything in another.
///
/// One line per sighting, so the set can be read as the table it is.
#[rustfmt::skip]
fn clusters() -> Vec<Vec<Sighting>> {
    vec![
        // A backend job on three boards: one copies it whole, one cuts the
        // company paragraph, one retitles it and appends its own call to action.
        vec![
            sighting("remotive", "r-1", "Acme Corp", "Backend Engineer", &["Remote"], "https://remotive.com/remote-jobs/r-1", BACKEND, Variant::Whole),
            sighting("himalayas", "h-1", "Acme Corp", "Backend Engineer", &["Remote"], "https://himalayas.app/jobs/h-1", BACKEND, Variant::NoTail),
            sighting("linkedin", "l-1", "Acme Corp", "Senior Backend Engineer", &["Remote"], "https://www.linkedin.com/jobs/view/l-1?utm_source=feed&trk=public", BACKEND, Variant::WithCta),
            // A fourth board rewrote the title until the key cannot see it: only
            // the description still says this is the same job.
            sighting("torre", "t-1", "Acme Corp", "Software Engineer, Backend Platform", &["Remote"], "https://torre.ai/jobs/t-1", BACKEND, Variant::WithCta),
        ],
        // The same employer, a different job. Must not be merged with the
        // cluster above, which shares its company.
        vec![
            sighting("remotive", "r-2", "Acme Corp", "Data Analyst", &["Remote"], "https://remotive.com/remote-jobs/r-2", DATA, Variant::Whole),
            sighting("himalayas", "h-2", "Acme Corp", "Data Analyst", &["Remote"], "https://himalayas.app/jobs/h-2", DATA, Variant::WithCta),
        ],
        // One job, two boards, and the board's own tracking query.
        vec![
            sighting("greenhouse", "g-3", "Initech", "Frontend Engineer", &["Remote"], "https://boards.greenhouse.io/initech/jobs/g-3", FRONTEND, Variant::Whole),
            sighting("greenhouse", "g-3", "Initech", "Frontend Engineer", &["Remote"], "https://boards.greenhouse.io/initech/jobs/g-3?gh_src=abc&utm_medium=jobboard", FRONTEND, Variant::Whole),
            sighting("remotive", "r-3", "Initech", "Frontend Engineer", &["Remote"], "https://remotive.com/remote-jobs/r-3", FRONTEND, Variant::NoTail),
        ],
        // A Spanish board and a global one, and a company name a board wrote
        // with its legal form.
        vec![
            sighting("torre", "t-4", "Bogotá Labs", "Backend Engineer", &["Bogotá, CO"], "https://torre.ai/jobs/t-4", BACKEND, Variant::Whole),
            sighting("remotive", "r-4", "Bogota Labs S.A.S.", "Backend Engineer", &["Bogota"], "https://remotive.com/remote-jobs/r-4", BACKEND, Variant::NoTail),
        ],
        // The same role as the first cluster, at another company, with the same
        // description text a recruiter reused. Another employer is another job.
        vec![
            sighting("remotive", "r-5", "Globex", "Backend Engineer", &["Remote"], "https://remotive.com/remote-jobs/r-5", BACKEND, Variant::Whole),
            sighting("torre", "t-5", "Globex", "Backend Engineer", &["Remote"], "https://torre.ai/jobs/t-5", BACKEND, Variant::Reordered),
        ],
        // One employer hiring the same role in two cities: two postings of one
        // job each, with the employer's boilerplate in both. They are two
        // clusters — one sighting each — because they are two openings.
        vec![sighting("remotive", "r-6", "Hooli", "Backend Engineer", &["Bogotá, CO"], "https://remotive.com/remote-jobs/r-6", BACKEND, Variant::Whole)],
        vec![sighting("remotive", "r-7", "Hooli", "Backend Engineer", &["New York, NY"], "https://remotive.com/remote-jobs/r-7", BACKEND, Variant::NoTail)],
        // Marketing, on two boards, one of which reorders the sentences.
        vec![
            sighting("remotive", "r-8", "Globex", "Growth Marketing Manager", &["Remote"], "https://remotive.com/remote-jobs/r-8", MARKETING, Variant::Whole),
            sighting("himalayas", "h-8", "Globex", "Growth Marketing Manager", &["Remote"], "https://himalayas.app/jobs/h-8", MARKETING, Variant::Reordered),
        ],
        // A design job whose title the second board parenthesised.
        vec![
            sighting("remotive", "r-9", "Umbrella", "Product Designer", &["Remote"], "https://remotive.com/remote-jobs/r-9", DESIGN, Variant::Whole),
            sighting("himalayas", "h-9", "Umbrella", "Product Designer (Remote)", &["Remote"], "https://himalayas.app/jobs/h-9", DESIGN, Variant::WithCta),
        ],
        // Sales, three boards, and a company written three ways.
        vec![
            sighting("remotive", "r-10", "Initech Inc.", "Sales Development Representative", &["New York, NY"], "https://remotive.com/remote-jobs/r-10", SALES, Variant::Whole),
            sighting("himalayas", "h-10", "Initech", "Sales Development Representative", &["New York, NY"], "https://himalayas.app/jobs/h-10", SALES, Variant::WithCta),
            sighting("linkedin", "l-10", "INITECH, INC", "Sales Development Representative", &["New York, NY"], "https://www.linkedin.com/jobs/view/l-10", SALES, Variant::NoTail),
        ],
        // A data job on two boards that both cut the company paragraph.
        vec![
            sighting("himalayas", "h-11", "Globex", "Data Analyst", &["Remote"], "https://himalayas.app/jobs/h-11", DATA, Variant::NoTail),
            sighting("torre", "t-11", "Globex", "Data Analyst", &["Remote"], "https://torre.ai/jobs/t-11", DATA, Variant::Whole),
        ],
        // A design job the employer runs on its own ATS and on one board.
        vec![
            sighting("greenhouse", "g-12", "Umbrella", "Product Designer", &["Madrid, ES"], "https://boards.greenhouse.io/umbrella/jobs/g-12", DESIGN, Variant::Whole),
            sighting("torre", "t-12", "Umbrella", "Product Designer", &["Madrid"], "https://torre.ai/jobs/t-12", DESIGN, Variant::NoTail),
        ],
        // Two different jobs at one employer that advertise through the same
        // careers index, and nothing else (reviewer-1 F1). One sighting each:
        // they are two openings, so a shared listing URL is all they share.
        vec![sighting("remotive", "r-13", "Vandelay", "Backend Engineer", &["Remote"], "https://vandelay.com/careers", BACKEND, Variant::Whole)],
        vec![sighting("torre", "t-13", "Vandelay", "Data Analyst", &["Madrid, ES"], "https://vandelay.com/careers/", DATA, Variant::Whole)],
        // Two different jobs at one employer, in one place, published with the
        // employer's boilerplate on both (reviewer-1 F2). One sighting each.
        vec![sighting("remotive", "r-14", "Soylent", "Backend Engineer", &["Remote"], "https://soylent.com/jobs/1", BACKEND, Variant::WithEmployerBoilerplate)],
        vec![sighting("himalayas", "h-14", "Soylent", "Data Analyst", &["Remote"], "https://soylent.com/jobs/2", DATA, Variant::WithEmployerBoilerplate)],
    ]
}

// A fixture row is a table row: eight columns, written one line each.
#[allow(clippy::too_many_arguments)]
fn sighting(
    source: &'static str,
    external_id: &'static str,
    company: &'static str,
    title: &'static str,
    locations: &'static [&'static str],
    url: &str,
    base: &str,
    variant: Variant,
) -> Sighting {
    Sighting {
        source,
        external_id,
        company,
        title,
        locations,
        url: url.to_string(),
        description: variant_of(base, variant),
    }
}

fn variant_of(base: &str, variant: Variant) -> String {
    match variant {
        Variant::Whole => base.to_string(),
        Variant::NoTail => match base.split_once(" About us.") {
            Some((head, _)) => head.to_string(),
            None => base.to_string(),
        },
        Variant::WithCta => format!("{base} Apply now through our careers page."),
        Variant::WithEmployerBoilerplate => format!("{EMPLOYER_BOILERPLATE} {base}"),
        Variant::Reordered => {
            let mut sentences: Vec<&str> = base.split_inclusive(". ").collect();
            sentences.reverse();
            sentences.concat()
        }
    }
}

fn raw_of(sighting: &Sighting) -> RawPosting {
    RawPosting {
        external_id: sighting.external_id.into(),
        title: sighting.title.into(),
        company: sighting.company.into(),
        url: sighting.url.clone(),
        locations: sighting.locations.iter().map(|l| l.to_string()).collect(),
        remote: Some(sighting.locations.iter().any(|l| l.contains("Remote"))),
        description_html: Some(format!("<p>{}</p>", sighting.description)),
        posted_at: Some("2026-10-01T00:00:00Z".into()),
        salary_text: None,
        raw: format!(r#"{{"id":"{}"}}"#, sighting.external_id),
    }
}

/// What the set scored: the two numbers of AC-2, and the pairs behind them.
struct Scored {
    sightings: usize,
    clusters: usize,
    recall: f64,
    false_rate: f64,
    wrong: Vec<String>,
}

/// Ingest the whole set in order, then score every pair of sightings: a pair
/// inside one cluster is a duplicate; a pair across clusters is not.
fn score_the_set() -> Scored {
    let repo = FakeIngestionRepo::default();
    let clusters = clusters();

    let (mut labels, mut postings) = (Vec::new(), Vec::new());
    for (cluster, sightings) in clusters.iter().enumerate() {
        for sighting in sightings {
            let outcome = ingest_deduped(&repo, &raw_of(sighting), sighting.source, NOW)
                .expect("the in-memory repo does not fail");
            labels.push(cluster);
            postings.push(outcome.posting_id().to_string());
        }
    }

    let (mut merged, mut duplicates) = (0usize, 0usize);
    let (mut false_merges, mut distinct) = (0usize, 0usize);
    let mut wrong = Vec::new();
    for left in 0..labels.len() {
        for right in left + 1..labels.len() {
            let same_job = labels[left] == labels[right];
            let same_posting = postings[left] == postings[right];
            if same_job {
                duplicates += 1;
                if same_posting {
                    merged += 1;
                } else {
                    wrong.push(format!(
                        "missed: cluster {} {:?} {left} vs cluster {} {:?} {right}",
                        labels[left], postings[left], labels[right], postings[right]
                    ));
                }
            } else {
                distinct += 1;
                if same_posting {
                    false_merges += 1;
                    wrong.push(format!(
                        "false merge: cluster {} vs cluster {} {}, as {:?}",
                        labels[left], labels[right], left, postings[left]
                    ));
                }
            }
        }
    }

    Scored {
        sightings: labels.len(),
        clusters: clusters.len(),
        recall: merged as f64 / duplicates as f64,
        false_rate: false_merges as f64 / distinct as f64,
        wrong,
    }
}

#[test]
fn the_golden_set_merges_duplicates_without_false_merges() {
    let scored = score_the_set();

    println!(
        "golden set: {} sightings of {} jobs, recall {:.3}, false merges {:.3}",
        scored.sightings, scored.clusters, scored.recall, scored.false_rate
    );
    for pair in &scored.wrong {
        println!("  {pair}");
    }

    assert!(
        scored.recall >= 0.95,
        "AC-2 asks for at least 95% of duplicate pairs merged, got {:.3}: {:#?}",
        scored.recall,
        scored.wrong
    );
    assert!(
        scored.false_rate < 0.02,
        "AC-2 asks for fewer than 2% false merges, got {:.3}: {:#?}",
        scored.false_rate,
        scored.wrong
    );
}

/// The set has to be worth measuring: every job in it must really be carried by
/// more than one board, or the recall number would be trivially perfect.
#[test]
fn the_fixture_contains_the_duplicates_it_claims_to() {
    let clusters = clusters();

    assert!(clusters.len() >= 10, "a set of jobs, not of one job");
    assert!(
        clusters
            .iter()
            .filter(|sightings| sightings.len() > 1)
            .count()
            >= 10,
        "jobs carried by more than one board"
    );
    assert!(
        clusters.iter().map(Vec::len).sum::<usize>() >= 25,
        "enough sightings to score"
    );
    assert!(
        clusters
            .iter()
            .flatten()
            .all(|sighting| !sighting.description.is_empty()),
        "every sighting has a description to compare"
    );
}

/// What one employer publishes under every posting it makes (F2). Long on
/// purpose: the finding is that a shared boilerplate might carry two different
/// roles over the description threshold.
const EMPLOYER_BOILERPLATE: &str = "About us. Soylent is a company of about four hundred people \
    building software for logistics teams across Europe and Latin America. We have been \
    profitable since 2019, we work in small product teams, and we keep meetings short. Everyone \
    here gets a learning budget, twenty five days of holiday, a remote first contract, private \
    health cover, a yearly bonus reviewed against clear goals, and the equipment they need to \
    do the job well. We hire for the long term, we promote from within, and we publish our \
    salary bands so that nobody has to negotiate in the dark. Our offices in Madrid and Bogota \
    are open to anyone who prefers to work from a desk, and our remote colleagues are full \
    members of every team. We believe good software comes from people who have time to think, \
    so we protect focus time, we write things down, and we review each other's work without \
    ceremony. We interview in three steps, we answer every application within a week, and we \
    give feedback even when the answer is no. We are an equal opportunity employer and we \
    consider every application on its merits, whatever your background, nationality, gender, \
    age or disability.";

const BACKEND: &str = "About the role. We are looking for a Backend Engineer to join our \
    platform team in a remote first company. You will design and build APIs in Rust and \
    PostgreSQL, own services end to end, and work with product managers to ship features that \
    our customers use every day. You will review code, write tests and keep our deployment \
    pipeline fast and boring. About you. You have four years or more building production \
    backends, you are comfortable with SQL, and you care about observability, queues and \
    clean interfaces. You know when to reach for a simpler design. About us. We are a \
    distributed team across Europe and Latin America and we keep meetings short.";

const DATA: &str = "About the role. We are hiring a Data Analyst to help our operations team \
    make decisions with evidence. You will build dashboards in Metabase, write SQL against our \
    warehouse, define the metrics the business runs on, and explain what the numbers mean to \
    people who do not read charts for a living. About you. You have three years of analytics \
    experience, you are fluent in SQL and spreadsheet modelling, and you have told a \
    stakeholder that their favourite metric is misleading. About us. We are a logistics company \
    with a small data team, and we would rather ship a rough answer this week than a perfect \
    one next quarter.";

const FRONTEND: &str = "About the role. We are looking for a Frontend Engineer to build the \
    interfaces our customers live in. You will work in TypeScript and React, own components \
    from design handoff to production, care about accessibility and performance, and push back \
    when a design will not survive contact with a slow connection. About you. You have four \
    years of frontend experience, you know how to profile a rendering problem, and you have \
    opinions about state management that you can defend without being tiresome about it. About \
    us. We are a small product team building a tool for warehouse managers, and we ship every \
    week.";

const MARKETING: &str = "About the role. We are looking for a Growth Marketing Manager to lead \
    paid acquisition across search and social. You will own the media budget, run experiments, \
    report on conversion, and partner with design on campaign creative for our brand. About \
    you. You have five years of performance marketing experience, you read a dashboard before \
    you trust a hunch, you are fluent in attribution, and you know how to brief a copywriter. \
    About us. We are a consumer brand with a small team, we move fast, and we measure \
    everything we spend.";

const DESIGN: &str = "About the role. We are looking for a Product Designer to own the end to \
    end experience of our web app. You will run discovery interviews, sketch flows, build \
    prototypes in Figma, and work with engineers to get the details right in production. About \
    you. You have four years designing software, a portfolio that shows how you got to the \
    answer, and the patience to write the copy when nobody else will. About us. We are a remote \
    team of twelve, we design in the open, and we review each other's work every Thursday.";

const SALES: &str = "About the role. We are looking for a Sales Development Representative to \
    open conversations with operations leaders at mid market companies. You will prospect, \
    qualify inbound leads, book meetings for the account executives, and keep the CRM honest. \
    About you. You have one or two years in outbound sales, you are comfortable on the phone, \
    you take feedback well, and you are curious about how warehouses actually run. About us. We \
    are a growing software company with a clear product and a long runway, and we promote from \
    within.";
