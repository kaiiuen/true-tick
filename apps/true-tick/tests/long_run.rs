mod support;

use std::time::Duration;
use support::{drive_workload, WORKLOAD_SEED};
use tick_diagnostics::recorder::MAX_TELEMETRY_DIRECTORY_BYTES;
use tick_diagnostics::HARD_MAX_EVENTS;

/// Seconds driven by the bounded long run test when the environment variable
/// is unset. The default keeps the fast suite fast while still exercising the
/// full workload mixer and every retention bound.
const DEFAULT_LONG_RUN_SECONDS: u64 = 5;

/// Runs the shared mixed workload for `TRUE_TICK_LONG_RUN_SECONDS` seconds,
/// defaulting to five seconds so the default suite stays fast. A runtime check
/// guards the fast path: when the configured duration exceeds ten seconds and
/// the variable was not explicitly set, the test prints a notice and returns
/// early instead of being compiled out. Setting the variable to any value is
/// the opt in, so a developer can request a longer pass without touching
/// attributes. The workload samples every 1,000 cycles and asserts zero bound
/// violations at the end.
#[test]
fn long_run_bounded_resource_stability() {
    let configured = std::env::var("TRUE_TICK_LONG_RUN_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok());
    let opted_in = configured.is_some();
    let seconds = configured.unwrap_or(DEFAULT_LONG_RUN_SECONDS);

    if seconds > 10 && !opted_in {
        eprintln!(
            "long_run skipped: configured duration {seconds}s exceeds the 10s fast bound, set TRUE_TICK_LONG_RUN_SECONDS to opt in"
        );
        return;
    }

    let budget = Duration::from_secs(seconds);
    let report = drive_workload(
        WORKLOAD_SEED,
        &|_iteration, elapsed| elapsed >= budget,
        "longrun",
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
        report.telemetry_bytes <= MAX_TELEMETRY_DIRECTORY_BYTES,
        "telemetry directory size {} exceeds cap",
        report.telemetry_bytes
    );
    assert_eq!(
        report.violations.total(),
        0,
        "resource bound violations detected"
    );
}
