mod support;

use std::time::{Duration, Instant};
use support::{drive_workload, MockPlatform, ScheduledTaskState, SAMPLE_STRIDE, WORKLOAD_SEED};
use tick_core::Hns;
use tick_diagnostics::{
    compute_entry_hash, genesis_hash, verify_event_chain, DiagnosticEvent, DiagnosticOutcome,
    DiagnosticPhase, DiagnosticRecord, DiagnosticSource, DiagnosticStore, NativeOutcome,
    HARD_MAX_EVENTS,
};
use tick_ipc::PidRateLimiter;
use tick_ownership::{OwnershipState, TimerController};
use tick_policy::{decide, PolicyInput, PowerState};

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
                    battery_lockout_enabled: true,
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
                    battery_lockout_enabled: true,
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
                    battery_lockout_enabled: true,
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

/// Total cyclic iterations driven by the long run resource stability test.
pub const LONG_RUN_CYCLES: usize = 250_000;

#[test]
fn soak_stress_long_run_resource_stability() {
    let report = drive_workload(
        WORKLOAD_SEED,
        &|iteration, _elapsed| iteration >= LONG_RUN_CYCLES as u64,
        "soak",
    );

    report.print("long_run");

    assert_eq!(
        report.violations.store, 0,
        "diagnostic store exceeded HARD_MAX_EVENTS"
    );
    assert_eq!(
        report.violations.ring, 0,
        "ring buffer exceeded EVENT_BUFFER_CAP"
    );
    assert_eq!(
        report.violations.limiter, 0,
        "rate limiter exceeded MAX_TRACKED_PIDS"
    );
    assert_eq!(
        report.violations.snapshot_count, 0,
        "snapshot retention exceeded MAX_RETAINED_SNAPSHOTS"
    );
    assert_eq!(
        report.violations.snapshot_bytes, 0,
        "snapshot retention exceeded MAX_TELEMETRY_DIRECTORY_BYTES"
    );
    assert_eq!(
        report.violations.log_count, 0,
        "log retention exceeded MAX_RETAINED_LOG_FILES"
    );
    assert_eq!(
        report.violations.log_age, 0,
        "log retention kept files older than LOG_RETENTION_DAYS"
    );

    if let Some(first) = report.first_store_len {
        if first >= HARD_MAX_EVENTS {
            assert!(
                report.last_store_len <= first,
                "store grew after reaching the retention cap: first={first} last={}",
                report.last_store_len
            );
        }
    }
    assert!(
        report.telemetry_bytes <= tick_diagnostics::recorder::MAX_TELEMETRY_DIRECTORY_BYTES,
        "telemetry directory size {} exceeds cap",
        report.telemetry_bytes
    );
    assert_eq!(
        report.violations.total(),
        0,
        "resource bound violations detected"
    );

    let _ = SAMPLE_STRIDE;
}
