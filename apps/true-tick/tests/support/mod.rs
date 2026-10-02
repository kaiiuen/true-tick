//! Shared soak workload used by `soak_stress.rs` and `long_run.rs` so both
//! integration tests drive byte identical operation mixes and bound checks.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tick_core::Hns;
use tick_diagnostics::recorder::{
    enforce_snapshot_retention, MAX_RETAINED_SNAPSHOTS, MAX_TELEMETRY_DIRECTORY_BYTES,
};
use tick_diagnostics::retention::{
    purge_expired_logs, utc_day_to_unix_seconds, LOG_RETENTION_DAYS, MAX_RETAINED_LOG_FILES,
};
use tick_diagnostics::{
    DiagnosticOutcome, DiagnosticPhase, DiagnosticRecord, DiagnosticSource, DiagnosticStore,
    NativeOutcome, HARD_MAX_EVENTS,
};
use tick_ipc::{PidRateLimiter, MAX_TRACKED_PIDS};
use tick_ownership::TimerController;
use tick_platform_windows::{
    KernelSettleProbe, TimerBounds, TimerError, TimerObservation, TimerPlatform, TimerQuery,
};
use tick_policy::{
    decide, evaluate_tier, OperatingTier, PolicyInput, PowerState, TierContext, TierTransition,
};

/// Sampling cadence for bound checks inside the hot loop.
pub const SAMPLE_STRIDE: usize = 1_000;

/// Fixed seed for the deterministic pseudo-random workload mixer so every run
/// is byte for byte reproducible.
pub const WORKLOAD_SEED: u64 = 0x5452_5545_5449_434b;

/// Must match `EVENT_BUFFER_CAP` in `apps/true-tick/src/ui/diagnostic_window.rs`.
/// The constant lives inside the binary crate so integration tests pin the
/// value locally and rely on the unit test there to catch a drift.
const EVENT_BUFFER_CAP: usize = 4096;

pub struct XorShift64 {
    state: u64,
}

