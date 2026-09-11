//! Serialized ownership bookkeeping for one Tick runtime instance.
//!
//! The controller records only a successful adapter request. It never infers
//! ownership from an effective value and never writes a guessed global default.

use tick_core::{CoreError, Hns, Status};
use tick_platform_windows::{TimerError, TimerPlatform};

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
    Unverified,
}

#[derive(Debug)]
pub struct TimerController<P> {
    platform: P,
    ownership: Ownership,
    interval: Hns,
    verification: Verification,
}

impl<P: TimerPlatform> TimerController<P> {
    pub const fn new(platform: P, interval: Hns) -> Self {
        Self {
            platform,
            ownership: Ownership::new(),
            interval,
            verification: Verification::NotCollected,
        }
    }

    pub fn start(&mut self) -> Result<Verification, TimerError> {
        if self.ownership.state() != OwnershipState::Released
            || self.verification == Verification::Unverified
        {
            return Ok(self.verification);
        }
        self.platform.preflight(self.interval)?;
        if let Err(error) = self.platform.request(self.interval) {
            if matches!(error, TimerError::PostconditionUnverified { .. }) {
                self.ownership.mark_uncertain();
                self.verification = Verification::Unverified;
            }
            return Err(error);
        }
        self.ownership.apply(Transition::Acquire).map_err(|_| {
            TimerError::PostconditionUnverified {
                raw_status: 0,
                reported_current: Hns::ZERO,
            }
        })?;
        self.verification = Verification::Verified;
        Ok(self.verification)
    }

    pub fn stop(&mut self) -> Result<bool, TimerError> {
        if self.ownership.state() == OwnershipState::Released {
            return Ok(false);
        }
        if let Err(error) = self.platform.release(self.interval) {
            self.verification = Verification::Unverified;
            return Err(error);
        }
        self.ownership
            .apply(Transition::Release)
            .map_err(|_| TimerError::ReleaseFailed { raw_status: 0 })?;
        self.verification = Verification::NotCollected;
        Ok(true)
    }

    pub const fn ownership(&self) -> OwnershipState {
        self.ownership.state()
    }

    pub const fn verification(&self) -> Verification {
        self.verification
    }

    pub const fn status(&self) -> Status {
        self.ownership.status()
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
}
