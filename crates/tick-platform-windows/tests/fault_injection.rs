use tick_core::Hns;
use tick_platform_windows::{
    HARDWARE_TIMER_TOLERANCE_HNS, KernelSettleProbe, NtStatus, TimerBounds, TimerError,
    TimerObservation, TimerPlatform, TimerQuery,
};

struct FaultyPlatform {
    query_result: Result<TimerQuery, TimerError>,
    preflight_result: Result<TimerQuery, TimerError>,
    request_result: Result<TimerObservation, TimerError>,
    release_result: Result<TimerObservation, TimerError>,
}

impl Default for FaultyPlatform {
    fn default() -> Self {
        Self {
            query_result: Err(TimerError::Unsupported),
            preflight_result: Err(TimerError::Unsupported),
            request_result: Err(TimerError::Unsupported),
            release_result: Err(TimerError::Unsupported),
        }
    }
}

impl TimerPlatform for FaultyPlatform {
    fn query(&mut self, _interval: Hns) -> Result<TimerQuery, TimerError> {
        self.query_result
    }

    fn preflight(&mut self, _interval: Hns) -> Result<TimerQuery, TimerError> {
        self.preflight_result
    }

    fn request(&mut self, _interval: Hns) -> Result<TimerObservation, TimerError> {
        self.request_result
    }

    fn release(&mut self, _interval: Hns) -> Result<TimerObservation, TimerError> {
        self.release_result
    }

    fn attempt_kernel_settle_probe(
        &mut self,
        _interval: Hns,
    ) -> Result<KernelSettleProbe, TimerError> {
        Err(TimerError::Unsupported)
    }
}

#[test]
fn interval_validation_rejects_zero_hns() {
    let zero_bounds = TimerBounds {
        minimum_interval: Hns::ZERO,
        maximum_interval: Hns::ZERO,
    };

    let query = TimerQuery {
        bounds: zero_bounds,
        reported_current: Hns::new(156_250),
        raw_status: 0,
    };

    assert_eq!(query.resolve_request(Hns::ZERO), Err(TimerError::InvalidInterval));
    assert_eq!(zero_bounds.smallest_supported_boundary(), Err(TimerError::InvalidInterval));
}

#[test]
fn interval_validation_rejects_values_below_minimum_resolution() {
    let bounds = TimerBounds {
        minimum_interval: Hns::new(5_000),
        maximum_interval: Hns::new(156_250),
    };

    let query = TimerQuery {
        bounds,
        reported_current: Hns::new(156_250),
        raw_status: 0,
    };

    assert_eq!(query.resolve_request(Hns::new(4_999)), Err(TimerError::InvalidInterval));
    assert_eq!(query.resolve_request(Hns::new(1)), Err(TimerError::InvalidInterval));
}

#[test]
fn interval_validation_rejects_values_above_maximum_resolution() {
    let bounds = TimerBounds {
        minimum_interval: Hns::new(5_000),
        maximum_interval: Hns::new(156_250),
    };

    let query = TimerQuery {
        bounds,
        reported_current: Hns::new(156_250),
        raw_status: 0,
    };

    assert_eq!(query.resolve_request(Hns::new(156_251)), Err(TimerError::InvalidInterval));
    assert_eq!(query.resolve_request(Hns::new(200_000)), Err(TimerError::InvalidInterval));
    assert_eq!(query.resolve_request(Hns::new(u32::MAX as u64 + 1)), Err(TimerError::InvalidInterval));
    assert_eq!(query.resolve_request(Hns::new(u64::MAX)), Err(TimerError::InvalidInterval));
}

#[test]
fn interval_validation_normalizes_inverted_boundaries_where_min_exceeds_max() {
    let inverted = TimerBounds {
        minimum_interval: Hns::new(156_250),
        maximum_interval: Hns::new(5_000),
    };

    assert_eq!(
        inverted.numeric_interval(),
        (Hns::new(5_000), Hns::new(156_250))
    );
    assert_eq!(inverted.smallest_supported_boundary(), Ok(Hns::new(5_000)));

    let query = TimerQuery {
        bounds: inverted,
        reported_current: Hns::new(156_250),
        raw_status: 0,
    };

    assert_eq!(query.resolve_request(Hns::new(10_000)), Ok(Hns::new(10_000)));
    assert_eq!(query.resolve_request(Hns::new(5_000)), Ok(Hns::new(5_000)));
    assert_eq!(query.resolve_request(Hns::new(156_250)), Ok(Hns::new(156_250)));
    assert_eq!(query.resolve_request(Hns::new(4_999)), Err(TimerError::InvalidInterval));
    assert_eq!(query.resolve_request(Hns::new(156_251)), Err(TimerError::InvalidInterval));
    assert_eq!(query.resolve_request(Hns::ZERO), Ok(Hns::new(5_000)));
}