impl XorShift64 {
    pub fn new(seed: u64) -> Self {
        Self { state: seed | 1 }
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    pub fn next_index(&mut self, bound: usize) -> usize {
        (self.next_u64() % bound as u64) as usize
    }
}

#[derive(Debug)]
pub struct MockPlatform {
    current_hns: Hns,
    requested_hns: Hns,
    active: bool,
}

impl MockPlatform {
    pub fn new() -> Self {
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
pub enum ScheduledTaskState {
    Idle,
    Scheduled { deadline_tick: u64 },
}

/// Deterministic mixed workload: acquire, release, schedule, schedule
/// replacement, cancel, suspend, resume, tier evaluation, and IPC rate limiter
/// calls in a pseudo-random order driven by `XorShift64`.
pub struct MixedWorkload {
    controller: TimerController<MockPlatform>,
    store: DiagnosticStore,
    limiter: PidRateLimiter,
    schedule_state: ScheduledTaskState,
    current_power: PowerState,
    policy_enabled: bool,
    tier: OperatingTier,
    rng: XorShift64,
    limiter_base: Instant,
    limiter_tick: u64,
}

impl MixedWorkload {
    pub fn new(seed: u64) -> Self {
        Self {
            controller: TimerController::new(MockPlatform::new(), Hns::new(5_000)),
            store: DiagnosticStore::new(HARD_MAX_EVENTS),
            limiter: PidRateLimiter::new(),
            schedule_state: ScheduledTaskState::Idle,
            current_power: PowerState::Ac,
            policy_enabled: true,
            tier: OperatingTier::Nominal,
            rng: XorShift64::new(seed),
            limiter_base: Instant::now(),
            limiter_tick: 0,
        }
    }

    pub fn store_len(&self) -> usize {
        self.store.snapshot().len()
    }

    pub fn limiter_records(&self) -> usize {
        self.limiter.active_caller_count()
    }

    pub fn run_one(&mut self, iteration: u64) {
        let op = self.rng.next_index(9);
        let op_context = self.store.begin_operation(DiagnosticSource::Internal);
        match op {
            0 => {
                let decision = decide(PolicyInput {
                    enabled: self.policy_enabled,
                    eligible_profile: true,
                    power: self.current_power,
                    battery_lockout_enabled: true,
                });
                if decision.status == tick_core::Status::Requested {
                    let _ = self.controller.start();
                }
                self.store.record_with_context(
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
                let _ = self.controller.stop();
                self.store.record_with_context(
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
                self.schedule_state = ScheduledTaskState::Scheduled {
                    deadline_tick: iteration + 60,
                };
                self.store.record_with_context(
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
                self.schedule_state = ScheduledTaskState::Scheduled {
                    deadline_tick: iteration + 120,
                };
                self.store.record_with_context(
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
                self.schedule_state = ScheduledTaskState::Idle;
                self.store.record_with_context(
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
                self.current_power = PowerState::Battery;
                let _ = self.controller.stop();
                self.store.record_with_context(
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
            6 => {
                self.current_power = PowerState::Ac;
                self.policy_enabled = true;
                let _ = decide(PolicyInput {
                    enabled: self.policy_enabled,
                    eligible_profile: true,
                    power: self.current_power,
                    battery_lockout_enabled: true,
                });
                self.store.record_with_context(
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
            7 => {
                let ctx = TierContext {
                    tray_surface_available: self.rng.next_u64() & 1 == 0,
                    kernel_rejected_requests: (self.rng.next_u64() % 5) as u32,
                    unhandled_anomalies: (self.rng.next_u64() % 5) as u32,
                    current_floor_hns: 0,
                    nominally_recoverable: self.rng.next_u64() & 1 == 0,
                };
                let transition = evaluate_tier(self.tier, &ctx);
                self.tier = match transition {
                    TierTransition::Stay => self.tier,
                    TierTransition::EnterSurfaceDegraded => OperatingTier::SurfaceDegraded,
                    TierTransition::EnterMetrologyDegraded { floored_hns } => {
                        OperatingTier::MetrologyDegraded { floored_hns }
                    }
                    TierTransition::EnterQuiescent => OperatingTier::Quiescent,
                    TierTransition::Recover => OperatingTier::Nominal,
                };
                self.store.record_with_context(
                    DiagnosticRecord {
                        context: op_context,
                        phase: DiagnosticPhase::Decide,
                        source: DiagnosticSource::Policy,
                        outcome: DiagnosticOutcome::Completed,
                        native: NativeOutcome::default(),
                    },
                    "tier.evaluate",
                    format!("iteration={iteration} tier={:?}", self.tier),
                );
            }
            _ => {
                self.limiter_tick += 1;
                let pid = (self.rng.next_u64() % (MAX_TRACKED_PIDS as u64 * 4)) as u32;
                let now = self.limiter_base + Duration::from_millis(self.limiter_tick % 4_000);
                let _ = self.limiter.check_and_record(pid, now);
                if self.limiter_tick.is_multiple_of(64) {
                    self.limiter.prune_idle_callers(now);
                }
                self.store.record_with_context(
                    DiagnosticRecord {
                        context: op_context,
                        phase: DiagnosticPhase::Observe,
                        source: DiagnosticSource::Internal,
                        outcome: DiagnosticOutcome::Completed,
                        native: NativeOutcome::default(),
                    },
                    "ipc.rate_limit",
                    format!("iteration={iteration} pid={pid}"),
                );
            }
        }
    }
}

/// Builds a clean temporary directory inside the crate target directory so
/// test artifacts never escape the workspace and never collide between runs.
pub fn temporary_directory(label: &str) -> PathBuf {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("test-tmp")
        .join(format!("{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    directory
}

fn unix_seconds_to_utc_day(unix_seconds: u64) -> (u16, u16, u16) {
    let days = (unix_seconds / 86_400) as i64;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u16;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u16;
    let year = (if month <= 2 { y + 1 } else { y }) as u16;
    (year, month, day)
}

fn write_snapshot_file(directory: &Path, index: usize, payload: &[u8]) {
    let path = directory.join(format!("anomaly-{index:08}.recorder"));
    std::fs::write(&path, payload).expect("snapshot write succeeds");
    let stamp = std::time::SystemTime::UNIX_EPOCH + Duration::from_millis(1_000 + index as u64);
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("snapshot open succeeds");
    file.set_modified(stamp).expect("snapshot stamp succeeds");
}

fn count_snapshot_files(directory: &Path) -> usize {
    std::fs::read_dir(directory)
        .expect("telemetry dir readable")
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_type()
                .map(|kind| kind.is_file())
                .unwrap_or(false)
        })
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .map(|name| name.starts_with("anomaly-") && name.ends_with(".recorder"))
                .unwrap_or(false)
        })
        .count()
}

pub fn telemetry_directory_bytes(directory: &Path) -> u64 {
    std::fs::read_dir(directory)
        .expect("telemetry dir readable")
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_type()
                .map(|kind| kind.is_file())
                .unwrap_or(false)
        })
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .map(|name| name.starts_with("anomaly-") && name.ends_with(".recorder"))
                .unwrap_or(false)
        })
        .filter_map(|entry| entry.metadata().ok())
        .map(|metadata| metadata.len())
        .sum()
}

fn count_daily_logs(directory: &Path) -> usize {
    std::fs::read_dir(directory)
        .expect("log dir readable")
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_type()
                .map(|kind| kind.is_file())
                .unwrap_or(false)
        })
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .map(|name| name.starts_with("true-tick-") && name.ends_with(".csv"))
                .unwrap_or(false)
        })
        .count()
}

fn daily_log_unix_seconds(directory: &Path) -> Vec<u64> {
    std::fs::read_dir(directory)
        .expect("log dir readable")
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_type()
                .map(|kind| kind.is_file())
                .unwrap_or(false)
        })
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_owned();
            let stem = name.strip_prefix("true-tick-")?.strip_suffix(".csv")?;
            if stem.len() != 10 {
                return None;
            }
            let bytes = stem.as_bytes();
            if bytes[4] != b'-' || bytes[7] != b'-' {
                return None;
            }
            let year: u16 = stem[0..4].parse().ok()?;
            let month: u16 = stem[5..7].parse().ok()?;
            let day: u16 = stem[8..10].parse().ok()?;
            utc_day_to_unix_seconds(year, month, day)
        })
        .collect()
}

