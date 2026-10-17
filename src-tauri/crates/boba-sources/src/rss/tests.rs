//! RSS/Atom tests (S4.8 AC2). Fixtures are inline: the module owns no files
//! outside `rss/`, and a feed document is small enough to read here.

use std::time::Duration;

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::http::{HttpClient, HttpClientConfig};

const RSS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>Boba Jobs</title>
    <link>https://jobs.example.com</link>
    <item>
      <title>Senior Rust Engineer</title>
      <link>https://jobs.example.com/rust-engineer</link>
      <guid>https://jobs.example.com/rust-engineer</guid>
      <description><![CDATA[Build <b>payments</b> infrastructure.]]></description>
      <pubDate>Mon, 12 Oct 2026 08:00:00 GMT</pubDate>
    </item>
    <item>
      <title>Product Designer</title>
      <link>https://jobs.example.com/product-designer</link>
      <guid>product-designer</guid>
      <description>Design the boba experience.</description>
    </item>
  </channel>
</rss>"#;

const ATOM: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Remote Board</title>
  <id>https://remote.example.com/</id>
  <updated>2026-10-12T08:00:00Z</updated>
  <entry>
    <title>Backend Engineer</title>
    <id>urn:uuid:1</id>
    <link href="https://remote.example.com/backend" rel="alternate"/>
    <content type="html">&lt;p&gt;Go and Rust.&lt;/p&gt;</content>
    <updated>2026-10-12T08:00:00Z</updated>
    <author><name>Boba Labs</name></author>
  </entry>
</feed>"#;

fn client() -> HttpClient {
    HttpClient::new(HttpClientConfig {
        min_interval: Duration::from_millis(0),
        retry_base_delay: Duration::from_millis(1),
        ..HttpClientConfig::default()
    })
    .expect("client")
}

async fn serve(server: &MockServer, body: &'static str) {
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/rss+xml"))
        .mount(server)
        .await;
}

fn feed(base: &str) -> CustomFeed {
    CustomFeed::test_with("rss:test", "Test feed", format!("{base}/feed.xml"))
}

fn ctx(http: &HttpClient) -> SourceCtx<'_> {
    SourceCtx {
        http,
        cancel: tokio_util::sync::CancellationToken::new(),
    }
}

#[test]
fn only_public_addresses_are_allowed_at_connect_time() {
    for private in [
        "127.0.0.1",
        "127.9.9.9",
        "10.0.0.1",
        "172.16.0.1",
        "172.31.255.255",
        "192.168.1.10",
        "169.254.169.254",
        "0.0.0.0",
        "255.255.255.255",
        "224.0.0.1",
        "100.64.0.1",
        "192.0.2.5",
        "::1",
        "::",
        "fe80::1",
        "fc00::1",
        "fd12:3456::1",
        "::ffff:127.0.0.1",
        "::ffff:10.0.0.1",
    ] {
        let ip: IpAddr = private.parse().unwrap();
        assert!(!public_address(ip), "{private} must be refused");
    }
    for public in [
        "8.8.8.8",
        "1.1.1.1",
        "93.184.216.34",
        "2606:4700:4700::1111",
        "::ffff:8.8.8.8",
    ] {
        let ip: IpAddr = public.parse().unwrap();
        assert!(public_address(ip), "{public} must be allowed");
    }
}

#[test]
fn a_host_that_resolves_inside_the_network_is_refused() {
    let loopback: IpAddr = "127.0.0.1".parse().unwrap();
    assert!(check_resolved(&[loopback]).is_err(), "loopback resolution");
    let private: IpAddr = "192.168.1.5".parse().unwrap();
    assert!(check_resolved(&[private]).is_err(), "private resolution");
    // One bad answer poisons the set: DNS may return several.
    let public: IpAddr = "93.184.216.34".parse().unwrap();
    assert!(
        check_resolved(&[public, loopback]).is_err(),
        "a mixed answer is refused"
    );
    // No answer at all is a failed fetch, never an allow.
    assert!(check_resolved(&[]).is_err(), "unresolved is refused");
    assert!(check_resolved(&[public]).is_ok());
}

#[test]
fn only_https_urls_with_a_host_are_accepted() {
    assert_eq!(
        validate_url(" https://jobs.example.com/feed.xml ").unwrap(),
        "https://jobs.example.com/feed.xml"
    );
    for bad in [
        "http://jobs.example.com/feed.xml",
        "javascript:alert(1)",
        "",
        "   ",
        "https://",
        "https:///feed.xml",
        "https://user:secret@jobs.example.com/feed.xml",
        "not a url",
    ] {
        assert!(validate_url(bad).is_err(), "{bad:?} must be refused");
    }
}

