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

pub fn append_log_lines(directory: PathBuf, filename: String, lines: Vec<String>) {
    std::thread::spawn(move || {
        if let Err(create_error) = std::fs::create_dir_all(&directory) {
            eprintln!("true-tick log directory create failed: {create_error}");
            return;
        }
        let path = directory.join(filename);
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            Ok(mut file) => {
                use std::io::Write;
                for line in lines {
                    if let Err(write_error) = writeln!(file, "{line}") {
                        eprintln!("true-tick log write failed: {write_error}");
                        return;
                    }
                }
            }
            Err(open_error) => {
                eprintln!("true-tick log open failed: {open_error}");
            }
        }
    });
}

pub fn flush_diagnostic_events_to_disk(
    diagnostics: &DiagnosticStore,
    last_persisted_event_sequence: &mut u64,
    log_directory: &Path,
) {
    let events = diagnostics.snapshot();
    let mut lines = Vec::new();
    let mut last_sequence = *last_persisted_event_sequence;
    for (index, event) in events.iter().enumerate() {
        if event.sequence <= *last_persisted_event_sequence {
            continue;
        }
        let row = diagnostic_grid_row(index.saturating_add(1), event);
        let mut cells: Vec<String> = row.cells.iter().map(|cell| log_csv_escape(cell)).collect();
        cells.push(log_csv_escape(&hex_hash_string(&event.prev_hash)));
        cells.push(log_csv_escape(&hex_hash_string(&event.entry_hash)));
        lines.push(cells.join(","));
        last_sequence = event.sequence;
    }
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
    use super::{daily_log_filename, hex_hash_string};

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
}
