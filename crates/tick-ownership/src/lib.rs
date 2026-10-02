//! Serialized ownership bookkeeping for one Tick runtime instance.
//!
//! The controller records only a successful adapter request. It never infers
//! ownership from an effective value and never writes a guessed global default.

use tick_core::{CoreError, Hns, Status};
use tick_platform_windows::{
    KernelSettleProbeOutcome, NtStatus, TimerError, TimerObservation, TimerPlatform,
};

/// Maximum effective resolution that still proves a prior fine token is held.
/// Any value at or below this boundary is treated as a high-resolution state
/// during the startup kernel settle probe.
pub const KERNEL_SETTLE_HIGH_RESOLUTION_THRESHOLD_HNS: Hns =
    Hns::new(tick_core::HNS_PER_MILLISECOND);

/// Candidate interval used by the startup kernel settle probe release call.
pub const KERNEL_SETTLE_PROBE_CANDIDATE_INTERVAL_HNS: Hns =
    Hns::new(tick_core::HNS_PER_MILLISECOND / 2);

/// Maximum number of startup settle probe attempts in the Tier 1 budget.
pub const SETTLE_PROBE_ATTEMPTS: u32 = 3;

/// Fixed spacing in milliseconds between settle probe attempts.
/// The ownership crate never sleeps, the caller supplies retry timing.
pub const SETTLE_PROBE_SPACING_MS: u64 = 250;

/// Remaining settle probe attempts after `attempts_used` have been consumed.
pub const fn settle_probe_budget_remaining(attempts_used: u32) -> u32 {
    SETTLE_PROBE_ATTEMPTS.saturating_sub(attempts_used)
}

/// Terminal outcome of the bounded startup kernel settle probe.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettleProbeOutcome {
    /// A restorative probe release moved the effective resolution coarser by
    /// `freed_hns`, proving an orphaned token from a prior ungraceful exit
    /// was dropped.
    Resolved { freed_hns: u64 },
    /// The effective resolution was already at or above the coarse baseline,
    /// or a probe confirmed an external client legitimately holds the fine
    /// resolution. No further attempt can change the classification.
    ExternalClientConfirmed,
    /// Every attempt in `SETTLE_PROBE_ATTEMPTS` was consumed without a change
    /// in the effective resolution.
    Exhausted,
}

