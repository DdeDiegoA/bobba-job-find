//! `EmbeddingPort` — optional on-device F3 semantic score (AD-8).
//! Owned by S6.9; adapter: `boba-embed` (cargo feature `embeddings`).
//!
//! The port is the only way the core reaches the encoder: `boba-app` never
//! depends on `fastembed` and never sees an ONNX runtime (AC3, AD-1). A build
//! without the feature simply has no adapter for this port, which is what
//! makes F3 unavailable without changing scoring (AC4).

use std::path::PathBuf;

use boba_domain::scoring::f3::ModelInstall;
use boba_domain::scoring::factors::SemanticScore;
use thiserror::Error;

/// Why the on-device encoder could not answer. The port carries its own error
/// so `boba-app` stays free of the adapter's types.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EmbeddingError {
    /// The model is not on this device and the caller did not opt in to
    /// fetching it (AC1: the download is the user's choice).
    #[error("the embeddings model is not installed: {0}")]
    NotInstalled(String),
    /// Fetching or loading the model failed.
    #[error("the embeddings model could not be prepared: {0}")]
    Download(String),
    /// The encoder ran but could not produce a vector.
    #[error("the on-device encoder failed: {0}")]
    Encode(String),
}

/// The optional on-device encoder that supplies F3 (FR-55, AD-8).
///
/// Everything it is asked to do is local: no network at score time, and the
/// one download is the explicit opt-in [`EmbeddingPort::install`] (AC1).
pub trait EmbeddingPort: Send + Sync {
    /// The model this adapter runs, e.g. `intfloat/multilingual-e5-small`.
    fn model_id(&self) -> &str;

    /// The directory the model is read from — and downloaded into. This is the
    /// destination the network-activity view lists (AC1).
    fn destination(&self) -> PathBuf;

    /// Whether the model files are already on disk. Never downloads.
    fn installed(&self) -> bool;

    /// Download the model when it is missing, and return where it landed
    /// (AC1). Idempotent: an installed model is loaded, not fetched again.
    fn install(&self) -> Result<ModelInstall, EmbeddingError>;

    /// The cosine similarity of the two prepared texts, on device.
    ///
    /// The texts are already encoded for E5 ([`boba_domain::scoring::f3`]),
    /// and the answer is already the 0–1 similarity F3 scores.
    fn similarity(
        &self,
        profile_text: &str,
        offer_text: &str,
    ) -> Result<SemanticScore, EmbeddingError>;
}
