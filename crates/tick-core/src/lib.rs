//! Platform-neutral domain types for the True™ Tick skeleton.
//!
//! This crate intentionally contains no operating-system calls. HNS is represented
//! as a value type only; no platform API is selected by this model.

use std::fmt;
use std::time::Duration;

/// A duration represented in 100-nanosecond units.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Hns(u64);

impl Hns {
    pub const ZERO: Self = Self(0);

    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u64 {
        self.0
    }

    pub fn to_duration(self) -> Duration {
        Duration::from_nanos(self.0.saturating_mul(100))
    }
}

impl From<Duration> for Hns {
    fn from(duration: Duration) -> Self {
        let nanos = duration.as_nanos() / 100;
        Self(nanos.min(u64::MAX as u128) as u64)
    }
}

impl fmt::Display for Hns {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} HNS", self.0)
    }
}

/// Logical lifecycle state; it is not evidence of platform activity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Lifecycle {
    Created,
    Running,
    Stopping,
    Stopped,
}

/// Truthful high-level status for a future composition layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    Released,
    Active,
    Requested,
    Warning,
    Blocked,
    Error,
    Unknown,
    Unsupported,
}

/// Pure event vocabulary shared by boundaries. Payloads are intentionally logical.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Event {
    Start,
    Stop,
    PolicyChanged,
    ObservationUnavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoreError {
    InvalidLifecycleTransition,
    Unsupported,
}

impl fmt::Display for CoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLifecycleTransition => write!(formatter, "invalid lifecycle transition"),
            Self::Unsupported => write!(formatter, "operation unsupported by this skeleton"),
        }
    }
}

impl std::error::Error for CoreError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hns_round_trip_is_bounded_and_deterministic() {
        let duration = Duration::from_micros(25);
        assert_eq!(Hns::from(duration).value(), 250);
        assert_eq!(Hns::from(duration).to_duration(), duration);
    }

    #[test]
    fn hns_display_is_explicit_about_units() {
        assert_eq!(Hns::new(5_000).to_string(), "5000 HNS");
    }
}
