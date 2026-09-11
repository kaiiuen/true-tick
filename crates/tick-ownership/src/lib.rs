//! Serialized ownership bookkeeping for one Tick runtime instance.
//!
//! The controller records only a successful adapter request. It never infers
//! ownership from an effective value and never writes a guessed global default.

use tick_core::{CoreError, Hns, Status};
use tick_platform_windows::{TimerError, TimerObservation, TimerPlatform};

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

#[derive(Debug)]
pub struct TimerController<P> {
    platform: P,
    ownership: Ownership,
    requested_interval: Hns,
    selected_interval: Option<Hns>,
    verification: Verification,
    observation: Option<TimerObservation>,
}

impl<P: TimerPlatform> TimerController<P> {
    pub const fn new(platform: P, requested_interval: Hns) -> Self {
        Self {
            platform,
            ownership: Ownership::new(),
            requested_interval,
            selected_interval: None,
            verification: Verification::NotCollected,
            observation: None,
        }
    }

    fn selected_for_query(&mut self) -> Result<Hns, TimerError> {
        if let Some(interval) = self.selected_interval {
            Ok(interval)
        } else {
            let interval = self.platform.resolve(self.requested_interval)?;
            self.selected_interval = Some(interval);
            Ok(interval)
        }
    }

    pub fn query(&mut self) -> Result<TimerObservation, TimerError> {
        let interval = self.selected_for_query()?;
        let query = self.platform.query(interval)?;
        let observation = TimerObservation {
            requested: interval,
            reported_current: query.reported_current,
            raw_status: query.raw_status,
        };
        self.observation = Some(observation);
        Ok(observation)
    }

    pub fn start(&mut self) -> Result<Verification, TimerError> {
        if self.ownership.state() != OwnershipState::Released
            || self.verification == Verification::Unverified
        {
            return Ok(self.verification);
        }
        let interval = self.platform.resolve(self.requested_interval)?;
        self.selected_interval = Some(interval);
        let query = self.platform.preflight(interval)?;
        self.observation = Some(TimerObservation {
            requested: interval,
            reported_current: query.reported_current,
            raw_status: query.raw_status,
        });
        let request_observation = match self.platform.request(interval) {
            Ok(observation) => observation,
            Err(error) => {
                if let TimerError::PostconditionUnverified {
                    raw_status,
                    reported_current,
                } = error
                {
                    self.observation = Some(TimerObservation {
                        requested: interval,
                        reported_current,
                        raw_status,
                    });
                    self.ownership.mark_uncertain();
                    self.verification = Verification::Unverified;
                }
                return Err(error);
            }
        };
        self.observation = Some(request_observation);
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
        self.observation = Some(observation);
        self.ownership
            .apply(Transition::Release)
            .map_err(|_| TimerError::ReleaseFailed { raw_status: 0 })?;
        self.verification = Verification::NotCollected;
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
        release_observation: TimerObservation,
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
    }

    impl TimerPlatform for FixturePlatform {
        fn query(&mut self, interval: Hns) -> Result<TimerQuery, TimerError> {
            Ok(self.query_result(interval))
        }

        fn preflight(&mut self, interval: Hns) -> Result<TimerQuery, TimerError> {
            Ok(self.query_result(interval))
        }

        fn request(&mut self, _interval: Hns) -> Result<TimerObservation, TimerError> {
            Ok(self.request_observation)
        }

        fn release(&mut self, _interval: Hns) -> Result<TimerObservation, TimerError> {
            Ok(self.release_observation)
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
                release_observation: TimerObservation {
                    requested: Hns::new(5_000),
                    reported_current: Hns::new(4_000),
                    raw_status: 0,
                },
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
        assert_eq!(observation.reported_current, Hns::new(4_000));
        assert_eq!(observation.requested, Hns::new(5_000));
        assert_eq!(observation.raw_status, 0);
        assert_eq!(controller.ownership(), OwnershipState::Released);
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
            reported_current: Hns::new(10_001),
            raw_status: 0,
        };
        assert!(!observation.is_satisfied());
    }
}
