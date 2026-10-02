//! Diagnostic copy and export transfer helpers for True™ Tick.
//!
//! Holds the clipboard copy, CSV and TSV export, range and transfer source
//! classification helpers, plus the search and filter predicates that shape
//! the visible diagnostic rows. Native Win32 declarations live here so the
//! clipboard and save dialog handles stay together with their callers.

use std::ffi::c_void;
use std::mem::size_of;
use std::path::{Path, PathBuf};

use tick_diagnostics::{
    format_csv, format_tsv, latest_row_selection, DiagnosticEvent, DiagnosticGridRow,
    EventCategory, RowSelection,
};

use crate::tray::{wide, App, GetLastError};

const CF_UNICODETEXT: u32 = 13;
const GMEM_MOVEABLE: u32 = 0x0002;
const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
const OFN_OVERWRITEPROMPT: u32 = 0x2;
const OFN_HIDEREADONLY: u32 = 0x4;
const OFN_NOCHANGEDIR: u32 = 0x8;
const OFN_PATHMUSTEXIST: u32 = 0x800;
const OFN_EXPLORER: u32 = 0x00080000;
const MAX_EXPORT_PATH_UTF16: usize = 32_768;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DiagnosticNativeAction {
    Completed,
    Cancelled,
    Failed { raw_error: u32 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NativeFailure {
    pub(crate) stage: &'static str,
    pub(crate) raw_error: u32,
}

pub(crate) fn map_native_action(success: bool, raw_error: u32) -> DiagnosticNativeAction {
    if success {
        DiagnosticNativeAction::Completed
    } else {
        DiagnosticNativeAction::Failed { raw_error }
    }
}

pub(crate) fn map_save_dialog_result(result: i32, extended_error: u32) -> DiagnosticNativeAction {
    if result != 0 {
        DiagnosticNativeAction::Completed
    } else if extended_error == 0 {
        DiagnosticNativeAction::Cancelled
    } else {
        DiagnosticNativeAction::Failed {
            raw_error: extended_error,
        }
    }
}

fn search_matches(search: &str, event_name: &str, details: &str) -> bool {
    let needle = search.trim();
    if needle.is_empty() {
        return true;
    }
    let needle = needle.to_lowercase();
    event_name.to_lowercase().contains(&needle) || details.to_lowercase().contains(&needle)
}

pub(crate) fn diagnostic_filtered_events<'a>(
    app: &App,
    events: &'a [DiagnosticEvent],
) -> Vec<&'a DiagnosticEvent> {
    events
        .iter()
        .filter(|event| {
            app.diagnostic_ui
                .diagnostic_selected_category
                .is_none_or(|category| EventCategory::from_event_name(&event.name) == category)
                && search_matches(
                    &app.diagnostic_ui.diagnostic_search_text,
                    &event.name,
                    &event.details,
                )
        })
        .collect()
}

pub(crate) fn diagnostic_filtered_visible_events<'a>(
    app: &App,
    events: &'a [DiagnosticEvent],
) -> Vec<&'a DiagnosticEvent> {
    let filtered = diagnostic_filtered_events(app, events);
    let selection = diagnostic_visible_selection(app, filtered.len());
    if selection.is_empty() {
        Vec::new()
    } else {
        filtered
            .into_iter()
            .skip(selection.start().saturating_sub(1))
            .take(selection.row_count())
            .collect()
    }
}

pub(crate) fn diagnostic_visible_selection(app: &App, retained_rows: usize) -> RowSelection {
    if app.diagnostic_ui.diagnostic_display_all {
        RowSelection::all(retained_rows)
    } else {
        latest_row_selection(app.diagnostic_ui.diagnostic_display_limit, retained_rows)
    }
}

