//! Apply Channel detection (S9.2, FR-81): how one Offer can be applied to.
//!
//! Detection is pure: it reads an [`Offer`] and says which channels are open.
//! Email Apply when the posting carries an apply address, ATS Form when its
//! apply URL is a Greenhouse, Lever, Ashby or Workable hosted form, and Manual —
//! the original page — always, because every posting can be applied to by hand.

use crate::posting::Offer;

/// The ATS whose hosted form an Offer points at (FR-81).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AtsProvider {
    Greenhouse,
    Lever,
    Ashby,
    Workable,
}

impl AtsProvider {
    pub const ALL: [AtsProvider; 4] = [
        AtsProvider::Greenhouse,
        AtsProvider::Lever,
        AtsProvider::Ashby,
        AtsProvider::Workable,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            AtsProvider::Greenhouse => "greenhouse",
            AtsProvider::Lever => "lever",
            AtsProvider::Ashby => "ashby",
            AtsProvider::Workable => "workable",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.as_str() == value)
    }

    /// The provider a host belongs to, `None` for anything else (FR-81).
    ///
    /// Matching is on the registrable domain and its subdomains, so a tenant
    /// board (`boards.greenhouse.io`, `jobs.eu.lever.co`) counts and a lookalike
    /// (`greenhouse.io.example.com`) does not.
    pub fn from_host(host: &str) -> Option<Self> {
        let host = host.trim().to_lowercase();
        let host = host.strip_prefix("www.").unwrap_or(&host);
        if host == "greenhouse.io" || host.ends_with(".greenhouse.io") {
            return Some(AtsProvider::Greenhouse);
        }
        if host == "lever.co" || host.ends_with(".lever.co") {
            return Some(AtsProvider::Lever);
        }
        if host == "ashbyhq.com" || host.ends_with(".ashbyhq.com") {
            return Some(AtsProvider::Ashby);
        }
        if host == "workable.com" || host.ends_with(".workable.com") {
            return Some(AtsProvider::Workable);
        }
        None
    }
}

/// One way to apply to an Offer, as detection found it (FR-81).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ApplyChannel {
    /// Write to the posting's apply address.
    EmailApply { address: String },
    /// Fill the employer's hosted ATS form.
    AtsForm { provider: AtsProvider, url: String },
    /// Open the original page and apply by hand.
    Manual { url: String },
}

impl ApplyChannel {
    /// The stored/wire kind (`email_apply`, `ats_form`, `manual`).
    pub fn kind(&self) -> &'static str {
        match self {
            ApplyChannel::EmailApply { .. } => "email_apply",
            ApplyChannel::AtsForm { .. } => "ats_form",
            ApplyChannel::Manual { .. } => "manual",
        }
    }
}

/// The channels one Offer has (FR-81), in the order the UI shows them:
/// Email Apply, ATS Form, Manual last.
pub fn detect(offer: &Offer) -> Vec<ApplyChannel> {
    let mut channels = Vec::new();

    if let Some(address) = offer.apply.email.as_deref().map(str::trim)
        && is_apply_email(address)
    {
        channels.push(ApplyChannel::EmailApply {
            address: address.to_string(),
        });
    }

    // Manual is the original page: only a page we can open is a channel.
    let url = offer.apply.url.trim();
    if let Some(host) = host_of(url) {
        // The ATS Form channel is driven by Boba, so it is https only: an
        // `http` form would be filled and submitted over a connection anyone
        // on the path can rewrite (S9.4 review N1; S4.6 already normalises
        // apply URLs upstream — this is the second lock).
        if is_https(url)
            && let Some(provider) = AtsProvider::from_host(host)
        {
            channels.push(ApplyChannel::AtsForm {
                provider,
                url: url.to_string(),
            });
        }
        channels.push(ApplyChannel::Manual {
            url: url.to_string(),
        });
    }
    channels
}

/// Whether a URL's scheme is `https` (case-insensitive).
fn is_https(url: &str) -> bool {
    url.split_once("://")
        .is_some_and(|(scheme, _)| scheme.trim().eq_ignore_ascii_case("https"))
}

/// The host of an `http(s)` URL, `None` for anything that is not one.
///
/// Hand-rolled for the same reason the canonical URL is (`dedup::url`): one
/// local string does not justify a dependency in a pure crate (AD-1).
fn host_of(url: &str) -> Option<&str> {
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.trim().to_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let authority = rest.split('/').next().unwrap_or_default();
    // Whitelist what may sit in a host:port. A backslash is a path separator
    // to a browser, and a fragment, query or `@` cannot be part of the
    // authority at all, so anything else is a URL the browser would read
    // differently from this check (reviewer-1 F1: it would visit another host
    // than the one matched here).
    if authority.is_empty()
        || !authority
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'))
    {
        return None;
    }
    let host = authority.split(':').next().unwrap_or_default();
    if host.is_empty() { None } else { Some(host) }
}

