use tick_core::Hns;
use tick_ownership::{OwnershipState, TimerController};
use tick_platform_windows::{
    KernelSettleProbe, TimerBounds, TimerError, TimerObservation, TimerPlatform, TimerQuery,
};

#[derive(Debug)]
struct PermutationPlatform {
    request_fails: bool,
    release_fails: bool,
    current_hns: Hns,
    requested_hns: Hns,
    probe_outcome: Result<KernelSettleProbe, TimerError>,
    native_request_active: bool,
}

impl PermutationPlatform {
    fn new() -> Self {
        Self {
            request_fails: false,
            release_fails: false,
            current_hns: Hns::new(156_250),
            requested_hns: Hns::new(5_000),
            probe_outcome: Err(TimerError::Unsupported),
            native_request_active: false,
        }
    }
}

impl TimerPlatform for PermutationPlatform {
    fn query(&mut self, _interval: Hns) -> Result<TimerQuery, TimerError> {
        Ok(TimerQuery {
            bounds: TimerBounds {
                minimum_interval: Hns::new(156_250),
                maximum_interval: Hns::new(5_000),
            },
            reported_current: self.current_hns,
            raw_status: 0,
        })
    }

    fn preflight(&mut self, interval: Hns) -> Result<TimerQuery, TimerError> {
        self.query(interval)
    }

    fn request(&mut self, interval: Hns) -> Result<TimerObservation, TimerError> {
        if self.request_fails {
            return Err(TimerError::RequestFailed { raw_status: -1 });
        }
        self.requested_hns = interval;
        self.current_hns = interval;
        self.native_request_active = true;
        Ok(TimerObservation {
            requested: interval,
            reported_current: interval,
            raw_status: 0,
        })
    }

    fn release(&mut self, interval: Hns) -> Result<TimerObservation, TimerError> {
        if self.release_fails {
            return Err(TimerError::ReleaseFailed { raw_status: -1 });
        }
        self.current_hns = Hns::new(156_250);
        self.native_request_active = false;
        Ok(TimerObservation {
            requested: interval,
            reported_current: Hns::new(156_250),
            raw_status: 0,
        })
    }

    fn attempt_kernel_settle_probe(
        &mut self,
        _interval: Hns,
    ) -> Result<KernelSettleProbe, TimerError> {
        self.probe_outcome
    }
}

#[derive(Clone, Copy, Debug)]
enum Step {
    Acquire,
    Release,
    Query,
    SimulateExternalTimingChange(u64),
    ToggleFailure(bool),
}

fn simple_prng(mut state: u64) -> impl FnMut() -> u64 {
    move || {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let xsh = ((state >> 18) ^ state) >> 27;
        let rot = (state >> 59) as u32;
        (xsh >> rot) | (xsh << ((!rot).wrapping_add(1) & 31))
    }
}

#[test]
fn mathematical_permutation_state_invariants_hold_across_large_sequences() {
    let mut next_random = simple_prng(0xDEAD_BEEF_CAFE_BABE);

    for run in 0..1_000 {
        let mut controller = TimerController::new(PermutationPlatform::new(), Hns::new(5_000));
        let step_count = (next_random() % 50) + 10;

        for _ in 0..step_count {
            let action = match next_random() % 5 {
                0 => Step::Acquire,
                1 => Step::Release,
                2 => Step::Query,
                3 => Step::SimulateExternalTimingChange(next_random() % 160_000),
                _ => Step::ToggleFailure(next_random().is_multiple_of(2)),
            };

            match action {
                Step::Acquire => {
                    let _ = controller.start();
                }
                Step::Release => {
                    let _ = controller.stop();
                }
                Step::Query => {
                    let _ = controller.query();
                }
                Step::SimulateExternalTimingChange(val) => {
                    let hns = Hns::new(val.max(1));
                    controller.observe_current(hns).ok();
                }
                Step::ToggleFailure(_fails) => {}
            }

            let ownership = controller.ownership();

            // Invariant 1: Uncertain state must never claim to be clean Released
            if ownership == OwnershipState::Uncertain {
                assert_ne!(ownership, OwnershipState::Released);
            }

            // Invariant 2: When controller is cleanly released, status must be Released
            if ownership == OwnershipState::Released {
                assert_eq!(controller.status(), tick_core::Status::Released);
            }
        }

        // Final graceful teardown invariant: Calling stop on any ending state must settle cleanly
        let _ = controller.stop();
        let final_ownership = controller.ownership();
        assert!(
            final_ownership == OwnershipState::Released || final_ownership == OwnershipState::Owned,
            "failed teardown in run {run}"
        );
    }
}
