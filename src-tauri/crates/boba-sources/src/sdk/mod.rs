//! Source Adapter SDK (AD-9, architecture §6.1).
//!
//! Every source implements [`SourceAdapter`] and returns [`RawPosting`] only.
//! Adapters never own their HTTP stack or opt-in logic: requests go through
//! [`crate::http::HttpClient`] and the gate lives in [`crate::registry`].

use std::future::Future;
use std::pin::Pin;

use tokio_util::sync::CancellationToken;

use crate::http::HttpClient;

pub use boba_domain::source::{
    AccessClass, ApplyCapability, LegalTier, RateLimits, RawPosting, SearchQuery, SourceError,
    SourceManifest,
};

/// A boxed future, so [`SourceAdapter`] stays object-safe (`Box<dyn SourceAdapter>`).
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Runtime context handed to an adapter for one call (architecture §6.1).
pub struct SourceCtx<'a> {
    pub http: &'a HttpClient,
    pub cancel: CancellationToken,
}

/// The contract every source adapter implements (AD-9).
pub trait SourceAdapter: Send + Sync {
    fn manifest(&self) -> SourceManifest;

    fn search<'a>(
        &'a self,
        query: &'a SearchQuery,
        ctx: &'a SourceCtx<'_>,
    ) -> BoxFuture<'a, Result<Vec<RawPosting>, SourceError>>;

    fn detail<'a>(
        &'a self,
        external_id: &'a str,
        ctx: &'a SourceCtx<'_>,
    ) -> BoxFuture<'a, Result<RawPosting, SourceError>>;
}
