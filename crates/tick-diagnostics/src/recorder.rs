//! Bounded anomaly snapshot recorder.
//!
//! Records a classified failure vector, a bounded detail string, a fixed set of
//! environment counters, and up to the most recent `MAX_SNAPSHOT_EVENTS`
//! diagnostic events. Each snapshot is serialized into a deterministic
//! line-oriented format and written to disk through a temporary file followed by
//! an atomic rename. The snapshot digest is a SHA-256 over the serialized
//! payload.

use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{format_event, truncate_utf8, DiagnosticEvent};

/// Maximum byte length of the bounded detail string stored in a snapshot.
pub const MAX_SNAPSHOT_DETAIL_BYTES: usize = 1024;

/// Maximum number of recent events retained in a snapshot.
pub const MAX_SNAPSHOT_EVENTS: usize = 64;

/// Maximum number of anomaly snapshot files retained in the telemetry directory.
pub const MAX_RETAINED_SNAPSHOTS: usize = 16;

/// Maximum combined byte size of anomaly snapshot files retained in the
/// telemetry directory.
pub const MAX_TELEMETRY_DIRECTORY_BYTES: u64 = 8 * 1024 * 1024;

/// Fixed set of machine counters captured at the time of the anomaly.
///
/// All fields are plain integer counters. Fields whose value is not
/// collectible on the current platform are set to zero by the caller.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EnvironmentSnapshot {
    pub os_major: u32,
    pub os_minor: u32,
    pub os_build: u32,
    pub os_ubr: u32,
    pub cpu_arch: u32,
    pub processor_count: u32,
    pub ac_line_status: u32,
    pub battery_percent: u32,
    pub uptime_ms: u64,
}

/// Outcome of a single snapshot retention pass over the telemetry directory.
///
/// `examined` counts every regular file matching the snapshot name pattern
/// before any deletion. `removed` and `bytes_removed` describe the files
/// deleted by the pass. `retained_bytes` is the summed size of the snapshot
/// files that remain.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RetentionReport {
    pub examined: usize,
    pub removed: usize,
    pub bytes_removed: u64,
    pub retained_bytes: u64,
}

impl RetentionReport {
    /// Renders a bounded single-line description of the retention pass.
    pub fn summary(&self) -> String {
        format!(
            "retention examined={} removed={} bytes_removed={} retained_bytes={}",
            self.examined, self.removed, self.bytes_removed, self.retained_bytes
        )
    }
}

/// Classification of the detected failure vector.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureVector {
    Panic,
    WatchdogStall,
    PostconditionUnverified,
    ConfigCorruption,
    LogWriteFailure,
    SessionMarkerCorruption,
    Unknown,
}

impl FailureVector {
    /// Returns the stable string label used in the serialized format.
    pub fn as_str(&self) -> &'static str {
        match self {
            FailureVector::Panic => "Panic",
            FailureVector::WatchdogStall => "WatchdogStall",
            FailureVector::PostconditionUnverified => "PostconditionUnverified",
            FailureVector::ConfigCorruption => "ConfigCorruption",
            FailureVector::LogWriteFailure => "LogWriteFailure",
            FailureVector::SessionMarkerCorruption => "SessionMarkerCorruption",
            FailureVector::Unknown => "Unknown",
        }
    }
}

/// Bounded record of a single anomaly.
///
/// The `detail` field is bounded to `MAX_SNAPSHOT_DETAIL_BYTES` and
/// `recent_events` is capped at `MAX_SNAPSHOT_EVENTS`. `snapshot_digest`
/// contains the lowercase hexadecimal SHA-256 of the serialized payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnomalySnapshot {
    pub vector: FailureVector,
    pub detail: String,
    pub environment: EnvironmentSnapshot,
    pub recent_events: Vec<DiagnosticEvent>,
    pub snapshot_digest: String,
}

