use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tick_diagnostics::retention::purge_expired_logs;
use tick_diagnostics::{diagnostic_grid_row, truncate_utf8, DiagnosticStore, MAX_FIELD_LENGTH};

pub const MIN_FREE_BYTES_BEFORE_WRITE: u64 = 50 * 1024 * 1024;

pub fn resolve_log_directory(executable: &Path) -> PathBuf {
    match crate::portable::portable_root_from_slot_executable(executable) {
        Ok(root) => root.join("Data").join("logs"),
        Err(_) => executable.parent().unwrap_or(Path::new(".")).join("logs"),
    }
}

pub fn resolve_state_directory(executable: &Path) -> PathBuf {
    match crate::portable::portable_root_from_slot_executable(executable) {
        Ok(root) => root.join("Data").join("state"),
        Err(_) => executable.parent().unwrap_or(Path::new(".")).join("state"),
    }
}

pub fn daily_log_filename(year: u16, month: u16, day: u16) -> String {
    let date = tick_core::format_ymd(i64::from(year), u32::from(month), u32::from(day));
    format!("true-tick-{date}.csv")
}

pub fn hex_hash_string(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(64);
    for byte in bytes.iter() {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

/// Current UTC calendar date derived from the epoch day count so the daily
/// file name matches the canonical tick-core civil date helpers.
pub fn utc_date_now() -> (u16, u16, u16) {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let (year, month, day) = tick_core::civil_from_days((seconds / 86_400) as i64);
    (year as u16, month as u16, day as u16)
}

pub fn log_csv_escape(cell: &str) -> String {
    let field = truncate_utf8(cell, MAX_FIELD_LENGTH);
    if !field.contains([',', '"', '\n', '\r']) {
        return field;
    }
    let mut quoted = String::with_capacity(field.len() + 2);
    quoted.push('"');
    for character in field.chars() {
        if character == '"' {
            quoted.push('"');
        }
        quoted.push(character);
    }
    quoted.push('"');
    quoted
}

/// Header row for the daily CSV log, listing the eleven report columns plus
/// the two integrity chain columns. It is written exactly once per daily file.
pub const LOG_HEADER: &str = "Row,Sequence,Elapsed,Operation,Parent,Correlation,Phase,Source,Outcome,Event,Details,PrevHash,EntryHash";

/// Serializes the header check and the append so a header can never be written
/// twice by concurrent flushes. Each flush runs on its own spawned thread, so
/// the process wide lock is the only shared state between writers.
static LOG_APPEND_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Limits detached log writer threads to one at a time. When a flush is already
/// active, additional append requests coalesce by dropping their payload because
/// the diagnostic buffer is snapshotted fresh on every flush and the next flush
/// cycle republishes anything the dropped thread would have written.
static LOG_WRITER_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Resets `LOG_WRITER_ACTIVE` when the writer thread finishes, including paths
/// that exit early through an IO error.
struct LogWriterActiveGuard;

impl Drop for LogWriterActiveGuard {
    fn drop(&mut self) {
        LOG_WRITER_ACTIVE.store(false, std::sync::atomic::Ordering::Release);
    }
}

/// Error classification for a suppressed or failed daily log write.
#[derive(Debug)]
pub enum LogWriteError {
    Io(std::io::Error),
    InsufficientFreeSpace { available: u64 },
}

impl std::fmt::Display for LogWriteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogWriteError::Io(error) => write!(formatter, "{error}"),
            LogWriteError::InsufficientFreeSpace { available } => {
                write!(formatter, "insufficient free space available={available}")
            }
        }
    }
}

impl std::error::Error for LogWriteError {}

impl From<std::io::Error> for LogWriteError {
    fn from(error: std::io::Error) -> Self {
        LogWriteError::Io(error)
    }
}