#[repr(C)]
struct OpenFileNameW {
    struct_size: u32,
    owner: *mut c_void,
    instance: *mut c_void,
    filter: *const u16,
    custom_filter: *mut u16,
    maximum_custom_filter: u32,
    filter_index: u32,
    file: *mut u16,
    maximum_file: u32,
    file_title: *mut u16,
    maximum_file_title: u32,
    initial_directory: *const u16,
    title: *const u16,
    flags: u32,
    file_offset: u16,
    file_extension: u16,
    default_extension: *const u16,
    custom_data: usize,
    hook: *mut c_void,
    template_name: *const u16,
    reserved: *mut c_void,
    reserved_flags: u32,
    flags_ex: u32,
}

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
pub(crate) unsafe fn copy_tsv_to_clipboard(
    owner: *mut c_void,
    text: &str,
) -> Result<(), NativeFailure> {
    if OpenClipboard(owner) == 0 {
        return Err(NativeFailure {
            stage: "OpenClipboard",
            raw_error: GetLastError(),
        });
    }
    if EmptyClipboard() == 0 {
        let raw_error = GetLastError();
        let _ = CloseClipboard();
        return Err(NativeFailure {
            stage: "EmptyClipboard",
            raw_error,
        });
    }
    let mut utf16 = text.encode_utf16().collect::<Vec<_>>();
    utf16.push(0);
    let bytes = utf16.len().saturating_mul(size_of::<u16>());
    let memory = GlobalAlloc(GMEM_MOVEABLE, bytes);
    if memory.is_null() {
        let raw_error = GetLastError();
        let _ = CloseClipboard();
        return Err(NativeFailure {
            stage: "GlobalAlloc",
            raw_error,
        });
    }
    let locked = GlobalLock(memory);
    if locked.is_null() {
        let raw_error = GetLastError();
        let _ = GlobalFree(memory);
        let _ = CloseClipboard();
        return Err(NativeFailure {
            stage: "GlobalLock",
            raw_error,
        });
    }
    std::ptr::copy_nonoverlapping(utf16.as_ptr().cast::<u8>(), locked.cast::<u8>(), bytes);
    let _ = GlobalUnlock(memory);
    if SetClipboardData(CF_UNICODETEXT, memory).is_null() {
        let raw_error = GetLastError();
        let _ = GlobalFree(memory);
        let _ = CloseClipboard();
        return Err(NativeFailure {
            stage: "SetClipboardData",
            raw_error,
        });
    }
    if CloseClipboard() == 0 {
        return Err(NativeFailure {
            stage: "CloseClipboard",
            raw_error: GetLastError(),
        });
    }
    Ok(())
}

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
pub(crate) unsafe fn choose_export_path(
    owner: *mut c_void,
) -> Result<Option<PathBuf>, NativeFailure> {
    let filter =
        wide("CSV files (*.csv)\0*.csv\0TSV files (*.tsv)\0*.tsv\0All files (*.*)\0*.*\0\0");
    let title = wide("Export True™ Tick diagnostic log");
    let default_extension = wide("csv");
    let mut file = wide("true-tick-log.csv");
    file.resize(MAX_EXPORT_PATH_UTF16, 0);
    let mut dialog = OpenFileNameW {
        struct_size: size_of::<OpenFileNameW>() as u32,
        owner,
        instance: std::ptr::null_mut(),
        filter: filter.as_ptr(),
        custom_filter: std::ptr::null_mut(),
        maximum_custom_filter: 0,
        filter_index: 1,
        file: file.as_mut_ptr(),
        maximum_file: file.len() as u32,
        file_title: std::ptr::null_mut(),
        maximum_file_title: 0,
        initial_directory: std::ptr::null(),
        title: title.as_ptr(),
        flags: OFN_EXPLORER
            | OFN_HIDEREADONLY
            | OFN_NOCHANGEDIR
            | OFN_OVERWRITEPROMPT
            | OFN_PATHMUSTEXIST,
        file_offset: 0,
        file_extension: 0,
        default_extension: default_extension.as_ptr(),
        custom_data: 0,
        hook: std::ptr::null_mut(),
        template_name: std::ptr::null(),
        reserved: std::ptr::null_mut(),
        reserved_flags: 0,
        flags_ex: 0,
    };
    let result = GetSaveFileNameW(&mut dialog);
    let extended_error = if result == 0 {
        CommDlgExtendedError()
    } else {
        0
    };
    match map_save_dialog_result(result, extended_error) {
        DiagnosticNativeAction::Completed => {
            let length = file.iter().position(|value| *value == 0).unwrap_or(0);
            Ok(Some(PathBuf::from(String::from_utf16_lossy(
                &file[..length],
            ))))
        }
        DiagnosticNativeAction::Cancelled => Ok(None),
        DiagnosticNativeAction::Failed { raw_error } => Err(NativeFailure {
            stage: "GetSaveFileNameW",
            raw_error,
        }),
    }
}

