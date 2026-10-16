//! The canonical apply URL (AD-19): one URL per apply page.
//!
//! Two boards link to the same apply page with different tracking noise —
//! `?utm_source=linkedin`, a `#apply` fragment, a capitalised host, a trailing
//! slash. They are one posting. Two apply pages that differ in the ATS job id
//! are two postings, so the query string is kept, minus the trackers.
//!
//! Hand-rolled on purpose: parsing one local URL does not justify a dependency
//! in a crate that has to stay pure and light (AD-1).

/// Query parameters that only say where the click came from.
const TRACKING: &[&str] = &[
    "utm_source",
    "utm_medium",
    "utm_campaign",
    "utm_term",
    "utm_content",
    "utm_id",
    "gclid",
    "fbclid",
    "msclkid",
    "yclid",
    "igshid",
    "mc_cid",
    "mc_eid",
    "ref",
    "ref_src",
    "source",
    "trk",
    "trkcampaign",
    "_hsenc",
    "_hsmi",
    "icid",
    "s_kwcid",
    "vero_id",
];

/// The canonical form of an apply URL, or `""` when there is no URL at all.
pub fn canonical_url(url: &str) -> String {
    let url = url.trim();
    let url = url.split('#').next().unwrap_or_default();
    let Some((scheme, rest)) = url.split_once("://") else {
        // Not a URL we can take apart (mailto:, a bare host, empty): hand it
        // back lowercased rather than inventing a scheme.
        return url.trim().to_lowercase();
    };

    let (authority, path_and_query) = match rest.find('/') {
        Some(slash) => (&rest[..slash], &rest[slash..]),
        None => (rest, ""),
    };
    let (path, query) = match path_and_query.split_once('?') {
        Some((path, query)) => (path, query),
        None => (path_and_query, ""),
    };

    let host = authority.to_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    let host = host
        .strip_suffix(":80")
        .or_else(|| host.strip_suffix(":443"))
        .unwrap_or(host);

    let path = path.trim_end_matches('/');
    let mut params: Vec<&str> = query
        .split('&')
        .filter(|param| !param.is_empty() && !is_tracking(param))
        .collect();
    params.sort_unstable();

    let mut canonical = format!("{}://{host}{path}", scheme.to_lowercase());
    if !params.is_empty() {
        canonical.push('?');
        canonical.push_str(&params.join("&"));
    }
    canonical
}

fn is_tracking(param: &str) -> bool {
    let name = param.split('=').next().unwrap_or_default().to_lowercase();
    TRACKING.contains(&name.as_str())
}