/// Queries free bytes available to the current user on the volume that hosts
/// `directory`. Returns `None` when the query fails so callers treat an
/// unavailable probe as neutral evidence rather than a exhaustion signal.
pub fn free_bytes_available(directory: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    #[repr(C)]
    #[derive(Default)]
    struct UlargeInteger {
        low_part: u32,
        high_part: u32,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetDiskFreeSpaceExW(
            directory_name: *const u16,
            free_bytes_available_to_caller: *mut UlargeInteger,
            total_number_of_bytes: *mut UlargeInteger,
            total_number_of_free_bytes: *mut UlargeInteger,
        ) -> i32;
    }
    let wide: Vec<u16> = directory
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut available = UlargeInteger::default();
    let mut total = UlargeInteger::default();
    let mut total_free = UlargeInteger::default();
    let ok =
        unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut available, &mut total, &mut total_free) };
    if ok == 0 {
        return None;
    }
    Some((u64::from(available.high_part) << 32) | u64::from(available.low_part))
}

/// Runs retention and records a diagnostic when it fails. Purge failures are
/// never allowed to block logging because a failed cleanup is less dangerous
/// than a missing forensic trail.
fn purge_expired_logs_with_diagnostics(log_dir: &Path, diagnostics: &DiagnosticStore) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    match purge_expired_logs(log_dir, now) {
        Ok(report) => {
            if report.removed > 0 {
                diagnostics.record(
                    "storage.retention.purge",
                    format!(
                        "examined={} removed={} retained={}",
                        report.examined, report.removed, report.retained
                    ),
                );
            }
        }
        Err(error) => {
            diagnostics.record_with_outcome(
                "storage.retention.purge.error",
                format!("error={error}"),
                tick_diagnostics::DiagnosticOutcome::Failed,
            );
        }
    }
}

/// Writes `lines` to the log file already opened for appending.
///
/// The header row is emitted when the file is absent (the open creates it empty)
/// or already empty. The header write and the first data write happen under the
/// same lock and the same append handle, so a fresh log is never left without a
/// header and never receives a duplicated header.
fn write_log_lines(file: &mut std::fs::File, lines: &[String]) -> std::io::Result<()> {
    use std::io::Write;
    if file.metadata()?.len() == 0 {
        writeln!(file, "{LOG_HEADER}")?;
    }
    for line in lines {
        writeln!(file, "{line}")?;
    }
    file.flush()?;
    file.sync_all()
}

/// Appends `lines` to `directory/filename` on the calling thread, creating the
/// directory and the file when needed. The `free_space_probe` hook lets tests
/// inject a deterministic free-space reading without requiring a genuinely
/// full disk.
/// Records a bounded `storage.append.error` diagnostic for a raw IO failure
/// so the anomaly ladder sees append faults that carry no forensic row. The
/// detail carries only the error kind so no OS message or path leaks into the
/// forensic log, and exactly one event is emitted per failed append.
fn record_append_io_error(diagnostics: Option<&DiagnosticStore>, error: &std::io::Error) {
    if let Some(store) = diagnostics {
        store.record_with_outcome(
            "storage.append.error",
            format!("result=io_failure kind={:?}", error.kind()),
            tick_diagnostics::DiagnosticOutcome::Failed,
        );
    }
}

/// Probe signature for free-space queries. Tests substitute a deterministic
/// closure so suppression can be exercised without a genuinely full disk.
type FreeSpaceProbe = dyn Fn(&Path) -> Option<u64>;

fn append_log_lines_sync_internal(
    directory: &Path,
    filename: &str,
    lines: &[String],
    diagnostics: Option<&DiagnosticStore>,
    free_space_probe: Option<&FreeSpaceProbe>,
) -> Result<(), LogWriteError> {
    std::fs::create_dir_all(directory).map_err(|error| {
        record_append_io_error(diagnostics, &error);
        LogWriteError::Io(error)
    })?;
    let path = directory.join(filename);
    let probe = free_space_probe.unwrap_or(&free_bytes_available);
    if let Some(available) = probe(directory) {
        if available < MIN_FREE_BYTES_BEFORE_WRITE {
            if let Some(store) = diagnostics {
                store.record_with_outcome(
                    "storage.write_suppressed",
                    format!("reason=low_free_space available={available}"),
                    tick_diagnostics::DiagnosticOutcome::Suppressed,
                );
            }
            return Err(LogWriteError::InsufficientFreeSpace { available });
        }
    }
    let file_is_new = !path.exists();
    let _guard = LOG_APPEND_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| {
            record_append_io_error(diagnostics, &error);
            LogWriteError::Io(error)
        })?;
    let result = write_log_lines(&mut file, lines).map_err(|error| {
        record_append_io_error(diagnostics, &error);
        LogWriteError::Io(error)
    });
    if file_is_new {
        if let Some(store) = diagnostics {
            purge_expired_logs_with_diagnostics(directory, store);
        }
    }
    result
}