/// Whether a posting's apply address is one we can write to: a bare address,
/// no spaces, a local part and a dotted domain.
fn is_apply_email(address: &str) -> bool {
    if address.is_empty() || address.chars().any(char::is_whitespace) {
        return false;
    }
    let Some((local, domain)) = address.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::posting::{ApplyHints, Offer, WorkMode};

    fn offer(url: &str, email: Option<&str>) -> Offer {
        Offer {
            id: "pst_01".into(),
            source_id: "src_01".into(),
            external_id: "ext-1".into(),
            title: "Backend Engineer".into(),
            company: "Nimbus".into(),
            locations: vec!["Remote (EU)".into()],
            work_mode: WorkMode::Remote,
            money: None,
            posted_at: None,
            deadline: None,
            first_seen: "2026-10-15T00:00:00Z".into(),
            last_seen: "2026-10-15T00:00:00Z".into(),
            lang: "en".into(),
            description_html: "<p>Hiring.</p>".into(),
            description_text: "Hiring.".into(),
            requirements: vec![],
            apply: ApplyHints {
                url: url.into(),
                email: email.map(str::to_string),
            },
        }
    }

    #[test]
    fn an_apply_address_is_an_email_channel() {
        let channels = detect(&offer(
            "https://nimbus.example/jobs/1",
            Some("jobs@nimbus.example"),
        ));
        assert_eq!(
            channels,
            vec![
                ApplyChannel::EmailApply {
                    address: "jobs@nimbus.example".into()
                },
                ApplyChannel::Manual {
                    url: "https://nimbus.example/jobs/1".into()
                },
            ]
        );
    }

    #[test]
    fn a_malformed_address_is_not_an_email_channel() {
        for bad in [
            "",
            "jobs",
            "jobs@",
            "@nimbus.example",
            "jobs @nimbus.example",
        ] {
            let channels = detect(&offer("https://nimbus.example/jobs/1", Some(bad)));
            assert_eq!(
                channels,
                vec![ApplyChannel::Manual {
                    url: "https://nimbus.example/jobs/1".into()
                }],
                "{bad:?} must not become a channel"
            );
        }
    }

    #[test]
    fn every_hosted_ats_is_an_ats_form_channel() {
        let cases = [
            (
                "https://boards.greenhouse.io/nimbus/jobs/123",
                AtsProvider::Greenhouse,
            ),
            (
                "https://job-boards.eu.greenhouse.io/nimbus/jobs/123",
                AtsProvider::Greenhouse,
            ),
            ("https://jobs.lever.co/nimbus/abc", AtsProvider::Lever),
            ("https://jobs.eu.lever.co/nimbus/abc", AtsProvider::Lever),
            ("https://jobs.ashbyhq.com/nimbus/abc", AtsProvider::Ashby),
            (
                "https://apply.workable.com/nimbus/j/ABC",
                AtsProvider::Workable,
            ),
        ];
        for (url, provider) in cases {
            let channels = detect(&offer(url, None));
            assert_eq!(
                channels,
                vec![
                    ApplyChannel::AtsForm {
                        provider,
                        url: url.into()
                    },
                    ApplyChannel::Manual { url: url.into() },
                ],
                "{url}"
            );
        }
    }

    #[test]
    fn host_matching_ignores_case_www_and_a_port() {
        let channels = detect(&offer(
            "HTTPS://WWW.Boards.Greenhouse.io:443/nimbus/jobs/1",
            None,
        ));
        assert_eq!(
            channels[0],
            ApplyChannel::AtsForm {
                provider: AtsProvider::Greenhouse,
                url: "HTTPS://WWW.Boards.Greenhouse.io:443/nimbus/jobs/1".into()
            }
        );
    }

    #[test]
    fn a_plain_http_form_is_not_an_ats_channel() {
        // The channel is filled and submitted by Boba, so it must be https
        // (S9.4 review N1); the page can still be opened by hand.
        for url in [
            "http://boards.greenhouse.io/nimbus/jobs/123",
            "HTTP://jobs.lever.co/nimbus/abc",
        ] {
            assert_eq!(
                detect(&offer(url, None)),
                vec![ApplyChannel::Manual { url: url.into() }],
                "{url}"
            );
        }
    }

    #[test]
    fn a_lookalike_or_unknown_host_is_manual_only() {
        for url in [
            "https://greenhouse.io.example.com/jobs/1",
            "https://nimbus.example/careers/1",
        ] {
            let channels = detect(&offer(url, None));
            assert_eq!(
                channels,
                vec![ApplyChannel::Manual { url: url.into() }],
                "{url}"
            );
        }
    }

    #[test]
    fn a_string_that_is_not_an_http_url_has_no_manual_channel() {
        for url in [
            "not a url",
            "",
            "mailto:jobs@nimbus.example",
            "ftp://boards.greenhouse.io/x",
        ] {
            assert_eq!(
                detect(&offer(url, None)),
                vec![],
                "{url:?} is not a page the bar could open"
            );
        }
    }

    #[test]
    fn an_authority_a_browser_reads_differently_is_not_an_ats_form() {
        // A backslash and a fragment are path/URL separators to a browser, so
        // the host it visits is not the one this check would match (reviewer-1
        // F1): every one of these opens an attacker's page, not an ATS form.
        for url in [
            "https://evil.com\\@boards.greenhouse.io/x",
            "https://evil.com#@boards.greenhouse.io/x",
            "https://evil.com?@boards.greenhouse.io/x",
            "https://user:pass@boards.greenhouse.io/x",
            "https://boards .greenhouse.io/x",
            "https://boards.greenhouse.io\u{1}/x",
        ] {
            assert_eq!(detect(&offer(url, None)), vec![], "{url:?}");
        }
    }

    #[test]
    fn email_and_ats_form_are_both_listed_before_manual() {
        let channels = detect(&offer(
            "https://jobs.lever.co/nimbus/abc",
            Some("jobs@nimbus.example"),
        ));
        assert_eq!(
            channels.iter().map(ApplyChannel::kind).collect::<Vec<_>>(),
            vec!["email_apply", "ats_form", "manual"]
        );
    }

    #[test]
    fn provider_names_round_trip() {
        for provider in AtsProvider::ALL {
            assert_eq!(AtsProvider::parse(provider.as_str()), Some(provider));
        }
        assert_eq!(AtsProvider::parse("smartrecruiters"), None);
    }
}
