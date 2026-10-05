//! Source registry (AD-9, S4.1 AC1/AC3).
//!
//! Holds the registered adapters and the set of sources the user has opted into.
//! It refuses to run gated classes (`html_scrape`, `session_browser`,
//! `external_process`) without a recorded opt-in, and it isolates failures: a
//! source that errors is reported as one item and never aborts the rest.

use std::collections::BTreeSet;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use boba_app::ports::source::{SourceFuture, SourcePort};
use boba_domain::source::{RawPosting, SearchQuery, SourceError, SourceFetch, SourceManifest};

use crate::http::HttpClient;
use crate::sdk::{SourceAdapter, SourceCtx};

/// The set of sources available to a run.
pub struct SourceRegistry {
    http: Arc<HttpClient>,
    adapters: Vec<Box<dyn SourceAdapter>>,
    opted_in: BTreeSet<String>,
}

impl SourceRegistry {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self {
            http,
            adapters: Vec::new(),
            opted_in: BTreeSet::new(),
        }
    }

    /// Replace the recorded opt-ins; the app reloads them from `SourcesRepo`.
    pub fn set_opted_in(&mut self, ids: impl IntoIterator<Item = String>) {
        self.opted_in = ids.into_iter().collect();
    }

    /// Register one adapter. Ids are unique.
    pub fn register(&mut self, adapter: Box<dyn SourceAdapter>) -> Result<(), SourceError> {
        let id = adapter.manifest().id;
        if self
            .adapters
            .iter()
            .any(|existing| existing.manifest().id == id)
        {
            return Err(SourceError::DuplicateSource { source_id: id });
        }
        self.adapters.push(adapter);
        Ok(())
    }

    /// Adapters that may run: every ungated source, plus gated ones with an opt-in.
    pub fn runnable(&self) -> Vec<&dyn SourceAdapter> {
        self.adapters
            .iter()
            .map(AsRef::as_ref)
            .filter(|adapter| self.is_runnable(&adapter.manifest()))
            .collect()
    }

    /// The opt-in gate for one adapter (AC1).
    pub fn check_runnable(&self, adapter: &dyn SourceAdapter) -> Result<(), SourceError> {
        let manifest = adapter.manifest();
        if self.is_runnable(&manifest) {
            Ok(())
        } else {
            Err(SourceError::OptInRequired {
                source_id: manifest.id,
            })
        }
    }

    fn is_runnable(&self, manifest: &SourceManifest) -> bool {
        !manifest.requires_opt_in() || self.opted_in.contains(&manifest.id)
    }
}

impl SourcePort for SourceRegistry {
    fn manifests(&self) -> Vec<SourceManifest> {
        self.adapters.iter().map(|a| a.manifest()).collect()
    }

    fn search<'a>(&'a self, query: &'a SearchQuery) -> SourceFuture<'a, Vec<SourceFetch>> {
        Box::pin(async move {
            let mut results = Vec::new();
            for adapter in self.runnable() {
                let manifest = adapter.manifest();
                let ctx = SourceCtx {
                    http: self.http.as_ref(),
                    cancel: CancellationToken::new(),
                };
                // A source that fails is one item; it never aborts the run (AC3).
                match adapter.search(query, &ctx).await {
                    Ok(postings) => results.push(SourceFetch::Ok {
                        source_id: manifest.id,
                        postings,
                    }),
                    Err(error) => results.push(SourceFetch::Err {
                        source_id: manifest.id,
                        error,
                    }),
                }
            }
            results
        })
    }