/// Compatibility wrapper used by existing tests and shutdown paths that do
/// not carry a diagnostic store.
#[cfg(test)]
fn append_log_lines_sync(
    directory: &Path,
    filename: &str,
    lines: &[String],
) -> std::io::Result<()> {
    append_log_lines_sync_internal(directory, filename, lines, None, None).map_err(|error| {
        match error {
            LogWriteError::Io(error) => error,
            LogWriteError::InsufficientFreeSpace { available } => std::io::Error::new(
                std::io::ErrorKind::StorageFull,
                format!("insufficient free space available={available}"),
            ),
        }
    })
}

/// Diagnostic-aware append used by the flush path so suppression is recorded.
fn append_log_lines_sync_with_diagnostics(
    directory: &Path,
    filename: &str,
    lines: &[String],
    diagnostics: &DiagnosticStore,
) -> Result<(), LogWriteError> {
    append_log_lines_sync_internal(directory, filename, lines, Some(diagnostics), None)
}

#[allow(dead_code)]
pub fn append_log_lines_with_commit(
    directory: PathBuf,
    filename: String,
    lines: Vec<String>,
    committed: Arc<AtomicU64>,
    last_sequence: u64,
) {
    if LOG_WRITER_ACTIVE
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        )
        .is_err()
    {
        return;
    }
    std::thread::spawn(move || {
        let _active_guard = LogWriterActiveGuard;
        match append_log_lines_sync_internal(&directory, &filename, &lines, None, None) {
            Ok(()) => {
                committed.store(last_sequence, Ordering::Release);
            }
            Err(write_error) => {
                eprintln!("true-tick log write failed: {write_error}");
            }
        }
    });
}

/// Diagnostic-aware variant that threads the store through to the worker so a
/// suppressed write can be recorded in the same session it was skipped.
pub fn append_log_lines_with_commit_and_diagnostics(
    directory: PathBuf,
    filename: String,
    lines: Vec<String>,
    committed: Arc<AtomicU64>,
    last_sequence: u64,
    diagnostics: DiagnosticStore,
) {
    if LOG_WRITER_ACTIVE
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        )
        .is_err()
    {
        return;
    }
    std::thread::spawn(move || {
        let _active_guard = LogWriterActiveGuard;
        match append_log_lines_sync_with_diagnostics(&directory, &filename, &lines, &diagnostics) {
            Ok(()) => {
                committed.store(last_sequence, Ordering::Release);
            }
            Err(write_error) => {
                eprintln!("true-tick log write failed: {write_error}");
            }
        }
    });
}

fn build_log_lines(
    diagnostics: &DiagnosticStore,
    last_persisted_event_sequence: u64,
) -> (Vec<String>, u64) {
    let events = diagnostics.snapshot();
    let mut lines = Vec::new();
    let mut last_sequence = last_persisted_event_sequence;
    for (index, event) in events.iter().enumerate() {
        if event.sequence <= last_persisted_event_sequence {
            continue;
        }
        let row = diagnostic_grid_row(index.saturating_add(1), event);
        let mut cells: Vec<String> = row.cells.iter().map(|cell| log_csv_escape(cell)).collect();
        cells.push(log_csv_escape(&hex_hash_string(&event.prev_hash)));
        cells.push(log_csv_escape(&hex_hash_string(&event.entry_hash)));
        lines.push(cells.join(","));
        last_sequence = event.sequence;
    }
    (lines, last_sequence)
}

/// Builds pending lines and appends them on the calling thread so shutdown
/// callers can guarantee the daily CSV is fully written before exit.
pub fn flush_diagnostic_events_to_disk_sync(
    diagnostics: &DiagnosticStore,
    last_persisted_event_sequence: &AtomicU64,
    log_directory: &Path,
) {
    let baseline = last_persisted_event_sequence.load(Ordering::Acquire);
    let (lines, last_sequence) = build_log_lines(diagnostics, baseline);
    if lines.is_empty() {
        return;
    }
    let (year, month, day) = utc_date_now();
    match append_log_lines_sync_with_diagnostics(
        log_directory,
        &daily_log_filename(year, month, day),
        &lines,
        diagnostics,
    ) {
        Ok(()) => {
            last_persisted_event_sequence.store(last_sequence, Ordering::Release);
        }
        Err(write_error) => {
            eprintln!("true-tick log write failed: {write_error}");
        }
    }
}

