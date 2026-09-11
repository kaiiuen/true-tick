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
            let (minimum, maximum, _current) = query_resolution()?;
            if interval < minimum
                || interval > maximum
                || interval == Hns::ZERO
                || interval.value() > u32::MAX as u64
            {
                return Err(TimerError::InvalidInterval);
            }
            return Ok(TimerBounds {
                minimum_interval: minimum,
                maximum_interval: maximum,
            });
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
fn query_resolution() -> Result<(Hns, Hns, Hns), TimerError> {
    let mut minimum = 0u32;
    let mut maximum = 0u32;
    let mut current = 0u32;
    let status = unsafe { nt_query_timer_resolution(&mut minimum, &mut maximum, &mut current) };
    if status != STATUS_SUCCESS {
        return Err(TimerError::QueryFailed { raw_status: status });
    }
    Ok((
        Hns::new(maximum as u64),
        Hns::new(minimum as u64),
        Hns::new(current as u64),
    ))
}

#[cfg(windows)]
extern "system" {
    fn NtQueryTimerResolution(
        maximum_resolution: *mut u32,
        minimum_resolution: *mut u32,
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
    maximum: *mut u32,
    minimum: *mut u32,
    current: *mut u32,
) -> u32 {
    NtQueryTimerResolution(maximum, minimum, current)
}

#[cfg(windows)]
unsafe fn nt_set_timer_resolution(desired: u32, set: bool, current: *mut u32) -> u32 {
    NtSetTimerResolution(desired, set, current)
}

#[cfg(test)]
mod tests {
    #[cfg(not(windows))]
    use super::*;

    #[test]
    fn non_windows_adapter_is_explicitly_unsupported() {
        #[cfg(not(windows))]
        assert_eq!(
            WindowsTimerPlatform::default().preflight(Hns::new(10_000)),
            Err(TimerError::Unsupported)
        );
    }
}