fn unix_time_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest.iter() {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

fn is_snapshot_file_name(name: &str) -> bool {
    name.starts_with("anomaly-") && name.ends_with(".recorder")
}

/// Builds an anomaly snapshot from the supplied fields.
///
/// The detail string is truncated at a UTF-8 boundary so it never exceeds
/// `MAX_SNAPSHOT_DETAIL_BYTES`. At most the last `MAX_SNAPSHOT_EVENTS` events
/// are copied. The snapshot digest is computed over the serialized payload.
pub fn build_snapshot(
    vector: FailureVector,
    detail: &str,
    environment: EnvironmentSnapshot,
    events: &[DiagnosticEvent],
) -> AnomalySnapshot {
    let bounded_detail = truncate_utf8(detail, MAX_SNAPSHOT_DETAIL_BYTES);
    let start = events.len().saturating_sub(MAX_SNAPSHOT_EVENTS);
    let recent_events: Vec<DiagnosticEvent> = events[start..].to_vec();

    let mut snapshot = AnomalySnapshot {
        vector,
        detail: bounded_detail,
        environment,
        recent_events,
        snapshot_digest: String::new(),
    };
    let payload = serialize_snapshot(&snapshot);
    snapshot.snapshot_digest = sha256_hex(payload.as_bytes());
    snapshot
}

/// Serializes a snapshot into a deterministic line-oriented text format.
///
/// The output contains a `[snapshot]` header, an `[environment]` block, an
/// `[events]` block, and a `[digest]` line. All field values are written as
/// ASCII text with one key per line.
pub fn serialize_snapshot(snapshot: &AnomalySnapshot) -> String {
    let mut output = String::new();
    output.push_str("[snapshot]\n");
    output.push_str(&format!("vector={}\n", snapshot.vector.as_str()));
    output.push_str(&format!("detail={}\n", snapshot.detail));
    output.push_str(&format!("event_count={}\n", snapshot.recent_events.len()));

    output.push_str("[environment]\n");
    output.push_str(&format!("os_major={}\n", snapshot.environment.os_major));
    output.push_str(&format!("os_minor={}\n", snapshot.environment.os_minor));
    output.push_str(&format!("os_build={}\n", snapshot.environment.os_build));
    output.push_str(&format!("os_ubr={}\n", snapshot.environment.os_ubr));
    output.push_str(&format!("cpu_arch={}\n", snapshot.environment.cpu_arch));
    output.push_str(&format!(
        "processor_count={}\n",
        snapshot.environment.processor_count
    ));
    output.push_str(&format!(
        "ac_line_status={}\n",
        snapshot.environment.ac_line_status
    ));
    output.push_str(&format!(
        "battery_percent={}\n",
        snapshot.environment.battery_percent
    ));
    output.push_str(&format!("uptime_ms={}\n", snapshot.environment.uptime_ms));

    output.push_str("[events]\n");
    for (index, event) in snapshot.recent_events.iter().enumerate() {
        let rendered = format_event(event);
        output.push_str(&format!("event[{index}]={rendered}\n"));
    }

    output.push_str("[digest]\n");
    output.push_str(&format!("sha256={}\n", snapshot.snapshot_digest));
    output
}

/// Enforces the snapshot retention bounds inside `telemetry_dir`.
///
/// Only regular files named `anomaly-*.recorder` directly inside
/// `telemetry_dir` are considered. Matching files are ordered by modification
/// time ascending with the file name as a deterministic tiebreak. While the
/// count exceeds `MAX_RETAINED_SNAPSHOTS` or the summed size exceeds
/// `MAX_TELEMETRY_DIRECTORY_BYTES`, the oldest matching file is deleted and
/// its size is subtracted. Non-matching files and directories are never
/// deleted and the scan never recurses.
pub fn enforce_snapshot_retention(telemetry_dir: &Path) -> Result<RetentionReport, io::Error> {
    struct Candidate {
        path: PathBuf,
        name: String,
        modified: SystemTime,
        size: u64,
    }

    let mut candidates: Vec<Candidate> = Vec::new();
    for entry in fs::read_dir(telemetry_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if !is_snapshot_file_name(&name) {
            continue;
        }
        let metadata = entry.metadata()?;
        candidates.push(Candidate {
            path: entry.path(),
            name,
            modified: metadata.modified().unwrap_or(UNIX_EPOCH),
            size: metadata.len(),
        });
    }

    candidates.sort_by(|left, right| {
        left.modified
            .cmp(&right.modified)
            .then_with(|| left.name.cmp(&right.name))
    });

    let examined = candidates.len();
    let mut total_bytes: u64 = candidates.iter().map(|candidate| candidate.size).sum();
    let mut removed = 0usize;
    let mut bytes_removed = 0u64;
    let mut index = 0usize;

    while index < candidates.len()
        && (candidates.len() - index > MAX_RETAINED_SNAPSHOTS
            || total_bytes > MAX_TELEMETRY_DIRECTORY_BYTES)
    {
        let candidate = &candidates[index];
        fs::remove_file(&candidate.path)?;
        total_bytes = total_bytes.saturating_sub(candidate.size);
        bytes_removed += candidate.size;
        removed += 1;
        index += 1;
    }

    Ok(RetentionReport {
        examined,
        removed,
        bytes_removed,
        retained_bytes: total_bytes,
    })
}

/// Writes the serialized snapshot to `target_dir/telemetry` atomically and
/// then enforces snapshot retention on the telemetry directory.
///
/// The write uses the same temporary file plus rename sequence as
/// `write_snapshot`. The returned tuple contains the final snapshot path and
/// the retention report. A retention failure does not remove or invalidate
/// the freshly written snapshot and is reported as `None`.
pub fn write_snapshot_with_retention(
    snapshot: &AnomalySnapshot,
    target_dir: &Path,
) -> Result<(PathBuf, Option<RetentionReport>), io::Error> {
    let telemetry_dir = target_dir.join("telemetry");
    fs::create_dir_all(&telemetry_dir)?;

    let unix_ms = unix_time_millis();
    let temp_name = format!("anomaly-{unix_ms}.recorder.tmp");
    let final_name = format!("anomaly-{unix_ms}.recorder");
    let temp_path = telemetry_dir.join(&temp_name);
    let final_path = telemetry_dir.join(&final_name);

    let serialized = serialize_snapshot(snapshot);

    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&temp_path)?;
    file.write_all(serialized.as_bytes())?;
    file.sync_all()?;
    drop(file);

    fs::rename(&temp_path, &final_path)?;
    let report = enforce_snapshot_retention(&telemetry_dir).ok();
    Ok((final_path, report))
}

