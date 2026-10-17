//! Filling a Session Board apply form in the user's own session (S11.2, FR-91,
//! AD-10).
//!
//! The ATS channel (`crate::forms`, `crate::driver`) drives the Boba profile;
//! this module drives **that Source's session profile** (S11.1a), so the user
//! is already signed in when the form opens. The split is the same:
//!
//! * [`page`] holds the scripts evaluated in the page and the pure decoder —
//!   unit-tested without a browser;
//! * the driver below speaks CDP to the user's Chrome, one profile per Source.
//!
//! What never changes: the browser is headed (the user watches and presses
//! submit), the CDP port is ephemeral, Boba never signs in, creates an account,
//! answers a challenge or submits.

pub mod driver;
pub mod page;

pub use driver::{SessionFormDriver, SessionPageDriver};