pub fn flush_diagnostic_events_to_disk(
    diagnostics: &DiagnosticStore,
    last_persisted_event_sequence: &Arc<AtomicU64>,
    log_directory: &Path,
) {
    let baseline = last_persisted_event_sequence.load(Ordering::Acquire);
    let (lines, last_sequence) = build_log_lines(diagnostics, baseline);
    if lines.is_empty() {
        return;
    }
    let (year, month, day) = utc_date_now();
    append_log_lines_with_commit_and_diagnostics(
        log_directory.to_path_buf(),
        daily_log_filename(year, month, day),
        lines,
        Arc::clone(last_persisted_event_sequence),
        last_sequence,
        diagnostics.clone(),
    );
}

/// Startup retention entry point. Called once after the log directory is
/// resolved so stale files are removed before the first append of the session.
pub fn purge_expired_logs_at_startup(log_directory: &Path, diagnostics: &DiagnosticStore) {
    purge_expired_logs_with_diagnostics(log_directory, diagnostics);
}

#[cfg(test)]
mod tests {
    use super::{
        append_log_lines_sync, append_log_lines_sync_internal, daily_log_filename,
        flush_diagnostic_events_to_disk_sync, free_bytes_available, hex_hash_string, utc_date_now,
        LogWriteError, LOG_HEADER, MIN_FREE_BYTES_BEFORE_WRITE,
    };

