use std::path::{Path, PathBuf};

use tick_diagnostics::{diagnostic_grid_row, truncate_utf8, DiagnosticStore, MAX_FIELD_LENGTH};

pub fn resolve_log_directory(executable: &Path) -> PathBuf {
    match crate::portable::portable_root_from_slot_executable(executable) {
        Ok(root) => root.join("Data").join("logs"),
        Err(_) => executable.parent().unwrap_or(Path::new(".")).join("logs"),
    }
}

pub fn daily_log_filename(year: u16, month: u16, day: u16) -> String {
    format!("true-tick-{year:04}-{month:02}-{day:02}.csv")
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

pub fn utc_date_now() -> (u16, u16, u16) {
    #[repr(C)]
    struct SystemTimeParts {
        year: u16,
        month: u16,
        day_of_week: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        milliseconds: u16,
    }
    unsafe {
        let mut parts = SystemTimeParts {
            year: 0,
            month: 0,
            day_of_week: 0,
            day: 0,
            hour: 0,
            minute: 0,
            second: 0,
            milliseconds: 0,
        };
        unsafe extern "system" {
            fn GetSystemTime(time: *mut SystemTimeParts);
        }
        GetSystemTime(&mut parts);
        (parts.year, parts.month, parts.day)
    }
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
    Ok(())
}

/// Appends `lines` to `directory/filename` on the calling thread, creating the
/// directory and the file when needed. Returns the first IO error encountered.
fn append_log_lines_sync(
    directory: &Path,
    filename: &str,
    lines: &[String],
) -> std::io::Result<()> {
    std::fs::create_dir_all(directory)?;
    let path = directory.join(filename);
    let _guard = LOG_APPEND_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    write_log_lines(&mut file, lines)
}

pub fn append_log_lines(directory: PathBuf, filename: String, lines: Vec<String>) {
    std::thread::spawn(move || {
        if let Err(write_error) = append_log_lines_sync(&directory, &filename, &lines) {
            eprintln!("true-tick log write failed: {write_error}");
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
    last_persisted_event_sequence: &mut u64,
    log_directory: &Path,
) {
    let (lines, last_sequence) = build_log_lines(diagnostics, *last_persisted_event_sequence);
    if lines.is_empty() {
        return;
    }
    let (year, month, day) = utc_date_now();
    match append_log_lines_sync(log_directory, &daily_log_filename(year, month, day), &lines) {
        Ok(()) => {
            *last_persisted_event_sequence = last_sequence;
        }
        Err(write_error) => {
            eprintln!("true-tick log write failed: {write_error}");
        }
    }
}

pub fn flush_diagnostic_events_to_disk(
    diagnostics: &DiagnosticStore,
    last_persisted_event_sequence: &mut u64,
    log_directory: &Path,
) {
    let (lines, last_sequence) = build_log_lines(diagnostics, *last_persisted_event_sequence);
    if lines.is_empty() {
        return;
    }
    *last_persisted_event_sequence = last_sequence;
    let (year, month, day) = utc_date_now();
    append_log_lines(
        log_directory.to_path_buf(),
        daily_log_filename(year, month, day),
        lines,
    );
}

#[cfg(test)]
mod tests {
    use super::{
        append_log_lines_sync, daily_log_filename, flush_diagnostic_events_to_disk_sync,
        hex_hash_string, utc_date_now, LOG_HEADER,
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
        let directory = temporary_log_directory("sync-flush");
        let store = tick_diagnostics::DiagnosticStore::new(8);
        store.record("test.event.alpha", "result=first");
        store.record("test.event.beta", "result=second");
        store.record("test.event.gamma", "result=third");
        let event_count = store.snapshot().len();
        let mut last_persisted = 0u64;

        flush_diagnostic_events_to_disk_sync(&store, &mut last_persisted, &directory);

        let (year, month, day) = utc_date_now();
        let path = directory.join(daily_log_filename(year, month, day));
        let contents = std::fs::read_to_string(&path).expect("daily log readable after sync flush");
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), event_count.saturating_add(1));
        assert_eq!(lines[0], LOG_HEADER);
        assert_eq!(lines.len(), 1 + 3);
        assert_eq!(last_persisted, 3);

        let _ = std::fs::remove_dir_all(&directory);
    }
}
