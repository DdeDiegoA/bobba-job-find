//! `boba-embed` — the on-device F3 sentence encoder (AD-8, Story 6.9).
//!
//! This is the adapter the hexagonal rule sends the ONNX runtime to: the core
//! (`boba-app`) names only `EmbeddingPort`, and nothing there depends on
//! `fastembed` (AC3). The crate is empty without its `embeddings` feature, so a
//! default build mounts no port and F3 drops out of scoring unchanged (AC4).

#[cfg(feature = "embeddings")]
mod encoder;

#[cfg(feature = "embeddings")]
pub use encoder::FastembedEncoder;
