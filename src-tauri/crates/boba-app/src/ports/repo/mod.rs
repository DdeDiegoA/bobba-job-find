//! Repository ports — one trait per AD-16 owner area. Each owns its tables and
//! is the only writer; other modules read through the owner's queries.
//!
//! Empty stubs on purpose (S1.3p); each gains its trait in the slice that first
//! writes that area. Append one `pub mod` line here, union-merged (AD-3).

pub mod agent;
pub mod apply;
pub mod auto_rules;
pub mod documents;
pub mod email;
pub mod ingestion;
pub mod outbox;
pub mod outbox_hold;
pub mod profile;
pub mod scoring;
pub mod sources;
pub mod system;
pub mod triage;
