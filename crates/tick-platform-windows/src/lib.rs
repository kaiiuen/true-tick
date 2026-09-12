//! Isolated Windows timer-resolution adapter.
//!
//! The native calls are intentionally kept behind this narrow boundary. A
//! successful call proves API acceptance and the adapter's postcondition only.
//! It does not prove a universal effective system value or exclusive ownership.

use std::sync::Arc;
use tick_core::Hns;
use tick_diagnostics::DiagnosticStore;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimerBounds {
    /// The value returned through the API's minimum-resolution output.
    ///
    /// Windows output labels are retained for diagnostics. They are not used
    /// as an ordering guarantee because the observed values can be reversed.
    pub minimum_interval: Hns,
    /// The value returned through the API's maximum-resolution output.
    ///
    /// Windows output labels are retained for diagnostics. They are not used
    /// as an ordering guarantee because the observed values can be reversed.
    pub maximum_interval: Hns,
}

impl TimerBounds {
    pub fn numeric_interval(self) -> (Hns, Hns) {
        if self.minimum_interval <= self.maximum_interval {
            (self.minimum_interval, self.maximum_interval)
        } else {
            (self.maximum_interval, self.minimum_interval)
        }
    }

    pub fn smallest_supported_boundary(self) -> Result<Hns, TimerError> {
        let (lower, _) = self.numeric_interval();
        if lower == Hns::ZERO || lower.value() > u32::MAX as u64 {
            Err(TimerError::InvalidInterval)
        } else {
            Ok(lower)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimerQuery {
    pub bounds: TimerBounds,
    pub reported_current: Hns,
    pub raw_status: NtStatus,
}

impl TimerQuery {
    pub fn resolve_request(self, requested: Hns) -> Result<Hns, TimerError> {
        let selected = if requested == Hns::ZERO {
            self.bounds.smallest_supported_boundary()?
        } else {
            requested
        };
        validate_interval(self.bounds, selected)?;
        Ok(selected)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimerObservation {
    pub requested: Hns,
    pub reported_current: Hns,
    pub raw_status: NtStatus,
}

impl TimerObservation {
    pub fn is_satisfied(self) -> bool {
        self.reported_current <= self.requested
    }

    pub fn is_finer_than_requested(self) -> bool {
        self.reported_current < self.requested
    }

    pub fn effective_relation(self) -> &'static str {
        if self.reported_current < self.requested {
            "finer"
        } else if self.reported_current == self.requested {
            "equal"
        } else {
            "unverified"
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimerError {
    Unsupported,
    InvalidInterval,
    QueryFailed {
        raw_status: NtStatus,
    },
    RequestFailed {
        raw_status: NtStatus,
    },
    ReleaseFailed {
        raw_status: NtStatus,
    },
    PostconditionUnverified {
        raw_status: NtStatus,
        reported_current: Hns,
    },
}

pub trait TimerPlatform {
    fn query(&mut self, interval: Hns) -> Result<TimerQuery, TimerError>;

    fn resolve(&mut self, requested: Hns) -> Result<Hns, TimerError> {
        let query = self.query(requested)?;
        query.resolve_request(requested)
    }

    fn preflight(&mut self, interval: Hns) -> Result<TimerQuery, TimerError>;
    fn request(&mut self, interval: Hns) -> Result<TimerObservation, TimerError>;
    fn release(&mut self, interval: Hns) -> Result<TimerObservation, TimerError>;
}

#[derive(Debug, Default)]
pub struct WindowsTimerPlatform {
    #[cfg(windows)]
    requested: Option<Hns>,
    diagnostics: Option<Arc<DiagnosticStore>>,
}

impl WindowsTimerPlatform {
    pub fn with_diagnostics(diagnostics: Arc<DiagnosticStore>) -> Self {
        Self {
            #[cfg(windows)]
            requested: None,
            diagnostics: Some(diagnostics),
        }
    }

    fn log(&self, name: &str, details: impl AsRef<str>) {
        if let Some(diagnostics) = &self.diagnostics {
            diagnostics.record(name, details);
        }
    }
}

impl TimerPlatform for WindowsTimerPlatform {
    fn query(&mut self, interval: Hns) -> Result<TimerQuery, TimerError> {
        self.log(
            "timer.query",
            format!(
                "requested_hns={} requested_ms={}",
                interval.value(),
                interval.format_milliseconds()
            ),
        );
        #[cfg(windows)]
        {
            let (bounds, current) = match query_resolution() {
                Ok(values) => values,
                Err(error) => {
                    self.log(
                        "native.NtQueryTimerResolution.error",
                        format!("error={error:?}"),
                    );
                    return Err(error);
                }
            };
            self.log(
                "native.NtQueryTimerResolution.result",
                format!(
                    "raw_status=0 minimum_hns={} maximum_hns={} current_hns={}",
                    bounds.minimum_interval.value(),
                    bounds.maximum_interval.value(),
                    current.value()
                ),
            );
            Ok(TimerQuery {
                bounds,
                reported_current: current,
                raw_status: STATUS_SUCCESS,
            })
        }
        #[cfg(not(windows))]
        {
            self.log("timer.query.unsupported", "platform=non_windows");
            let _ = interval;
            Err(TimerError::Unsupported)
        }
    }

    fn resolve(&mut self, requested: Hns) -> Result<Hns, TimerError> {
        let query = self.query(requested)?;
        let selected = query.resolve_request(requested);
        match selected {
            Ok(interval) => {
                self.log(
                    "timer.selection",
                    format!(
                        "mode={} raw_status={} raw_minimum_hns={} raw_maximum_hns={} selected_hns={} requested_hns={} effective_hns={} effective_relation={}",
                        if requested == Hns::ZERO { "automatic" } else { "fixed" },
                        query.raw_status,
                        query.bounds.minimum_interval.value(),
                        query.bounds.maximum_interval.value(),
                        interval.value(),
                        requested.value(),
                        query.reported_current.value(),
                        effective_relation(query.reported_current, interval)
                    ),
                );
                Ok(interval)
            }
            Err(error) => {
                self.log(
                    "timer.selection.rejected",
                    format!(
                        "mode={} raw_status={} raw_minimum_hns={} raw_maximum_hns={} requested_hns={} effective_hns={} effective_relation={} error={error:?}",
                        if requested == Hns::ZERO { "automatic" } else { "fixed" },
                        query.raw_status,
                        query.bounds.minimum_interval.value(),
                        query.bounds.maximum_interval.value(),
                        requested.value(),
                        query.reported_current.value(),
                        effective_relation(query.reported_current, requested)
                    ),
                );
                Err(error)
            }
        }
    }

    fn preflight(&mut self, interval: Hns) -> Result<TimerQuery, TimerError> {
        self.log(
            "timer.preflight",
            format!(
                "requested_hns={} requested_ms={}",
                interval.value(),
                interval.format_milliseconds()
            ),
        );
        let query = self.query(interval)?;
        if let Err(error) = validate_interval(query.bounds, interval) {
            self.log("timer.preflight.invalid", format!("error={error:?}"));
            return Err(error);
        }
        self.log("timer.preflight.accepted", "interval within native bounds");
        Ok(query)
    }

    fn request(&mut self, interval: Hns) -> Result<TimerObservation, TimerError> {
        self.log(
            "native.NtSetTimerResolution.request",
            format!("desired_hns={} set=true", interval.value()),
        );
        #[cfg(windows)]
        {
            let _ = self.preflight(interval)?;
            let mut current = 0u32;
            let status =
                unsafe { nt_set_timer_resolution(interval.value() as u32, true, &mut current) };
            self.log(
                "native.NtSetTimerResolution.result",
                format!(
                    "raw_status={} requested_hns={} effective_hns={} effective_relation={}",
                    status,
                    interval.value(),
                    current,
                    effective_relation(Hns::new(current as u64), interval)
                ),
            );
            if status != STATUS_SUCCESS {
                return Err(TimerError::RequestFailed { raw_status: status });
            }
            let observation = TimerObservation {
                requested: interval,
                reported_current: Hns::new(current as u64),
                raw_status: status,
            };
            self.requested = Some(interval);
            if !observation.is_satisfied() {
                self.log(
                    "timer.postcondition.unverified",
                    format!(
                        "requested_hns={} effective_hns={} effective_relation=unverified",
                        observation.requested.value(),
                        observation.reported_current.value()
                    ),
                );
                return Err(TimerError::PostconditionUnverified {
                    raw_status: status,
                    reported_current: observation.reported_current,
                });
            }
            if observation.is_finer_than_requested() {
                self.log(
                    "timer.postcondition.finer_than_requested",
                    format!(
                        "requested_hns={} effective_hns={} effective_relation=finer",
                        observation.requested.value(),
                        observation.reported_current.value()
                    ),
                );
            } else {
                self.log(
                    "timer.postcondition.verified",
                    format!(
                        "requested_hns={} effective_hns={} effective_relation=equal",
                        observation.requested.value(),
                        observation.reported_current.value()
                    ),
                );
            }
            Ok(observation)
        }
        #[cfg(not(windows))]
        {
            self.log("timer.request.unsupported", "platform=non_windows");
            let _ = interval;
            Err(TimerError::Unsupported)
        }
    }

    fn release(&mut self, interval: Hns) -> Result<TimerObservation, TimerError> {
        self.log(
            "native.NtSetTimerResolution.release",
            format!("desired_hns={} set=false", interval.value()),
        );
        #[cfg(windows)]
        {
            if self.requested != Some(interval) {
                self.log("timer.release.rejected", "ownership_interval_not_tracked");
                return Err(TimerError::InvalidInterval);
            }
            let mut current = 0u32;
            let status =
                unsafe { nt_set_timer_resolution(interval.value() as u32, false, &mut current) };
            self.log(
                "native.NtSetTimerResolution.release_result",
                format!(
                    "raw_status={} requested_hns={} effective_hns={} effective_relation={}",
                    status,
                    interval.value(),
                    current,
                    effective_relation(Hns::new(current as u64), interval)
                ),
            );
            if status != STATUS_SUCCESS {
                return Err(TimerError::ReleaseFailed { raw_status: status });
            }
            self.requested = None;
            Ok(TimerObservation {
                requested: interval,
                reported_current: Hns::new(current as u64),
                raw_status: status,
            })
        }
        #[cfg(not(windows))]
        {
            self.log("timer.release.unsupported", "platform=non_windows");
            let _ = interval;
            Err(TimerError::Unsupported)
        }
    }
}

pub type NtStatus = i32;
type NtBoolean = u8;
const STATUS_SUCCESS: NtStatus = 0;

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

fn effective_relation(effective: Hns, requested: Hns) -> &'static str {
    if effective < requested {
        "finer"
    } else if effective == requested {
        "equal"
    } else {
        "unverified"
    }
}

fn validate_interval(bounds: TimerBounds, interval: Hns) -> Result<(), TimerError> {
    let (lower, upper) = bounds.numeric_interval();
    if interval == Hns::ZERO
        || interval.value() > u32::MAX as u64
        || interval < lower
        || interval > upper
    {
        return Err(TimerError::InvalidInterval);
    }
    Ok(())
}

#[cfg(windows)]
#[link(name = "ntdll")]
extern "system" {
    fn NtQueryTimerResolution(
        minimum_resolution: *mut u32,
        maximum_resolution: *mut u32,
        current_resolution: *mut u32,
    ) -> NtStatus;
    fn NtSetTimerResolution(
        desired_resolution: u32,
        set_resolution: NtBoolean,
        current_resolution: *mut u32,
    ) -> NtStatus;
}

#[cfg(windows)]
unsafe fn nt_query_timer_resolution(
    minimum: *mut u32,
    maximum: *mut u32,
    current: *mut u32,
) -> NtStatus {
    NtQueryTimerResolution(minimum, maximum, current)
}

const fn nt_boolean(value: bool) -> NtBoolean {
    if value {
        1
    } else {
        0
    }
}

#[cfg(windows)]
unsafe fn nt_set_timer_resolution(desired: u32, set: bool, current: *mut u32) -> NtStatus {
    NtSetTimerResolution(desired, nt_boolean(set), current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_fixture_preserves_raw_fields_and_normalizes_numeric_order() {
        let (bounds, current) = native_resolution_values(156_250, 5_000, 4_966);
        assert_eq!(bounds.minimum_interval, Hns::new(156_250));
        assert_eq!(bounds.maximum_interval, Hns::new(5_000));
        assert_eq!(
            bounds.numeric_interval(),
            (Hns::new(5_000), Hns::new(156_250))
        );
        assert_eq!(current, Hns::new(4_966));
    }

    #[test]
    fn captured_reversed_boundaries_accept_the_requested_interval() {
        let (bounds, current) = native_resolution_values(156_250, 5_000, 9_966);
        assert_eq!(validate_interval(bounds, Hns::new(10_000)), Ok(()));
        assert_eq!(current, Hns::new(9_966));
    }

    #[test]
    fn automatic_selection_uses_the_smallest_numeric_boundary() {
        let (bounds, current) = native_resolution_values(156_250, 5_000, 9_966);
        let query = TimerQuery {
            bounds,
            reported_current: current,
            raw_status: STATUS_SUCCESS,
        };
        assert_eq!(query.resolve_request(Hns::ZERO), Ok(Hns::new(5_000)));
    }

    #[test]
    fn automatic_selection_rejects_unavailable_boundaries() {
        let query = TimerQuery {
            bounds: TimerBounds {
                minimum_interval: Hns::ZERO,
                maximum_interval: Hns::new(15_625),
            },
            reported_current: Hns::ZERO,
            raw_status: STATUS_SUCCESS,
        };
        assert_eq!(
            query.resolve_request(Hns::ZERO),
            Err(TimerError::InvalidInterval)
        );
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
    fn interval_validation_accepts_both_numeric_boundaries() {
        let bounds = TimerBounds {
            minimum_interval: Hns::new(15_625),
            maximum_interval: Hns::new(5_000),
        };
        assert_eq!(validate_interval(bounds, Hns::new(5_000)), Ok(()));
        assert_eq!(validate_interval(bounds, Hns::new(15_625)), Ok(()));
    }

    #[test]
    fn interval_validation_rejects_zero_and_out_of_range_values() {
        let bounds = TimerBounds {
            minimum_interval: Hns::new(15_625),
            maximum_interval: Hns::new(5_000),
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
    fn finer_effective_observation_satisfies_the_request() {
        let observation = TimerObservation {
            requested: Hns::new(10_000),
            reported_current: Hns::new(9_966),
            raw_status: STATUS_SUCCESS,
        };
        assert!(observation.is_satisfied());
        assert!(observation.is_finer_than_requested());
    }

    #[test]
    fn coarser_effective_observation_is_not_satisfied() {
        let observation = TimerObservation {
            requested: Hns::new(10_000),
            reported_current: Hns::new(10_001),
            raw_status: STATUS_SUCCESS,
        };
        assert!(!observation.is_satisfied());
        assert!(!observation.is_finer_than_requested());
    }

    #[test]
    fn non_windows_adapter_is_explicitly_unsupported() {
        #[cfg(not(windows))]
        assert_eq!(
            WindowsTimerPlatform::default().preflight(Hns::new(10_000)),
            Err(TimerError::Unsupported)
        );
    }

    #[test]
    fn native_boolean_wrapper_uses_windows_values() {
        assert_eq!(nt_boolean(false), 0);
        assert_eq!(nt_boolean(true), 1);
    }

    #[test]
    fn nt_status_values_are_signed_and_preserved() {
        let failure: NtStatus = -1;
        assert_ne!(failure, STATUS_SUCCESS);
        assert_eq!(failure, -1);
    }
}
