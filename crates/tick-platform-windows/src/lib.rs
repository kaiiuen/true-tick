//! Windows platform boundary placeholder.
//!
//! No Windows API, native FFI, compatibility claim, or active behavior exists
//! here. API selection remains an unresolved project decision.

use tick_core::{CoreError, Hns};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformCapability {
    TimerRequest,
}

pub trait TimerPlatform {
    fn request(&mut self, _interval: Hns) -> Result<(), CoreError>;
    fn release(&mut self) -> Result<(), CoreError>;
}

#[derive(Debug, Default)]
pub struct UnsupportedWindowsPlatform;

impl TimerPlatform for UnsupportedWindowsPlatform {
    fn request(&mut self, _interval: Hns) -> Result<(), CoreError> {
        Err(CoreError::Unsupported)
    }

    fn release(&mut self) -> Result<(), CoreError> {
        Err(CoreError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_refuses_request_and_release() {
        let mut platform = UnsupportedWindowsPlatform;
        assert_eq!(platform.request(Hns::new(1)), Err(CoreError::Unsupported));
        assert_eq!(platform.release(), Err(CoreError::Unsupported));
    }
}
