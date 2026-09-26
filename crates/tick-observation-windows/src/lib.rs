//! Event-driven Windows power observation.
//!
//! This adapter performs an explicit query at startup and after a supplied
//! power-broadcast event. It does not poll and does not infer Battery Saver
//! from unrelated fields. An unknown saver state remains conservative.

use tick_core::{CoreError, Event};
use tick_policy::PowerState;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PowerSnapshot {
    pub state: PowerState,
    pub battery_saver: Option<bool>,
}

pub trait ObservationSource {
    fn next_event(&mut self) -> Result<Option<Event>, CoreError>;
    fn power(&self) -> PowerSnapshot;
    fn refresh_power(&mut self) -> Result<PowerSnapshot, CoreError>;
}

#[derive(Debug)]
pub struct WindowsObservation {
    power: PowerSnapshot,
    power_error: Option<CoreError>,
}

impl Default for WindowsObservation {
    fn default() -> Self {
        Self {
            power: PowerSnapshot {
                state: PowerState::Unknown,
                battery_saver: None,
            },
            power_error: None,
        }
    }
}

impl ObservationSource for WindowsObservation {
    fn next_event(&mut self) -> Result<Option<Event>, CoreError> {
        Err(CoreError::Unsupported)
    }

    fn power(&self) -> PowerSnapshot {
        self.power
    }

    fn refresh_power(&mut self) -> Result<PowerSnapshot, CoreError> {
        let result = {
            #[cfg(windows)]
            {
                query_power()
            }
            #[cfg(not(windows))]
            {
                Err(CoreError::Unsupported)
            }
        };
        self.apply_power_query_result(result)
    }
}

impl WindowsObservation {
    pub fn power_error(&self) -> Option<CoreError> {
        self.power_error
    }

    fn apply_power_query_result(
        &mut self,
        result: Result<PowerSnapshot, CoreError>,
    ) -> Result<PowerSnapshot, CoreError> {
        match result {
            Ok(snapshot) => {
                self.power = snapshot;
                self.power_error = None;
                Ok(snapshot)
            }
            Err(error) => {
                self.power = PowerSnapshot {
                    state: PowerState::Unknown,
                    battery_saver: None,
                };
                self.power_error = Some(error);
                Err(error)
            }
        }
    }
}

#[cfg(windows)]
fn query_power() -> Result<PowerSnapshot, CoreError> {
    let mut status = SystemPowerStatus {
        ac_line_status: 255,
        battery_flag: 255,
        battery_life_percent: 255,
        system_status_flag: 0,
        battery_life_time: u32::MAX,
        battery_full_life_time: u32::MAX,
    };
    let ok = unsafe { GetSystemPowerStatus(&mut status) } != 0;
    if !ok {
        return Err(CoreError::ObservationFailed {
            raw_status: unsafe { GetLastError() },
        });
    }
    let battery_saver = Some(status.system_status_flag & 1 != 0);
    let base_state = match status.ac_line_status {
        1 => PowerState::Ac,
        0 => PowerState::Battery,
        _ => PowerState::Unknown,
    };
    let state = if battery_saver == Some(true) {
        PowerState::BatterySaver
    } else {
        base_state
    };
    Ok(PowerSnapshot {
        state,
        battery_saver,
    })
}

#[cfg(windows)]
#[repr(C)]
struct SystemPowerStatus {
    ac_line_status: u8,
    battery_flag: u8,
    battery_life_percent: u8,
    system_status_flag: u8,
    battery_life_time: u32,
    battery_full_life_time: u32,
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn GetSystemPowerStatus(status: *mut SystemPowerStatus) -> i32;
    fn GetLastError() -> u32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_failure_after_ac_clears_stale_power_and_records_error() {
        let mut observation = WindowsObservation::default();
        observation
            .apply_power_query_result(Ok(PowerSnapshot {
                state: PowerState::Ac,
                battery_saver: Some(false),
            }))
            .unwrap();
        let error = CoreError::ObservationFailed { raw_status: 31 };
        assert_eq!(observation.apply_power_query_result(Err(error)), Err(error));
        assert_eq!(observation.power().state, PowerState::Unknown);
        assert_eq!(observation.power().battery_saver, None);
        assert_eq!(observation.power_error(), Some(error));
    }

    #[test]
    fn query_failure_after_battery_clears_stale_power_and_records_error() {
        let mut observation = WindowsObservation::default();
        observation
            .apply_power_query_result(Ok(PowerSnapshot {
                state: PowerState::Battery,
                battery_saver: Some(false),
            }))
            .unwrap();
        let error = CoreError::ObservationFailed { raw_status: 32 };
        assert_eq!(observation.apply_power_query_result(Err(error)), Err(error));
        assert_eq!(observation.power().state, PowerState::Unknown);
        assert_eq!(observation.power_error(), Some(error));
    }

    #[test]
    fn successful_query_clears_a_previous_power_error() {
        let mut observation = WindowsObservation::default();
        let error = CoreError::ObservationFailed { raw_status: 33 };
        let _ = observation.apply_power_query_result(Err(error));
        let snapshot = PowerSnapshot {
            state: PowerState::Ac,
            battery_saver: Some(false),
        };
        assert_eq!(
            observation.apply_power_query_result(Ok(snapshot)),
            Ok(snapshot)
        );
        assert_eq!(observation.power_error(), None);
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_power_is_explicitly_unsupported() {
        let mut observation = WindowsObservation::default();
        assert_eq!(observation.refresh_power(), Err(CoreError::Unsupported));
        assert_eq!(observation.power().state, PowerState::Unknown);
        assert_eq!(observation.power_error(), Some(CoreError::Unsupported));
    }
}
