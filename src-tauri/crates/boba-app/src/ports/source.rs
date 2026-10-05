//! `SourcePort` — discover and fetch postings from the configured sources
//! (AD-9, AD-10). Adapter: `boba-sources`.
//!
//! The port speaks domain types only (spine: every type that crosses a port
//! lives in `boba-domain`), so an adapter's own types never leak upward.

use std::future::Future;
use std::pin::Pin;

use boba_domain::source::{RawPosting, SearchQuery, SourceError, SourceFetch, SourceManifest};

/// A boxed future so the trait stays object-safe (`dyn SourcePort`) while its
/// methods are async; Rust 2024 async-fn-in-trait is not dyn-safe on its own.
pub type SourceFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The app-facing view of the source subsystem.
pub trait SourcePort: Send + Sync {
    /// Manifests of every registered source (connections UI, apply policy).
    fn manifests(&self) -> Vec<SourceManifest>;

    /// Search every runnable (enabled + opted-in) source. A source that fails
    /// is returned as a [`SourceFetch::Err`] item and never aborts the others.
    fn search<'a>(&'a self, query: &'a SearchQuery) -> SourceFuture<'a, Vec<SourceFetch>>;

    /// Fetch one posting by its source-local id.
    fn detail<'a>(
        &'a self,
        source_id: &'a str,
        external_id: &'a str,
    ) -> SourceFuture<'a, Result<RawPosting, SourceError>>;
}
