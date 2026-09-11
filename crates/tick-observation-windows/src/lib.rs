//! Windows observation boundary placeholder.
//!
//! No event hooks, notifications, process detection, power API, or polling is
//! implemented. The placeholder reports that observation is unavailable.

use tick_core::{CoreError, Event};

pub trait ObservationSource {
    fn next_event(&mut self) -> Result<Option<Event>, CoreError>;
}

#[derive(Debug, Default)]
pub struct UnsupportedObservation;

impl ObservationSource for UnsupportedObservation {
    fn next_event(&mut self) -> Result<Option<Event>, CoreError> {
        Err(CoreError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_refuses_observation() {
        let mut source = UnsupportedObservation;
        assert_eq!(source.next_event(), Err(CoreError::Unsupported));
    }
}