/// Per bound violation tallies collected at each sample point. Counting rather
/// than asserting inside the hot loop lets a failure report a full count.
#[derive(Debug, Default)]
pub struct ViolationCounts {
    pub store: usize,
    pub ring: usize,
    pub limiter: usize,
    pub snapshot_count: usize,
    pub snapshot_bytes: usize,
    pub log_count: usize,
    pub log_age: usize,
}

impl ViolationCounts {
    pub fn total(&self) -> usize {
        self.store
            + self.ring
            + self.limiter
            + self.snapshot_count
            + self.snapshot_bytes
            + self.log_count
            + self.log_age
    }
}

/// Observed state at the end of a soak run for the final report line.
#[derive(Debug)]
pub struct RunReport {
    pub cycles: u64,
    pub elapsed: Duration,
    pub first_store_len: Option<usize>,
    pub last_store_len: usize,
    pub ring_len: usize,
    pub limiter_records: usize,
    pub snapshot_files: usize,
    pub telemetry_bytes: u64,
    pub log_files: usize,
    pub violations: ViolationCounts,
}

impl RunReport {
    pub fn print(&self, label: &str) {
        eprintln!(
            "{label} cycles={} elapsed_ms={} cycles_per_sec={:.0} store_len_first={:?} store_len_last={} ring_len={} limiter_records={} snapshot_files={} telemetry_bytes={} log_files={} violations={{store:{} ring:{} limiter:{} snapshot_count:{} snapshot_bytes:{} log_count:{} log_age:{}}}",
            self.cycles,
            self.elapsed.as_millis(),
            self.cycles as f64 / self.elapsed.as_secs_f64().max(f64::EPSILON),
            self.first_store_len,
            self.last_store_len,
            self.ring_len,
            self.limiter_records,
            self.snapshot_files,
            self.telemetry_bytes,
            self.log_files,
            self.violations.store,
            self.violations.ring,
            self.violations.limiter,
            self.violations.snapshot_count,
            self.violations.snapshot_bytes,
            self.violations.log_count,
            self.violations.log_age,
        );
    }
}

