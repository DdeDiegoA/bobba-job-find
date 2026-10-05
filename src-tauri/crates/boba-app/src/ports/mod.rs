//! Port traits (hexagonal core, AD-1). Adapters implement these; use cases
//! depend only on the traits. Naming: `trait <Thing>Port` (spine Conventions).
//!
//! Empty stubs on purpose (S1.3p): each file gains its trait in the slice that
//! first needs it — append one `pub mod` line here, union-merged (AD-3).

pub mod agent;
pub mod browser;
pub mod clock;
pub mod embedding;
pub mod mail;
pub mod notifier;
pub mod render;
pub mod repo;
pub mod secret;
pub mod source;
