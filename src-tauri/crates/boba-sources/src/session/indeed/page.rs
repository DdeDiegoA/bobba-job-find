//! The Indeed page side (S11.1b, AD-7, AD-10).
//!
//! [`READ_SEARCH_JS`] and [`READ_DETAIL_JS`] are evaluated by the session
//! driver in the page the user is signed into. They are read-only by
//! construction: no clicks, no typing, no submission, no network, no cookie
//! access — the test suite asserts it. Their JSON is decoded here, so every
//! reading rule is unit-tested without a browser (the same split as
//! `boba-browser/src/forms`).

use boba_domain::source::RawPosting;
use serde::{Deserialize, Serialize};

use super::{BoardError, detail_url, valid_key};
use crate::ats::json::https_url;
use crate::session::sdk::{ChallengeKind, classify_challenge};

/// Evaluate in an Indeed search page: read the job cards and the page signals,
/// return JSON. Never acts on the page.
pub const READ_SEARCH_JS: &str = r#"(() => {
  const clean = (value) => (value || '').replace(/\s+/g, ' ').trim();
  const textOf = (el) => clean(el && (el.innerText || el.textContent));
  const body = document.body ? (document.body.innerText || '') : '';
  const cards = [];
  document.querySelectorAll('.job_seen_beacon, div[data-jk]').forEach((el) => {
    const link = el.querySelector('h2.jobTitle a, a[href*="/viewjob"], a[href*="jk="]');
    const href = link ? link.href : '';
    const fromUrl = (href.match(/[?&]jk=([A-Za-z0-9_-]+)/) || [])[1] || '';
    const jk = el.getAttribute('data-jk') || fromUrl;
    if (!jk) return;
    cards.push({
      jk,
      title: textOf(el.querySelector('h2.jobTitle, [data-testid="jobTitle"]')),
      company: textOf(el.querySelector('[data-testid="company-name"]')),
      location: textOf(el.querySelector('[data-testid="text-location"]')),
      url: href || (location.origin + '/viewjob?jk=' + jk),
      snippet: textOf(el.querySelector('.job-snippet, [data-testid="belowJobSnippet"]')),
      salary: textOf(el.querySelector('.salary-snippet-container, [data-testid="attribute_snippet_testid"]'))
    });
  });
  return JSON.stringify({
    url: location.href,
    title: document.title || '',
    text: body.slice(0, 12000),
    captcha: !!document.querySelector('[id*="captcha" i], iframe[src*="captcha" i], .cf-turnstile, #challenge-form'),
    loginWall: /^\/(login|account|auth)/.test(location.pathname) || !!document.querySelector('input[type="password"]'),
    unusualActivity: /unusual activity|temporarily restricted|suspicious activity/i.test(body.slice(0, 4000)),
    rateLimited: /too many requests|rate limit|rate-limit/i.test(body.slice(0, 4000)),
    cards
  });
})()"#;

/// Evaluate in an Indeed job page: read the posting and the page signals,
/// return JSON. Never acts on the page.
pub const READ_DETAIL_JS: &str = r#"(() => {
  const clean = (value) => (value || '').replace(/\s+/g, ' ').trim();
  const textOf = (el) => clean(el && (el.innerText || el.textContent));
  const body = document.body ? (document.body.innerText || '') : '';
  const jk = (location.search.match(/[?&]jk=([A-Za-z0-9_-]+)/) || [])[1] || '';
  const markup = document.querySelector('#jobDescriptionText, .jobsearch-jobDescriptionText');
  return JSON.stringify({
    url: location.href,
    title: document.title || '',
    text: body.slice(0, 12000),
    captcha: !!document.querySelector('[id*="captcha" i], iframe[src*="captcha" i], .cf-turnstile, #challenge-form'),
    loginWall: /^\/(login|account|auth)/.test(location.pathname) || !!document.querySelector('input[type="password"]'),
    unusualActivity: /unusual activity|temporarily restricted|suspicious activity/i.test(body.slice(0, 4000)),
    rateLimited: /too many requests|rate limit|rate-limit/i.test(body.slice(0, 4000)),
    job: jk ? {
      jk,
      title: textOf(document.querySelector('h1.jobsearch-JobInfoHeader-title, [data-testid="jobsearch-JobInfoHeader-title"]')),
      company: textOf(document.querySelector('[data-testid="inlineHeader-companyName"], .jobsearch-CompanyInfoContainer a')),
      location: textOf(document.querySelector('[data-testid="inlineHeader-companyLocation"]')),
      salary: textOf(document.querySelector('#salaryInfoAndJobType, .salary-snippet-container')),
      descriptionHtml: markup ? markup.innerHTML : ''
    } : null
  });
})()"#;

