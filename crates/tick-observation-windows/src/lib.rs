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
}

impl Default for WindowsObservation {
    fn default() -> Self {
        Self {
            power: PowerSnapshot {
                state: PowerState::Unknown,
                battery_saver: None,
            },
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
        #[cfg(windows)]
        {
            self.power = query_power();
            return Ok(self.power);
        }
        #[cfg(not(windows))]
        {
            Err(CoreError::Unsupported)
        }
    }
}

#[cfg(windows)]
fn query_power() -> PowerSnapshot {
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
        return PowerSnapshot {
            state: PowerState::Unknown,
            battery_saver: None,
        };
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
    PowerSnapshot {
        state,
        battery_saver,
    }
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
extern "system" {
    fn GetSystemPowerStatus(status: *mut SystemPowerStatus) -> i32;
}

#[cfg(test)]
mod tests {
    #[cfg(not(windows))]
    use super::*;

    #[test]
    fn non_windows_power_is_explicitly_unsupported() {
        #[cfg(not(windows))]
        assert_eq!(
            WindowsObservation::default().refresh_power(),
            Err(CoreError::Unsupported)
        );
    }
}
