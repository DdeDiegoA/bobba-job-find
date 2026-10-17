//! `KillSwitchPort` — the one flag that stops every automatic send (S9.8,
//! FR-86).
//!
//! S9.7 owns the automatic send path, so it leaves the check point S9.8 toggles:
//! the Outbox re-reads this flag at release and again just before the channel
//! submits. Until S9.8 mounts its adapter the port is unmounted, which reads as
//! "not engaged" — the switch exists to stop automation, never to block the app
//! from starting.

/// Whether the user engaged the Kill Switch.
pub trait KillSwitchPort: Send + Sync {
    /// `true` while every automatic send must stop (FR-86).
    fn engaged(&self) -> bool;
}
