//! Serialized ownership bookkeeping for one Tick runtime instance.
//!
//! The controller records only a successful adapter request. It never infers
//! ownership from an effective value and never writes a guessed global default.

use tick_core::{CoreError, Hns, Status};
use tick_platform_windows::{NtStatus, TimerError, TimerObservation, TimerPlatform};

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
        self.ownership.apply(Transition::Acquire).map_err(|_| {
            TimerError::PostconditionUnverified {
                raw_status: 0,
                reported_current: Hns::ZERO,
            }
        })?;
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
            .map_err(|_| TimerError::ReleaseFailed { raw_status: 0 })?;
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

    pub const fn release_boundary(&self) -> Option<Hns> {
        self.release_boundary
    }

    /// Ends the release-observation window without changing the last snapshot.
    pub fn clear_release_boundary(&mut self) {
        self.release_boundary = None;
        self.snapshot.selected = None;
    }

    pub const fn status(&self) -> Status {
        self.ownership.status()
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
    use tick_platform_windows::{TimerBounds, TimerQuery};

    #[derive(Debug)]
    struct FixturePlatform {
        request_observation: TimerObservation,
        release_result: Result<TimerObservation, TimerError>,
        query_results: Vec<Result<TimerQuery, TimerError>>,
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
}