/// Result of driving one startup settle probe attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettleProbeStep {
    /// The probe does not apply, no effective observation exists or tracked
    /// ownership is still held.
    NotApplicable,
    /// The attempt was consumed without a resolution change. The budget
    /// still permits another attempt after the caller waits
    /// `SETTLE_PROBE_SPACING_MS`.
    Retry { budget_remaining: u32 },
    /// The probe reached a terminal classification.
    Settled(SettleProbeOutcome),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OwnershipState {
    Released,
    Owned,
    Uncertain,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Transition {
    Acquire,
    Release,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ownership {
    state: OwnershipState,
}

impl Ownership {
    pub const fn new() -> Self {
        Self {
            state: OwnershipState::Released,
        }
    }

    pub const fn state(self) -> OwnershipState {
        self.state
    }

    pub fn apply(&mut self, transition: Transition) -> Result<bool, CoreError> {
        let changed = match (self.state, transition) {
            (OwnershipState::Released, Transition::Acquire) => {
                self.state = OwnershipState::Owned;
                true
            }
            (OwnershipState::Owned | OwnershipState::Uncertain, Transition::Release) => {
                self.state = OwnershipState::Released;
                true
            }
            (OwnershipState::Owned | OwnershipState::Uncertain, Transition::Acquire)
            | (OwnershipState::Released, Transition::Release) => false,
        };
        Ok(changed)
    }

    pub fn mark_uncertain(&mut self) {
        self.state = OwnershipState::Uncertain;
    }

    pub const fn status(self) -> Status {
        match self.state {
            OwnershipState::Released => Status::Released,
            OwnershipState::Owned => Status::Active,
            OwnershipState::Uncertain => Status::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Verification {
    NotCollected,
    Verified,
    FinerThanRequested,
    Unverified,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TimingSnapshot {
    pub requested: Option<Hns>,
    pub selected: Option<Hns>,
    pub effective: Option<Hns>,
    pub minimum_interval: Option<Hns>,
    pub maximum_interval: Option<Hns>,
    pub raw_status: Option<NtStatus>,
}

impl TimingSnapshot {
    pub fn effective_relation(self) -> Option<&'static str> {
        let boundary = self.selected.or(self.requested);
        match (boundary, self.effective) {
            (Some(requested), Some(effective)) if effective < requested => Some("finer"),
            (Some(requested), Some(effective)) if effective == requested => Some("equal"),
            (Some(_), Some(_)) => Some("unverified"),
            _ => None,
        }
    }

    pub fn invalidate_effective(&mut self) {
        self.effective = None;
        self.raw_status = None;
    }
}

#[derive(Debug)]
pub struct TimerController<P> {
    platform: P,
    ownership: Ownership,
    requested_interval: Hns,
    selected_interval: Option<Hns>,
    release_boundary: Option<Hns>,
    verification: Verification,
    observation: Option<TimerObservation>,
    snapshot: TimingSnapshot,
}

impl<P: TimerPlatform> TimerController<P> {
    pub fn set_operation_context(&mut self, context: Option<(u64, Option<u64>, u64)>) {
        self.platform.set_operation_context(context);
    }

    pub fn new(platform: P, requested_interval: Hns) -> Self {
        Self {
            platform,
            ownership: Ownership::new(),
            requested_interval,
            selected_interval: None,
            release_boundary: None,
            verification: Verification::NotCollected,
            observation: None,
            snapshot: TimingSnapshot {
                requested: None,
                selected: None,
                effective: None,
                minimum_interval: None,
                maximum_interval: None,
                raw_status: None,
            },
        }
    }

    fn selected_for_query(&mut self) -> Result<Hns, TimerError> {
        if let Some(interval) = self.release_boundary {
            return Ok(interval);
        }
        if let Some(interval) = self.selected_interval {
            return Ok(interval);
        }
        let interval = self.platform.resolve(self.requested_interval)?;
        self.selected_interval = Some(interval);
        Ok(interval)
    }

    fn record_query(&mut self, interval: Hns, reported_current: Hns, raw_status: NtStatus) {
        self.observation = Some(TimerObservation {
            requested: interval,
            reported_current,
            raw_status,
        });
        self.snapshot.requested = Some(interval);
        self.snapshot.effective = Some(reported_current);
        self.snapshot.raw_status = Some(raw_status);
    }

    fn invalidate_query(&mut self) {
        self.observation = None;
        self.snapshot.invalidate_effective();
    }

    pub fn query(&mut self) -> Result<TimerObservation, TimerError> {
        let interval = match self.selected_for_query() {
            Ok(interval) => interval,
            Err(error) => {
                self.invalidate_query();
                return Err(error);
            }
        };
        let query = match self.platform.query(interval) {
            Ok(query) => query,
            Err(error) => {
                self.invalidate_query();
                return Err(error);
            }
        };
        self.snapshot.selected = self.selected_interval.or(self.release_boundary);
        self.snapshot.minimum_interval = Some(query.bounds.minimum_interval);
        self.snapshot.maximum_interval = Some(query.bounds.maximum_interval);
        self.record_query(interval, query.reported_current, query.raw_status);
        Ok(self.observation.expect("query observation was recorded"))
    }

    /// Observe current timing using an existing release boundary.
    ///
    /// This never resolves a new request interval. It is used only by the
    /// bounded release handoff watcher.
    pub fn observe_current(&mut self, boundary: Hns) -> Result<TimerObservation, TimerError> {
        if self.release_boundary != Some(boundary) {
            return Err(TimerError::InvalidInterval);
        }
        let query = match self.platform.query(boundary) {
            Ok(query) => query,
            Err(error) => {
                self.invalidate_query();
                return Err(error);
            }
        };
        self.snapshot.selected = Some(boundary);
        self.snapshot.minimum_interval = Some(query.bounds.minimum_interval);
        self.snapshot.maximum_interval = Some(query.bounds.maximum_interval);
        self.record_query(boundary, query.reported_current, query.raw_status);
        Ok(self.observation.expect("handoff observation was recorded"))
    }

    pub fn start(&mut self) -> Result<Verification, TimerError> {
        if self.ownership.state() != OwnershipState::Released
            || self.verification == Verification::Unverified
        {
            return Ok(self.verification);
        }
        let interval = match self.platform.resolve(self.requested_interval) {
            Ok(interval) => interval,
            Err(error) => {
                self.invalidate_query();
                return Err(error);
            }
        };
        self.selected_interval = Some(interval);
        self.release_boundary = None;
        let query = match self.platform.preflight(interval) {
            Ok(query) => query,
            Err(error) => {
                self.invalidate_query();
                return Err(error);
            }
        };
        self.snapshot.selected = Some(interval);
        self.snapshot.minimum_interval = Some(query.bounds.minimum_interval);
        self.snapshot.maximum_interval = Some(query.bounds.maximum_interval);
        self.record_query(interval, query.reported_current, query.raw_status);
        let request_observation = match self.platform.request(interval) {
            Ok(observation) => observation,
            Err(error) => {
                if let TimerError::PostconditionUnverified {
                    raw_status,
                    reported_current,
                } = error
                {
                    self.snapshot.selected = Some(interval);
                    self.record_query(interval, reported_current, raw_status);
                    self.ownership.mark_uncertain();
                    self.verification = Verification::Unverified;
                }
                return Err(error);
            }
        };
        self.snapshot.selected = Some(interval);
        self.record_query(
            request_observation.requested,
            request_observation.reported_current,
            request_observation.raw_status,
        );
        self.ownership
            .apply(Transition::Acquire)
            .map_err(|_| TimerError::InvalidStateTransition)?;
        self.verification = verification_for_observation(request_observation);
        Ok(self.verification)
    }

    pub fn stop(&mut self) -> Result<bool, TimerError> {
        if self.ownership.state() == OwnershipState::Released {
            return Ok(false);
        }
        let interval = self.selected_interval.ok_or(TimerError::InvalidInterval)?;
        let observation = match self.platform.release(interval) {
            Ok(observation) => observation,
            Err(error) => {
                self.verification = Verification::Unverified;
                return Err(error);
            }
        };
        self.snapshot.selected = Some(interval);
        self.record_query(
            observation.requested,
            observation.reported_current,
            observation.raw_status,
        );
        self.ownership
            .apply(Transition::Release)
            .map_err(|_| TimerError::InvalidStateTransition)?;
        self.verification = Verification::NotCollected;
        self.release_boundary = Some(interval);
        self.selected_interval = None;
        Ok(true)
    }

    pub const fn ownership(&self) -> OwnershipState {
        self.ownership.state()
    }

    pub const fn verification(&self) -> Verification {
        self.verification
    }

    pub const fn observation(&self) -> Option<TimerObservation> {
        self.observation
    }

    pub const fn snapshot(&self) -> TimingSnapshot {
        self.snapshot
    }

    pub const fn selected_interval(&self) -> Option<Hns> {
        self.selected_interval
    }

    pub const fn requested_interval(&self) -> Hns {
        self.requested_interval
    }

    /// Raises the requested interval floor so future acquisitions never
    /// resolve a value finer than `floor`. A resolved selection below the
    /// floor is discarded so the next acquisition resolves again.
    pub fn clamp_requested_interval_floor(&mut self, floor: Hns) {
        if self.requested_interval < floor {
            self.requested_interval = floor;
        }
        if self.selected_interval.is_some_and(|value| value < floor) {
            self.selected_interval = None;
        }
    }

    pub const fn release_boundary(&self) -> Option<Hns> {
        self.release_boundary
    }

    /// Ends the release-observation window without changing the last snapshot.
    pub fn clear_release_boundary(&mut self) {
        self.release_boundary = None;
        self.snapshot.selected = None;
    }

    /// Drive one attempt of the startup kernel settle probe.
    ///
    /// True Tick presumes zero prior ownership at startup. When the effective
    /// resolution is already fine while ownership is released, the probe
    /// issues a native release with `SetResolution = FALSE` on the candidate
    /// interval and compares the recomputed effective value. A coarser move
    /// proves an orphaned token was dropped and settles to `Resolved` with
    /// the freed delta. An unchanged probe confirms external timing and
    /// settles to `ExternalClientConfirmed`. An effective value already at
    /// or above the coarse baseline confirms external timing immediately
    /// without a platform call.
    ///
    /// `attempts_used` is the number of attempts the caller has already
    /// consumed. When an attempt ends without a change the step is `Retry`
    /// while budget remains, or `Settled(Exhausted)` once
    /// `SETTLE_PROBE_ATTEMPTS` is fully consumed. The probe never sleeps,
    /// the caller applies `SETTLE_PROBE_SPACING_MS` between attempts.
    pub fn attempt_startup_kernel_settle_probe(&mut self, attempts_used: u32) -> SettleProbeStep {
        let Some(effective) = self.snapshot.effective else {
            return SettleProbeStep::NotApplicable;
        };
        if self.ownership.state() != OwnershipState::Released {
            return SettleProbeStep::NotApplicable;
        }
        if effective > KERNEL_SETTLE_HIGH_RESOLUTION_THRESHOLD_HNS {
            return SettleProbeStep::Settled(SettleProbeOutcome::ExternalClientConfirmed);
        }
        if settle_probe_budget_remaining(attempts_used) == 0 {
            return SettleProbeStep::Settled(SettleProbeOutcome::Exhausted);
        }
        let attempts_consumed = attempts_used.saturating_add(1);
        let probe = match self
            .platform
            .attempt_kernel_settle_probe(KERNEL_SETTLE_PROBE_CANDIDATE_INTERVAL_HNS)
        {
            Ok(probe) => probe,
            Err(_) => return settle_probe_no_change(attempts_consumed),
        };
        self.observation = Some(TimerObservation {
            requested: KERNEL_SETTLE_PROBE_CANDIDATE_INTERVAL_HNS,
            reported_current: probe.after_effective,
            raw_status: 0,
        });
        self.snapshot.requested = Some(KERNEL_SETTLE_PROBE_CANDIDATE_INTERVAL_HNS);
        self.snapshot.effective = Some(probe.after_effective);
        self.snapshot.raw_status = Some(0);
        self.release_boundary = None;
        self.selected_interval = None;
        match probe.outcome {
            KernelSettleProbeOutcome::Restored
                if probe.after_effective > probe.before_effective =>
            {
                SettleProbeStep::Settled(SettleProbeOutcome::Resolved {
                    freed_hns: probe
                        .after_effective
                        .value()
                        .saturating_sub(probe.before_effective.value()),
                })
            }
            KernelSettleProbeOutcome::ExternalTiming => {
                SettleProbeStep::Settled(SettleProbeOutcome::ExternalClientConfirmed)
            }
            _ => settle_probe_no_change(attempts_consumed),
        }
    }

    pub const fn status(&self) -> Status {
        self.ownership.status()
    }
}

fn settle_probe_no_change(attempts_consumed: u32) -> SettleProbeStep {
    let budget_remaining = settle_probe_budget_remaining(attempts_consumed);
    if budget_remaining == 0 {
        SettleProbeStep::Settled(SettleProbeOutcome::Exhausted)
    } else {
        SettleProbeStep::Retry { budget_remaining }
    }
}

fn verification_for_observation(observation: TimerObservation) -> Verification {
    if observation.is_finer_than_requested() {
        Verification::FinerThanRequested
    } else {
        Verification::Verified
    }
}

impl Default for Ownership {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tick_platform_windows::{
        KernelSettleProbe, KernelSettleProbeOutcome, TimerBounds, TimerQuery,
    };

    #[derive(Debug)]
    struct FixturePlatform {
        request_observation: TimerObservation,
        release_result: Result<TimerObservation, TimerError>,
        query_results: Vec<Result<TimerQuery, TimerError>>,
        settle_probe_result: Result<KernelSettleProbe, TimerError>,
    }

    impl FixturePlatform {
        fn query_result(&mut self, _interval: Hns) -> TimerQuery {
            TimerQuery {
                bounds: TimerBounds {
                    minimum_interval: Hns::new(156_250),
                    maximum_interval: Hns::new(5_000),
                },
                reported_current: Hns::new(9_966),
                raw_status: 0,
            }
        }

        fn next_query(&mut self, interval: Hns) -> Result<TimerQuery, TimerError> {
            if self.query_results.is_empty() {
                Ok(self.query_result(interval))
            } else {
                self.query_results.remove(0)
            }
        }
    }

    impl TimerPlatform for FixturePlatform {
        fn query(&mut self, interval: Hns) -> Result<TimerQuery, TimerError> {
            self.next_query(interval)
        }

        fn preflight(&mut self, interval: Hns) -> Result<TimerQuery, TimerError> {
            self.next_query(interval)
        }

        fn request(&mut self, _interval: Hns) -> Result<TimerObservation, TimerError> {
            Ok(self.request_observation)
        }

        fn release(&mut self, _interval: Hns) -> Result<TimerObservation, TimerError> {
            self.release_result
        }

        fn attempt_kernel_settle_probe(
            &mut self,
            _interval: Hns,
        ) -> Result<KernelSettleProbe, TimerError> {
            self.settle_probe_result
        }
    }

    #[derive(Debug)]
    struct RecordingPlatform {
        intervals: Vec<Hns>,
    }

    impl TimerPlatform for RecordingPlatform {
        fn query(&mut self, interval: Hns) -> Result<TimerQuery, TimerError> {
            self.intervals.push(interval);
            Ok(TimerQuery {
                bounds: TimerBounds {
                    minimum_interval: Hns::new(5_000),
                    maximum_interval: Hns::new(156_250),
                },
                reported_current: Hns::new(4_966),
                raw_status: 0,
            })
        }

        fn preflight(&mut self, interval: Hns) -> Result<TimerQuery, TimerError> {
            self.query(interval)
        }

        fn request(&mut self, interval: Hns) -> Result<TimerObservation, TimerError> {
            Ok(TimerObservation {
                requested: interval,
                reported_current: interval,
                raw_status: 0,
            })
        }

        fn release(&mut self, interval: Hns) -> Result<TimerObservation, TimerError> {
            Ok(TimerObservation {
                requested: interval,
                reported_current: Hns::new(4_966),
                raw_status: 0,
            })
        }

        fn attempt_kernel_settle_probe(
            &mut self,
            _interval: Hns,
        ) -> Result<KernelSettleProbe, TimerError> {
            Err(TimerError::Unsupported)
        }
    }

    impl<P> TimerController<P> {
        fn into_platform(self) -> P {
            self.platform
        }
    }

    fn fixture_controller() -> TimerController<FixturePlatform> {
        TimerController::new(
            FixturePlatform {
                request_observation: TimerObservation {
                    requested: Hns::new(5_000),
                    reported_current: Hns::new(4_966),
                    raw_status: 0,
                },
                release_result: Ok(TimerObservation {
                    requested: Hns::new(5_000),
                    reported_current: Hns::new(4_966),
                    raw_status: 0,
                }),
                query_results: Vec::new(),
                settle_probe_result: Err(TimerError::Unsupported),
            },
            Hns::new(5_000),
        )
    }

    #[test]
    fn acquisition_and_release_are_matching_transitions() {
        let mut ownership = Ownership::new();
        assert!(ownership.apply(Transition::Acquire).unwrap());
        assert_eq!(ownership.state(), OwnershipState::Owned);
        assert!(ownership.apply(Transition::Release).unwrap());
        assert_eq!(ownership.state(), OwnershipState::Released);
    }

    #[test]
    fn duplicate_transitions_are_idempotent() {
        let mut ownership = Ownership::new();
        assert!(ownership.apply(Transition::Acquire).unwrap());
        assert!(!ownership.apply(Transition::Acquire).unwrap());
        assert!(ownership.apply(Transition::Release).unwrap());
        assert!(!ownership.apply(Transition::Release).unwrap());
    }

    #[test]
    fn release_without_tracked_ownership_is_a_noop() {
        let mut ownership = Ownership::new();
        assert!(!ownership.apply(Transition::Release).unwrap());
        assert_eq!(ownership.state(), OwnershipState::Released);
    }

    #[test]
    fn uncertain_ownership_blocks_acquisition_until_controlled_release() {
        let mut ownership = Ownership::new();
        ownership.mark_uncertain();
        assert_eq!(ownership.state(), OwnershipState::Uncertain);
        assert!(!ownership.apply(Transition::Acquire).unwrap());
        assert!(ownership.apply(Transition::Release).unwrap());
        assert_eq!(ownership.state(), OwnershipState::Released);
    }

    #[test]
    fn startup_controller_presumes_zero_prior_ownership_without_writing_defaults() {
        let controller = fixture_controller();
        assert_eq!(controller.ownership(), OwnershipState::Released);
        assert_eq!(controller.selected_interval(), None);
        assert_eq!(controller.release_boundary(), None);
        assert_eq!(controller.verification(), Verification::NotCollected);
        assert_eq!(controller.observation(), None);
        assert_eq!(controller.snapshot(), TimingSnapshot::default());
        assert_eq!(controller.status(), Status::Released);
    }

    #[test]
    fn failed_or_uncertain_release_keeps_ownership_tracked_for_retry() {
        let mut controller = TimerController::new(
            FixturePlatform {
                request_observation: TimerObservation {
                    requested: Hns::new(5_000),
                    reported_current: Hns::new(4_966),
                    raw_status: 0,
                },
                release_result: Err(TimerError::ReleaseFailed { raw_status: -1 }),
                query_results: Vec::new(),
                settle_probe_result: Err(TimerError::Unsupported),
            },
            Hns::new(5_000),
        );
        controller.start().unwrap();
        assert_eq!(controller.ownership(), OwnershipState::Owned);
        assert_eq!(
            controller.stop(),
            Err(TimerError::ReleaseFailed { raw_status: -1 })
        );
        assert_eq!(controller.ownership(), OwnershipState::Owned);
        assert_eq!(controller.selected_interval(), Some(Hns::new(5_000)));
        assert_eq!(controller.verification(), Verification::Unverified);
    }

    #[test]
    fn startup_query_observes_current_timing_without_ownership() {
        let mut controller = fixture_controller();
        let observation = controller.query().unwrap();
        assert_eq!(observation.reported_current, Hns::new(9_966));
        assert_eq!(controller.ownership(), OwnershipState::Released);
        assert_eq!(controller.observation(), Some(observation));
    }

    #[test]
    fn startup_query_failure_keeps_timing_unknown_and_released() {
        let mut controller = TimerController::new(
            FixturePlatform {
                request_observation: TimerObservation {
                    requested: Hns::new(5_000),
                    reported_current: Hns::new(4_966),
                    raw_status: 0,
                },
                release_result: Ok(TimerObservation {
                    requested: Hns::new(5_000),
                    reported_current: Hns::new(4_966),
                    raw_status: 0,
                }),
                query_results: vec![Err(TimerError::QueryFailed { raw_status: -7 })],
                settle_probe_result: Err(TimerError::Unsupported),
            },
            Hns::new(5_000),
        );
        assert_eq!(
            controller.query(),
            Err(TimerError::QueryFailed { raw_status: -7 })
        );
        assert_eq!(controller.observation(), None);
        assert_eq!(controller.ownership(), OwnershipState::Released);
    }

    #[test]
    fn query_failure_invalidates_a_previous_effective_observation() {
        let query = TimerQuery {
            bounds: TimerBounds {
                minimum_interval: Hns::new(156_250),
                maximum_interval: Hns::new(5_000),
            },
            reported_current: Hns::new(9_966),
            raw_status: 0,
        };
        let mut controller = TimerController::new(
            FixturePlatform {
                request_observation: TimerObservation {
                    requested: Hns::new(5_000),
                    reported_current: Hns::new(4_966),
                    raw_status: 0,
                },
                release_result: Ok(TimerObservation {
                    requested: Hns::new(5_000),
                    reported_current: Hns::new(4_966),
                    raw_status: 0,
                }),
                query_results: vec![
                    Ok(query),
                    Ok(query),
                    Err(TimerError::QueryFailed { raw_status: -8 }),
                ],
                settle_probe_result: Err(TimerError::Unsupported),
            },
            Hns::new(5_000),
        );
        controller.query().unwrap();
        assert_eq!(controller.snapshot().effective, Some(Hns::new(9_966)));
        assert_eq!(
            controller.query(),
            Err(TimerError::QueryFailed { raw_status: -8 })
        );
        assert_eq!(controller.snapshot().effective, None);
        assert_eq!(controller.observation(), None);
    }

    #[test]
    fn handoff_observes_without_resolving_a_new_release_boundary() {
        let mut controller = TimerController::new(
            RecordingPlatform {
                intervals: Vec::new(),
            },
            Hns::new(5_000),
        );
        controller.start().unwrap();
        controller.stop().unwrap();
        assert_eq!(controller.release_boundary(), Some(Hns::new(5_000)));
        controller.observe_current(Hns::new(5_000)).unwrap();
        let intervals = controller.into_platform().intervals;
        assert_eq!(intervals.last(), Some(&Hns::new(5_000)));
    }

    #[test]
    fn successful_request_replaces_preflight_effective_observation() {
        let mut controller = fixture_controller();
        assert_eq!(
            controller.query().unwrap().reported_current,
            Hns::new(9_966)
        );
        assert_eq!(controller.start(), Ok(Verification::FinerThanRequested));
        assert_eq!(
            controller.observation().unwrap().reported_current,
            Hns::new(4_966)
        );
        assert_eq!(controller.observation().unwrap().raw_status, 0);
    }

    #[test]
    fn successful_release_updates_the_effective_observation() {
        let mut controller = fixture_controller();
        controller.start().unwrap();
        assert_eq!(controller.stop(), Ok(true));
        let observation = controller.observation().unwrap();
        assert_eq!(observation.reported_current, Hns::new(4_966));
        assert_eq!(observation.requested, Hns::new(5_000));
        assert_eq!(observation.raw_status, 0);
        assert_eq!(controller.ownership(), OwnershipState::Released);
    }

    #[test]
    fn released_external_effective_value_can_return_to_baseline() {
        let mut controller = fixture_controller();
        controller.start().unwrap();
        controller.stop().unwrap();
        assert_eq!(
            controller.observation().unwrap().reported_current,
            Hns::new(4_966)
        );
        assert_eq!(
            controller.query().unwrap().reported_current,
            Hns::new(9_966)
        );
        assert_eq!(controller.ownership(), OwnershipState::Released);
    }

    #[test]
    fn failed_release_keeps_ownership_and_marks_verification_unverified() {
        let mut controller = TimerController::new(
            FixturePlatform {
                request_observation: TimerObservation {
                    requested: Hns::new(5_000),
                    reported_current: Hns::new(4_966),
                    raw_status: 0,
                },
                release_result: Err(TimerError::ReleaseFailed { raw_status: -1 }),
                query_results: Vec::new(),
                settle_probe_result: Err(TimerError::Unsupported),
            },
            Hns::new(5_000),
        );
        controller.start().unwrap();
        assert_eq!(
            controller.stop(),
            Err(TimerError::ReleaseFailed { raw_status: -1 })
        );
        assert_eq!(controller.ownership(), OwnershipState::Owned);
        assert_eq!(controller.verification(), Verification::Unverified);
    }

    #[test]
    fn finer_observation_is_a_satisfied_verification_state() {
        let observation = TimerObservation {
            requested: Hns::new(10_000),
            reported_current: Hns::new(4_966),
            raw_status: 0,
        };
        assert_eq!(
            verification_for_observation(observation),
            Verification::FinerThanRequested
        );
    }

    #[test]
    fn coarser_observation_is_not_satisfied() {
        let observation = TimerObservation {
            requested: Hns::new(10_000),
            reported_current: Hns::new(10_200),
            raw_status: 0,
        };
        assert!(!observation.is_satisfied());
    }

    #[test]
    fn settle_probe_unwedges_an_orphaned_token_to_clean_released() {
        let query = TimerQuery {
            bounds: TimerBounds {
                minimum_interval: Hns::new(156_250),
                maximum_interval: Hns::new(5_000),
            },
            reported_current: Hns::new(4_966),
            raw_status: 0,
        };
        let mut controller = TimerController::new(
            FixturePlatform {
                request_observation: TimerObservation {
                    requested: Hns::new(5_000),
                    reported_current: Hns::new(4_966),
                    raw_status: 0,
                },
                release_result: Ok(TimerObservation {
                    requested: Hns::new(5_000),
                    reported_current: Hns::new(4_966),
                    raw_status: 0,
                }),
                query_results: vec![Ok(query)],
                settle_probe_result: Ok(KernelSettleProbe {
                    outcome: KernelSettleProbeOutcome::Restored,
                    before_effective: Hns::new(4_966),
                    after_effective: Hns::new(156_250),
                }),
            },
            Hns::new(5_000),
        );
        controller.query().unwrap();
        assert_eq!(
            controller.attempt_startup_kernel_settle_probe(0),
            SettleProbeStep::Settled(SettleProbeOutcome::Resolved { freed_hns: 151_284 })
        );
        assert_eq!(controller.ownership(), OwnershipState::Released);
        assert_eq!(controller.snapshot().effective, Some(Hns::new(156_250)));
        assert_eq!(controller.release_boundary(), None);
        assert_eq!(controller.selected_interval(), None);
    }

    #[test]
    fn settle_probe_confirms_external_timing_when_resolution_is_held() {
        let query = TimerQuery {
            bounds: TimerBounds {
                minimum_interval: Hns::new(156_250),
                maximum_interval: Hns::new(5_000),
            },
            reported_current: Hns::new(4_966),
            raw_status: 0,
        };
        let mut controller = TimerController::new(
            FixturePlatform {
                request_observation: TimerObservation {
                    requested: Hns::new(5_000),
                    reported_current: Hns::new(4_966),
                    raw_status: 0,
                },
                release_result: Ok(TimerObservation {
                    requested: Hns::new(5_000),
                    reported_current: Hns::new(4_966),
                    raw_status: 0,
                }),
                query_results: vec![Ok(query)],
                settle_probe_result: Ok(KernelSettleProbe {
                    outcome: KernelSettleProbeOutcome::ExternalTiming,
                    before_effective: Hns::new(4_966),
                    after_effective: Hns::new(4_966),
                }),
            },
            Hns::new(5_000),
        );
        controller.query().unwrap();
        assert_eq!(
            controller.attempt_startup_kernel_settle_probe(0),
            SettleProbeStep::Settled(SettleProbeOutcome::ExternalClientConfirmed)
        );
        assert_eq!(controller.ownership(), OwnershipState::Released);
        assert_eq!(controller.snapshot().effective, Some(Hns::new(4_966)));
    }

    #[test]
    fn settle_probe_budget_decrements_to_zero() {
        assert_eq!(settle_probe_budget_remaining(0), SETTLE_PROBE_ATTEMPTS);
        assert_eq!(settle_probe_budget_remaining(1), SETTLE_PROBE_ATTEMPTS - 1);
        assert_eq!(settle_probe_budget_remaining(2), 1);
        assert_eq!(settle_probe_budget_remaining(SETTLE_PROBE_ATTEMPTS), 0);
        assert_eq!(settle_probe_budget_remaining(SETTLE_PROBE_ATTEMPTS + 1), 0);
    }

    #[test]
    fn settle_probe_returns_external_client_confirmed_when_resolution_is_held() {
        let query = TimerQuery {
            bounds: TimerBounds {
                minimum_interval: Hns::new(156_250),
                maximum_interval: Hns::new(5_000),
            },
            reported_current: Hns::new(156_250),
            raw_status: 0,
        };
        let mut controller = TimerController::new(
            FixturePlatform {
                request_observation: TimerObservation {
                    requested: Hns::new(5_000),
                    reported_current: Hns::new(4_966),
                    raw_status: 0,
                },
                release_result: Ok(TimerObservation {
                    requested: Hns::new(5_000),
                    reported_current: Hns::new(4_966),
                    raw_status: 0,
                }),
                query_results: vec![Ok(query), Ok(query)],
                settle_probe_result: Err(TimerError::Unsupported),
            },
            Hns::new(5_000),
        );
        controller.query().unwrap();
        for attempt in 0..SETTLE_PROBE_ATTEMPTS {
            assert_eq!(
                controller.attempt_startup_kernel_settle_probe(attempt),
                SettleProbeStep::Settled(SettleProbeOutcome::ExternalClientConfirmed)
            );
        }
    }

    #[test]
    fn settle_probe_returns_exhausted_after_budget() {
        let query = TimerQuery {
            bounds: TimerBounds {
                minimum_interval: Hns::new(156_250),
                maximum_interval: Hns::new(5_000),
            },
            reported_current: Hns::new(4_966),
            raw_status: 0,
        };
        let mut controller = TimerController::new(
            FixturePlatform {
                request_observation: TimerObservation {
                    requested: Hns::new(5_000),
                    reported_current: Hns::new(4_966),
                    raw_status: 0,
                },
                release_result: Ok(TimerObservation {
                    requested: Hns::new(5_000),
                    reported_current: Hns::new(4_966),
                    raw_status: 0,
                }),
                query_results: vec![Ok(query)],
                settle_probe_result: Err(TimerError::Unsupported),
            },
            Hns::new(5_000),
        );
        controller.query().unwrap();
        let mut attempts_used = 0;
        let final_step = loop {
            match controller.attempt_startup_kernel_settle_probe(attempts_used) {
                SettleProbeStep::Retry { budget_remaining } => {
                    assert_eq!(
                        budget_remaining,
                        settle_probe_budget_remaining(attempts_used + 1)
                    );
                    assert!(budget_remaining > 0);
                    attempts_used += 1;
                }
                step => break step,
            }
        };
        assert_eq!(attempts_used + 1, SETTLE_PROBE_ATTEMPTS);
        assert_eq!(
            final_step,
            SettleProbeStep::Settled(SettleProbeOutcome::Exhausted)
        );
        assert_eq!(
            controller.attempt_startup_kernel_settle_probe(SETTLE_PROBE_ATTEMPTS),
            SettleProbeStep::Settled(SettleProbeOutcome::Exhausted)
        );
    }
}
