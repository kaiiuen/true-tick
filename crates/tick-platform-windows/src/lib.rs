//! Isolated Windows timer-resolution adapter.
//!
//! The native calls are intentionally kept behind this narrow boundary. A
//! successful call proves API acceptance and the adapter's postcondition only.
//! It does not prove a universal effective system value or exclusive ownership.

use tick_core::Hns;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimerBounds {
    pub minimum_interval: Hns,
    pub maximum_interval: Hns,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimerObservation {
    pub requested: Hns,
    pub reported_current: Hns,
    pub raw_status: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimerError {
    Unsupported,
    InvalidInterval,
    QueryFailed {
        raw_status: u32,
    },
    RequestFailed {
        raw_status: u32,
    },
    ReleaseFailed {
        raw_status: u32,
    },
    PostconditionUnverified {
        raw_status: u32,
        reported_current: Hns,
    },
}

pub trait TimerPlatform {
    fn preflight(&mut self, interval: Hns) -> Result<TimerBounds, TimerError>;
    fn request(&mut self, interval: Hns) -> Result<TimerObservation, TimerError>;
    fn release(&mut self, interval: Hns) -> Result<TimerObservation, TimerError>;
}

#[derive(Debug, Default)]
pub struct WindowsTimerPlatform {
    #[cfg(windows)]
    requested: Option<Hns>,
}

impl TimerPlatform for WindowsTimerPlatform {
    fn preflight(&mut self, interval: Hns) -> Result<TimerBounds, TimerError> {
        #[cfg(windows)]
        {
            let bounds = query_resolution()?.0;
            validate_interval(bounds, interval)?;
            return Ok(bounds);
        }
        #[cfg(not(windows))]
        {
            let _ = interval;
            Err(TimerError::Unsupported)
        }
    }

    fn request(&mut self, interval: Hns) -> Result<TimerObservation, TimerError> {
        #[cfg(windows)]
        {
            let _ = self.preflight(interval)?;
            let mut current = 0u32;
            let status =
                unsafe { nt_set_timer_resolution(interval.value() as u32, true, &mut current) };
            if status != STATUS_SUCCESS {
                return Err(TimerError::RequestFailed { raw_status: status });
            }
            let observation = TimerObservation {
                requested: interval,
                reported_current: Hns::new(current as u64),
                raw_status: status,
            };
            if observation.reported_current != interval {
                return Err(TimerError::PostconditionUnverified {
                    raw_status: status,
                    reported_current: observation.reported_current,
                });
            }
            self.requested = Some(interval);
            return Ok(observation);
        }
        #[cfg(not(windows))]
        {
            let _ = interval;
            Err(TimerError::Unsupported)
        }
    }

    fn release(&mut self, interval: Hns) -> Result<TimerObservation, TimerError> {
        #[cfg(windows)]
        {
            if self.requested != Some(interval) {
                return Err(TimerError::InvalidInterval);
            }
            let mut current = 0u32;
            let status =
                unsafe { nt_set_timer_resolution(interval.value() as u32, false, &mut current) };
            if status != STATUS_SUCCESS {
                return Err(TimerError::ReleaseFailed { raw_status: status });
            }
            self.requested = None;
            return Ok(TimerObservation {
                requested: interval,
                reported_current: Hns::new(current as u64),
                raw_status: status,
            });
        }
        #[cfg(not(windows))]
        {
            let _ = interval;
            Err(TimerError::Unsupported)
        }
    }
}

#[cfg(windows)]
const STATUS_SUCCESS: u32 = 0;

#[cfg(windows)]
fn query_resolution() -> Result<(TimerBounds, Hns), TimerError> {
    let mut minimum_resolution = 0u32;
    let mut maximum_resolution = 0u32;
    let mut current_resolution = 0u32;
    let status = unsafe {
        nt_query_timer_resolution(
            &mut minimum_resolution,
            &mut maximum_resolution,
            &mut current_resolution,
        )
    };
    if status != STATUS_SUCCESS {
        return Err(TimerError::QueryFailed { raw_status: status });
    }
    Ok(native_resolution_values(
        minimum_resolution,
        maximum_resolution,
        current_resolution,
    ))
}

fn native_resolution_values(
    minimum_resolution: u32,
    maximum_resolution: u32,
    current_resolution: u32,
) -> (TimerBounds, Hns) {
    (
        TimerBounds {
            minimum_interval: Hns::new(minimum_resolution as u64),
            maximum_interval: Hns::new(maximum_resolution as u64),
        },
        Hns::new(current_resolution as u64),
    )
}

fn validate_interval(bounds: TimerBounds, interval: Hns) -> Result<(), TimerError> {
    if interval == Hns::ZERO
        || interval.value() > u32::MAX as u64
        || bounds.minimum_interval > bounds.maximum_interval
        || interval < bounds.minimum_interval
        || interval > bounds.maximum_interval
    {
        return Err(TimerError::InvalidInterval);
    }
    Ok(())
}

#[cfg(windows)]
extern "system" {
    fn NtQueryTimerResolution(
        minimum_resolution: *mut u32,
        maximum_resolution: *mut u32,
        current_resolution: *mut u32,
    ) -> u32;
    fn NtSetTimerResolution(
        desired_resolution: u32,
        set_resolution: bool,
        current_resolution: *mut u32,
    ) -> u32;
}

#[cfg(windows)]
unsafe fn nt_query_timer_resolution(
    minimum: *mut u32,
    maximum: *mut u32,
    current: *mut u32,
) -> u32 {
    NtQueryTimerResolution(minimum, maximum, current)
}

#[cfg(windows)]
unsafe fn nt_set_timer_resolution(desired: u32, set: bool, current: *mut u32) -> u32 {
    NtSetTimerResolution(desired, set, current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_fixture_preserves_native_bound_order() {
        let (bounds, current) = native_resolution_values(5_000, 15_625, 10_000);
        assert_eq!(bounds.minimum_interval, Hns::new(5_000));
        assert_eq!(bounds.maximum_interval, Hns::new(15_625));
        assert_eq!(current, Hns::new(10_000));
    }

    #[test]
    fn interval_validation_accepts_an_ordinary_value() {
        let bounds = TimerBounds {
            minimum_interval: Hns::new(5_000),
            maximum_interval: Hns::new(15_625),
        };
        assert_eq!(validate_interval(bounds, Hns::new(10_000)), Ok(()));
    }

    #[test]
    fn interval_validation_accepts_both_boundaries() {
        let bounds = TimerBounds {
            minimum_interval: Hns::new(5_000),
            maximum_interval: Hns::new(15_625),
        };
        assert_eq!(validate_interval(bounds, bounds.minimum_interval), Ok(()));
        assert_eq!(validate_interval(bounds, bounds.maximum_interval), Ok(()));
    }

    #[test]
    fn interval_validation_rejects_zero_and_out_of_range_values() {
        let bounds = TimerBounds {
            minimum_interval: Hns::new(5_000),
            maximum_interval: Hns::new(15_625),
        };
        assert_eq!(
            validate_interval(bounds, Hns::ZERO),
            Err(TimerError::InvalidInterval)
        );
        assert_eq!(
            validate_interval(bounds, Hns::new(4_999)),
            Err(TimerError::InvalidInterval)
        );
        assert_eq!(
            validate_interval(bounds, Hns::new(15_626)),
            Err(TimerError::InvalidInterval)
        );
    }

    #[test]
    fn non_windows_adapter_is_explicitly_unsupported() {
        #[cfg(not(windows))]
        assert_eq!(
            WindowsTimerPlatform::default().preflight(Hns::new(10_000)),
            Err(TimerError::Unsupported)
        );
    }
}