/// Writes the serialized snapshot to `target_dir/telemetry` atomically.
///
/// The content is first written to `anomaly-<unix_ms>.recorder.tmp`, the file
/// is synchronized to disk, and the file is renamed to
/// `anomaly-<unix_ms>.recorder`. The final path is returned. Snapshot
/// retention is enforced after the write and the report is discarded.
pub fn write_snapshot(snapshot: &AnomalySnapshot, target_dir: &Path) -> Result<PathBuf, io::Error> {
    write_snapshot_with_retention(snapshot, target_dir).map(|(path, _report)| path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        compute_entry_hash, genesis_hash, DiagnosticOutcome, DiagnosticPhase, DiagnosticSource,
        NativeOutcome,
    };
    use std::time::Duration;

    fn sample_environment() -> EnvironmentSnapshot {
        EnvironmentSnapshot {
            os_major: 10,
            os_minor: 0,
            os_build: 22631,
            os_ubr: 1,
            cpu_arch: 0x8664,
            processor_count: 8,
            ac_line_status: 1,
            battery_percent: 100,
            uptime_ms: 60_000,
        }
    }

    fn sample_event(sequence: u64) -> DiagnosticEvent {
        let name = "recorder.test".to_owned();
        let details = format!("seq={sequence}");
        let entry_hash = compute_entry_hash(genesis_hash(), sequence, 0u128, &name, &details);
        DiagnosticEvent {
            sequence,
            elapsed: Duration::from_millis(sequence),
            operation_id: sequence,
            parent_operation_id: None,
            correlation_id: 1,
            phase: DiagnosticPhase::Observe,
            source: DiagnosticSource::Internal,
            outcome: DiagnosticOutcome::Completed,
            native: NativeOutcome {
                ntstatus: None,
                win32_last_error: None,
                requested_hns: None,
                selected_hns: None,
                effective_hns: None,
            },
            name,
            details,
            prev_hash: genesis_hash(),
            entry_hash,
        }
    }

    fn unique_test_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "tick-diagnostics-{tag}-{}-{}",
            std::process::id(),
            unix_time_millis()
        ))
    }

    fn write_raw_file(path: &Path, bytes: &[u8]) {
        fs::write(path, bytes).expect("raw write failed");
    }

    fn snapshot_file_count(telemetry_dir: &Path) -> usize {
        fs::read_dir(telemetry_dir)
            .expect("read_dir failed")
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry.file_name().to_string_lossy().starts_with("anomaly-")
                    && entry.file_name().to_string_lossy().ends_with(".recorder")
            })
            .count()
    }

    #[test]
    fn snapshot_detail_truncates_at_unicode_boundary() {
        let mut detail = "x".repeat(MAX_SNAPSHOT_DETAIL_BYTES);
        detail.push('\u{2603}');
        detail.push_str("extra");
        let snapshot = build_snapshot(FailureVector::Unknown, &detail, sample_environment(), &[]);
        assert!(snapshot.detail.len() <= MAX_SNAPSHOT_DETAIL_BYTES + 3);
        assert!(snapshot.detail.is_char_boundary(snapshot.detail.len()));
        assert!(snapshot.detail.ends_with("..."));
    }

    #[test]
    fn snapshot_events_capped_at_sixty_four() {
        let events: Vec<DiagnosticEvent> = (0..100).map(sample_event).collect();
        let snapshot = build_snapshot(
            FailureVector::Panic,
            "cap test",
            sample_environment(),
            &events,
        );
        assert_eq!(snapshot.recent_events.len(), MAX_SNAPSHOT_EVENTS);
        assert_eq!(snapshot.recent_events[0].sequence, 36);
    }

    #[test]
    fn snapshot_digest_is_deterministic_for_same_input() {
        let events = vec![sample_event(1), sample_event(2)];
        let first = build_snapshot(
            FailureVector::WatchdogStall,
            "stable detail",
            sample_environment(),
            &events,
        );
        let second = build_snapshot(
            FailureVector::WatchdogStall,
            "stable detail",
            sample_environment(),
            &events,
        );
        assert_eq!(first.snapshot_digest, second.snapshot_digest);
        assert_eq!(first.snapshot_digest.len(), 64);
        assert!(first.snapshot_digest.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn serialize_contains_all_sections() {
        let events = vec![sample_event(7)];
        let snapshot = build_snapshot(
            FailureVector::LogWriteFailure,
            "io error",
            sample_environment(),
            &events,
        );
        let serialized = serialize_snapshot(&snapshot);
        assert!(serialized.contains("[snapshot]"));
        assert!(serialized.contains("[environment]"));
        assert!(serialized.contains("[events]"));
        assert!(serialized.contains("[digest]"));
        assert!(serialized.contains("vector=LogWriteFailure"));
        assert!(serialized.contains("detail=io error"));
        assert!(serialized.contains("sha256="));
        assert!(serialized.contains("event[0]="));
    }

    #[test]
    fn write_snapshot_creates_telemetry_directory_and_leaves_no_temp_file() {
        let target = std::env::temp_dir().join(format!(
            "tick-diagnostics-recorder-test-{}",
            unix_time_millis()
        ));
        let _ = fs::remove_dir_all(&target);

        let snapshot = build_snapshot(
            FailureVector::ConfigCorruption,
            "config mismatch",
            sample_environment(),
            &[sample_event(3)],
        );
        let written = write_snapshot(&snapshot, &target).expect("write failed");
        assert!(written.exists());
        assert!(written.extension().is_some_and(|ext| ext == "recorder"));
        assert!(!written.to_string_lossy().ends_with(".tmp"));

        let telemetry_dir = target.join("telemetry");
        assert!(telemetry_dir.is_dir());

        let mut temp_files = 0;
        let mut final_files = 0;
        for entry in fs::read_dir(&telemetry_dir).expect("read_dir failed") {
            let entry = entry.expect("entry failed");
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(".tmp") {
                temp_files += 1;
            } else if name.ends_with(".recorder") {
                final_files += 1;
            }
        }
        assert_eq!(temp_files, 0);
        assert_eq!(final_files, 1);

        let content = fs::read_to_string(&written).expect("read failed");
        assert!(content.contains("[snapshot]"));
        assert!(content.contains("vector=ConfigCorruption"));

        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn retention_removes_oldest_beyond_count_cap() {
        let target = unique_test_dir("retention-count");
        let telemetry_dir = target.join("telemetry");
        let _ = fs::remove_dir_all(&target);
        fs::create_dir_all(&telemetry_dir).expect("create_dir_all failed");

        let overflow = 3usize;
        let total = MAX_RETAINED_SNAPSHOTS + overflow;
        for index in 0..total {
            let name = format!("anomaly-{index:05}.recorder");
            write_raw_file(&telemetry_dir.join(&name), b"x");
            std::thread::sleep(Duration::from_millis(2));
        }

        let report = enforce_snapshot_retention(&telemetry_dir).expect("retention failed");
        assert_eq!(report.examined, total);
        assert_eq!(report.removed, overflow);
        assert_eq!(snapshot_file_count(&telemetry_dir), MAX_RETAINED_SNAPSHOTS);
        for index in 0..overflow {
            let name = format!("anomaly-{index:05}.recorder");
            assert!(!telemetry_dir.join(&name).exists());
        }
        for index in overflow..total {
            let name = format!("anomaly-{index:05}.recorder");
            assert!(telemetry_dir.join(&name).exists());
        }

        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn retention_removes_oldest_beyond_byte_cap() {
        let target = unique_test_dir("retention-bytes");
        let telemetry_dir = target.join("telemetry");
        let _ = fs::remove_dir_all(&target);
        fs::create_dir_all(&telemetry_dir).expect("create_dir_all failed");

        // Fewer files than the count cap, each large enough that two files
        // exceed the byte cap. The oldest must be removed first.
        let file_size = (MAX_TELEMETRY_DIRECTORY_BYTES / 2 + 1) as usize;
        let payload = vec![b'y'; file_size];
        for index in 0..2usize {
            let name = format!("anomaly-{index:05}.recorder");
            write_raw_file(&telemetry_dir.join(&name), &payload);
            std::thread::sleep(Duration::from_millis(2));
        }

        let report = enforce_snapshot_retention(&telemetry_dir).expect("retention failed");
        assert_eq!(report.examined, 2);
        assert_eq!(report.removed, 1);
        assert_eq!(report.bytes_removed, file_size as u64);
        assert_eq!(report.retained_bytes, file_size as u64);
        assert!(!telemetry_dir.join("anomaly-00000.recorder").exists());
        assert!(telemetry_dir.join("anomaly-00001.recorder").exists());

        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn retention_ignores_non_matching_files() {
        let target = unique_test_dir("retention-ignore");
        let telemetry_dir = target.join("telemetry");
        let _ = fs::remove_dir_all(&target);
        fs::create_dir_all(&telemetry_dir).expect("create_dir_all failed");

        write_raw_file(&telemetry_dir.join("notes.txt"), b"keep");
        write_raw_file(&telemetry_dir.join("anomaly-00000.recorder.tmp"), b"keep");
        write_raw_file(&telemetry_dir.join("anomaly-x.recorder.bak"), b"keep");
        write_raw_file(&telemetry_dir.join("other-00000.recorder"), b"keep");

        let report = enforce_snapshot_retention(&telemetry_dir).expect("retention failed");
        assert_eq!(report.examined, 0);
        assert_eq!(report.removed, 0);
        assert!(telemetry_dir.join("notes.txt").exists());
        assert!(telemetry_dir.join("anomaly-00000.recorder.tmp").exists());
        assert!(telemetry_dir.join("anomaly-x.recorder.bak").exists());
        assert!(telemetry_dir.join("other-00000.recorder").exists());

        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn retention_never_deletes_directories() {
        let target = unique_test_dir("retention-dirs");
        let telemetry_dir = target.join("telemetry");
        let _ = fs::remove_dir_all(&target);
        fs::create_dir_all(&telemetry_dir).expect("create_dir_all failed");

        let matching_dir = telemetry_dir.join("anomaly-00000.recorder");
        fs::create_dir_all(&matching_dir).expect("create_dir failed");
        write_raw_file(&telemetry_dir.join("anomaly-00001.recorder"), b"x");

        let report = enforce_snapshot_retention(&telemetry_dir).expect("retention failed");
        assert_eq!(report.examined, 1);
        assert_eq!(report.removed, 0);
        assert!(matching_dir.is_dir());
        assert!(telemetry_dir.join("anomaly-00001.recorder").exists());

        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn retention_report_counts_are_accurate() {
        let target = unique_test_dir("retention-report");
        let telemetry_dir = target.join("telemetry");
        let _ = fs::remove_dir_all(&target);
        fs::create_dir_all(&telemetry_dir).expect("create_dir_all failed");

        let sizes = [10u64, 20, 30, 40];
        for (index, size) in sizes.iter().enumerate() {
            let name = format!("anomaly-{index:05}.recorder");
            write_raw_file(&telemetry_dir.join(&name), &vec![b'z'; *size as usize]);
            std::thread::sleep(Duration::from_millis(2));
        }
        write_raw_file(&telemetry_dir.join("unrelated.bin"), b"ignored");

        let report = enforce_snapshot_retention(&telemetry_dir).expect("retention failed");
        assert_eq!(report.examined, 4);
        assert_eq!(report.removed, 0);
        assert_eq!(report.bytes_removed, 0);
        assert_eq!(report.retained_bytes, 100);

        let summary = report.summary();
        assert!(!summary.contains('\n'));
        assert!(summary.contains("examined=4"));
        assert!(summary.contains("removed=0"));
        assert!(summary.contains("bytes_removed=0"));
        assert!(summary.contains("retained_bytes=100"));

        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn write_snapshot_with_retention_reports_cleanup() {
        let target = unique_test_dir("retention-write");
        let telemetry_dir = target.join("telemetry");
        let _ = fs::remove_dir_all(&target);
        fs::create_dir_all(&telemetry_dir).expect("create_dir_all failed");

        for index in 0..MAX_RETAINED_SNAPSHOTS {
            let name = format!("anomaly-{index:05}.recorder");
            write_raw_file(&telemetry_dir.join(&name), b"old");
            std::thread::sleep(Duration::from_millis(2));
        }

        let snapshot = build_snapshot(
            FailureVector::Panic,
            "retention write",
            sample_environment(),
            &[sample_event(1)],
        );
        let (written, report) =
            write_snapshot_with_retention(&snapshot, &target).expect("write failed");
        assert!(written.exists());
        let report = report.expect("report missing");
        assert_eq!(report.examined, MAX_RETAINED_SNAPSHOTS + 1);
        assert_eq!(report.removed, 1);
        assert!(report.bytes_removed > 0);
        assert_eq!(snapshot_file_count(&telemetry_dir), MAX_RETAINED_SNAPSHOTS);

        let _ = fs::remove_dir_all(&target);
    }
}
