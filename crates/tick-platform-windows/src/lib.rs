//! Isolated Windows timer-resolution adapter.
//!
//! The native calls are intentionally kept behind this narrow boundary. A
//! successful call proves API acceptance and the adapter's postcondition only.
//! It does not prove a universal effective system value or exclusive ownership.

use std::sync::Arc;
use tick_core::Hns;
use tick_diagnostics::{
    DiagnosticOutcome, DiagnosticPhase, DiagnosticRecord, DiagnosticSource, DiagnosticStore,
    NativeOutcome, OperationContext,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KernelSettleProbeOutcome {
    Restored,
    ExternalTiming,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KernelSettleProbe {
    pub outcome: KernelSettleProbeOutcome,
    pub before_effective: Hns,
    pub after_effective: Hns,
}

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

/// Hardware crystal divisor quantization can nudge the effective timer value a
/// few 100-nanosecond units above the requested boundary. A small tolerance
/// keeps that physical jitter from being misclassified as an unverified
/// postcondition.
pub const HARDWARE_TIMER_TOLERANCE_HNS: u64 = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimerObservation {
    pub requested: Hns,
    pub reported_current: Hns,
    pub raw_status: NtStatus,
}

impl TimerObservation {
    pub fn is_satisfied(self) -> bool {
        self.reported_current <= self.requested
            || self
                .reported_current
                .value()
                .saturating_sub(self.requested.value())
                <= HARDWARE_TIMER_TOLERANCE_HNS
    }

    pub fn is_finer_than_requested(self) -> bool {
        self.reported_current < self.requested
    }

    pub fn effective_relation(self) -> &'static str {
        if self.reported_current < self.requested {
            "finer"
        } else if self.reported_current == self.requested {
            "equal"
        } else if self
            .reported_current
            .value()
            .saturating_sub(self.requested.value())
            <= HARDWARE_TIMER_TOLERANCE_HNS
        {
            "satisfied"
        } else {
            "unverified"
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimerError {
    Unsupported,
    InvalidInterval,
    InvalidStateTransition,
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
    fn set_operation_context(&mut self, _context: Option<(u64, Option<u64>, u64)>) {}

    fn query(&mut self, interval: Hns) -> Result<TimerQuery, TimerError>;

    fn resolve(&mut self, requested: Hns) -> Result<Hns, TimerError> {
        let query = self.query(requested)?;
        query.resolve_request(requested)
    }

    fn preflight(&mut self, interval: Hns) -> Result<TimerQuery, TimerError>;
    fn request(&mut self, interval: Hns) -> Result<TimerObservation, TimerError>;
    fn release(&mut self, interval: Hns) -> Result<TimerObservation, TimerError>;
    fn attempt_kernel_settle_probe(
        &mut self,
        interval: Hns,
    ) -> Result<KernelSettleProbe, TimerError>;
}

/// Number of timer-resolution samples taken when verifying a request
/// postcondition. Redundant sampling keeps a single anomalous read under
/// power throttling or a driver context switch from producing a false
/// rollback.
pub const VERIFICATION_SAMPLE_COUNT: usize = 3;

/// Returns the middle value of three samples using comparisons only so a
/// single outlier cannot shift the result.
pub fn median_of_three(a: u64, b: u64, c: u64) -> u64 {
    if a > b {
        if b > c {
            b
        } else if a > c {
            c
        } else {
            a
        }
    } else if a > c {
        a
    } else if b > c {
        c
    } else {
        b
    }
}

fn sample_satisfies_tolerance(reported: u64, requested: u64) -> bool {
    reported <= requested || reported.saturating_sub(requested) <= HARDWARE_TIMER_TOLERANCE_HNS
}

/// Issues up to [`VERIFICATION_SAMPLE_COUNT`] queries for the same interval
/// and returns the median of the successful reads.
///
/// Returns `None` when fewer than two reads succeed so the caller treats the
/// postcondition as unverified rather than trusting a single sample. When
/// exactly two reads succeed, the sample the tolerance check would accept is
/// preferred, and the smaller value when both are within tolerance.
pub fn sample_interval_median<P: TimerPlatform>(platform: &mut P, interval: u64) -> Option<u64> {
    let mut samples = [0u64; VERIFICATION_SAMPLE_COUNT];
    let mut successes = 0usize;
    for _ in 0..VERIFICATION_SAMPLE_COUNT {
        if let Ok(query) = platform.query(Hns::new(interval)) {
            samples[successes] = query.reported_current.value();
            successes += 1;
        }
    }
    match successes {
        0 | 1 => None,
        2 => {
            let first_ok = sample_satisfies_tolerance(samples[0], interval);
            let second_ok = sample_satisfies_tolerance(samples[1], interval);
            Some(match (first_ok, second_ok) {
                (true, false) => samples[0],
                (false, true) => samples[1],
                _ => samples[0].min(samples[1]),
            })
        }
        _ => Some(median_of_three(samples[0], samples[1], samples[2])),
    }
}

#[derive(Debug, Default)]
pub struct WindowsTimerPlatform {
    #[cfg(windows)]
    requested: Option<Hns>,
    diagnostics: Option<Arc<DiagnosticStore>>,
    operation_context: Option<(u64, Option<u64>, u64)>,
}

impl WindowsTimerPlatform {
    pub fn with_diagnostics(diagnostics: Arc<DiagnosticStore>) -> Self {
        Self {
            #[cfg(windows)]
            requested: None,
            diagnostics: Some(diagnostics),
            operation_context: None,
        }
    }

    #[cfg(windows)]
    fn apply_release_outcome(
        &mut self,
        interval: Hns,
        status: NtStatus,
        effective: Hns,
    ) -> Result<TimerObservation, TimerError> {
        if status != STATUS_SUCCESS {
            return Err(TimerError::ReleaseFailed { raw_status: status });
        }
        self.requested = None;
        Ok(TimerObservation {
            requested: interval,
            reported_current: effective,
            raw_status: status,
        })
    }

    fn log(&self, name: &str, details: impl AsRef<str>) {
        if let Some(diagnostics) = &self.diagnostics {
            let details = details.as_ref();
            if let Some((operation_id, parent_operation_id, correlation_id)) =
                self.operation_context
            {
                diagnostics.record_with_context(
                    DiagnosticRecord {
                        context: OperationContext {
                            operation_id,
                            parent_operation_id,
                            correlation_id,
                        },
                        phase: if name.contains("release") {
                            DiagnosticPhase::Release
                        } else if name.contains("request") {
                            DiagnosticPhase::Acquire
                        } else {
                            DiagnosticPhase::Observe
                        },
                        source: DiagnosticSource::Native,
                        outcome: if name.contains("error") || name.contains("rejected") {
                            DiagnosticOutcome::Failed
                        } else {
                            DiagnosticOutcome::Completed
                        },
                        native: native_outcome(name, details),
                    },
                    name,
                    details,
                );
            } else {
                let outcome = if name.contains("error")
                    || name.contains("rejected")
                    || name.contains("invalid")
                    || name.contains("unsupported")
                    || name.contains("failed")
                {
                    DiagnosticOutcome::Failed
                } else if name.contains("unverified") {
                    DiagnosticOutcome::Unverified
                } else {
                    DiagnosticOutcome::Completed
                };
                diagnostics.record_with_outcome(name, details, outcome);
            }
        }
    }
}

impl TimerPlatform for WindowsTimerPlatform {
    fn set_operation_context(&mut self, context: Option<(u64, Option<u64>, u64)>) {
        self.operation_context = context;
    }

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
            let sampled_current = sample_interval_median(self, interval.value());
            self.log(
                "timer.postcondition.sampled",
                format!(
                    "requested_hns={} sampled_effective_hns={} sample_count={}",
                    interval.value(),
                    sampled_current.unwrap_or(0),
                    VERIFICATION_SAMPLE_COUNT
                ),
            );
            let observation = TimerObservation {
                requested: interval,
                reported_current: sampled_current
                    .map_or_else(|| Hns::new(current as u64), Hns::new),
                raw_status: status,
            };
            self.requested = Some(interval);
            if sampled_current.is_none() || !observation.is_satisfied() {
                self.log(
                    "timer.postcondition.unverified",
                    format!(
                        "requested_hns={} effective_hns={} effective_relation=unverified",
                        observation.requested.value(),
                        observation.reported_current.value()
                    ),
                );
                let mut rollback_current = 0u32;
                let rollback_status = unsafe {
                    nt_set_timer_resolution(interval.value() as u32, false, &mut rollback_current)
                };
                self.requested = None;
                self.log(
                    "native.NtSetTimerResolution.postcondition_rollback",
                    format!(
                        "raw_status={} requested_hns={} effective_hns={} rollback_status={} rollback_effective_hns={}",
                        status,
                        interval.value(),
                        current,
                        rollback_status,
                        rollback_current
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
                self.log(
                    "timer.release.restorative",
                    format!(
                        "tracked_hns={} requested_hns={} action=restorative_release",
                        self.requested.map(|value| value.value()).unwrap_or(0),
                        interval.value()
                    ),
                );
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
            self.apply_release_outcome(interval, status, Hns::new(current as u64))
        }
        #[cfg(not(windows))]
        {
            self.log("timer.release.unsupported", "platform=non_windows");
            let _ = interval;
            Err(TimerError::Unsupported)
        }
    }

    fn attempt_kernel_settle_probe(
        &mut self,
        interval: Hns,
    ) -> Result<KernelSettleProbe, TimerError> {
        self.log(
            "kernel.self_heal.settle_probe",
            format!("candidate_hns={} set=false", interval.value()),
        );
        #[cfg(windows)]
        {
            let before_effective = query_current_resolution()?;
            let mut current = 0u32;
            let status =
                unsafe { nt_set_timer_resolution(interval.value() as u32, false, &mut current) };
            self.log(
                "native.NtSetTimerResolution.settle_probe",
                format!(
                    "raw_status={} candidate_hns={} effective_hns={} set=false",
                    status,
                    interval.value(),
                    current
                ),
            );
            if status != STATUS_SUCCESS {
                self.log(
                    "kernel.self_heal.settle_probe.failed",
                    format!("raw_status={status}"),
                );
                return Err(TimerError::ReleaseFailed { raw_status: status });
            }
            let immediate_effective = Hns::new(current as u64);
            let after_effective = query_current_resolution()?;
            self.log(
                "kernel.self_heal.settle_probe.result",
                format!(
                    "before_hns={} immediate_hns={} after_hns={} outcome={}",
                    before_effective.value(),
                    immediate_effective.value(),
                    after_effective.value(),
                    if after_effective > before_effective {
                        "restored"
                    } else {
                        "external_timing"
                    }
                ),
            );
            if after_effective > before_effective {
                self.requested = None;
                Ok(KernelSettleProbe {
                    outcome: KernelSettleProbeOutcome::Restored,
                    before_effective,
                    after_effective,
                })
            } else {
                Ok(KernelSettleProbe {
                    outcome: KernelSettleProbeOutcome::ExternalTiming,
                    before_effective,
                    after_effective,
                })
            }
        }
        #[cfg(not(windows))]
        {
            self.log(
                "kernel.self_heal.settle_probe.unsupported",
                "platform=non_windows",
            );
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

#[cfg(windows)]
fn query_current_resolution() -> Result<Hns, TimerError> {
    let (_, current) = query_resolution()?;
    Ok(current)
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

fn native_detail(details: &str, key: &str) -> Option<i64> {
    details.split_whitespace().find_map(|field| {
        let (field_key, value) = field.split_once('=')?;
        (field_key == key)
            .then(|| value.parse::<i64>().ok())
            .flatten()
    })
}

fn native_outcome(name: &str, details: &str) -> NativeOutcome {
    let raw_status = native_detail(details, "raw_status");
    NativeOutcome {
        ntstatus: (name.contains("Nt") || name.starts_with("timer."))
            .then_some(raw_status)
            .flatten()
            .and_then(|value| i32::try_from(value).ok()),
        win32_last_error: (!name.contains("Nt") && !name.starts_with("timer."))
            .then_some(raw_status)
            .flatten()
            .and_then(|value| u32::try_from(value).ok()),
        requested_hns: native_detail(details, "requested_hns").map(|value| value as u64),
        selected_hns: native_detail(details, "selected_hns").map(|value| value as u64),
        effective_hns: native_detail(details, "effective_hns").map(|value| value as u64),
    }
}

fn effective_relation(effective: Hns, requested: Hns) -> &'static str {
    if effective < requested {
        "finer"
    } else if effective == requested {
        "equal"
    } else if effective.value().saturating_sub(requested.value()) <= HARDWARE_TIMER_TOLERANCE_HNS {
        "satisfied"
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
    fn boundary_normalization_is_numeric_regardless_of_arrival_order() {
        // Same pair arriving in each possible order must normalize identically.
        let (forward, _) = native_resolution_values(5_000, 156_250, 9_966);
        let (inverted, _) = native_resolution_values(156_250, 5_000, 9_966);
        assert_eq!(forward.numeric_interval(), inverted.numeric_interval());
        assert_eq!(
            forward.numeric_interval(),
            (Hns::new(5_000), Hns::new(156_250))
        );
        assert_eq!(
            forward.smallest_supported_boundary(),
            inverted.smallest_supported_boundary()
        );

        // Validation outcome for the same probe value must match across orders.
        for bounds in [forward, inverted] {
            assert_eq!(validate_interval(bounds, Hns::new(10_000)), Ok(()));
            assert_eq!(
                validate_interval(bounds, Hns::new(4_999)),
                Err(TimerError::InvalidInterval)
            );
            assert_eq!(
                validate_interval(bounds, Hns::new(156_251)),
                Err(TimerError::InvalidInterval)
            );
        }
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
    fn automatic_selection_rejects_inverted_unavailable_boundaries() {
        // Zero bound arriving in the maximum slot still fails cleanly.
        let query = TimerQuery {
            bounds: TimerBounds {
                minimum_interval: Hns::new(15_625),
                maximum_interval: Hns::ZERO,
            },
            reported_current: Hns::ZERO,
            raw_status: STATUS_SUCCESS,
        };
        assert_eq!(
            query.resolve_request(Hns::ZERO),
            Err(TimerError::InvalidInterval)
        );

        // Bounds beyond the u32 domain fail cleanly even when inverted.
        let query = TimerQuery {
            bounds: TimerBounds {
                minimum_interval: Hns::new(u32::MAX as u64 + 2),
                maximum_interval: Hns::new(u32::MAX as u64 + 1),
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
    fn automatic_selection_resolves_smallest_boundary_for_inverted_pair() {
        let (bounds, current) = native_resolution_values(5_000, 156_250, 9_966);
        let query = TimerQuery {
            bounds,
            reported_current: current,
            raw_status: STATUS_SUCCESS,
        };
        assert_eq!(query.resolve_request(Hns::ZERO), Ok(Hns::new(5_000)));
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
        assert_eq!(
            validate_interval(bounds, Hns::new(u32::MAX as u64 + 1)),
            Err(TimerError::InvalidInterval)
        );
    }

    #[test]
    fn fixed_selection_passes_requested_value_through_unchanged() {
        let (bounds, current) = native_resolution_values(156_250, 5_000, 9_966);
        let query = TimerQuery {
            bounds,
            reported_current: current,
            raw_status: STATUS_SUCCESS,
        };
        assert_eq!(
            query.resolve_request(Hns::new(10_000)),
            Ok(Hns::new(10_000))
        );
        assert_eq!(
            query.resolve_request(Hns::new(200_000)),
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
            reported_current: Hns::new(10_200),
            raw_status: STATUS_SUCCESS,
        };
        assert!(!observation.is_satisfied());
        assert!(!observation.is_finer_than_requested());
    }

    #[test]
    fn hardware_timer_tolerance_accepts_pll_jitter_within_threshold() {
        let observation = TimerObservation {
            requested: Hns::new(5_000),
            reported_current: Hns::new(5_033),
            raw_status: STATUS_SUCCESS,
        };
        assert!(observation.is_satisfied());
        assert!(!observation.is_finer_than_requested());
        assert_eq!(observation.effective_relation(), "satisfied");
        assert_eq!(
            effective_relation(observation.reported_current, observation.requested),
            "satisfied"
        );
    }

    #[test]
    fn hardware_timer_tolerance_boundary_is_inclusive() {
        let at_boundary = TimerObservation {
            requested: Hns::new(5_000),
            reported_current: Hns::new(5_100),
            raw_status: STATUS_SUCCESS,
        };
        assert!(at_boundary.is_satisfied());
        assert_eq!(at_boundary.effective_relation(), "satisfied");

        let beyond_boundary = TimerObservation {
            requested: Hns::new(5_000),
            reported_current: Hns::new(5_101),
            raw_status: STATUS_SUCCESS,
        };
        assert!(!beyond_boundary.is_satisfied());
        assert_eq!(beyond_boundary.effective_relation(), "unverified");
    }

    #[test]
    fn hardware_tolerance_accepts_exactly_one_hundred_hns_overshoot() {
        // A 100 HNS overshoot is the maximum satisfied deviation.
        let observation = TimerObservation {
            requested: Hns::new(10_000),
            reported_current: Hns::new(10_000 + HARDWARE_TIMER_TOLERANCE_HNS),
            raw_status: STATUS_SUCCESS,
        };
        assert!(observation.is_satisfied());
        assert!(!observation.is_finer_than_requested());
        assert_eq!(observation.effective_relation(), "satisfied");

        let overshot = TimerObservation {
            requested: Hns::new(10_000),
            reported_current: Hns::new(10_000 + HARDWARE_TIMER_TOLERANCE_HNS + 1),
            raw_status: STATUS_SUCCESS,
        };
        assert!(!overshot.is_satisfied());
        assert_eq!(overshot.effective_relation(), "unverified");
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
    fn native_diagnostic_types_distinguish_ntstatus_from_win32_status() {
        let nt = native_outcome("timer.resolve", "raw_status=-7 requested_hns=5000");
        assert_eq!(nt.ntstatus, Some(-7));
        assert_eq!(nt.win32_last_error, None);
        let win32 = native_outcome("native.SetTimer.error", "raw_status=1400");
        assert_eq!(win32.ntstatus, None);
        assert_eq!(win32.win32_last_error, Some(1400));
    }

    #[test]
    fn ntstatus_error_mapping_preserves_negative_and_specific_codes() {
        // Simulated NTSTATUS values
        const STATUS_TIMER_RESOLUTION_NOT_SET: NtStatus = 0xC0000245u32 as i32;
        const STATUS_INVALID_PARAMETER: NtStatus = 0xC000000Du32 as i32;
        const STATUS_UNSUCCESSFUL: NtStatus = 0xC0000001u32 as i32;

        let query_err = TimerError::QueryFailed {
            raw_status: STATUS_TIMER_RESOLUTION_NOT_SET,
        };
        assert_eq!(
            query_err,
            TimerError::QueryFailed {
                raw_status: -1073741243,
            }
        );

        let request_err = TimerError::RequestFailed {
            raw_status: STATUS_INVALID_PARAMETER,
        };
        assert_eq!(
            request_err,
            TimerError::RequestFailed {
                raw_status: -1073741811,
            }
        );

        let release_err = TimerError::ReleaseFailed {
            raw_status: STATUS_UNSUCCESSFUL,
        };
        assert_eq!(
            release_err,
            TimerError::ReleaseFailed {
                raw_status: -1073741823,
            }
        );

        let unverified_err = TimerError::PostconditionUnverified {
            raw_status: STATUS_TIMER_RESOLUTION_NOT_SET,
            reported_current: Hns::new(156_250),
        };
        if let TimerError::PostconditionUnverified {
            raw_status,
            reported_current,
        } = unverified_err
        {
            assert_eq!(raw_status, STATUS_TIMER_RESOLUTION_NOT_SET);
            assert_eq!(reported_current, Hns::new(156_250));
        } else {
            panic!("unexpected variant");
        }
    }

    #[test]
    fn invalid_boundary_handling_and_inverted_values() {
        // Reversed boundaries: minimum_interval (coarse) > maximum_interval (fine)
        let inverted_bounds = TimerBounds {
            minimum_interval: Hns::new(156_250),
            maximum_interval: Hns::new(5_000),
        };
        assert_eq!(
            inverted_bounds.numeric_interval(),
            (Hns::new(5_000), Hns::new(156_250))
        );
        assert_eq!(
            inverted_bounds.smallest_supported_boundary(),
            Ok(Hns::new(5_000))
        );

        // Boundary with zero lower bound is rejected
        let zero_lower_bounds = TimerBounds {
            minimum_interval: Hns::ZERO,
            maximum_interval: Hns::new(156_250),
        };
        assert_eq!(
            zero_lower_bounds.smallest_supported_boundary(),
            Err(TimerError::InvalidInterval)
        );

        // Boundary exceeding u32::MAX is rejected
        let excessive_bounds = TimerBounds {
            minimum_interval: Hns::new(u32::MAX as u64 + 1),
            maximum_interval: Hns::new(u32::MAX as u64 + 2),
        };
        assert_eq!(
            excessive_bounds.smallest_supported_boundary(),
            Err(TimerError::InvalidInterval)
        );
        assert_eq!(
            validate_interval(excessive_bounds, Hns::new(u32::MAX as u64 + 1)),
            Err(TimerError::InvalidInterval)
        );

        // Value strictly outside bounds
        let normal_bounds = TimerBounds {
            minimum_interval: Hns::new(5_000),
            maximum_interval: Hns::new(156_250),
        };
        assert_eq!(
            validate_interval(normal_bounds, Hns::new(4_999)),
            Err(TimerError::InvalidInterval)
        );
        assert_eq!(
            validate_interval(normal_bounds, Hns::new(156_251)),
            Err(TimerError::InvalidInterval)
        );
        assert_eq!(
            validate_interval(normal_bounds, Hns::new(u64::MAX)),
            Err(TimerError::InvalidInterval)
        );
    }

    #[test]
    fn simulated_query_failures_in_diagnostics_formatting() {
        const STATUS_TIMER_RESOLUTION_NOT_SET: NtStatus = 0xC0000245u32 as i32;
        let details = format!("raw_status={STATUS_TIMER_RESOLUTION_NOT_SET} requested_hns=5000");
        let outcome = native_outcome("timer.query", &details);
        assert_eq!(outcome.ntstatus, Some(STATUS_TIMER_RESOLUTION_NOT_SET));
        assert_eq!(outcome.win32_last_error, None);
        assert_eq!(outcome.requested_hns, Some(5000));
    }

    #[test]
    fn simulated_request_postcondition_unverified() {
        let observation = TimerObservation {
            requested: Hns::new(5_000),
            reported_current: Hns::new(10_000),
            raw_status: STATUS_SUCCESS,
        };
        assert!(!observation.is_satisfied());
        assert!(!observation.is_finer_than_requested());
        assert_eq!(observation.effective_relation(), "unverified");
    }

    #[cfg(windows)]
    #[test]
    fn release_without_tracked_request_issues_native_call() {
        // Restorative release must not short circuit on missing ownership.
        // A release for an interval that was never tracked should still reach
        // the kernel and clear any stale tracking marker.
        let mut platform = WindowsTimerPlatform::default();
        let result = platform.release(Hns::new(5_000));
        // The kernel call either succeeds (Ok) or fails with a native status
        // (ReleaseFailed). It must not be rejected as InvalidInterval.
        match result {
            Ok(observation) => {
                assert_eq!(observation.requested, Hns::new(5_000));
                assert_eq!(observation.raw_status, STATUS_SUCCESS);
            }
            Err(TimerError::ReleaseFailed { raw_status }) => {
                assert_ne!(raw_status, STATUS_SUCCESS);
            }
            Err(other) => panic!("unexpected error variant: {other:?}"),
        }
        assert_eq!(platform.requested, None);
    }

    #[cfg(windows)]
    #[test]
    fn release_with_mismatched_interval_still_issues_native_call() {
        // Restorative release must fire even when the caller interval differs
        // from the tracked one so a wedged kernel state can be unwound. This
        // exercises the release outcome path directly through a scripted
        // native result so the test is deterministic and does not depend on
        // host timer state.
        let mut platform = WindowsTimerPlatform {
            requested: Some(Hns::new(10_000)),
            ..Default::default()
        };
        let result =
            platform.apply_release_outcome(Hns::new(5_000), STATUS_SUCCESS, Hns::new(156_250));
        assert_eq!(
            result,
            Ok(TimerObservation {
                requested: Hns::new(5_000),
                reported_current: Hns::new(156_250),
                raw_status: STATUS_SUCCESS,
            })
        );
        assert_eq!(platform.requested, None);
    }

    #[cfg(windows)]
    #[test]
    fn postcondition_unverified_clears_tracked_request() {
        // Drive a request for the coarsest supported boundary. The kernel
        // reports the effective current resolution after NtSetTimerResolution.
        // When the reported value lands outside the hardware tolerance the
        // adapter must roll back the acquisition and clear `requested`.
        // This test asserts the invariant that a failed postcondition never
        // leaves a stale tracked interval behind, regardless of which branch
        // the kernel takes on this host.
        let mut platform = WindowsTimerPlatform::default();
        let probe = platform.query(Hns::ZERO);
        let Ok(query) = probe else {
            return;
        };
        let (_, upper) = query.bounds.numeric_interval();
        let candidate = upper;
        if candidate == Hns::ZERO || candidate.value() > u32::MAX as u64 {
            return;
        }
        match platform.request(candidate) {
            Ok(observation) => {
                assert!(observation.is_satisfied());
                assert_eq!(platform.requested, Some(candidate));
            }
            Err(TimerError::PostconditionUnverified { .. }) => {
                assert_eq!(platform.requested, None);
            }
            Err(_) => {
                assert_eq!(platform.requested, None);
            }
        }
    }

    struct ScriptedPlatform {
        script: std::collections::VecDeque<Result<TimerQuery, TimerError>>,
    }

    impl ScriptedPlatform {
        fn returning(results: impl IntoIterator<Item = Result<TimerQuery, TimerError>>) -> Self {
            Self {
                script: results.into_iter().collect(),
            }
        }

        fn read(value: u64) -> Result<TimerQuery, TimerError> {
            Ok(TimerQuery {
                bounds: TimerBounds {
                    minimum_interval: Hns::new(156_250),
                    maximum_interval: Hns::new(5_000),
                },
                reported_current: Hns::new(value),
                raw_status: STATUS_SUCCESS,
            })
        }

        fn failed() -> Result<TimerQuery, TimerError> {
            Err(TimerError::QueryFailed {
                raw_status: 0xC000_0001_u32 as i32,
            })
        }
    }

    impl TimerPlatform for ScriptedPlatform {
        fn query(&mut self, _interval: Hns) -> Result<TimerQuery, TimerError> {
            self.script.pop_front().unwrap_or_else(Self::failed)
        }

        fn preflight(&mut self, _interval: Hns) -> Result<TimerQuery, TimerError> {
            Err(TimerError::Unsupported)
        }

        fn request(&mut self, _interval: Hns) -> Result<TimerObservation, TimerError> {
            Err(TimerError::Unsupported)
        }

        fn release(&mut self, _interval: Hns) -> Result<TimerObservation, TimerError> {
            Err(TimerError::Unsupported)
        }

        fn attempt_kernel_settle_probe(
            &mut self,
            _interval: Hns,
        ) -> Result<KernelSettleProbe, TimerError> {
            Err(TimerError::Unsupported)
        }
    }

    #[test]
    fn median_of_three_returns_middle_value() {
        assert_eq!(median_of_three(5_000, 5_010, 5_020), 5_010);
        assert_eq!(median_of_three(5_020, 5_000, 5_010), 5_010);
        assert_eq!(median_of_three(5_010, 5_020, 5_000), 5_010);
        assert_eq!(median_of_three(5_000, 5_000, 5_000), 5_000);
    }

    #[test]
    fn median_of_three_ignores_single_outlier_high() {
        assert_eq!(median_of_three(5_000, 156_250, 5_050), 5_050);
        assert_eq!(median_of_three(156_250, 5_000, 5_050), 5_050);
        assert_eq!(median_of_three(5_050, 5_000, 156_250), 5_050);
    }

    #[test]
    fn median_of_three_ignores_single_outlier_low() {
        assert_eq!(median_of_three(5_000, 5_050, 1), 5_000);
        assert_eq!(median_of_three(1, 5_050, 5_000), 5_000);
        assert_eq!(median_of_three(5_050, 1, 5_000), 5_000);
    }

    #[test]
    fn sample_median_returns_none_when_fewer_than_two_reads_succeed() {
        let mut all_fail = ScriptedPlatform::returning([
            ScriptedPlatform::failed(),
            ScriptedPlatform::failed(),
            ScriptedPlatform::failed(),
        ]);
        assert_eq!(sample_interval_median(&mut all_fail, 5_000), None);

        let mut one_success = ScriptedPlatform::returning([
            ScriptedPlatform::read(5_000),
            ScriptedPlatform::failed(),
            ScriptedPlatform::failed(),
        ]);
        assert_eq!(sample_interval_median(&mut one_success, 5_000), None);
    }

    #[test]
    fn sample_median_suppresses_single_anomalous_read() {
        let mut platform = ScriptedPlatform::returning([
            ScriptedPlatform::read(5_000),
            ScriptedPlatform::read(156_250),
            ScriptedPlatform::read(5_010),
        ]);
        assert_eq!(sample_interval_median(&mut platform, 5_000), Some(5_010));

        let mut low_outlier = ScriptedPlatform::returning([
            ScriptedPlatform::read(5_000),
            ScriptedPlatform::read(1),
            ScriptedPlatform::read(5_010),
        ]);
        assert_eq!(sample_interval_median(&mut low_outlier, 5_000), Some(5_000));
    }

    #[test]
    fn verification_still_rolls_back_when_all_reads_are_out_of_tolerance() {
        let interval = 5_000u64;
        let mut platform = ScriptedPlatform::returning([
            ScriptedPlatform::read(156_250),
            ScriptedPlatform::read(156_200),
            ScriptedPlatform::read(156_100),
        ]);
        let sampled = sample_interval_median(&mut platform, interval)
            .expect("three successful reads must yield a median");
        let observation = TimerObservation {
            requested: Hns::new(interval),
            reported_current: Hns::new(sampled),
            raw_status: STATUS_SUCCESS,
        };
        assert!(!observation.is_satisfied());
        assert_eq!(observation.effective_relation(), "unverified");
    }
}
