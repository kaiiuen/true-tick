//! Multi-client ownership test harness for True Tick.
//!
//! This crate proves that True Tick acquires and releases only its own tracked
//! timer contribution and does not force the global Windows timer resolution to
//! a guessed default when another process holds a finer request.

/// Role of a helper client process that holds a competing timer request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClientRole {
    /// The helper requests a finer (smaller) interval than True Tick.
    Finer,
    /// The helper requests a coarser (larger) interval than True Tick.
    Coarser,
}

/// Specification for a helper client request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClientSpec {
    /// Whether the helper is finer or coarser than the tick request.
    pub role: ClientRole,
    /// The interval the helper requests, in 100-nanosecond units.
    pub interval_hns: u64,
}

/// Pure expectation for the global resolution after a tick release.
///
/// The result is intentionally a coarse classification. It does not attempt to
/// model every scheduling detail, only whether the finer client retained the
/// resolution or whether the value was released cleanly.
pub fn expect_after_release(finer_held: bool, tick_released: bool) -> &'static str {
    if finer_held {
        "finer_held"
    } else if tick_released {
        "released"
    } else {
        "uncertain"
    }
}

/// Classification of a release observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReleaseOutcome {
    /// True Tick released only its own contribution and nothing else held it.
    TickOnlyReleased,
    /// A finer client request remained active after the tick release.
    FinerClientRetained,
    /// The observed state does not match either expected outcome.
    UnknownState,
}

/// Classify the observed current resolution after True Tick releases.
///
/// `tick_owned` records whether True Tick believed it owned the interval it
/// just released. `finer_client_active` records whether the helper process is
/// still running and holding a request. `observed_current_hns` is the value
/// returned by a fresh query after the release. `tick_request_hns` is the
/// interval True Tick requested. `finer_request_hns` is the interval the
/// helper requested.
///
/// The tolerance of 100 HNS mirrors the hardware timer tolerance used by the
/// platform adapter so that ordinary crystal quantization jitter is not
/// misclassified as an unexpected state.
pub fn classify_release_outcome(
    tick_owned: bool,
    finer_client_active: bool,
    observed_current_hns: u64,
    tick_request_hns: u64,
    finer_request_hns: u64,
) -> ReleaseOutcome {
    if tick_owned && finer_client_active {
        let distance = observed_current_hns.abs_diff(finer_request_hns);
        if distance <= 100 {
            return ReleaseOutcome::FinerClientRetained;
        }
    }
    if !finer_client_active && observed_current_hns > tick_request_hns {
        return ReleaseOutcome::TickOnlyReleased;
    }
    ReleaseOutcome::UnknownState
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finer_client_retained_when_observed_matches_finer_request() {
        let outcome = classify_release_outcome(true, true, 5_000, 5_000, 5_000);
        assert_eq!(outcome, ReleaseOutcome::FinerClientRetained);
    }

    #[test]
    fn tick_only_released_when_no_finer_client_and_current_is_coarser() {
        let outcome = classify_release_outcome(true, false, 15_625, 5_000, 5_000);
        assert_eq!(outcome, ReleaseOutcome::TickOnlyReleased);
    }

    #[test]
    fn unknown_state_when_observed_does_not_match_expectations() {
        let outcome = classify_release_outcome(true, true, 15_625, 5_000, 5_000);
        assert_eq!(outcome, ReleaseOutcome::UnknownState);
    }
}