/// Drives the mixed workload until `should_stop` reports done, sampling the
/// resource bounds every `SAMPLE_STRIDE` cycles and exercising the telemetry
/// and daily log retention passes on scratch directories. Returns the run
/// report with violation tallies and final buffered sizes.
pub fn drive_workload(
    seed: u64,
    should_stop: &dyn Fn(u64, Duration) -> bool,
    label: &str,
) -> RunReport {
    let telemetry_dir = temporary_directory(&format!("{label}-telemetry"));
    let log_dir = temporary_directory(&format!("{label}-logs"));
    std::fs::create_dir_all(&telemetry_dir).expect("create telemetry dir");
    std::fs::create_dir_all(&log_dir).expect("create log dir");

    let mut workload = MixedWorkload::new(seed);
    let mut ring_buffer: VecDeque<u64> = VecDeque::new();
    let mut violations = ViolationCounts::default();
    let mut first_store_len: Option<usize> = None;
    let mut last_store_len = 0usize;

    let log_now = 1_800_000_000u64;
    let mut snapshot_index = 0usize;
    let mut log_index = 0usize;
    let payload = vec![0xABu8; 65_536];

    let started = Instant::now();
    let mut iteration = 0u64;

    loop {
        iteration += 1;
        workload.run_one(iteration);

        ring_buffer.push_back(iteration);
        if ring_buffer.len() > EVENT_BUFFER_CAP {
            ring_buffer.pop_front();
        }

        if iteration.is_multiple_of(SAMPLE_STRIDE as u64) {
            let store_len = workload.store_len();
            if first_store_len.is_none() {
                first_store_len = Some(store_len);
            }
            last_store_len = store_len;
            if store_len > HARD_MAX_EVENTS {
                violations.store += 1;
            }
            if ring_buffer.len() > EVENT_BUFFER_CAP {
                violations.ring += 1;
            }
            if workload.limiter_records() > MAX_TRACKED_PIDS {
                violations.limiter += 1;
            }

            write_snapshot_file(&telemetry_dir, snapshot_index, &payload);
            snapshot_index += 1;
            let report =
                enforce_snapshot_retention(&telemetry_dir).expect("snapshot retention succeeds");
            if report.retained_bytes > MAX_TELEMETRY_DIRECTORY_BYTES {
                violations.snapshot_bytes += 1;
            }
            if count_snapshot_files(&telemetry_dir) > MAX_RETAINED_SNAPSHOTS {
                violations.snapshot_count += 1;
            }

            let day_unix =
                log_now.saturating_sub((log_index as u64).saturating_mul(43_200)) / 86_400 * 86_400;
            let (year, month, day) = unix_seconds_to_utc_day(day_unix);
            let name = format!("true-tick-{year:04}-{month:02}-{day:02}.csv");
            std::fs::write(log_dir.join(&name), "Row\n1,x\n").expect("log write");
            log_index += 1;
            let purge = purge_expired_logs(&log_dir, log_now).expect("log purge succeeds");
            if purge.retained > MAX_RETAINED_LOG_FILES {
                violations.log_count += 1;
            }
            let cutoff = log_now.saturating_sub(LOG_RETENTION_DAYS * 86_400);
            if daily_log_unix_seconds(&log_dir)
                .iter()
                .any(|day_unix| *day_unix < cutoff)
            {
                violations.log_age += 1;
            }
            if count_daily_logs(&log_dir) > MAX_RETAINED_LOG_FILES {
                violations.log_count += 1;
            }
        }

        if should_stop(iteration, started.elapsed()) {
            break;
        }
    }

    let report = RunReport {
        cycles: iteration,
        elapsed: started.elapsed(),
        first_store_len,
        last_store_len,
        ring_len: ring_buffer.len(),
        limiter_records: workload.limiter_records(),
        snapshot_files: count_snapshot_files(&telemetry_dir),
        telemetry_bytes: telemetry_directory_bytes(&telemetry_dir),
        log_files: count_daily_logs(&log_dir),
        violations,
    };

    let _ = std::fs::remove_dir_all(&telemetry_dir);
    let _ = std::fs::remove_dir_all(&log_dir);
    report
}