#[test]
fn internal_hosts_are_refused_as_an_ssrf_guard() {
    for bad in [
        "https://localhost/feed.xml",
        "https://localhost:8443/feed.xml",
        "https://boba.localhost/feed.xml",
        "https://127.0.0.1/feed.xml",
        "https://127.9.9.9/feed.xml",
        "https://10.0.0.5/feed.xml",
        "https://172.16.4.2/feed.xml",
        "https://192.168.1.10/feed.xml",
        "https://169.254.169.254/latest/meta-data",
        "https://0.0.0.0/feed.xml",
        "https://[::1]/feed.xml",
        "https://intranet/feed.xml",
    ] {
        assert!(validate_url(bad).is_err(), "{bad:?} must be refused");
    }
    // A public host with a domain and a port stays valid.
    assert!(validate_url("https://jobs.example.com:8443/feed.xml").is_ok());
}

#[tokio::test]
async fn test_fetch_reports_the_count_and_a_sample() {
    let server = MockServer::start().await;
    serve(&server, RSS).await;
    let http = client();

    let probe = test_fetch(&feed(&server.uri()), &http).await.unwrap();
    assert_eq!(probe.count, 2);
    assert_eq!(probe.sample.len(), 2);

    let first = &probe.sample[0];
    assert_eq!(first.title, "Senior Rust Engineer");
    assert_eq!(first.company, "Boba Jobs");
    assert_eq!(first.url, "https://jobs.example.com/rust-engineer");
    assert_eq!(
        first.posted_at.as_deref(),
        Some("2026-10-12T08:00:00+00:00")
    );
    assert!(
        first
            .description_html
            .as_deref()
            .unwrap()
            .contains("Build <b>payments</b>")
    );

    // A guid that is not a URL falls back to the item's link.
    assert_eq!(
        probe.sample[1].url,
        "https://jobs.example.com/product-designer"
    );
}

#[tokio::test]
async fn atom_feeds_map_the_same_way() {
    let server = MockServer::start().await;
    serve(&server, ATOM).await;
    let http = client();

    let probe = test_fetch(&feed(&server.uri()), &http).await.unwrap();
    assert_eq!(probe.count, 1);
    let entry = &probe.sample[0];
    assert_eq!(entry.title, "Backend Engineer");
    assert_eq!(entry.company, "Boba Labs");
    assert_eq!(entry.url, "https://remote.example.com/backend");
    assert!(
        entry
            .description_html
            .as_deref()
            .unwrap()
            .contains("<p>Go and Rust.</p>")
    );
}

#[tokio::test]
async fn a_payload_that_is_not_a_feed_is_reported() {
    let server = MockServer::start().await;
    serve(&server, "<html>not a feed</html>").await;
    let http = client();

    let error = test_fetch(&feed(&server.uri()), &http)
        .await
        .expect_err("not a feed");
    assert!(
        matches!(error, SourceError::InvalidPayload { .. }),
        "{error:?}"
    );
}

#[tokio::test]
async fn the_adapter_manifest_is_a_safe_link_out_rss_source() {
    let feed =
        CustomFeed::new("rss:test", "Test feed", "https://jobs.example.com/feed.xml").unwrap();
    let manifest = feed.manifest();
    assert_eq!(manifest.id, "rss:test");
    assert_eq!(manifest.name, "Test feed");
    assert_eq!(manifest.access_class, AccessClass::Rss);
    assert_eq!(manifest.legal_tier, LegalTier::Safe);
    assert_eq!(manifest.apply_capability, ApplyCapability::LinkOut);
    assert!(!manifest.requires_opt_in());
}

#[tokio::test]
async fn search_keeps_only_matching_entries() {
    let server = MockServer::start().await;
    serve(&server, RSS).await;
    let http = client();
    let feed = feed(&server.uri());

    let all = feed
        .search(&SearchQuery::default(), &ctx(&http))
        .await
        .unwrap();
    assert_eq!(all.len(), 2);

    let query = SearchQuery {
        roles: vec!["designer".into()],
        ..SearchQuery::default()
    };
    let matching = feed.search(&query, &ctx(&http)).await.unwrap();
    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0].title, "Product Designer");
}

#[tokio::test]
async fn detail_finds_one_item_and_reports_a_miss() {
    let server = MockServer::start().await;
    serve(&server, RSS).await;
    let http = client();
    let feed = feed(&server.uri());

    let posting = feed.detail("product-designer", &ctx(&http)).await.unwrap();
    assert_eq!(posting.title, "Product Designer");

    let error = feed.detail("missing", &ctx(&http)).await.expect_err("miss");
    assert!(
        matches!(error, SourceError::InvalidPayload { .. }),
        "{error:?}"
    );
}