fn export_io_error(error: &std::io::Error) -> u32 {
    error.raw_os_error().unwrap_or(1).try_into().unwrap_or(1)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DiagnosticExportFormat {
    Csv,
    Tsv,
}

impl DiagnosticExportFormat {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Csv => "CSV",
            Self::Tsv => "TSV",
        }
    }
}

pub(crate) fn export_format_for_path(path: &Path) -> DiagnosticExportFormat {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("tsv") => DiagnosticExportFormat::Tsv,
        _ => DiagnosticExportFormat::Csv,
    }
}

pub(crate) fn format_export_rows(
    rows: &[DiagnosticGridRow],
    format: DiagnosticExportFormat,
) -> String {
    match format {
        DiagnosticExportFormat::Csv => format_csv(rows),
        DiagnosticExportFormat::Tsv => format_tsv(rows),
    }
}

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
pub(crate) unsafe fn write_export_tsv(path: &Path, text: &str) -> Result<(), NativeFailure> {
    let temporary = PathBuf::from(format!("{}.tmp", path.to_string_lossy()));
    if let Err(error) = std::fs::write(&temporary, text.as_bytes()) {
        let _ = std::fs::remove_file(&temporary);
        return Err(NativeFailure {
            stage: "write_temporary",
            raw_error: export_io_error(&error),
        });
    }
    let temporary_wide = wide(&temporary.to_string_lossy());
    let path_wide = wide(&path.to_string_lossy());
    if MoveFileExW(
        temporary_wide.as_ptr(),
        path_wide.as_ptr(),
        MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
    ) == 0
    {
        let raw_error = GetLastError();
        let _ = std::fs::remove_file(&temporary);
        return Err(NativeFailure {
            stage: "MoveFileExW",
            raw_error,
        });
    }
    Ok(())
}

pub(crate) fn diagnostic_range_details(selection: RowSelection, retained_rows: usize) -> String {
    format!(
        "selected_range={}-{} retained_rows={} row_count={} format=TSV",
        selection.start(),
        selection.end(),
        retained_rows,
        selection.row_count()
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DiagnosticTransferSource {
    GridSelection,
    RangeSelection,
}

impl DiagnosticTransferSource {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::GridSelection => "grid-selection",
            Self::RangeSelection => "range-selection",
        }
    }
}

pub(crate) fn diagnostic_transfer_details(
    source: DiagnosticTransferSource,
    selected_row_count: usize,
    retained_rows: usize,
    range: Option<RowSelection>,
) -> String {
    let range_details = range.map_or_else(String::new, |selection| {
        format!(" selected_range={}-{}", selection.start(), selection.end())
    });
    format!(
        "source={} selected_row_count={} retained_rows={} format=TSV{}",
        source.label(),
        selected_row_count,
        retained_rows,
        range_details
    )
}

pub(crate) fn diagnostic_transfer_source(
    grid_selection_count: usize,
    range_selection_count: usize,
) -> Option<DiagnosticTransferSource> {
    if grid_selection_count > 0 {
        Some(DiagnosticTransferSource::GridSelection)
    } else if range_selection_count > 0 {
        Some(DiagnosticTransferSource::RangeSelection)
    } else {
        None
    }
}

#[link(name = "user32")]
extern "system" {
    fn OpenClipboard(owner: *mut c_void) -> i32;
    fn EmptyClipboard() -> i32;
    fn SetClipboardData(format: u32, data: *mut c_void) -> *mut c_void;
    fn CloseClipboard() -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GlobalAlloc(flags: u32, bytes: usize) -> *mut c_void;
    fn GlobalLock(memory: *mut c_void) -> *mut c_void;
    fn GlobalUnlock(memory: *mut c_void) -> i32;
    fn GlobalFree(memory: *mut c_void) -> *mut c_void;
    fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
}

