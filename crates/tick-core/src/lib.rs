//! Platform-neutral domain types for the True™ Tick skeleton.
//!
//! This crate intentionally contains no operating-system calls. HNS is represented
//! as a value type only; no platform API is selected by this model.

use std::fmt;
use std::time::Duration;

/// A duration represented in 100-nanosecond units.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Hns(u64);

/// Number of 100-nanosecond units in one millisecond.
pub const HNS_PER_MILLISECOND: u64 = 10_000;
/// Number of 100-nanosecond units in one thousandth of a millisecond.
pub const HNS_PER_THOUSANDTH_MILLISECOND: u64 = HNS_PER_MILLISECOND / 1_000;

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

    /// Formats the value for concise user-facing millisecond display.
    pub fn format_milliseconds(self) -> String {
        let thousandths = self.0.saturating_add(HNS_PER_THOUSANDTH_MILLISECOND / 2)
            / HNS_PER_THOUSANDTH_MILLISECOND;
        format!("{}.{:03}", thousandths / 1_000, thousandths % 1_000)
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DesiredIntent {
    Acquire,
    Release,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DesiredIntentQueue {
    pending: Option<DesiredIntent>,
}

impl DesiredIntentQueue {
    pub const fn new() -> Self {
        Self { pending: None }
    }

    pub const fn pending(self) -> Option<DesiredIntent> {
        self.pending
    }

    pub fn request(&mut self, intent: DesiredIntent) {
        self.pending = Some(intent);
    }

    pub fn take(&mut self) -> Option<DesiredIntent> {
        self.pending.take()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
    ObservationFailed { raw_status: u32 },
}

impl fmt::Display for CoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLifecycleTransition => write!(formatter, "invalid lifecycle transition"),
            Self::Unsupported => write!(formatter, "operation unsupported by this skeleton"),
            Self::ObservationFailed { raw_status } => {
                write!(
                    formatter,
                    "power observation failed with status {raw_status}"
                )
            }
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

    #[test]
    fn hns_millisecond_formatting_uses_the_shared_conversion() {
        assert_eq!(Hns::new(4_966).format_milliseconds(), "0.497");
        assert_eq!(Hns::new(5_000).format_milliseconds(), "0.500");
    }

    #[test]
    fn desired_intent_has_only_the_two_serialized_targets() {
        assert_ne!(DesiredIntent::Acquire, DesiredIntent::Release);
    }

    #[test]
    fn desired_intent_queue_keeps_only_the_latest_request() {
        let mut queue = DesiredIntentQueue::new();
        queue.request(DesiredIntent::Acquire);
        queue.request(DesiredIntent::Release);
        assert_eq!(queue.take(), Some(DesiredIntent::Release));
        assert_eq!(queue.take(), None);
    }
}