    /// Builds a clean temporary directory inside the crate target directory so
    /// test artifacts never escape the workspace and never collide between runs.
    fn temporary_log_directory(label: &str) -> std::path::PathBuf {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!("{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    #[test]
    fn daily_log_filename_formats_utc_date() {
        assert_eq!(daily_log_filename(2026, 9, 16), "true-tick-2026-09-16.csv");
        assert_eq!(daily_log_filename(2031, 1, 5), "true-tick-2031-01-05.csv");
    }

    #[test]
    fn hex_hash_string_formats_bytes() {
        let bytes: [u8; 32] = [
            0x00, 0x01, 0x0a, 0x0f, 0x10, 0x1f, 0xa0, 0xff, 0xde, 0xad, 0xbe, 0xef, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00,
        ];
        let rendered = hex_hash_string(&bytes);
        assert_eq!(rendered.len(), 64);
        assert!(rendered.starts_with("00010a0f101fa0ffdeadbeef"));
        assert!(rendered
            .chars()
            .all(|character| character.is_ascii_hexdigit()));
        let zeros = hex_hash_string(&[0u8; 32]);
        assert_eq!(zeros, "0".repeat(64));
    }

    #[test]
    fn daily_log_header_written_once_on_first_append() {
        let directory = temporary_log_directory("header-once");
        let filename = daily_log_filename(2026, 9, 17);
        let path = directory.join(&filename);

        append_log_lines_sync(&directory, &filename, &[String::from("1,alpha")])
            .expect("first append succeeds");
        let after_first = std::fs::read_to_string(&path).expect("daily log readable");
        let first_lines: Vec<&str> = after_first.lines().collect();
        assert_eq!(first_lines.len(), 2);
        assert_eq!(first_lines[0], LOG_HEADER);
        assert_eq!(first_lines[1], "1,alpha");

        append_log_lines_sync(&directory, &filename, &[String::from("2,beta")])
            .expect("second append succeeds");
        let after_second = std::fs::read_to_string(&path).expect("daily log readable");
        let second_lines: Vec<&str> = after_second.lines().collect();
        assert_eq!(second_lines.len(), 3);
        assert_eq!(second_lines[0], LOG_HEADER);
        assert_eq!(second_lines[1], "1,alpha");
        assert_eq!(second_lines[2], "2,beta");
        assert_eq!(after_second.matches(LOG_HEADER).count(), 1);

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn daily_log_header_written_for_pre_existing_empty_file() {
        let directory = temporary_log_directory("empty-file");
        std::fs::create_dir_all(&directory).expect("create temp log directory");
        let filename = daily_log_filename(2026, 9, 17);
        let path = directory.join(&filename);
        std::fs::File::create(&path).expect("create empty daily log");

        append_log_lines_sync(&directory, &filename, &[String::from("7,gamma")])
            .expect("append succeeds");
        let contents = std::fs::read_to_string(&path).expect("daily log readable");
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], LOG_HEADER);
        assert_eq!(lines[1], "7,gamma");

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn sync_flush_writes_header_and_all_event_rows() {
        use std::sync::atomic::{AtomicU64, Ordering};
        let directory = temporary_log_directory("sync-flush");
        let store = tick_diagnostics::DiagnosticStore::new(8);
        store.record("test.event.alpha", "result=first");
        store.record("test.event.beta", "result=second");
        store.record("test.event.gamma", "result=third");
        let event_count = store.snapshot().len();
        let last_persisted = AtomicU64::new(0);

        flush_diagnostic_events_to_disk_sync(&store, &last_persisted, &directory);

        let (year, month, day) = utc_date_now();
        let path = directory.join(daily_log_filename(year, month, day));
        let contents = std::fs::read_to_string(&path).expect("daily log readable after sync flush");
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), event_count.saturating_add(1));
        assert_eq!(lines[0], LOG_HEADER);
        assert_eq!(lines.len(), 1 + 3);
        assert_eq!(last_persisted.load(Ordering::SeqCst), 3);

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn free_bytes_available_returns_some_on_existing_directory() {
        let directory = temporary_log_directory("free-space");
        std::fs::create_dir_all(&directory).expect("create temp log directory");
        let free = free_bytes_available(&directory);
        assert!(free.is_some());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn write_is_suppressed_when_free_space_below_threshold() {
        let directory = temporary_log_directory("low-free-space");
        std::fs::create_dir_all(&directory).expect("create temp log directory");
        let filename = daily_log_filename(2026, 9, 17);
        let path = directory.join(&filename);
        let store = tick_diagnostics::DiagnosticStore::new(8);

        let low_probe = |_directory: &std::path::Path| -> Option<u64> {
            Some(MIN_FREE_BYTES_BEFORE_WRITE.saturating_sub(1))
        };
        let result = append_log_lines_sync_internal(
            &directory,
            &filename,
            &[String::from("1,alpha")],
            Some(&store),
            Some(&low_probe),
        );
        assert!(matches!(
            result,
            Err(LogWriteError::InsufficientFreeSpace { available }) if available == MIN_FREE_BYTES_BEFORE_WRITE - 1
        ));
        assert!(!path.exists());

        let suppressed = store.snapshot().iter().any(|event| {
            event.name == "storage.write_suppressed"
                && event.details.contains("reason=low_free_space")
                && event
                    .details
                    .contains(&format!("available={}", MIN_FREE_BYTES_BEFORE_WRITE - 1))
        });
        assert!(suppressed);

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn io_failure_records_storage_append_error_event() {
        let directory = temporary_log_directory("io-failure");
        std::fs::create_dir_all(&directory).expect("create temp log directory");
        let blocked = directory.join("blocked");
        std::fs::write(&blocked, b"not a directory").expect("create blocking file");
        let store = tick_diagnostics::DiagnosticStore::new(8);

        let result = append_log_lines_sync_internal(
            &blocked,
            "true-tick-2026-09-17.csv",
            &[String::from("1,alpha")],
            Some(&store),
            None,
        );
        assert!(matches!(result, Err(LogWriteError::Io(_))));

        let matching: Vec<_> = store
            .snapshot()
            .into_iter()
            .filter(|event| event.name == "storage.append.error")
            .collect();
        assert_eq!(matching.len(), 1);
        assert!(matching[0].details.contains("result=io_failure"));
        assert!(matching[0].details.contains("kind="));
        assert!(matches!(
            matching[0].outcome,
            tick_diagnostics::DiagnosticOutcome::Failed
        ));

        let _ = std::fs::remove_dir_all(&directory);
    }
}
