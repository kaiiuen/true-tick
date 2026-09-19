use std::time::{Duration, Instant};
use tick_core::Hns;
use tick_diagnostics::{
    compute_entry_hash, genesis_hash, verify_event_chain, DiagnosticEvent, DiagnosticOutcome,
    DiagnosticPhase, DiagnosticRecord, DiagnosticSource, DiagnosticStore, NativeOutcome,
};
use tick_ipc::PidRateLimiter;
use tick_ownership::{OwnershipState, TimerController};
use tick_platform_windows::{
    KernelSettleProbe, TimerBounds, TimerError, TimerObservation, TimerPlatform, TimerQuery,
};
use tick_policy::{decide, PolicyInput, PowerState};

#[derive(Debug)]
struct MockPlatform {
    current_hns: Hns,
    requested_hns: Hns,
    active: bool,
}

impl MockPlatform {
    fn new() -> Self {
        Self {
            current_hns: Hns::new(156_250),
            requested_hns: Hns::new(5_000),
            active: false,
        }
    }
}

impl TimerPlatform for MockPlatform {
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
        self.requested_hns = interval;
        self.current_hns = interval;
        self.active = true;
        Ok(TimerObservation {
            requested: interval,
            reported_current: interval,
            raw_status: 0,
        })
    }

    fn release(&mut self, interval: Hns) -> Result<TimerObservation, TimerError> {
        self.current_hns = Hns::new(156_250);
        self.active = false;
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
        Err(TimerError::Unsupported)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScheduledTaskState {
    Idle,
    Scheduled { deadline_tick: u64 },
}

#[test]
fn soak_stress_cyclic_transitions_hash_chain_and_pid_rate_limiter() {
    let mut controller = TimerController::new(MockPlatform::new(), Hns::new(5_000));
    let store = DiagnosticStore::new(15_000);
    let mut schedule_state = ScheduledTaskState::Idle;
    let mut current_power = PowerState::Ac;
    let mut policy_enabled = true;

    for iteration in 1..=10_000 {
        let op_context = store.begin_operation(DiagnosticSource::Internal);

        match iteration % 7 {
            0 => {
                let decision = decide(PolicyInput {
                    enabled: policy_enabled,
                    eligible_profile: true,
                    power: current_power,
                });
                if decision.status == tick_core::Status::Requested {
                    let _ = controller.start();
                    assert_eq!(controller.ownership(), OwnershipState::Owned);
                }
                store.record_with_context(
                    DiagnosticRecord {
                        context: op_context,
                        phase: DiagnosticPhase::Acquire,
                        source: DiagnosticSource::Ownership,
                        outcome: DiagnosticOutcome::Completed,
                        native: NativeOutcome::default(),
                    },
                    "ownership.acquire",
                    format!("iteration={iteration}"),
                );
            }
            1 => {
                let _ = controller.stop();
                assert_eq!(controller.ownership(), OwnershipState::Released);
                store.record_with_context(
                    DiagnosticRecord {
                        context: op_context,
                        phase: DiagnosticPhase::Release,
                        source: DiagnosticSource::Ownership,
                        outcome: DiagnosticOutcome::Completed,
                        native: NativeOutcome::default(),
                    },
                    "ownership.release",
                    format!("iteration={iteration}"),
                );
            }
            2 => {
                schedule_state = ScheduledTaskState::Scheduled {
                    deadline_tick: iteration + 60,
                };
                store.record_with_context(
                    DiagnosticRecord {
                        context: op_context,
                        phase: DiagnosticPhase::Timer,
                        source: DiagnosticSource::Timer,
                        outcome: DiagnosticOutcome::Completed,
                        native: NativeOutcome::default(),
                    },
                    "schedule.started",
                    format!("iteration={iteration} deadline={}", iteration + 60),
                );
            }
            3 => {
                schedule_state = ScheduledTaskState::Scheduled {
                    deadline_tick: iteration + 120,
                };
                store.record_with_context(
                    DiagnosticRecord {
                        context: op_context,
                        phase: DiagnosticPhase::Timer,
                        source: DiagnosticSource::Timer,
                        outcome: DiagnosticOutcome::Completed,
                        native: NativeOutcome::default(),
                    },
                    "schedule.replaced",
                    format!("iteration={iteration} deadline={}", iteration + 120),
                );
            }
            4 => {
                schedule_state = ScheduledTaskState::Idle;
                store.record_with_context(
                    DiagnosticRecord {
                        context: op_context,
                        phase: DiagnosticPhase::Timer,
                        source: DiagnosticSource::Timer,
                        outcome: DiagnosticOutcome::Cancelled,
                        native: NativeOutcome::default(),
                    },
                    "schedule.cancelled",
                    format!("iteration={iteration}"),
                );
            }
            5 => {
                current_power = PowerState::Battery;
                let decision = decide(PolicyInput {
                    enabled: policy_enabled,
                    eligible_profile: true,
                    power: current_power,
                });
                assert_eq!(decision.status, tick_core::Status::Blocked);
                let _ = controller.stop();
                store.record_with_context(
                    DiagnosticRecord {
                        context: op_context,
                        phase: DiagnosticPhase::Observe,
                        source: DiagnosticSource::PowerEvent,
                        outcome: DiagnosticOutcome::Completed,
                        native: NativeOutcome::default(),
                    },
                    "power.suspend",
                    format!("iteration={iteration} power=Battery"),
                );
            }
            _ => {
                current_power = PowerState::Ac;
                policy_enabled = true;
                let decision = decide(PolicyInput {
                    enabled: policy_enabled,
                    eligible_profile: true,
                    power: current_power,
                });
                assert_eq!(decision.status, tick_core::Status::Requested);
                store.record_with_context(
                    DiagnosticRecord {
                        context: op_context,
                        phase: DiagnosticPhase::Observe,
                        source: DiagnosticSource::PowerEvent,
                        outcome: DiagnosticOutcome::Completed,
                        native: NativeOutcome::default(),
                    },
                    "power.resume",
                    format!("iteration={iteration} power=Ac"),
                );
            }
        }
    }

    assert_eq!(schedule_state, ScheduledTaskState::Idle);

    let snapshot_events = store.snapshot();
    assert_eq!(snapshot_events.len(), 512);
    assert!(
        verify_event_chain(&snapshot_events).is_ok(),
        "Bounded ring buffer snapshot must verify cleanly across eviction boundary"
    );

    let mut full_chain: Vec<DiagnosticEvent> = Vec::with_capacity(10_000);
    let mut prev_hash = genesis_hash();
    let start_instant = Instant::now();

    for seq in 1..=10_000u64 {
        let elapsed = start_instant.elapsed();
        let name = "telemetry.event".to_string();
        let details = format!("event_index={seq}");
        let entry_hash = compute_entry_hash(prev_hash, seq, elapsed.as_nanos(), &name, &details);

        full_chain.push(DiagnosticEvent {
            sequence: seq,
            elapsed,
            operation_id: seq,
            parent_operation_id: None,
            correlation_id: seq,
            phase: DiagnosticPhase::Observe,
            source: DiagnosticSource::Internal,
            outcome: DiagnosticOutcome::Completed,
            native: NativeOutcome::default(),
            name,
            details,
            prev_hash,
            entry_hash,
        });

        prev_hash = entry_hash;
    }

    assert_eq!(full_chain.len(), 10_000);

    for (idx, event) in full_chain.iter().enumerate() {
        assert_eq!(
            event.sequence,
            (idx as u64) + 1,
            "Sequence must be monotonically incremented at index {idx}"
        );
    }

    assert!(
        verify_event_chain(&full_chain).is_ok(),
        "SHA-256 telemetry hash chain must verify cleanly across all 10000 events without hash corruption"
    );

    let mut limiter = PidRateLimiter::new();
    let base_instant = Instant::now();

    for call_index in 0..10_000 {
        let pid = (call_index % 50) as u32;
        let call_offset = Duration::from_millis((call_index / 50) * 100);
        let now = base_instant + call_offset;

        let _ = limiter.check_and_record(pid, now);

        if call_index % 500 == 0 {
            limiter.prune_idle_callers(now);
            assert!(
                limiter.active_caller_count() <= 50,
                "Tracked caller count must stay bounded"
            );
        }
    }

    let final_now = base_instant + Duration::from_secs(3_600);
    limiter.prune_idle_callers(final_now);
    assert_eq!(
        limiter.active_caller_count(),
        0,
        "All idle callers must be cleanly pruned"
    );
}