#[link(name = "comdlg32")]
extern "system" {
    fn GetSaveFileNameW(file_name: *mut OpenFileNameW) -> i32;
    fn CommDlgExtendedError() -> u32;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tick_diagnostics::parse_row_selection;

    #[test]
    fn clipboard_and_save_dialog_decision_mapping_is_pure() {
        assert_eq!(
            map_native_action(true, 0),
            DiagnosticNativeAction::Completed
        );
        assert_eq!(
            map_native_action(false, 5),
            DiagnosticNativeAction::Failed { raw_error: 5 }
        );
        assert_eq!(
            map_save_dialog_result(1, 0),
            DiagnosticNativeAction::Completed
        );
        assert_eq!(
            map_save_dialog_result(0, 0),
            DiagnosticNativeAction::Cancelled
        );
        assert_eq!(
            map_save_dialog_result(0, 1223),
            DiagnosticNativeAction::Failed { raw_error: 1223 }
        );
    }

    #[test]
    fn export_format_is_csv_by_default_and_tsv_only_for_tsv_extension() {
        assert_eq!(
            export_format_for_path(Path::new("true-tick-log.csv")),
            DiagnosticExportFormat::Csv
        );
        assert_eq!(
            export_format_for_path(Path::new("true-tick-log.CSV")),
            DiagnosticExportFormat::Csv
        );
        assert_eq!(
            export_format_for_path(Path::new("true-tick-log.tsv")),
            DiagnosticExportFormat::Tsv
        );
        assert_eq!(
            export_format_for_path(Path::new("true-tick-log.TSV")),
            DiagnosticExportFormat::Tsv
        );
        assert_eq!(
            export_format_for_path(Path::new("true-tick-log")),
            DiagnosticExportFormat::Csv
        );
        assert_eq!(
            export_format_for_path(Path::new("true-tick-log.txt")),
            DiagnosticExportFormat::Csv
        );
    }

    #[test]
    fn export_formatter_produces_rfc4180_csv_and_sanitized_tsv() {
        let row = DiagnosticGridRow {
            cells: vec![
                "1".to_owned(),
                "a,b".to_owned(),
                "say \"hi\"".to_owned(),
                "line\nbreak".to_owned(),
            ],
        };
        let csv = format_export_rows(std::slice::from_ref(&row), DiagnosticExportFormat::Csv);
        assert_eq!(
            csv,
            "Row,Sequence,Elapsed,Operation,Parent,Correlation,Phase,Source,Outcome,Event,Details\r\n1,\"a,b\",\"say \"\"hi\"\"\",\"line\nbreak\",,,,,,,\r\n"
        );
        let tsv = format_export_rows(&[row], DiagnosticExportFormat::Tsv);
        assert!(tsv.starts_with(
            "Row\tSequence\tElapsed\tOperation\tParent\tCorrelation\tPhase\tSource\tOutcome\tEvent\tDetails\r\n"
        ));
        assert!(tsv.contains("a,b"));
        assert!(tsv.contains("line break"));
    }

    #[test]
    fn transfer_source_precedence_is_shared_by_ctrl_c_copy_and_export() {
        for action in ["ctrl-c", "copy", "export"] {
            assert_eq!(
                diagnostic_transfer_source(3, 8),
                Some(DiagnosticTransferSource::GridSelection),
                "grid selection must win for {action}"
            );
            assert_eq!(
                diagnostic_transfer_source(0, 8),
                Some(DiagnosticTransferSource::RangeSelection),
                "range fallback must apply for {action}"
            );
        }
    }

    #[test]
    fn no_selection_has_no_transfer_source_and_the_range_parser_is_empty() {
        assert_eq!(diagnostic_transfer_source(0, 0), None);
        assert_eq!(parse_row_selection("", 0), Ok(RowSelection::all(0)));
        assert_eq!(
            diagnostic_transfer_details(DiagnosticTransferSource::GridSelection, 3, 8, None),
            "source=grid-selection selected_row_count=3 retained_rows=8 format=TSV"
        );
        assert_eq!(
            diagnostic_transfer_details(
                DiagnosticTransferSource::RangeSelection,
                2,
                8,
                Some(RowSelection::new(3, 4)),
            ),
            "source=range-selection selected_row_count=2 retained_rows=8 format=TSV selected_range=3-4"
        );
    }

    #[test]
    fn search_matches_treats_empty_and_blank_queries_as_match_all() {
        assert!(search_matches("", "timer.tick", "result=ok"));
        assert!(search_matches("   ", "timer.tick", "result=ok"));
    }

    #[test]
    fn search_matches_event_name_case_insensitively() {
        assert!(search_matches("TICK", "timer.tick", "result=ok"));
        assert!(search_matches("timer.T", "Timer.Tick", ""));
    }

    #[test]
    fn search_matches_details_case_insensitively() {
        assert!(search_matches("RAW_STATUS", "native.error", "raw_status=5"));
        assert!(search_matches("hwnd", "diag", "HWND_VALID=1"));
    }

    #[test]
    fn search_matches_rejects_text_absent_from_name_and_details() {
        assert!(!search_matches("clipboard", "timer.tick", "result=ok"));
    }
}
