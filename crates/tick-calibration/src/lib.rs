//! Bounded calibration boundary model.
//!
//! No timing loop, benchmark, platform call, machine state, or real calibration
//! exists in this skeleton. The interface refuses execution explicitly.

use tick_core::CoreError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CalibrationOutcome {
    Unsupported,
    NotImplemented,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CalibrationResult {
    pub outcome: CalibrationOutcome,
}

pub trait Calibrator {
    fn run(&mut self) -> Result<CalibrationResult, CoreError>;
}

#[derive(Debug, Default)]
pub struct UnsupportedCalibration;

impl Calibrator for UnsupportedCalibration {
    fn run(&mut self) -> Result<CalibrationResult, CoreError> {
        Ok(CalibrationResult {
            outcome: CalibrationOutcome::Unsupported,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibration_is_explicitly_unsupported() {
        let mut calibration = UnsupportedCalibration;
        assert_eq!(
            calibration.run().unwrap().outcome,
            CalibrationOutcome::Unsupported
        );
    }
}