    fn detail<'a>(
        &'a self,
        source_id: &'a str,
        external_id: &'a str,
    ) -> SourceFuture<'a, Result<RawPosting, SourceError>> {
        Box::pin(async move {
            let adapter = self
                .adapters
                .iter()
                .find(|adapter| adapter.manifest().id == source_id)
                .ok_or_else(|| SourceError::UnknownSource {
                    source_id: source_id.to_string(),
                })?;
            self.check_runnable(adapter.as_ref())?;
            let ctx = SourceCtx {
                http: self.http.as_ref(),
                cancel: CancellationToken::new(),
            };
            adapter.detail(external_id, &ctx).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use boba_domain::source::{AccessClass, ApplyCapability, LegalTier, RateLimits};

    struct FakeAdapter {
        manifest: SourceManifest,
        result: Result<Vec<RawPosting>, SourceError>,
    }

    impl SourceAdapter for FakeAdapter {
        fn manifest(&self) -> SourceManifest {
            self.manifest.clone()
        }

        fn search<'a>(
            &'a self,
            _query: &'a SearchQuery,
            _ctx: &'a SourceCtx<'_>,
        ) -> crate::sdk::BoxFuture<'a, Result<Vec<RawPosting>, SourceError>> {
            let result = self.result.clone();
            Box::pin(async move { result })
        }

        fn detail<'a>(
            &'a self,
            external_id: &'a str,
            _ctx: &'a SourceCtx<'_>,
        ) -> crate::sdk::BoxFuture<'a, Result<RawPosting, SourceError>> {
            let id = external_id.to_string();
            Box::pin(async move { Err(SourceError::UnknownSource { source_id: id }) })
        }
    }

    fn manifest(id: &str, access_class: AccessClass) -> SourceManifest {
        SourceManifest {
            id: id.into(),
            name: id.into(),
            access_class,
            legal_tier: LegalTier::Safe,
            markets: vec![],
            rate_limits: RateLimits::default(),
            attribution: None,
            apply_capability: ApplyCapability::LinkOut,
            required_secrets: vec![],
            config_schema: None,
        }
    }

    fn posting(id: &str) -> RawPosting {
        RawPosting {
            external_id: id.into(),
            title: "Backend Engineer".into(),
            company: "Boba".into(),
            url: format!("https://example.test/{id}"),
            locations: vec!["Remote".into()],
            remote: Some(true),
            description_html: None,
            posted_at: None,
            salary_text: None,
            raw: "{}".into(),
        }
    }

    fn registry() -> SourceRegistry {
        SourceRegistry::new(Arc::new(HttpClient::honest().expect("http client")))
    }

    #[tokio::test]
    async fn gated_source_without_opt_in_is_refused() {
        let mut registry = registry();
        registry
            .register(Box::new(FakeAdapter {
                manifest: manifest("compu", AccessClass::HtmlScrape),
                result: Ok(vec![posting("1")]),
            }))
            .unwrap();

        assert!(registry.runnable().is_empty());

        let results = registry.search(&SearchQuery::default()).await;
        assert!(results.is_empty(), "a refused source must not run");

        let adapter = &registry.adapters[0];
        let error = registry
            .check_runnable(adapter.as_ref())
            .expect_err("gated");
        assert!(matches!(error, SourceError::OptInRequired { .. }));
    }

    #[tokio::test]
    async fn public_source_runs_without_opt_in_and_gated_with_one() {
        let mut registry = registry();
        registry
            .register(Box::new(FakeAdapter {
                manifest: manifest("remotive", AccessClass::PublicJson),
                result: Ok(vec![posting("1")]),
            }))
            .unwrap();
        registry
            .register(Box::new(FakeAdapter {
                manifest: manifest("linkedin", AccessClass::SessionBrowser),
                result: Ok(vec![posting("2")]),
            }))
            .unwrap();

        assert_eq!(registry.runnable().len(), 1, "session source is gated");

        registry.set_opted_in(["linkedin".to_string()]);
        assert_eq!(registry.runnable().len(), 2);

        let results = registry.search(&SearchQuery::default()).await;
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let mut registry = registry();
        let make = || {
            Box::new(FakeAdapter {
                manifest: manifest("dup", AccessClass::PublicJson),
                result: Ok(vec![]),
            }) as Box<dyn SourceAdapter>
        };
        registry.register(make()).unwrap();
        let error = registry.register(make()).expect_err("duplicate");
        assert!(matches!(error, SourceError::DuplicateSource { .. }));
    }

    #[tokio::test]
    async fn a_failing_adapter_does_not_abort_the_rest() {
        let mut registry = registry();
        registry
            .register(Box::new(FakeAdapter {
                manifest: manifest("broken", AccessClass::PublicJson),
                result: Err(SourceError::InvalidPayload {
                    message: "boom".into(),
                }),
            }))
            .unwrap();
        registry
            .register(Box::new(FakeAdapter {
                manifest: manifest("healthy", AccessClass::PublicJson),
                result: Ok(vec![posting("1")]),
            }))
            .unwrap();

        let results = registry.search(&SearchQuery::default()).await;
        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|r| matches!(
            r,
            SourceFetch::Err { source_id, .. } if source_id == "broken"
        )));
        assert!(results.iter().any(|r| matches!(
            r,
            SourceFetch::Ok { source_id, postings } if source_id == "healthy" && postings.len() == 1
        )));
    }

    #[tokio::test]
    async fn detail_rejects_an_unknown_source() {
        let registry = registry();
        let error = registry
            .detail("nope", "1")
            .await
            .expect_err("unknown source");
        assert!(matches!(error, SourceError::UnknownSource { .. }));
    }
}