/// The page as the read scripts described it.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Page {
    #[serde(default)]
    title: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    captcha: bool,
    #[serde(default)]
    login_wall: bool,
    #[serde(default)]
    unusual_activity: bool,
    #[serde(default)]
    rate_limited: bool,
    #[serde(default)]
    cards: Vec<Card>,
    /// The posting a detail page described (the list template carries none).
    #[serde(default)]
    job: Option<Card>,
}

/// One job card, or the posting a detail page described.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Card {
    #[serde(default)]
    jk: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    company: String,
    #[serde(default)]
    location: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    snippet: String,
    #[serde(default)]
    salary: String,
    #[serde(default)]
    description_html: String,
}

/// Read a search page into postings; a challenge page yields none.
pub fn parse_search(json: &str) -> Result<Vec<RawPosting>, BoardError> {
    let page = decode(json)?;
    check_challenge(&page)?;
    Ok(page.cards.iter().filter_map(posting).collect())
}

/// Read a job page into one posting; a challenge page yields none.
pub fn parse_detail(json: &str) -> Result<RawPosting, BoardError> {
    let page = decode(json)?;
    check_challenge(&page)?;
    let job = page
        .job
        .as_ref()
        .ok_or_else(|| BoardError::Malformed("the page carries no job".into()))?;
    detail_posting(job)
}

/// Refuse a challenge page (AD-10): Boba never retries around a wall.
pub(crate) fn check_challenge(page: &Page) -> Result<(), BoardError> {
    match challenge(page) {
        Some(kind) => Err(BoardError::Challenge(kind)),
        None => Ok(()),
    }
}

/// The most severe challenge the page showed, or `None` for an ordinary page.
///
/// The script's own flags come first (a password input, a CAPTCHA widget, an
/// "unusual activity" banner, a rate limit). Only a page that produced no
/// payload is read for challenge words: a signed-in board's chrome keeps a
/// "Sign in" link and salaries contain "429", and neither is a wall (S11.1a R5).
fn challenge(page: &Page) -> Option<ChallengeKind> {
    if page.rate_limited {
        return Some(ChallengeKind::RateLimited);
    }
    if page.unusual_activity {
        return Some(ChallengeKind::UnusualActivity);
    }
    if page.captcha {
        return Some(ChallengeKind::Captcha);
    }
    if page.login_wall {
        return Some(ChallengeKind::LoginWall);
    }
    if !page.cards.is_empty() || page.job.is_some() {
        return None;
    }
    classify_challenge(&format!("{}\n{}", page.title, page.text))
}

fn decode(json: &str) -> Result<Page, BoardError> {
    serde_json::from_str(json).map_err(|error| BoardError::Malformed(error.to_string()))
}

/// One card as a posting; an unclear job key, an empty title or an unsafe URL
/// drops that card alone (AD-7, AD-19).
pub(crate) fn posting(card: &Card) -> Option<RawPosting> {
    let jk = card.jk.trim();
    if !valid_key(jk) {
        return None;
    }
    let title = card.title.trim();
    if title.is_empty() {
        return None;
    }
    let url = https_url(Some(card.url.trim().to_string()))?;
    Some(RawPosting {
        external_id: jk.to_string(),
        title: title.to_string(),
        company: card.company.trim().to_string(),
        url,
        locations: locations(&card.location),
        remote: remote(&card.location),
        // The result list carries no description; the detail page does.
        description_html: None,
        // The result list shows a relative age ("3 days ago"), not a timestamp.
        posted_at: None,
        salary_text: non_empty(&card.salary),
        raw: serde_json::to_string(card).ok()?,
    })
}

/// One job page as a posting; the canonical URL is rebuilt from the job key, so
/// a tracking parameter on the page never becomes the Offer's link.
fn detail_posting(job: &Card) -> Result<RawPosting, BoardError> {
    let jk = job.jk.trim();
    let title = job.title.trim();
    if title.is_empty() {
        return Err(BoardError::Malformed("the job has no title".into()));
    }
    let raw =
        serde_json::to_string(job).map_err(|error| BoardError::Malformed(error.to_string()))?;
    Ok(RawPosting {
        external_id: jk.to_string(),
        title: title.to_string(),
        company: job.company.trim().to_string(),
        url: detail_url(jk)?,
        locations: locations(&job.location),
        remote: remote(&job.location),
        description_html: non_empty(&job.description_html),
        // Indeed shows a relative age ("3 days ago"), which is not a timestamp.
        posted_at: None,
        salary_text: non_empty(&job.salary),
        raw,
    })
}

fn locations(location: &str) -> Vec<String> {
    let location = location.trim();
    if location.is_empty() {
        Vec::new()
    } else {
        vec![location.to_string()]
    }
}

/// A location that says "remote" is remote; an office one is not claimed.
fn remote(location: &str) -> Option<bool> {
    location.to_lowercase().contains("remote").then_some(true)
}

fn non_empty(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}
