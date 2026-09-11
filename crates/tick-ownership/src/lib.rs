//! Serialized logical ownership bookkeeping.
//!
//! This is not an implementation of any Windows ownership mechanism. It models
//! one runtime instance's tracked contribution and never restores a guessed
//! global default.

use tick_core::{CoreError, Status};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OwnershipState {
    Released,
    Owned,
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
            (OwnershipState::Owned, Transition::Release) => {
                self.state = OwnershipState::Released;
                true
            }
            (OwnershipState::Owned, Transition::Acquire)
            | (OwnershipState::Released, Transition::Release) => false,
        };
        Ok(changed)
    }

    pub const fn status(self) -> Status {
        match self.state {
            OwnershipState::Released => Status::Released,
            OwnershipState::Owned => Status::Requested,
        }
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
}