#[test]
fn interval_validation_rejects_inverted_boundaries_with_zero_or_overflow() {
    let inverted_zero = TimerBounds {
        minimum_interval: Hns::new(156_250),
        maximum_interval: Hns::ZERO,
    };

    assert_eq!(
        inverted_zero.smallest_supported_boundary(),
        Err(TimerError::InvalidInterval)
    );

    let query_zero = TimerQuery {
        bounds: inverted_zero,
        reported_current: Hns::new(156_250),
        raw_status: 0,
    };
    assert_eq!(query_zero.resolve_request(Hns::ZERO), Err(TimerError::InvalidInterval));

    let inverted_overflow = TimerBounds {
        minimum_interval: Hns::new(u32::MAX as u64 + 50),
        maximum_interval: Hns::new(u32::MAX as u64 + 10),
    };
    assert_eq!(
        inverted_overflow.smallest_supported_boundary(),
        Err(TimerError::InvalidInterval)
    );
    let query_overflow = TimerQuery {
        bounds: inverted_overflow,
        reported_current: Hns::new(156_250),
        raw_status: 0,
    };
    assert_eq!(query_overflow.resolve_request(Hns::new(10_000)), Err(TimerError::InvalidInterval));
}

#[test]
fn simulated_kernel_failure_status_access_denied_maps_to_structured_error() {
    const STATUS_ACCESS_DENIED: NtStatus = 0xC0000022u32 as i32;

    let query_error = TimerError::QueryFailed {
        raw_status: STATUS_ACCESS_DENIED,
    };
    assert_eq!(
        query_error,
        TimerError::QueryFailed {
            raw_status: -1073741790,
        }
    );

    let request_error = TimerError::RequestFailed {
        raw_status: STATUS_ACCESS_DENIED,
    };
    assert_eq!(
        request_error,
        TimerError::RequestFailed {
            raw_status: -1073741790,
        }
    );

    let release_error = TimerError::ReleaseFailed {
        raw_status: STATUS_ACCESS_DENIED,
    };
    assert_eq!(
        release_error,
        TimerError::ReleaseFailed {
            raw_status: -1073741790,
        }
    );

    let mut platform = FaultyPlatform {
        request_result: Err(request_error),
        ..Default::default()
    };
    assert_eq!(platform.request(Hns::new(5_000)), Err(request_error));
}

#[test]
fn simulated_kernel_failure_status_invalid_parameter_maps_to_structured_error() {
    const STATUS_INVALID_PARAMETER: NtStatus = 0xC000000Du32 as i32;

    let query_error = TimerError::QueryFailed {
        raw_status: STATUS_INVALID_PARAMETER,
    };
    assert_eq!(
        query_error,
        TimerError::QueryFailed {
            raw_status: -1073741811,
        }
    );

    let request_error = TimerError::RequestFailed {
        raw_status: STATUS_INVALID_PARAMETER,
    };
    assert_eq!(
        request_error,
        TimerError::RequestFailed {
            raw_status: -1073741811,
        }
    );

    let release_error = TimerError::ReleaseFailed {
        raw_status: STATUS_INVALID_PARAMETER,
    };
    assert_eq!(
        release_error,
        TimerError::ReleaseFailed {
            raw_status: -1073741811,
        }
    );

    let mut platform = FaultyPlatform {
        query_result: Err(query_error),
        ..Default::default()
    };
    assert_eq!(platform.query(Hns::new(5_000)), Err(query_error));
}

#[test]
fn simulated_postcondition_mismatch_kernel_success_with_unverified_rate() {
    const STATUS_SUCCESS: NtStatus = 0;

    let requested = Hns::new(5_000);
    let coarse_rate = Hns::new(156_250);

    let observation = TimerObservation {
        requested,
        reported_current: coarse_rate,
        raw_status: STATUS_SUCCESS,
    };

    assert!(!observation.is_satisfied());
    assert!(!observation.is_finer_than_requested());
    assert_eq!(observation.effective_relation(), "unverified");

    let postcondition_err = TimerError::PostconditionUnverified {
        raw_status: STATUS_SUCCESS,
        reported_current: coarse_rate,
    };

    let mut platform = FaultyPlatform {
        request_result: Err(postcondition_err),
        ..Default::default()
    };

    assert_eq!(
        platform.request(requested),
        Err(TimerError::PostconditionUnverified {
            raw_status: STATUS_SUCCESS,
            reported_current: coarse_rate,
        })
    );
}

#[test]
fn simulated_postcondition_mismatch_boundary_beyond_hardware_tolerance() {
    const STATUS_SUCCESS: NtStatus = 0;
    let requested = Hns::new(5_000);

    let just_within = TimerObservation {
        requested,
        reported_current: Hns::new(requested.value() + HARDWARE_TIMER_TOLERANCE_HNS),
        raw_status: STATUS_SUCCESS,
    };
    assert!(just_within.is_satisfied());
    assert_eq!(just_within.effective_relation(), "satisfied");

    let outside_tolerance = TimerObservation {
        requested,
        reported_current: Hns::new(requested.value() + HARDWARE_TIMER_TOLERANCE_HNS + 1),
        raw_status: STATUS_SUCCESS,
    };
    assert!(!outside_tolerance.is_satisfied());
    assert_eq!(outside_tolerance.effective_relation(), "unverified");
}
