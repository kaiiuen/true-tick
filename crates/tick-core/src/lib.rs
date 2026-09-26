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
    ///
    /// The output is exact to four decimal places because one HNS is exactly
    /// one ten-thousandth of a millisecond. No rounding is applied.
    pub fn format_milliseconds(self) -> String {
        let mut buffer = [0u8; 32];
        let len = self.write_milliseconds_bytes(&mut buffer);
        // SAFETY: write_milliseconds_bytes strictly emits ASCII digits and a period
        unsafe { std::str::from_utf8_unchecked(&buffer[..len]) }.to_owned()
    }

    /// Writes concise millisecond display bytes directly into a stack buffer.
    pub fn write_milliseconds_bytes(self, buffer: &mut [u8; 32]) -> usize {
        use std::io::Write;
        let whole = self.0 / HNS_PER_MILLISECOND;
        let rem = self.0 - (whole * HNS_PER_MILLISECOND);
        let mut cursor = std::io::Cursor::new(&mut buffer[..]);
        let _ = write!(cursor, "{whole}.{rem:04}");
        cursor.position() as usize
    }

    /// Formats the value with both exact milliseconds and the raw HNS count.
    pub fn format_detailed(self) -> String {
        let whole = self.0 / HNS_PER_MILLISECOND;
        let rem = self.0 - (whole * HNS_PER_MILLISECOND);
        format!("{whole}.{rem:04} ms ({} HNS)", self.0)
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

/// Converts a civil (year, month, day) date to days since 1970-01-01.
///
/// Uses the Hinnant civil calendar algorithm. Pure integer arithmetic with no
/// platform calls. Months are one based, days are one based, and year zero is
/// the astronomical convention (1 BCE is year -1).
pub fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let adjusted_year = if month <= 2 { year - 1 } else { year };
    let era = if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    } / 400;
    let year_of_era = adjusted_year - era * 400;
    let shifted_month = ((month + 9) % 12) as i64;
    let day_of_year = (153 * shifted_month + 2) / 5 + day as i64 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Converts days since 1970-01-01 back to a civil (year, month, day) date.
///
/// Uses the Hinnant civil calendar algorithm. Pure integer arithmetic with no
/// platform calls. The returned month and day are one based.
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    (
        if month <= 2 { year + 1 } else { year },
        month as u32,
        day as u32,
    )
}

/// Formats a civil date as zero padded `YYYY-MM-DD`.
///
/// Negative years are rendered with a leading minus sign followed by the
/// zero padded absolute value, for example `-0001-02-03`.
pub fn format_ymd(year: i64, month: u32, day: u32) -> String {
    if year < 0 {
        format!("-{:04}-{:02}-{:02}", year.unsigned_abs(), month, day)
    } else {
        format!("{year:04}-{month:02}-{day:02}")
    }
}

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
        assert_eq!(Hns::new(4_966).format_milliseconds(), "0.4966");
        assert_eq!(Hns::new(5_000).format_milliseconds(), "0.5000");
        assert_eq!(Hns::new(9_999).format_milliseconds(), "0.9999");
        assert_eq!(Hns::new(156_250).format_milliseconds(), "15.6250");
    }

    #[test]
    fn hns_detailed_formatting_includes_raw_units() {
        assert_eq!(Hns::new(4_966).format_detailed(), "0.4966 ms (4966 HNS)");
        assert_eq!(
            Hns::new(156_250).format_detailed(),
            "15.6250 ms (156250 HNS)"
        );
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

    #[test]
    fn epoch_day_zero_is_1970_01_01() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[test]
    fn epoch_day_minus_one_is_1969_12_31() {
        assert_eq!(days_from_civil(1969, 12, 31), -1);
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }

    #[test]
    fn day_20721_is_2026_09_25() {
        // Verified independently by taking the UTC epoch seconds of the
        // calendar date and dividing by the seconds in one day.
        assert_eq!(days_from_civil(2026, 9, 25), 20_721);
        assert_eq!(civil_from_days(20_721), (2026, 9, 25));
    }

    #[test]
    fn leap_day_round_trips() {
        assert_eq!(days_from_civil(2024, 2, 29), 19_782);
        assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2, 29));
    }

    #[test]
    fn format_ymd_pads_components_correctly() {
        assert_eq!(format_ymd(2026, 9, 5), "2026-09-05");
        assert_eq!(format_ymd(20_721, 1, 1), "20721-01-01");
    }
}
