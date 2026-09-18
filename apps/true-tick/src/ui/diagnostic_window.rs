//! Diagnostic window implementation for True™ Tick.
//!
//! Provides the diagnostic and status window, covering creation, layout,
//! message handling, marquee backed selection, copy and export, and DPI
//! aware presentation.

use std::ffi::c_void;
use std::mem::size_of;
use std::path::{Path, PathBuf};

use tick_diagnostics::{
    diagnostic_grid_rows, diagnostic_grid_rows_for_sequences, format_csv, format_tsv,
    latest_row_selection, parse_display_limit, parse_row_selection, retention_summary,
    row_selection_for_sequences, selected_event_sequences, selected_event_sequences_for_sequences,
    snapshot_is_truncated, verify_event_chain, DiagnosticEvent, DiagnosticGridRow,
    DiagnosticOutcome, DiagnosticPhase, DiagnosticRecord, DiagnosticSource, EventCategory,
    RowSelection, REPORT_COLUMNS,
};
use tick_observation_windows::ObservationSource;
use tick_policy::PowerState;

use crate::tray::list_view_native::*;
use crate::tray::{
    app_create_params, destroy_created_diagnostic_controls, diagnostic_create_failure_result,
    diagnostic_outcome, diagnostic_phase, native_outcome, scale_logical,
    set_diagnostic_control_font, wide, App, BeginPaint, CreateStruct, CreateWindowExW,
    DefWindowProcW, DeleteObject, DestroyWindow, EndPaint, FillRect, GetClientRect,
    GetDpiForSystem, GetDpiForWindow, GetKeyState, GetLastError, GetModuleHandleW, GetStockObject,
    GetSysColor, GetSysColorBrush, GetSystemMetrics, GetWindowLongPtrW, GetWindowTextLengthW,
    GetWindowTextW, InvalidateRect, IsIconic, IsWindow, PaintStruct, Point, Rect, SendMessageW,
    SetBkColor, SetForegroundWindow, SetTextColor, SetWindowLongPtrW, SetWindowPos, SetWindowTextW,
    ShowWindow, UpdateWindow, BN_CLICKED, BS_PUSHBUTTON, COLOR_WINDOW, COLOR_WINDOWTEXT,
    DEFAULT_GUI_FONT, EM_LIMITTEXT, EN_CHANGE, ES_AUTOHSCROLL, GWLP_USERDATA, GWLP_WNDPROC,
    SS_LEFT, SWP_NOACTIVATE, SWP_NOZORDER, SW_RESTORE, SW_SHOWNORMAL, WM_CLOSE, WM_COMMAND,
    WM_CREATE, WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC, WM_DPICHANGED, WM_ERASEBKGND, WM_NCDESTROY,
    WM_PAINT, WM_SETFONT, WM_SETTINGCHANGE, WM_SIZE, WM_SYSCOLORCHANGE, WM_THEMECHANGED, WS_BORDER,
    WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_CLIENTEDGE, WS_TABSTOP, WS_VISIBLE,
    WS_VSCROLL,
};
use crate::tray_surface::{status_menu_items, GITHUB_URL};

pub const DIAGNOSTIC_WINDOW_CLASS: &str = "TrueTickDiagnosticClass";

const WM_APP: u32 = 0x8000;
const WM_DIAGNOSTIC_REFRESH: u32 = WM_APP + 2;

const ID_DIAGNOSTIC_RANGE: usize = 1201;
const ID_DIAGNOSTIC_COPY: usize = 1202;
const ID_DIAGNOSTIC_EXPORT: usize = 1203;
const ID_DIAGNOSTIC_DISPLAY_LIMIT: usize = 1204;
const ID_DIAGNOSTIC_SHOW_ALL: usize = 1205;
const ID_DIAGNOSTIC_CATEGORY_FILTER: usize = 1206;
const ID_DIAGNOSTIC_EXPORT_ALL: usize = 1207;
const ID_DIAGNOSTIC_SEARCH: usize = 1208;
const CBN_SELCHANGE: usize = 1;
const CBS_DROPDOWNLIST: u32 = 0x0003;
const CB_ADDSTRING: u32 = 0x0143;
const CB_SETCURSEL: u32 = 0x014E;
const CB_GETCURSEL: u32 = 0x0147;
const WM_SETREDRAW: u32 = 0x000B;
const WM_SETFOCUS: u32 = 0x0007;
const WM_GETMINMAXINFO: u32 = 0x0024;
const GWL_STYLE: i32 = -16;
const GWL_EXSTYLE: i32 = -20;
const WS_OVERLAPPEDWINDOW: u32 = 0x00cf0000;
const WS_EX_TOOLWINDOW: u32 = 0x00000080;
const WS_EX_APPWINDOW: u32 = 0x00040000;
const DIAGNOSTIC_WINDOW_TITLE: &str = "True™ Tick Status and Diagnostics";
const DIAGNOSTIC_LOADING_SUMMARY: &str = "Loading True™ Tick diagnostics...";
const DIAGNOSTIC_WINDOW_PARENT: *mut c_void = std::ptr::null_mut();
const DIAGNOSTIC_SUMMARY_HEIGHT: i32 = 140;
const DIAGNOSTIC_HUD_STATE_HEIGHT: i32 = 32;
const DIAGNOSTIC_SEPARATOR_HEIGHT: i32 = 2;
const DIAGNOSTIC_TOOLBAR_HEIGHT: i32 = 36;
const DIAGNOSTIC_GRID_MIN_HEIGHT: i32 = 96;
const DIAGNOSTIC_TOOLBAR_MARGIN: i32 = 8;
const DIAGNOSTIC_TOOLBAR_GAP: i32 = 8;
const DIAGNOSTIC_TOOLBAR_MIN_MESSAGE_WIDTH: i32 = 140;
// Compact toolbar widths at 96 DPI in left to right order. Both WM_CREATE sizes and
// diagnostic_toolbar_layout consume this same list so creation and layout never drift.
const DIAGNOSTIC_TOOLBAR_CONTROL_WIDTHS: [i32; 12] =
    [64, 52, 64, 100, 54, 104, 128, 66, 124, 56, 56, 70];
const DIAGNOSTIC_COLUMN_WIDTHS: [i32; 11] = [54, 70, 78, 78, 70, 86, 82, 90, 86, 160, 240];
const DIAGNOSTIC_COLUMN_MIN_WIDTHS: [i32; 11] = [36, 52, 64, 64, 52, 64, 60, 68, 64, 84, 240];
const DIAGNOSTIC_COLUMN_MAX_WIDTHS: [i32; 11] =
    [84, 120, 140, 144, 120, 144, 112, 124, 124, 260, 640];
// Compact logical client default. The outer rectangle is DPI adjusted before creation.
const DIAGNOSTIC_DEFAULT_WIDTH: i32 = 1_280;
const DIAGNOSTIC_DEFAULT_HEIGHT: i32 = 520;
const DIAGNOSTIC_RANGE_INPUT_LIMIT: usize = 64;
const WS_HSCROLL: u32 = 0x00100000;
const SWP_NOSENDCHANGING: u32 = 0x0400;
const SB_HORZ: i32 = 0;
const SM_CXWORKAREA: i32 = 60;
const SM_CYWORKAREA: i32 = 61;
const SS_NOPREFIX: u32 = 0x0000_0080;
const SS_ETCHEDHORZ: u32 = 0x0000_0010;
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
enum DiagnosticNativeAction {
    Completed,
    Cancelled,
    Failed { raw_error: u32 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NativeFailure {
    stage: &'static str,
    raw_error: u32,
}

fn map_native_action(success: bool, raw_error: u32) -> DiagnosticNativeAction {
    if success {
        DiagnosticNativeAction::Completed
    } else {
        DiagnosticNativeAction::Failed { raw_error }
    }
}

fn map_save_dialog_result(result: i32, extended_error: u32) -> DiagnosticNativeAction {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DisplayLimitInputDecision {
    Apply(usize),
    PreserveEmpty,
    Invalid,
}

// Classifies raw Show rows text before any state or operation side effects.
// Blank input keeps the previous limit so a transient empty edit during
// creation or mid edit typing never marks an unrelated operation Failed.
fn display_limit_input_decision(input: &str) -> DisplayLimitInputDecision {
    if input.trim().is_empty() {
        DisplayLimitInputDecision::PreserveEmpty
    } else {
        match parse_display_limit(input) {
            Ok(limit) => DisplayLimitInputDecision::Apply(limit),
            Err(_) => DisplayLimitInputDecision::Invalid,
        }
    }
}
const fn diagnostic_window_style() -> u32 {
    WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN | WS_CLIPSIBLINGS
}

const fn diagnostic_window_extended_style() -> u32 {
    WS_EX_APPWINDOW
}
#[repr(C)]
#[derive(Clone, Copy)]
struct LogFontW {
    height: i32,
    width: i32,
    escapement: i32,
    orientation: i32,
    weight: i32,
    italic: u8,
    underline: u8,
    strike_out: u8,
    charset: u8,
    out_precision: u8,
    clip_precision: u8,
    quality: u8,
    pitch_and_family: u8,
    face_name: [u16; 32],
}
fn clear_diagnostic_state(app: &mut App) {
    unsafe {
        if let Some(font) = app.diagnostic_state_font.take() {
            let _ = DeleteObject(font);
        }
    }
    app.diagnostic_window = None;
    app.diagnostic_hud_state = None;
    app.diagnostic_summary = None;
    app.diagnostic_hud_separator = None;
    app.diagnostic_toolbar_separator = None;
    app.diagnostic_toolbar_div_filter = None;
    app.diagnostic_toolbar_div_search = None;
    app.diagnostic_toolbar_div_export = None;
    app.diagnostic_display_label = None;
    app.diagnostic_display_input = None;
    app.diagnostic_show_all_button = None;
    app.diagnostic_category_filter = None;
    app.diagnostic_search_label = None;
    app.diagnostic_search_input = None;
    app.diagnostic_search_text.clear();
    app.diagnostic_toolbar_label = None;
    app.diagnostic_selection_summary = None;
    app.diagnostic_range_input = None;
    app.diagnostic_copy_button = None;
    app.diagnostic_export_button = None;
    app.diagnostic_export_all_button = None;
    app.diagnostic_selected_category = None;
    app.diagnostic_message = None;
    app.diagnostic_list = None;
    app.diagnostic_list_prev_proc = None;
    app.diagnostic_marquee_active = false;
    app.diagnostic_marquee_pending = false;
    app.diagnostic_marquee_anchor = Point::default();
    app.diagnostic_marquee_current = Point::default();
    app.diagnostic_marquee_initial_selected.clear();
    app.diagnostic_selection = None;
    app.diagnostic_selection_sequences = None;
    app.diagnostic_grid_selection_sequences.clear();
    app.diagnostic_grid_selection_reset = false;
    app.diagnostic_selection_reset = false;
    app.diagnostic_message_text.clear();
    app.diagnostic_controls_initializing = false;
    app.diagnostic_refresh_pending = false;
    app.diagnostic_refreshing = false;
    app.diagnostic_refresh_direct_recorded = false;
    app.diagnostic_refresh_follow_up_scheduled = false;
    app.diagnostic_layout_stable = false;
    app.diagnostic_snapshot_key = None;
    app.diagnostic_snapshot_generation = 0;
    app.diagnostic_auto_fit_generation = None;
}
unsafe fn diagnostic_children_ready(app: &App) -> bool {
    [
        app.diagnostic_hud_state,
        app.diagnostic_summary,
        app.diagnostic_hud_separator,
        app.diagnostic_toolbar_separator,
        app.diagnostic_display_label,
        app.diagnostic_display_input,
        app.diagnostic_show_all_button,
        app.diagnostic_category_filter,
        app.diagnostic_search_label,
        app.diagnostic_search_input,
        app.diagnostic_toolbar_label,
        app.diagnostic_selection_summary,
        app.diagnostic_range_input,
        app.diagnostic_copy_button,
        app.diagnostic_export_button,
        app.diagnostic_export_all_button,
        app.diagnostic_message,
        app.diagnostic_list,
    ]
    .into_iter()
    .all(|control| control.is_some_and(|value| IsWindow(value) != 0))
}
pub(crate) unsafe fn destroy_diagnostic_window(app: &mut App) {
    if let Some(window) = app.diagnostic_window.take() {
        if IsWindow(window) != 0 {
            let result = DestroyWindow(window);
            app.record(
                "native.DestroyWindow.diagnostic",
                format!(
                    "result={} raw_status={}",
                    result != 0,
                    if result == 0 { GetLastError() } else { 0 }
                ),
            );
        }
    }
    clear_diagnostic_state(app);
}
pub unsafe fn open_diagnostic_window(app: &mut App) -> bool {
    if let Some(window) = app.diagnostic_window {
        if IsWindow(window) == 0 {
            record_diagnostic_event_with_context(
                app,
                "diagnostic.window.invalid",
                "result=cleared reason=parent_not_a_window",
                DiagnosticPhase::Render,
                DiagnosticOutcome::Failed,
            );
            clear_diagnostic_state(app);
        } else if !diagnostic_children_ready(app) {
            record_diagnostic_event_with_context(
                app,
                "diagnostic.window.invalid",
                "result=destroyed reason=required_child_missing",
                DiagnosticPhase::Render,
                DiagnosticOutcome::Failed,
            );
            let _ = DestroyWindow(window);
            clear_diagnostic_state(app);
        } else {
            if IsIconic(window) != 0 {
                ShowWindow(window, SW_RESTORE);
            } else {
                ShowWindow(window, SW_SHOWNORMAL);
            }
            UpdateWindow(window);
            SetForegroundWindow(window);
            request_diagnostic_refresh(app);
            return true;
        }
    }
    let class_name = wide(DIAGNOSTIC_WINDOW_CLASS);
    let title = wide(DIAGNOSTIC_WINDOW_TITLE);
    let dpi = GetDpiForSystem().max(96);
    let outer = diagnostic_default_outer_size(dpi);
    let window = CreateWindowExW(
        diagnostic_window_extended_style(),
        class_name.as_ptr(),
        title.as_ptr(),
        diagnostic_window_style(),
        120,
        120,
        outer.x,
        outer.y,
        DIAGNOSTIC_WINDOW_PARENT,
        std::ptr::null_mut(),
        GetModuleHandleW(std::ptr::null()),
        app as *mut App as *mut c_void,
    );
    let diagnostic_hwnd_valid = !window.is_null();
    let diagnostic_is_window = if diagnostic_hwnd_valid {
        IsWindow(window)
    } else {
        0
    };
    let diagnostic_create_error = if diagnostic_hwnd_valid {
        0
    } else {
        GetLastError()
    };
    let diagnostic_outcome = if !diagnostic_hwnd_valid {
        DiagnosticOutcome::Failed
    } else if diagnostic_is_window == 0 {
        DiagnosticOutcome::Unverified
    } else {
        DiagnosticOutcome::Completed
    };
    let diagnostic_context = app.operation.map_or_else(
        || {
            app.diagnostics
                .begin_operation(DiagnosticSource::TrayCommand)
        },
        |root| {
            app.diagnostics
                .child_operation(root, DiagnosticSource::TrayCommand)
        },
    );
    app.diagnostics.record_verification(
        diagnostic_context,
        DiagnosticSource::TrayCommand,
        diagnostic_outcome,
        "window.diagnostic.verify",
        format!(
            "hwnd_valid={diagnostic_hwnd_valid} is_window={diagnostic_is_window} raw_status={diagnostic_create_error}"
        ),
    );
    if window.is_null() {
        app.record(
            "diagnostic.window.result",
            format!("result=create_failed raw_status={}", GetLastError()),
        );
        false
    } else {
        if SetWindowTextW(window, title.as_ptr()) == 0 {
            app.record(
                "native.SetWindowTextW.diagnostic.error",
                format!("raw_status={}", GetLastError()),
            );
            DestroyWindow(window);
            return false;
        }
        app.diagnostic_window = Some(window);
        if !diagnostic_children_ready(app) {
            record_diagnostic_event_with_context(
                app,
                "diagnostic.window.invalid",
                "result=destroyed reason=create_returned_without_required_children",
                DiagnosticPhase::Render,
                DiagnosticOutcome::Failed,
            );
            let _ = DestroyWindow(window);
            clear_diagnostic_state(app);
            return false;
        }
        if !layout_diagnostic_controls(window, app) {
            record_diagnostic_event_with_context(
                app,
                "diagnostic.window.invalid",
                "result=destroyed reason=initial_layout_failed",
                DiagnosticPhase::Render,
                DiagnosticOutcome::Failed,
            );
            let _ = DestroyWindow(window);
            clear_diagnostic_state(app);
            return false;
        }
        // Populate the hidden window so its first visible frame is already real data.
        refresh_diagnostic_window(window, app, false);
        ShowWindow(window, SW_SHOWNORMAL);
        let diagnostic_visible = IsWindowVisible(window) != 0;
        let diagnostic_visible_outcome = if diagnostic_visible {
            DiagnosticOutcome::Completed
        } else {
            DiagnosticOutcome::Unverified
        };
        let diagnostic_visible_context = app.operation.map_or_else(
            || {
                app.diagnostics
                    .begin_operation(DiagnosticSource::TrayCommand)
            },
            |root| {
                app.diagnostics
                    .child_operation(root, DiagnosticSource::TrayCommand)
            },
        );
        app.diagnostics.record_verification(
            diagnostic_visible_context,
            DiagnosticSource::TrayCommand,
            diagnostic_visible_outcome,
            "window.diagnostic.visible.verify",
            format!("visible={diagnostic_visible} raw_status=0"),
        );
        UpdateWindow(window);
        SetForegroundWindow(window);
        let style = GetWindowLongPtrW(window, GWL_STYLE) as u32;
        let extended_style = GetWindowLongPtrW(window, GWL_EXSTYLE) as u32;
        app.record(
            "diagnostic.window.styles",
            format!(
                "overlapped={} appwindow={} toolwindow={} title={DIAGNOSTIC_WINDOW_TITLE}",
                style & WS_OVERLAPPEDWINDOW == WS_OVERLAPPEDWINDOW,
                extended_style & WS_EX_APPWINDOW != 0,
                extended_style & WS_EX_TOOLWINDOW != 0,
            ),
        );
        app.record("diagnostic.window.result", "result=opened");
        request_diagnostic_refresh(app);
        true
    }
}

fn power_state_label(power: PowerState) -> &'static str {
    match power {
        PowerState::Ac => "AC",
        PowerState::Battery => "Battery",
        PowerState::BatterySaver => "Battery Saver",
        PowerState::Unknown => "Unknown",
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

fn diagnostic_filtered_events<'a>(
    app: &App,
    events: &'a [DiagnosticEvent],
) -> Vec<&'a DiagnosticEvent> {
    events
        .iter()
        .filter(|event| {
            app.diagnostic_selected_category
                .is_none_or(|category| EventCategory::from_event_name(&event.name) == category)
                && search_matches(&app.diagnostic_search_text, &event.name, &event.details)
        })
        .collect()
}

fn diagnostic_filtered_visible_events<'a>(
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

fn diagnostic_visible_selection(app: &App, retained_rows: usize) -> RowSelection {
    if app.diagnostic_display_all {
        RowSelection::all(retained_rows)
    } else {
        latest_row_selection(app.diagnostic_display_limit, retained_rows)
    }
}

struct DiagnosticHudFields<'a> {
    effective: &'a str,
    ownership: &'a str,
    power: &'a str,
    startup: &'a str,
    running_duration: &'a str,
    next_action: &'a str,
    history: &'a str,
    visible: usize,
    retained: usize,
    chain_status: &'a str,
    slot: &'a str,
    root: &'a str,
}

fn diagnostic_hud_field_text(fields: DiagnosticHudFields<'_>) -> String {
    let DiagnosticHudFields {
        effective,
        ownership,
        power,
        startup,
        running_duration,
        next_action,
        history,
        visible,
        retained,
        chain_status,
        slot,
        root,
    } = fields;
    let version = env!("CARGO_PKG_VERSION");
    format!(
        "Effective: {effective}  |  Ownership: {ownership}  |  Power: {power}\r\nStartup: {startup}  |  Running: {running_duration}  |  Next: {next_action}\r\nHistory: {history}  |  Showing {visible} of {retained} retained rows  |  Chain: {chain_status}\r\nSlot: {slot}  |  Root: {root}\r\nTrue\u{2122} Tick v{version} by kaiiuen  |  {GITHUB_URL}\r\n"
    )
}

fn diagnostic_chain_status_text(events: &[DiagnosticEvent]) -> String {
    match verify_event_chain(events) {
        Ok(()) => "Verified (SHA-256)".to_owned(),
        Err((index, _reason)) => format!("TAMPER/ERROR at row {index}"),
    }
}

fn diagnostic_summary_text(app: &App, retained: usize) -> String {
    let status = status_menu_items(
        app.lifecycle_status(),
        app.timing_values(),
        app.controller.ownership(),
        app.pause.current(),
        app.running_duration(),
        std::time::Instant::now(),
    );
    let snapshot = app.diagnostics.snapshot();
    let chain_status = diagnostic_chain_status_text(&snapshot);
    let visible = diagnostic_filtered_visible_events(app, &snapshot).len();
    let history = retention_summary(
        retained,
        app.diagnostics.maximum_events(),
        snapshot_is_truncated(&snapshot),
    );
    diagnostic_hud_field_text(DiagnosticHudFields {
        effective: status[1]
            .label
            .strip_prefix("Timing: ")
            .unwrap_or(&status[1].label),
        ownership: status[4]
            .label
            .strip_prefix("Ownership: ")
            .unwrap_or(&status[4].label),
        power: power_state_label(app.observation.power().state),
        startup: if app.config.startup_enabled {
            "On"
        } else {
            "Off"
        },
        running_duration: status[2]
            .label
            .strip_prefix("Running for: ")
            .unwrap_or(&status[2].label),
        next_action: status[3]
            .label
            .strip_prefix("Next action: ")
            .unwrap_or(&status[3].label),
        history: &history,
        visible,
        retained,
        chain_status: &chain_status,
        slot: &app.operating_slot_label,
        root: &app.portable_root_label,
    })
}

pub(crate) fn diagnostic_open_operation_outcome(opened: bool) -> DiagnosticOutcome {
    if opened {
        DiagnosticOutcome::Completed
    } else {
        DiagnosticOutcome::Failed
    }
}

fn diagnostic_state_text(app: &App) -> String {
    let state = status_menu_items(
        app.lifecycle_status(),
        app.timing_values(),
        app.controller.ownership(),
        app.pause.current(),
        app.running_duration(),
        std::time::Instant::now(),
    );
    state[0]
        .label
        .strip_prefix("State: ")
        .unwrap_or(&state[0].label)
        .to_owned()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DiagnosticLayoutRect {
    left: i32,
    top: i32,
    width: i32,
    height: i32,
}

#[cfg(test)]
impl DiagnosticLayoutRect {
    fn right(self) -> i32 {
        self.left.saturating_add(self.width)
    }

    fn bottom(self) -> i32 {
        self.top.saturating_add(self.height)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DiagnosticToolbarLayout {
    display_label: DiagnosticLayoutRect,
    display_input: DiagnosticLayoutRect,
    show_all: DiagnosticLayoutRect,
    category_filter: DiagnosticLayoutRect,
    search_label: DiagnosticLayoutRect,
    search_input: DiagnosticLayoutRect,
    transfer_label: DiagnosticLayoutRect,
    range_input: DiagnosticLayoutRect,
    selection_summary: DiagnosticLayoutRect,
    copy: DiagnosticLayoutRect,
    export: DiagnosticLayoutRect,
    export_all: DiagnosticLayoutRect,
    message: DiagnosticLayoutRect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DiagnosticLayout {
    hud_state: DiagnosticLayoutRect,
    summary: DiagnosticLayoutRect,
    hud_separator: DiagnosticLayoutRect,
    toolbar: DiagnosticToolbarLayout,
    toolbar_separator: DiagnosticLayoutRect,
    list: DiagnosticLayoutRect,
}

fn diagnostic_toolbar_min_width(dpi: u32) -> i32 {
    let fixed_width = DIAGNOSTIC_TOOLBAR_CONTROL_WIDTHS
        .into_iter()
        .fold(0i32, i32::saturating_add);
    let gaps = DIAGNOSTIC_TOOLBAR_CONTROL_WIDTHS.len() as i32 * DIAGNOSTIC_TOOLBAR_GAP;
    scale_logical(
        DIAGNOSTIC_TOOLBAR_MARGIN * 2 + fixed_width + gaps + DIAGNOSTIC_TOOLBAR_MIN_MESSAGE_WIDTH,
        dpi.max(96),
    )
}

fn diagnostic_min_client_height(dpi: u32) -> i32 {
    scale_logical(
        DIAGNOSTIC_SUMMARY_HEIGHT
            + DIAGNOSTIC_SEPARATOR_HEIGHT
            + DIAGNOSTIC_TOOLBAR_HEIGHT
            + DIAGNOSTIC_SEPARATOR_HEIGHT
            + DIAGNOSTIC_GRID_MIN_HEIGHT,
        dpi.max(96),
    )
}

fn outer_size_from_client(_client_width: i32, _client_height: i32, frame: Rect) -> Point {
    // The adjusted frame already contains the full outer rectangle including the client.
    Point {
        x: (frame.right - frame.left).max(0),
        y: (frame.bottom - frame.top).max(0),
    }
}

unsafe fn diagnostic_outer_size(client_width: i32, client_height: i32, dpi: u32) -> Point {
    let mut frame = Rect {
        left: 0,
        top: 0,
        right: client_width,
        bottom: client_height,
    };
    let adjusted = AdjustWindowRectExForDpi(
        &mut frame,
        diagnostic_window_style(),
        0,
        diagnostic_window_extended_style(),
        dpi,
    ) != 0;
    let adjusted = if adjusted {
        true
    } else {
        frame = Rect {
            left: 0,
            top: 0,
            right: client_width,
            bottom: client_height,
        };
        AdjustWindowRectEx(
            &mut frame,
            diagnostic_window_style(),
            0,
            diagnostic_window_extended_style(),
        ) != 0
    };
    if adjusted {
        outer_size_from_client(client_width, client_height, frame)
    } else {
        Point {
            x: client_width,
            y: client_height,
        }
    }
}

unsafe fn diagnostic_min_outer_size(
    window: *mut c_void,
    client_width: i32,
    client_height: i32,
    dpi: u32,
) -> Point {
    let _ = window;
    diagnostic_outer_size(client_width, client_height, dpi)
}

fn clamp_outer_to_work_area(outer: Point, work_width: i32, work_height: i32) -> Point {
    let safe_width = work_width.max(320);
    let safe_height = work_height.max(240);
    Point {
        x: outer.x.min(safe_width).max(320),
        y: outer.y.min(safe_height).max(240),
    }
}

unsafe fn diagnostic_default_outer_size(dpi: u32) -> Point {
    let dpi = dpi.max(96);
    let scaled_client_width = scale_logical(DIAGNOSTIC_DEFAULT_WIDTH, dpi);
    let scaled_client_height = scale_logical(DIAGNOSTIC_DEFAULT_HEIGHT, dpi);
    let outer = diagnostic_outer_size(scaled_client_width, scaled_client_height, dpi);
    let work_width = GetSystemMetrics(SM_CXWORKAREA).max(0);
    let work_height = GetSystemMetrics(SM_CYWORKAREA).max(0);
    clamp_outer_to_work_area(outer, work_width, work_height)
}

fn diagnostic_toolbar_layout(
    width: i32,
    top: i32,
    height: i32,
    dpi: u32,
) -> DiagnosticToolbarLayout {
    let dpi = dpi.max(96);
    let row_height = scale_logical(24, dpi).min(height.max(0));
    let row_top = top.saturating_add((height.saturating_sub(row_height)) / 2);
    let gap = scale_logical(DIAGNOSTIC_TOOLBAR_GAP, dpi);
    let margin = scale_logical(DIAGNOSTIC_TOOLBAR_MARGIN, dpi);
    let widths = DIAGNOSTIC_TOOLBAR_CONTROL_WIDTHS.map(|value| scale_logical(value, dpi));
    let mut cursor = margin;
    let mut next = |control_width: i32| {
        let rect = DiagnosticLayoutRect {
            left: cursor,
            top: row_top,
            width: control_width,
            height: row_height,
        };
        cursor = cursor.saturating_add(control_width).saturating_add(gap);
        rect
    };
    let display_label = next(widths[0]);
    let display_input = next(widths[1]);
    let show_all = next(widths[2]);
    let category_filter = next(widths[3]);
    let search_label = next(widths[4]);
    let search_input = next(widths[5]);
    let transfer_label = next(widths[6]);
    let range_input = next(widths[7]);
    let selection_summary = next(widths[8]);
    let copy = next(widths[9]);
    let export = next(widths[10]);
    let export_all = next(widths[11]);
    let message_left = cursor;
    let message_width = width
        .saturating_sub(message_left)
        .saturating_sub(margin)
        .max(scale_logical(DIAGNOSTIC_TOOLBAR_MIN_MESSAGE_WIDTH, dpi));
    let message = DiagnosticLayoutRect {
        left: message_left,
        top: row_top,
        width: message_width,
        height: row_height,
    };
    DiagnosticToolbarLayout {
        display_label,
        display_input,
        show_all,
        category_filter,
        search_label,
        search_input,
        transfer_label,
        range_input,
        selection_summary,
        copy,
        export,
        export_all,
        message,
    }
}

fn diagnostic_layout(width: i32, height: i32, dpi: u32) -> DiagnosticLayout {
    let dpi = dpi.max(96);
    let hud_height = scale_logical(DIAGNOSTIC_SUMMARY_HEIGHT, dpi);
    let state_height = scale_logical(DIAGNOSTIC_HUD_STATE_HEIGHT, dpi);
    let separator_height = scale_logical(DIAGNOSTIC_SEPARATOR_HEIGHT, dpi);
    let toolbar_height = scale_logical(DIAGNOSTIC_TOOLBAR_HEIGHT, dpi);
    let grid_minimum = scale_logical(DIAGNOSTIC_GRID_MIN_HEIGHT, dpi);
    let available_hud = height
        .saturating_sub(separator_height)
        .saturating_sub(toolbar_height)
        .saturating_sub(separator_height)
        .saturating_sub(grid_minimum)
        .max(0);
    let hud_height = hud_height.min(available_hud);
    let hud_separator_top = hud_height;
    let toolbar_top = hud_separator_top.saturating_add(separator_height);
    let toolbar_separator_top = toolbar_top.saturating_add(toolbar_height);
    let list_top = toolbar_separator_top.saturating_add(separator_height);
    DiagnosticLayout {
        hud_state: DiagnosticLayoutRect {
            left: scale_logical(DIAGNOSTIC_TOOLBAR_MARGIN, dpi),
            top: scale_logical(DIAGNOSTIC_TOOLBAR_MARGIN, dpi),
            width: width
                .saturating_sub(scale_logical(DIAGNOSTIC_TOOLBAR_MARGIN * 2, dpi))
                .max(0),
            height: state_height,
        },
        summary: DiagnosticLayoutRect {
            left: scale_logical(DIAGNOSTIC_TOOLBAR_MARGIN, dpi),
            top: state_height.saturating_add(scale_logical(DIAGNOSTIC_TOOLBAR_MARGIN, dpi)),
            width: width
                .saturating_sub(scale_logical(DIAGNOSTIC_TOOLBAR_MARGIN * 2, dpi))
                .max(0),
            height: hud_height
                .saturating_sub(state_height)
                .saturating_sub(scale_logical(DIAGNOSTIC_TOOLBAR_MARGIN * 2, dpi))
                .max(0),
        },
        hud_separator: DiagnosticLayoutRect {
            left: 0,
            top: hud_separator_top,
            width: width.max(0),
            height: separator_height,
        },
        toolbar: diagnostic_toolbar_layout(width, toolbar_top, toolbar_height, dpi),
        toolbar_separator: DiagnosticLayoutRect {
            left: 0,
            top: toolbar_separator_top,
            width: width.max(0),
            height: separator_height,
        },
        list: DiagnosticLayoutRect {
            left: 0,
            top: list_top,
            width: width.max(0),
            height: height.saturating_sub(list_top).max(0),
        },
    }
}

fn diagnostic_summary_style() -> u32 {
    WS_CHILD | WS_VISIBLE | SS_LEFT | SS_NOPREFIX
}

unsafe fn diagnostic_display_text(app: &App) -> String {
    let Some(input) = app.diagnostic_display_input else {
        return String::new();
    };
    let length = GetWindowTextLengthW(input);
    if length <= 0 {
        return String::new();
    }
    let mut buffer = vec![0u16; length as usize + 1];
    let copied = GetWindowTextW(input, buffer.as_mut_ptr(), buffer.len() as i32);
    String::from_utf16_lossy(&buffer[..copied as usize])
}

unsafe fn diagnostic_search_text(app: &App) -> String {
    let Some(input) = app.diagnostic_search_input else {
        return String::new();
    };
    let length = GetWindowTextLengthW(input);
    if length <= 0 {
        return String::new();
    }
    let mut buffer = vec![0u16; length as usize + 1];
    let copied = GetWindowTextW(input, buffer.as_mut_ptr(), buffer.len() as i32);
    String::from_utf16_lossy(&buffer[..copied as usize])
}

unsafe fn diagnostic_range_text(app: &App) -> String {
    let Some(input) = app.diagnostic_range_input else {
        return String::new();
    };
    let length = GetWindowTextLengthW(input);
    if length <= 0 {
        return String::new();
    }
    let mut buffer = vec![0u16; length as usize + 1];
    let copied = GetWindowTextW(input, buffer.as_mut_ptr(), buffer.len() as i32);
    String::from_utf16_lossy(&buffer[..copied as usize])
}

fn record_diagnostic_event_with_context(
    app: &App,
    name: &str,
    details: impl AsRef<str>,
    phase: DiagnosticPhase,
    outcome: DiagnosticOutcome,
) {
    let source = DiagnosticSource::Diagnostic;
    let context = app.operation.map_or_else(
        || app.diagnostics.begin_operation(source),
        |root| app.diagnostics.child_operation(root, source),
    );
    app.diagnostics.record_with_context(
        DiagnosticRecord {
            context,
            phase,
            source,
            outcome,
            native: native_outcome(name, details.as_ref()),
        },
        name,
        details,
    );
}

fn record_diagnostic_refresh_event(app: &mut App, name: &str, details: impl AsRef<str>) {
    if app.diagnostic_refreshing {
        app.diagnostic_refresh_direct_recorded = true;
    }
    let details = details.as_ref();
    record_diagnostic_event_with_context(
        app,
        name,
        details,
        diagnostic_phase(name),
        diagnostic_outcome(name, details),
    );
}

unsafe fn set_diagnostic_message(app: &mut App, message: impl Into<String>) {
    app.diagnostic_message_text = message.into();
    if let Some(control) = app.diagnostic_message {
        let text = wide(&app.diagnostic_message_text);
        if SetWindowTextW(control, text.as_ptr()) == 0 {
            app.diagnostics.record(
                "native.SetWindowTextW.diagnostic_message.error",
                format!("raw_status={}", GetLastError()),
            );
        }
    }
}

fn diagnostic_selection_summary(app: &App, retained_rows: usize) -> String {
    if !app.diagnostic_grid_selection_sequences.is_empty() {
        if app.diagnostic_grid_selection_reset {
            format!(
                "Selected: {} rows (some no longer retained)",
                app.diagnostic_grid_selection_sequences.len()
            )
        } else {
            format!(
                "Selected: {} rows",
                app.diagnostic_grid_selection_sequences.len()
            )
        }
    } else if app.diagnostic_grid_selection_reset {
        "Selection reset: selected rows are no longer retained".to_owned()
    } else {
        match app.diagnostic_selection {
            Some(selection) if selection.is_all(retained_rows) => {
                format!("Selected: 0 rows; range fallback: all {retained_rows}")
            }
            Some(selection) => format!(
                "Selected: 0 rows; range fallback: {}-{}",
                selection.start(),
                selection.end()
            ),
            None => "Selected: 0 rows; range unavailable".to_owned(),
        }
    }
}

unsafe fn set_diagnostic_selection_summary(app: &mut App, text: impl AsRef<str>) {
    if let Some(summary) = app.diagnostic_selection_summary {
        let text = wide(text.as_ref());
        let _ = SetWindowTextW(summary, text.as_ptr());
    }
}

unsafe fn set_diagnostic_action_enabled(app: &App, enabled: bool) {
    if let Some(copy) = app.diagnostic_copy_button {
        EnableWindow(copy, i32::from(enabled));
    }
    if let Some(export) = app.diagnostic_export_button {
        EnableWindow(export, i32::from(enabled));
    }
}

unsafe fn refresh_diagnostic_controls(
    app: &mut App,
    events: &[tick_diagnostics::DiagnosticEvent],
    preserve_selection: bool,
) {
    let retained_rows = events.len();
    let input = diagnostic_range_text(app);
    let input_is_all = input.trim().is_empty() || input.trim().eq_ignore_ascii_case("all");
    if preserve_selection && app.diagnostic_selection_reset {
        set_diagnostic_selection_summary(app, diagnostic_selection_summary(app, retained_rows));
        set_diagnostic_action_enabled(app, !app.diagnostic_grid_selection_sequences.is_empty());
        return;
    }
    if preserve_selection && !input_is_all {
        if let Some(sequences) = app.diagnostic_selection_sequences.clone() {
            if let Some(selection) = row_selection_for_sequences(events, &sequences) {
                app.diagnostic_selection = Some(selection);
                set_diagnostic_selection_summary(
                    app,
                    diagnostic_selection_summary(app, retained_rows),
                );
                set_diagnostic_action_enabled(
                    app,
                    !app.diagnostic_grid_selection_sequences.is_empty() || !selection.is_empty(),
                );
                return;
            }
            app.diagnostic_selection = None;
            app.diagnostic_selection_sequences = None;
            app.diagnostic_selection_reset = true;
            set_diagnostic_selection_summary(app, diagnostic_selection_summary(app, retained_rows));
            set_diagnostic_action_enabled(app, !app.diagnostic_grid_selection_sequences.is_empty());
            set_diagnostic_message(
                app,
                "Selection reset: selected rows are no longer retained.",
            );
            record_diagnostic_refresh_event(
                app,
                "diagnostic.selection.reset",
                format!("reason=range_rows_not_retained retained_rows={retained_rows}"),
            );
            request_diagnostic_refresh(app);
            return;
        }
    }
    match parse_row_selection(&input, retained_rows) {
        Ok(selection) => {
            app.diagnostic_selection = Some(selection);
            app.diagnostic_selection_sequences = Some(selected_event_sequences(events, selection));
            app.diagnostic_selection_reset = false;
            if app.diagnostic_message_text.starts_with("Invalid range:") {
                set_diagnostic_message(app, "");
            }
        }
        Err(error) => {
            app.diagnostic_selection = None;
            app.diagnostic_selection_sequences = None;
            app.diagnostic_selection_reset = false;
            set_diagnostic_message(app, format!("Invalid range: {error}"));
        }
    }
    set_diagnostic_selection_summary(app, diagnostic_selection_summary(app, retained_rows));
    set_diagnostic_action_enabled(
        app,
        !app.diagnostic_grid_selection_sequences.is_empty()
            || app
                .diagnostic_selection
                .is_some_and(|selection| !selection.is_empty()),
    );
}

pub(crate) unsafe fn list_selected_item_positions(list: *mut c_void) -> Vec<usize> {
    let mut previous = -1i32;
    let mut positions = Vec::new();
    loop {
        let next = SendMessageW(
            list,
            LVM_GETNEXTITEM,
            previous as usize,
            LVNI_SELECTED as isize,
        );
        if next < 0 {
            break;
        }
        let next = next as usize;
        positions.push(next);
        previous = next as i32;
    }
    positions
}

pub(crate) unsafe fn update_diagnostic_grid_selection(app: &mut App) {
    if app.diagnostic_refreshing {
        return;
    }
    let Some(list) = app.diagnostic_list else {
        return;
    };
    let events = app.diagnostics.snapshot();
    let positions = list_selected_item_positions(list);
    let visible = diagnostic_filtered_visible_events(app, &events);
    let mut selected = Vec::new();
    for pos in positions {
        if let Some(event) = visible.get(pos) {
            selected.push(event.sequence);
        }
    }
    app.diagnostic_grid_selection_sequences = selected;
    app.diagnostic_grid_selection_reset = false;
    if app.diagnostic_message_text.starts_with("Selection reset:") {
        set_diagnostic_message(app, "");
    }
    set_diagnostic_selection_summary(app, diagnostic_selection_summary(app, events.len()));
    set_diagnostic_action_enabled(
        app,
        !app.diagnostic_grid_selection_sequences.is_empty()
            || app
                .diagnostic_selection
                .is_some_and(|selection| !selection.is_empty()),
    );
}

fn preserve_diagnostic_grid_selection(app: &mut App, events: &[tick_diagnostics::DiagnosticEvent]) {
    if app.diagnostic_grid_selection_sequences.is_empty() {
        return;
    }
    let previous_count = app.diagnostic_grid_selection_sequences.len();
    let selected =
        selected_event_sequences_for_sequences(events, &app.diagnostic_grid_selection_sequences);
    let invalid_count = previous_count.saturating_sub(selected.len());
    app.diagnostic_grid_selection_sequences = selected;
    if invalid_count > 0 {
        app.diagnostic_grid_selection_reset = true;
        let message = if invalid_count == 1 {
            "Selection reset: 1 selected row is no longer retained."
        } else {
            "Selection reset: selected rows are no longer retained."
        };
        unsafe {
            set_diagnostic_message(app, message);
        }
        record_diagnostic_refresh_event(
            app,
            "diagnostic.selection.reset",
            format!(
                "source=grid-selection invalid_rows={invalid_count} retained_rows={}",
                events.len()
            ),
        );
    }
}

unsafe fn apply_diagnostic_grid_selection(
    app: &App,
    list: *mut c_void,
    events: &[tick_diagnostics::DiagnosticEvent],
) {
    let visible = diagnostic_filtered_visible_events(app, events);
    let positions: Vec<usize> = visible
        .iter()
        .enumerate()
        .filter_map(|(idx, event)| {
            if app
                .diagnostic_grid_selection_sequences
                .contains(&event.sequence)
            {
                Some(idx)
            } else {
                None
            }
        })
        .collect();
    for position in positions {
        let item = ListViewItem {
            mask: LVIF_STATE,
            item: position as i32,
            subitem: 0,
            state: LVIS_SELECTED,
            state_mask: LVIS_SELECTED,
            text: std::ptr::null_mut(),
            text_maximum: 0,
            image: 0,
            parameter: 0,
            indent: 0,
            group_id: 0,
            columns: 0,
            column_indices: std::ptr::null_mut(),
            column_formats: std::ptr::null_mut(),
            group: 0,
        };
        let _ = SendMessageW(
            list,
            LVM_SETITEMSTATE,
            position,
            (&item as *const ListViewItem).cast::<c_void>() as isize,
        );
    }
}

unsafe fn record_diagnostic_layout_failure(
    app: &App,
    stage: &str,
    index: Option<usize>,
    raw_status: u32,
) {
    record_diagnostic_event_with_context(
        app,
        "diagnostic.layout.error",
        format!(
            "stage={stage} control_index={} raw_status={raw_status}",
            index.map_or_else(|| "none".to_owned(), |value| value.to_string())
        ),
        DiagnosticPhase::Render,
        DiagnosticOutcome::Failed,
    );
}

unsafe fn apply_diagnostic_layout(
    window: *mut c_void,
    app: &App,
    controls: &[(Option<*mut c_void>, DiagnosticLayoutRect)],
) -> bool {
    let mut success = true;
    for (index, (control, rect)) in controls.iter().enumerate() {
        let Some(control) = *control else {
            record_diagnostic_layout_failure(app, "missing_control", Some(index), 0);
            success = false;
            break;
        };
        if IsWindow(control) == 0 {
            record_diagnostic_event_with_context(
                app,
                "diagnostic.layout.skipped",
                format!(
                    "control_index={index} reason=not_a_window raw_status={}",
                    GetLastError()
                ),
                DiagnosticPhase::Render,
                DiagnosticOutcome::Suppressed,
            );
            continue;
        }
        if rect.width <= 0 || rect.height <= 0 {
            record_diagnostic_event_with_context(
                app,
                "diagnostic.layout.skipped",
                format!(
                    "control_index={index} reason=empty_rect width={} height={}",
                    rect.width, rect.height
                ),
                DiagnosticPhase::Render,
                DiagnosticOutcome::Suppressed,
            );
            continue;
        }
        if SetWindowPos(
            control,
            std::ptr::null_mut(),
            rect.left,
            rect.top,
            rect.width,
            rect.height,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOSENDCHANGING,
        ) == 0
        {
            record_diagnostic_layout_failure(app, "SetWindowPos", Some(index), GetLastError());
            success = false;
            break;
        }
    }
    let _ = InvalidateRect(window, std::ptr::null(), 1);
    success
}

unsafe fn fit_diagnostic_details_to_viewport(list: *mut c_void, client_width: i32, dpi: u32) {
    if list.is_null() {
        return;
    }
    let horizontal_scroll = GetScrollPos(list, SB_HORZ);
    let mut fixed_width = 0i32;
    for index in 0..REPORT_COLUMNS.len() - 1 {
        let current = SendMessageW(list, LVM_GETCOLUMNWIDTH, index, 0).max(0) as i32;
        let minimum = scale_logical(DIAGNOSTIC_COLUMN_MIN_WIDTHS[index], dpi);
        let maximum = scale_logical(DIAGNOSTIC_COLUMN_MAX_WIDTHS[index], dpi);
        let width = current.clamp(minimum, maximum);
        let _ = SendMessageW(list, LVM_SETCOLUMNWIDTH, index, width as isize);
        fixed_width = fixed_width.saturating_add(width);
    }
    let details_index = REPORT_COLUMNS.len() - 1;
    let details_minimum = scale_logical(DIAGNOSTIC_COLUMN_MIN_WIDTHS[details_index], dpi);
    let details_width = client_width
        .saturating_sub(fixed_width)
        .max(details_minimum);
    let _ = SendMessageW(
        list,
        LVM_SETCOLUMNWIDTH,
        details_index,
        details_width as isize,
    );
    let _ = SetScrollPos(list, SB_HORZ, horizontal_scroll, 0);
}

unsafe fn layout_diagnostic_controls(window: *mut c_void, app: &App) -> bool {
    let mut client = Rect {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    if GetClientRect(window, &mut client) == 0 {
        record_diagnostic_layout_failure(app, "GetClientRect", None, GetLastError());
        return false;
    }
    let width = (client.right - client.left).max(0);
    let height = (client.bottom - client.top).max(0);
    let dpi = GetDpiForWindow(window).max(96);
    let layout = diagnostic_layout(width, height, dpi);
    let toolbar = layout.toolbar;
    let controls = [
        (app.diagnostic_hud_state, layout.hud_state),
        (app.diagnostic_summary, layout.summary),
        (app.diagnostic_hud_separator, layout.hud_separator),
        (app.diagnostic_toolbar_separator, layout.toolbar_separator),
        (app.diagnostic_display_label, toolbar.display_label),
        (app.diagnostic_display_input, toolbar.display_input),
        (app.diagnostic_show_all_button, toolbar.show_all),
        (app.diagnostic_category_filter, toolbar.category_filter),
        (app.diagnostic_search_label, toolbar.search_label),
        (app.diagnostic_search_input, toolbar.search_input),
        (app.diagnostic_toolbar_label, toolbar.transfer_label),
        (app.diagnostic_range_input, toolbar.range_input),
        (app.diagnostic_selection_summary, toolbar.selection_summary),
        (app.diagnostic_copy_button, toolbar.copy),
        (app.diagnostic_export_button, toolbar.export),
        (app.diagnostic_export_all_button, toolbar.export_all),
        (app.diagnostic_message, toolbar.message),
        (app.diagnostic_list, layout.list),
    ];
    let positioned = apply_diagnostic_layout(window, app, &controls);
    if let Some(list) = app.diagnostic_list {
        fit_diagnostic_details_to_viewport(list, layout.list.width, dpi);
    }
    positioned
}

fn diagnostic_snapshot_key(events: &[tick_diagnostics::DiagnosticEvent]) -> Option<(usize, u64)> {
    events.last().map(|event| (events.len(), event.sequence))
}

fn diagnostic_data_snapshot_changed(
    previous: Option<(usize, u64)>,
    current: Option<(usize, u64)>,
) -> bool {
    previous != current
}

fn diagnostic_auto_fit_once(
    layout_stable: bool,
    snapshot_changed: bool,
    generation: u64,
    fitted_generation: Option<u64>,
) -> bool {
    !layout_stable || (snapshot_changed && fitted_generation != Some(generation))
}

fn diagnostic_refresh_is_coalesced(pending: bool, refreshing: bool) -> bool {
    pending || refreshing
}

fn diagnostic_follow_up_needed(generated_direct_records: bool, follow_up_refresh: bool) -> bool {
    generated_direct_records && !follow_up_refresh
}

#[cfg(test)]
fn diagnostic_auto_fit_reason_allows(reason: DiagnosticRefreshReason) -> bool {
    matches!(
        reason,
        DiagnosticRefreshReason::Initial | DiagnosticRefreshReason::Data
    )
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DiagnosticRefreshReason {
    Initial,
    Data,
    View,
    Resize,
    Scroll,
}

#[cfg(test)]
fn diagnostic_loading_summary_is_nonblank() -> bool {
    !DIAGNOSTIC_LOADING_SUMMARY.trim().is_empty()
}

#[cfg(test)]
fn diagnostic_summary_is_non_loading(summary: &str) -> bool {
    let summary = summary.trim();
    !summary.is_empty() && summary != DIAGNOSTIC_LOADING_SUMMARY
}

unsafe fn auto_fit_diagnostic_columns(list: *mut c_void, client_width: i32, dpi: u32) {
    let mut measured = [0i32; REPORT_COLUMNS.len()];
    for (index, value) in measured.iter_mut().enumerate() {
        let _ = SendMessageW(list, LVM_SETCOLUMNWIDTH, index, LVSCW_AUTOSIZE as isize);
        let content_width = SendMessageW(list, LVM_GETCOLUMNWIDTH, index, 0).max(0) as i32;
        let _ = SendMessageW(
            list,
            LVM_SETCOLUMNWIDTH,
            index,
            LVSCW_AUTOSIZE_USEHEADER as isize,
        );
        let header_width = SendMessageW(list, LVM_GETCOLUMNWIDTH, index, 0).max(0) as i32;
        *value = content_width.max(header_width);
    }

    let mut fixed_width = 0i32;
    for index in 0..REPORT_COLUMNS.len() - 1 {
        let minimum = scale_logical(DIAGNOSTIC_COLUMN_MIN_WIDTHS[index], dpi);
        let maximum = scale_logical(DIAGNOSTIC_COLUMN_MAX_WIDTHS[index], dpi);
        let width = measured[index].clamp(minimum, maximum);
        let _ = SendMessageW(list, LVM_SETCOLUMNWIDTH, index, width as isize);
        fixed_width = fixed_width.saturating_add(width);
    }

    let details_index = REPORT_COLUMNS.len() - 1;
    let details_minimum = scale_logical(DIAGNOSTIC_COLUMN_MIN_WIDTHS[details_index], dpi);
    let details_maximum = scale_logical(DIAGNOSTIC_COLUMN_MAX_WIDTHS[details_index], dpi);
    let measured_details = measured[details_index].clamp(details_minimum, details_maximum);
    let remaining_width = client_width.saturating_sub(fixed_width);
    let details_width = measured_details.max(details_minimum).max(remaining_width);
    let _ = SendMessageW(
        list,
        LVM_SETCOLUMNWIDTH,
        details_index,
        details_width as isize,
    );
}

const fn diagnostic_list_extended_style() -> usize {
    LVS_EX_GRIDLINES | LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER
}

unsafe fn initialize_diagnostic_list(list: *mut c_void) -> Result<(), u32> {
    let extended_style = diagnostic_list_extended_style();
    let _ = SendMessageW(
        list,
        LVM_SETEXTENDEDLISTVIEWSTYLE,
        extended_style,
        extended_style as isize,
    );
    for (index, label) in REPORT_COLUMNS.iter().enumerate() {
        let mut text = wide(label);
        let column = ListViewColumn {
            mask: LVCF_TEXT | LVCF_WIDTH,
            format: LVCFMT_LEFT,
            width: DIAGNOSTIC_COLUMN_WIDTHS[index],
            text: text.as_mut_ptr(),
            text_maximum: text.len() as i32,
            subitem: index as i32,
            image: 0,
            order: index as i32,
            minimum_width: 0,
            default_width: 0,
            ideal_width: 0,
        };
        let insert_result = SendMessageW(
            list,
            LVM_INSERTCOLUMNW,
            index,
            (&column as *const ListViewColumn).cast::<c_void>() as isize,
        );
        if insert_result < 0 {
            return Err(GetLastError());
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DiagnosticChildRedrawTarget {
    HudState,
    Summary,
    HudSeparator,
    ToolbarSeparator,
    DisplayLabel,
    DisplayInput,
    ShowAllButton,
    CategoryFilter,
    SearchLabel,
    SearchInput,
    ToolbarLabel,
    SelectionSummary,
    RangeInput,
    CopyButton,
    ExportButton,
    ExportAllButton,
    Message,
    List,
}

const DIAGNOSTIC_POST_REDRAW_CHILD_TARGETS: [DiagnosticChildRedrawTarget; 18] = [
    DiagnosticChildRedrawTarget::HudState,
    DiagnosticChildRedrawTarget::Summary,
    DiagnosticChildRedrawTarget::HudSeparator,
    DiagnosticChildRedrawTarget::ToolbarSeparator,
    DiagnosticChildRedrawTarget::DisplayLabel,
    DiagnosticChildRedrawTarget::DisplayInput,
    DiagnosticChildRedrawTarget::ShowAllButton,
    DiagnosticChildRedrawTarget::CategoryFilter,
    DiagnosticChildRedrawTarget::SearchLabel,
    DiagnosticChildRedrawTarget::SearchInput,
    DiagnosticChildRedrawTarget::ToolbarLabel,
    DiagnosticChildRedrawTarget::SelectionSummary,
    DiagnosticChildRedrawTarget::RangeInput,
    DiagnosticChildRedrawTarget::CopyButton,
    DiagnosticChildRedrawTarget::ExportButton,
    DiagnosticChildRedrawTarget::ExportAllButton,
    DiagnosticChildRedrawTarget::Message,
    DiagnosticChildRedrawTarget::List,
];

unsafe fn set_diagnostic_redraw(app: &App, enabled: bool) {
    let redraw = usize::from(enabled);
    if let Some(window) = app.diagnostic_window {
        let _ = SendMessageW(window, WM_SETREDRAW, redraw, 0);
    }
    for control in [
        app.diagnostic_hud_state,
        app.diagnostic_summary,
        app.diagnostic_hud_separator,
        app.diagnostic_toolbar_separator,
        app.diagnostic_display_label,
        app.diagnostic_display_input,
        app.diagnostic_show_all_button,
        app.diagnostic_category_filter,
        app.diagnostic_search_label,
        app.diagnostic_search_input,
        app.diagnostic_toolbar_label,
        app.diagnostic_selection_summary,
        app.diagnostic_range_input,
        app.diagnostic_copy_button,
        app.diagnostic_export_button,
        app.diagnostic_export_all_button,
        app.diagnostic_message,
        app.diagnostic_list,
    ]
    .into_iter()
    .flatten()
    {
        let _ = SendMessageW(control, WM_SETREDRAW, redraw, 0);
    }
}

unsafe fn finish_diagnostic_redraw(window: *mut c_void, app: &App) {
    set_diagnostic_redraw(app, true);
    for target in DIAGNOSTIC_POST_REDRAW_CHILD_TARGETS {
        let control = match target {
            DiagnosticChildRedrawTarget::HudState => app.diagnostic_hud_state,
            DiagnosticChildRedrawTarget::Summary => app.diagnostic_summary,
            DiagnosticChildRedrawTarget::HudSeparator => app.diagnostic_hud_separator,
            DiagnosticChildRedrawTarget::ToolbarSeparator => app.diagnostic_toolbar_separator,
            DiagnosticChildRedrawTarget::DisplayLabel => app.diagnostic_display_label,
            DiagnosticChildRedrawTarget::DisplayInput => app.diagnostic_display_input,
            DiagnosticChildRedrawTarget::ShowAllButton => app.diagnostic_show_all_button,
            DiagnosticChildRedrawTarget::CategoryFilter => app.diagnostic_category_filter,
            DiagnosticChildRedrawTarget::SearchLabel => app.diagnostic_search_label,
            DiagnosticChildRedrawTarget::SearchInput => app.diagnostic_search_input,
            DiagnosticChildRedrawTarget::ToolbarLabel => app.diagnostic_toolbar_label,
            DiagnosticChildRedrawTarget::SelectionSummary => app.diagnostic_selection_summary,
            DiagnosticChildRedrawTarget::RangeInput => app.diagnostic_range_input,
            DiagnosticChildRedrawTarget::CopyButton => app.diagnostic_copy_button,
            DiagnosticChildRedrawTarget::ExportButton => app.diagnostic_export_button,
            DiagnosticChildRedrawTarget::ExportAllButton => app.diagnostic_export_all_button,
            DiagnosticChildRedrawTarget::Message => app.diagnostic_message,
            DiagnosticChildRedrawTarget::List => app.diagnostic_list,
        };
        if let Some(control) = control {
            if control.is_null() {
                continue;
            }
            let _ = InvalidateRect(control, std::ptr::null(), 1);
            let _ = UpdateWindow(control);
        }
    }
    let _ = InvalidateRect(window, std::ptr::null(), 1);
    let _ = UpdateWindow(window);
}

fn diagnostic_grid_refresh_message(
    snapshot_rows: usize,
    inserted_rows: usize,
    item_count: isize,
    insert_failures: usize,
    set_text_failures: usize,
) -> String {
    format!(
        "snapshot_rows={snapshot_rows} inserted_rows={inserted_rows} item_count={item_count} insert_failures={insert_failures} set_text_failures={set_text_failures}"
    )
}

unsafe fn refresh_diagnostic_window(
    window: *mut c_void,
    app: &mut App,
    allow_follow_up_post: bool,
) {
    if app.diagnostic_refreshing {
        return;
    }
    app.diagnostic_refreshing = true;
    let follow_up_refresh = app.diagnostic_refresh_follow_up_scheduled;
    app.diagnostic_refresh_follow_up_scheduled = false;
    app.diagnostic_refresh_direct_recorded = false;
    set_diagnostic_redraw(app, false);
    let events = app.diagnostics.snapshot();
    let retained_rows = events.len();
    let snapshot_key = diagnostic_snapshot_key(&events);
    let snapshot_changed =
        diagnostic_data_snapshot_changed(app.diagnostic_snapshot_key, snapshot_key);
    if snapshot_changed {
        app.diagnostic_snapshot_generation = app.diagnostic_snapshot_generation.saturating_add(1);
        app.diagnostic_snapshot_key = snapshot_key;
    }
    let auto_fit = diagnostic_auto_fit_once(
        app.diagnostic_layout_stable,
        snapshot_changed,
        app.diagnostic_snapshot_generation,
        app.diagnostic_auto_fit_generation,
    );
    if auto_fit {
        app.diagnostic_auto_fit_generation = Some(app.diagnostic_snapshot_generation);
    }
    let horizontal_scroll = app
        .diagnostic_layout_stable
        .then(|| app.diagnostic_list.map(|list| GetScrollPos(list, SB_HORZ)))
        .flatten();
    preserve_diagnostic_grid_selection(app, &events);
    refresh_diagnostic_controls(app, &events, true);
    let Some(list) = app.diagnostic_list else {
        app.diagnostics
            .record("diagnostic.grid.refresh.error", "list_handle_null");
        finish_diagnostic_redraw(window, app);
        finish_diagnostic_refresh(app, false);
        return;
    };
    // Capture the scroll position before the rebuild so we can decide whether the
    // viewport was already pinned to the newest row. An empty list defaults to
    // bottom tracking so a fresh window starts auto scrolling.
    let count_before = SendMessageW(list, LVM_GETITEMCOUNT, 0, 0);
    let was_at_bottom = if count_before > 0 {
        let top = SendMessageW(list, LVM_GETTOPINDEX, 0, 0).max(0);
        let page = SendMessageW(list, LVM_GETCOUNTPERPAGE, 0, 0).max(0);
        top.saturating_add(page) >= count_before
    } else {
        true
    };
    let snapshot_rows = events.len();
    let _ = SendMessageW(list, LVM_DELETEALLITEMS, 0, 0);
    let visible_events = diagnostic_filtered_visible_events(app, &events);
    let rows: Vec<DiagnosticGridRow> = visible_events
        .iter()
        .enumerate()
        .map(|(index, event)| tick_diagnostics::diagnostic_grid_row(index.saturating_add(1), event))
        .collect();
    let mut inserted_rows = 0usize;
    let mut insert_failures = 0usize;
    let mut set_text_failures = 0usize;
    for (row_index, row) in rows.into_iter().enumerate() {
        let mut first = wide(&row.cells[0]);
        let item = ListViewItem {
            mask: LVIF_TEXT,
            item: row_index as i32,
            subitem: 0,
            state: 0,
            state_mask: 0,
            text: first.as_mut_ptr(),
            text_maximum: first.len().saturating_sub(1) as i32,
            image: 0,
            parameter: 0,
            indent: 0,
            group_id: 0,
            columns: 0,
            column_indices: std::ptr::null_mut(),
            column_formats: std::ptr::null_mut(),
            group: 0,
        };
        let insert_result = SendMessageW(
            list,
            LVM_INSERTITEMW,
            0,
            (&item as *const ListViewItem).cast::<c_void>() as isize,
        );
        if insert_result < 0 {
            insert_failures = insert_failures.saturating_add(1);
            record_diagnostic_refresh_event(
                app,
                "native.LVM_INSERTITEMW.error",
                format!(
                    "row={row_index} result={insert_result} raw_status={}",
                    GetLastError()
                ),
            );
            break;
        }
        inserted_rows = inserted_rows.saturating_add(1);
        for (column_index, value) in row.cells.iter().enumerate().skip(1) {
            let mut text = wide(value);
            let mut cell = ListViewItem {
                mask: LVIF_TEXT,
                item: row_index as i32,
                subitem: column_index as i32,
                state: 0,
                state_mask: 0,
                text: text.as_mut_ptr(),
                text_maximum: text.len().saturating_sub(1) as i32,
                image: 0,
                parameter: 0,
                indent: 0,
                group_id: 0,
                columns: 0,
                column_indices: std::ptr::null_mut(),
                column_formats: std::ptr::null_mut(),
                group: 0,
            };
            let set_text_result = SendMessageW(
                list,
                LVM_SETITEMTEXTW,
                row_index,
                (&mut cell as *mut ListViewItem).cast::<c_void>() as isize,
            );
            if set_text_result == 0 {
                set_text_failures = set_text_failures.saturating_add(1);
                record_diagnostic_refresh_event(
                    app,
                    "native.LVM_SETITEMTEXTW.error",
                    format!(
                        "row={row_index} column={column_index} result={set_text_result} raw_status={}",
                        GetLastError()
                    ),
                );
            }
        }
    }
    apply_diagnostic_grid_selection(app, list, &events);
    let item_count = SendMessageW(list, LVM_GETITEMCOUNT, 0, 0);
    if !follow_up_refresh {
        record_diagnostic_refresh_event(
            app,
            "diagnostic.grid.refresh",
            diagnostic_grid_refresh_message(
                snapshot_rows,
                inserted_rows,
                item_count,
                insert_failures,
                set_text_failures,
            ),
        );
    }
    if auto_fit {
        let mut client = Rect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        if GetClientRect(list, &mut client) != 0 {
            auto_fit_diagnostic_columns(
                list,
                (client.right - client.left).max(0),
                GetDpiForWindow(window).max(96),
            );
        }
    }
    // If the user was already watching the newest rows, keep the tail of the
    // log in view after the rebuild. When they scrolled up to inspect history
    // we leave the viewport untouched so their reading position is preserved.
    if was_at_bottom {
        let count_after = SendMessageW(list, LVM_GETITEMCOUNT, 0, 0);
        if count_after > 0 {
            let _ = SendMessageW(list, LVM_ENSUREVISIBLE, (count_after - 1) as usize, 0);
        }
    }
    if let Some(position) = horizontal_scroll {
        let _ = SetScrollPos(list, SB_HORZ, position, 0);
    }
    if let Some(state) = app.diagnostic_hud_state {
        let text = wide(&diagnostic_state_text(app));
        if SetWindowTextW(state, text.as_ptr()) == 0 {
            app.diagnostics.record(
                "native.SetWindowTextW.diagnostic_hud.error",
                format!("raw_status={}", GetLastError()),
            );
        }
    }
    if let Some(summary) = app.diagnostic_summary {
        let text = wide(&diagnostic_summary_text(app, retained_rows));
        if SetWindowTextW(summary, text.as_ptr()) == 0 {
            app.diagnostics.record(
                "native.SetWindowTextW.diagnostic_summary.error",
                format!("raw_status={}", GetLastError()),
            );
        }
    }
    app.diagnostic_layout_stable = true;
    let generated_direct_records = app.diagnostic_refresh_direct_recorded;
    finish_diagnostic_redraw(window, app);
    finish_diagnostic_refresh(
        app,
        allow_follow_up_post
            && diagnostic_follow_up_needed(generated_direct_records, follow_up_refresh),
    );
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

unsafe fn copy_tsv_to_clipboard(owner: *mut c_void, text: &str) -> Result<(), NativeFailure> {
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

unsafe fn choose_export_path(owner: *mut c_void) -> Result<Option<PathBuf>, NativeFailure> {
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
enum DiagnosticExportFormat {
    Csv,
    Tsv,
}

impl DiagnosticExportFormat {
    fn label(self) -> &'static str {
        match self {
            Self::Csv => "CSV",
            Self::Tsv => "TSV",
        }
    }
}

fn export_format_for_path(path: &Path) -> DiagnosticExportFormat {
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

fn format_export_rows(rows: &[DiagnosticGridRow], format: DiagnosticExportFormat) -> String {
    match format {
        DiagnosticExportFormat::Csv => format_csv(rows),
        DiagnosticExportFormat::Tsv => format_tsv(rows),
    }
}

unsafe fn write_export_tsv(path: &Path, text: &str) -> Result<(), NativeFailure> {
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

fn diagnostic_range_details(selection: RowSelection, retained_rows: usize) -> String {
    format!(
        "selected_range={}-{} retained_rows={} row_count={} format=TSV",
        selection.start(),
        selection.end(),
        retained_rows,
        selection.row_count()
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DiagnosticTransferSource {
    GridSelection,
    RangeSelection,
}

impl DiagnosticTransferSource {
    fn label(self) -> &'static str {
        match self {
            Self::GridSelection => "grid-selection",
            Self::RangeSelection => "range-selection",
        }
    }
}

fn diagnostic_transfer_details(
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

fn diagnostic_transfer_source(
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

const fn diagnostic_list_style() -> u32 {
    (WS_CHILD
        | WS_VISIBLE
        | WS_CLIPCHILDREN
        | WS_BORDER
        | WS_HSCROLL
        | LVS_REPORT
        | LVS_SHOWSELALWAYS)
        & !LVS_SINGLESEL
}

fn diagnostic_toolbar_action(app: &mut App, action: &str) {
    let events = app.diagnostics.snapshot();
    let retained_rows = events.len();
    app.begin_operation(DiagnosticSource::Diagnostic);
    let grid_sequences =
        selected_event_sequences_for_sequences(&events, &app.diagnostic_grid_selection_sequences);
    let (source, rows, range) = if !grid_sequences.is_empty() {
        (
            diagnostic_transfer_source(grid_sequences.len(), 0).expect("grid selection has rows"),
            diagnostic_grid_rows_for_sequences(&events, &grid_sequences),
            None,
        )
    } else {
        let input = unsafe { diagnostic_range_text(app) };
        let parsed = parse_row_selection(&input, retained_rows);
        let selection = match parsed {
            Ok(selection) if !selection.is_empty() => selection,
            Ok(selection) => {
                let details = diagnostic_transfer_details(
                    DiagnosticTransferSource::RangeSelection,
                    0,
                    retained_rows,
                    Some(selection),
                );
                app.record("diagnostic.range.parsed", format!("result=empty {details}"));
                app.record(
                    "diagnostic.validation_failure",
                    format!("{details} result=failed reason=no_retained_events"),
                );
                unsafe {
                    set_diagnostic_message(app, "No retained events.");
                }
                app.finish_operation(DiagnosticOutcome::Failed);
                return;
            }
            Err(error) => {
                let details = diagnostic_transfer_details(
                    DiagnosticTransferSource::RangeSelection,
                    0,
                    retained_rows,
                    None,
                );
                app.record(
                    "diagnostic.range.parsed",
                    format!("{details} result=invalid error={error}"),
                );
                app.record(
                    "diagnostic.validation_failure",
                    format!("{details} result=failed error={error}"),
                );
                unsafe {
                    set_diagnostic_message(app, format!("Invalid range: {error}"));
                }
                app.finish_operation(DiagnosticOutcome::Failed);
                return;
            }
        };
        app.diagnostic_selection = Some(selection);
        app.diagnostic_selection_sequences = Some(selected_event_sequences(&events, selection));
        app.diagnostic_selection_reset = false;
        (
            diagnostic_transfer_source(0, selection.row_count()).expect("range selection has rows"),
            diagnostic_grid_rows(&events, selection),
            Some(selection),
        )
    };
    let selected_row_count = rows.len();
    let details = format!(
        "{} {}",
        diagnostic_transfer_details(source, selected_row_count, retained_rows, range),
        retention_summary(
            retained_rows,
            app.diagnostics.maximum_events(),
            snapshot_is_truncated(&events),
        )
    );
    let tsv = format_tsv(&rows);
    app.record(
        "diagnostic.transfer.selection",
        format!("{details} result=selected"),
    );
    match action {
        "copy" => {
            app.record(
                "diagnostic.copy.request",
                format!("{details} result=requested"),
            );
            let result = unsafe {
                copy_tsv_to_clipboard(app.diagnostic_window.unwrap_or(std::ptr::null_mut()), &tsv)
            };
            match result {
                Ok(()) => {
                    if !matches!(
                        map_native_action(true, 0),
                        DiagnosticNativeAction::Completed
                    ) {
                        unreachable!("successful clipboard operation must map to completed");
                    }
                    unsafe {
                        set_diagnostic_message(app, format!("Copied {} rows as TSV.", rows.len()));
                    }
                    app.record(
                        "diagnostic.copy.result",
                        format!("result=success {details}"),
                    );
                    app.finish_operation(DiagnosticOutcome::Completed);
                }
                Err(error) => {
                    let DiagnosticNativeAction::Failed { raw_error } =
                        map_native_action(false, error.raw_error)
                    else {
                        unreachable!("failed clipboard operation must map to failed");
                    };
                    app.record(
                        "native.clipboard.error",
                        format!(
                            "stage={} {details} result=failed raw_status={raw_error}",
                            error.stage
                        ),
                    );
                    unsafe {
                        set_diagnostic_message(
                            app,
                            format!("Copy failed (native error {}).", error.raw_error),
                        );
                    }
                    app.record(
                        "diagnostic.copy.result",
                        format!("result=failed {details} raw_status={}", error.raw_error),
                    );
                    app.finish_operation(DiagnosticOutcome::Failed);
                }
            }
        }
        "export" => {
            app.record(
                "diagnostic.export.request",
                format!("{details} result=requested"),
            );
            let path = match unsafe {
                choose_export_path(app.diagnostic_window.unwrap_or(std::ptr::null_mut()))
            } {
                Ok(Some(path)) => path,
                Ok(None) => {
                    unsafe {
                        set_diagnostic_message(app, "Export cancelled.");
                    }
                    app.record(
                        "diagnostic.export.cancelled",
                        format!("result=cancelled {details}"),
                    );
                    app.record(
                        "diagnostic.export.result",
                        format!("result=cancelled {details}"),
                    );
                    app.finish_operation(DiagnosticOutcome::Cancelled);
                    return;
                }
                Err(error) => {
                    app.record(
                        "native.GetSaveFileNameW.error",
                        format!("{details} result=failed raw_status={}", error.raw_error),
                    );
                    unsafe {
                        set_diagnostic_message(
                            app,
                            format!("Export dialog failed (native error {}).", error.raw_error),
                        );
                    }
                    app.record(
                        "diagnostic.export.result",
                        format!("result=failed {details} raw_status={}", error.raw_error),
                    );
                    app.finish_operation(DiagnosticOutcome::Failed);
                    return;
                }
            };
            let format = export_format_for_path(&path);
            let export_text = format_export_rows(&rows, format);
            let write_result = unsafe { write_export_tsv(&path, &export_text) };
            match write_result {
                Ok(()) => {
                    unsafe {
                        set_diagnostic_message(
                            app,
                            format!("Exported {} rows as {}.", rows.len(), format.label()),
                        );
                    }
                    app.record(
                        "diagnostic.export.result",
                        format!("result=success {details} path=redacted"),
                    );
                    app.finish_operation(DiagnosticOutcome::Completed);
                }
                Err(error) => {
                    app.record(
                        "native.export.file.error",
                        format!(
                            "stage={} {details} result=failed raw_status={}",
                            error.stage, error.raw_error
                        ),
                    );
                    unsafe {
                        set_diagnostic_message(
                            app,
                            format!("Export failed (native error {}).", error.raw_error),
                        );
                    }
                    app.record(
                        "diagnostic.export.result",
                        format!(
                            "result=failed {details} path=redacted raw_status={}",
                            error.raw_error
                        ),
                    );
                    app.finish_operation(DiagnosticOutcome::Failed);
                }
            }
        }
        "export_all" => {
            let all_rows = diagnostic_grid_rows(&events, RowSelection::all(retained_rows));
            let all_details = format!(
                "source=export-all selected_row_count={} retained_rows={} format=TSV {}",
                all_rows.len(),
                retained_rows,
                retention_summary(
                    retained_rows,
                    app.diagnostics.maximum_events(),
                    snapshot_is_truncated(&events),
                )
            );
            app.record(
                "diagnostic.export.request",
                format!("{all_details} result=requested mode=all"),
            );
            let path = match unsafe {
                choose_export_path(app.diagnostic_window.unwrap_or(std::ptr::null_mut()))
            } {
                Ok(Some(path)) => path,
                Ok(None) => {
                    unsafe {
                        set_diagnostic_message(app, "Export cancelled.");
                    }
                    app.record(
                        "diagnostic.export.cancelled",
                        format!("result=cancelled {all_details}"),
                    );
                    app.record(
                        "diagnostic.export.result",
                        format!("result=cancelled {all_details}"),
                    );
                    app.finish_operation(DiagnosticOutcome::Cancelled);
                    return;
                }
                Err(error) => {
                    app.record(
                        "native.GetSaveFileNameW.error",
                        format!("{all_details} result=failed raw_status={}", error.raw_error),
                    );
                    unsafe {
                        set_diagnostic_message(
                            app,
                            format!("Export dialog failed (native error {}).", error.raw_error),
                        );
                    }
                    app.record(
                        "diagnostic.export.result",
                        format!("result=failed {all_details} raw_status={}", error.raw_error),
                    );
                    app.finish_operation(DiagnosticOutcome::Failed);
                    return;
                }
            };
            let format = export_format_for_path(&path);
            let export_text = format_export_rows(&all_rows, format);
            let write_result = unsafe { write_export_tsv(&path, &export_text) };
            match write_result {
                Ok(()) => {
                    unsafe {
                        set_diagnostic_message(
                            app,
                            format!(
                                "Exported all {} rows as {}.",
                                all_rows.len(),
                                format.label()
                            ),
                        );
                    }
                    app.record(
                        "diagnostic.export.result",
                        format!("result=success {all_details} path=redacted mode=all"),
                    );
                    app.finish_operation(DiagnosticOutcome::Completed);
                }
                Err(error) => {
                    app.record(
                        "native.export.file.error",
                        format!(
                            "stage={} {all_details} result=failed raw_status={}",
                            error.stage, error.raw_error
                        ),
                    );
                    unsafe {
                        set_diagnostic_message(
                            app,
                            format!("Export failed (native error {}).", error.raw_error),
                        );
                    }
                    app.record(
                        "diagnostic.export.result",
                        format!(
                            "result=failed {all_details} path=redacted raw_status={}",
                            error.raw_error
                        ),
                    );
                    app.finish_operation(DiagnosticOutcome::Failed);
                }
            }
        }
        _ => {
            app.record(
                "diagnostic.toolbar.request",
                format!("action={action} result=ignored"),
            );
            app.finish_operation(DiagnosticOutcome::Suppressed);
        }
    }
}

fn diagnostic_range_changed(app: &mut App) {
    let events = app.diagnostics.snapshot();
    let retained_rows = events.len();
    let input = unsafe { diagnostic_range_text(app) };
    app.begin_operation(DiagnosticSource::Diagnostic);
    match parse_row_selection(&input, retained_rows) {
        Ok(selection) => {
            app.diagnostic_selection = Some(selection);
            app.diagnostic_selection_sequences = Some(selected_event_sequences(&events, selection));
            app.diagnostic_selection_reset = false;
            app.record(
                "diagnostic.range.parsed",
                format!(
                    "result=success {}",
                    diagnostic_range_details(selection, retained_rows)
                ),
            );
            app.finish_operation(DiagnosticOutcome::Completed);
        }
        Err(error) => {
            app.record(
                "diagnostic.range.parsed",
                format!(
                    "selected_range=none retained_rows={retained_rows} row_count=0 format=TSV result=invalid error={error}"
                ),
            );
            app.record(
                "diagnostic.validation_failure",
                format!(
                    "selected_range=none retained_rows={retained_rows} row_count=0 format=TSV result=failed error={error}"
                ),
            );
            app.diagnostic_selection = None;
            app.diagnostic_selection_sequences = None;
            app.diagnostic_selection_reset = false;
            app.finish_operation(DiagnosticOutcome::Failed);
        }
    }
    unsafe {
        refresh_diagnostic_controls(app, &events, false);
    }
}

fn diagnostic_display_changed(app: &mut App) {
    let input = unsafe { diagnostic_display_text(app) };
    match display_limit_input_decision(&input) {
        DisplayLimitInputDecision::PreserveEmpty => {
            app.begin_operation(DiagnosticSource::Diagnostic);
            app.record(
                "diagnostic.display_limit.validation",
                format!(
                    "result=empty input_bytes={} retained_cap={}",
                    input.len(),
                    app.diagnostics.maximum_events()
                ),
            );
            app.finish_operation(DiagnosticOutcome::Suppressed);
        }
        DisplayLimitInputDecision::Invalid => {
            app.begin_operation(DiagnosticSource::Diagnostic);
            app.record(
                "diagnostic.display_limit.validation",
                format!(
                    "result=invalid input_bytes={} retained_cap={}",
                    input.len(),
                    app.diagnostics.maximum_events()
                ),
            );
            unsafe {
                set_diagnostic_message(
                    app,
                    format!(
                        "Invalid Show rows value: {}",
                        parse_display_limit(&input).unwrap_err()
                    ),
                );
            }
            app.finish_operation(DiagnosticOutcome::Suppressed);
        }
        DisplayLimitInputDecision::Apply(limit) => {
            app.diagnostic_display_limit = limit;
            app.diagnostic_display_all = false;
            app.begin_operation(DiagnosticSource::Diagnostic);
            app.record(
                "diagnostic.display_limit.changed",
                format!("result=success limit={limit} override=none"),
            );
            unsafe {
                if app
                    .diagnostic_message_text
                    .starts_with("Invalid Show rows value:")
                {
                    set_diagnostic_message(app, "");
                }
            }
            app.finish_operation(DiagnosticOutcome::Completed);
        }
    }
}

fn diagnostic_show_all(app: &mut App) {
    app.diagnostic_display_all = true;
    app.begin_operation(DiagnosticSource::Diagnostic);
    app.record(
        "diagnostic.display_limit.override",
        format!(
            "result=success mode=all retained_cap={}",
            app.diagnostics.maximum_events()
        ),
    );
    unsafe {
        if app
            .diagnostic_message_text
            .starts_with("Invalid Show rows value:")
        {
            set_diagnostic_message(app, "");
        }
    }
    app.finish_operation(DiagnosticOutcome::Completed);
}

fn diagnostic_category_filter_changed(app: &mut App) {
    let Some(combo) = app.diagnostic_category_filter else {
        return;
    };
    let index = unsafe { SendMessageW(combo, CB_GETCURSEL, 0, 0) };
    let category = match index {
        1 => Some(EventCategory::Startup),
        2 => Some(EventCategory::Timer),
        3 => Some(EventCategory::Power),
        4 => Some(EventCategory::UI),
        5 => Some(EventCategory::Schedule),
        6 => Some(EventCategory::System),
        _ => None,
    };
    app.diagnostic_selected_category = category;
    app.begin_operation(DiagnosticSource::Diagnostic);
    app.record(
        "diagnostic.category_filter.changed",
        format!("category={}", category.map_or("All", |c| c.as_str())),
    );
    app.finish_operation(DiagnosticOutcome::Completed);
    request_diagnostic_refresh(app);
}

fn diagnostic_search_changed(app: &mut App) {
    let search = unsafe { diagnostic_search_text(app) };
    if search == app.diagnostic_search_text {
        return;
    }
    app.diagnostic_search_text = search;
    app.begin_operation(DiagnosticSource::Diagnostic);
    app.record(
        "diagnostic.search.changed",
        format!("query_bytes={}", app.diagnostic_search_text.len()),
    );
    app.finish_operation(DiagnosticOutcome::Completed);
    request_diagnostic_refresh(app);
}

pub(crate) fn request_diagnostic_refresh(app: &mut App) {
    if diagnostic_refresh_is_coalesced(app.diagnostic_refresh_pending, app.diagnostic_refreshing) {
        if app.diagnostic_refreshing {
            app.diagnostic_refresh_pending = true;
        }
        return;
    }
    let Some(window) = app.diagnostic_window else {
        return;
    };
    app.diagnostic_refresh_pending = true;
    unsafe {
        if PostMessageW(window, WM_DIAGNOSTIC_REFRESH, 0, 0) == 0 {
            app.diagnostic_refresh_pending = false;
        }
    }
}

fn finish_diagnostic_refresh(app: &mut App, follow_up_needed: bool) {
    app.diagnostic_refreshing = false;
    crate::logging::flush_diagnostic_events_to_disk(
        &app.diagnostics,
        &mut app.last_persisted_event_sequence,
        &app.log_directory,
    );
    if follow_up_needed {
        app.diagnostic_refresh_follow_up_scheduled = true;
    }
    if app.diagnostic_refresh_pending || follow_up_needed {
        app.diagnostic_refresh_pending = false;
        request_diagnostic_refresh(app);
    }
}

unsafe fn select_all_diagnostic_rows(app: &mut App) {
    let Some(list) = app.diagnostic_list else {
        return;
    };
    let item = ListViewItem {
        mask: LVIF_STATE,
        item: -1,
        subitem: 0,
        state: LVIS_SELECTED,
        state_mask: LVIS_SELECTED,
        text: std::ptr::null_mut(),
        text_maximum: 0,
        image: 0,
        parameter: 0,
        indent: 0,
        group_id: 0,
        columns: 0,
        column_indices: std::ptr::null_mut(),
        column_formats: std::ptr::null_mut(),
        group: 0,
    };
    let _ = SendMessageW(
        list,
        LVM_SETITEMSTATE,
        usize::MAX,
        (&item as *const ListViewItem).cast::<c_void>() as isize,
    );
    update_diagnostic_grid_selection(app);
}

pub(crate) unsafe fn set_list_view_item_selected(list: *mut c_void, index: usize, selected: bool) {
    let state = if selected { LVIS_SELECTED } else { 0 };
    let item = ListViewItem {
        mask: LVIF_STATE,
        item: index as i32,
        subitem: 0,
        state,
        state_mask: LVIS_SELECTED,
        text: std::ptr::null_mut(),
        text_maximum: 0,
        image: 0,
        parameter: 0,
        indent: 0,
        group_id: 0,
        columns: 0,
        column_indices: std::ptr::null_mut(),
        column_formats: std::ptr::null_mut(),
        group: 0,
    };
    let _ = SendMessageW(
        list,
        LVM_SETITEMSTATE,
        index,
        (&item as *const ListViewItem).cast::<c_void>() as isize,
    );
}

unsafe fn handle_diagnostic_notify(app: &mut App, l_param: isize) -> bool {
    if l_param == 0 || app.diagnostic_refreshing {
        return false;
    }
    let header = &*(l_param as *const NotifyHeader);
    match header.code {
        LVN_ITEMCHANGED => {
            let notification = &*(l_param as *const ListViewNotification);
            if notification.changed & LVIF_STATE != 0
                && ((notification.old_state ^ notification.new_state) & LVIS_SELECTED) != 0
            {
                update_diagnostic_grid_selection(app);
            }
            true
        }
        LVN_KEYDOWN => {
            let notification = &*(l_param as *const ListViewKeyDownNotification);
            if GetKeyState(VK_CONTROL) < 0 {
                match notification.virtual_key {
                    key if key == b'C' as u16 => {
                        diagnostic_toolbar_action(app, "copy");
                        return true;
                    }
                    key if key == b'A' as u16 => {
                        select_all_diagnostic_rows(app);
                        return true;
                    }
                    _ => {}
                }
            }
            true
        }
        _ => false,
    }
}
unsafe fn set_diagnostic_control_fonts(app: &mut App) {
    for control in [
        app.diagnostic_hud_state,
        app.diagnostic_summary,
        app.diagnostic_hud_separator,
        app.diagnostic_toolbar_separator,
        app.diagnostic_display_label,
        app.diagnostic_display_input,
        app.diagnostic_show_all_button,
        app.diagnostic_category_filter,
        app.diagnostic_search_label,
        app.diagnostic_search_input,
        app.diagnostic_toolbar_label,
        app.diagnostic_selection_summary,
        app.diagnostic_range_input,
        app.diagnostic_copy_button,
        app.diagnostic_export_button,
        app.diagnostic_export_all_button,
        app.diagnostic_message,
        app.diagnostic_list,
    ]
    .into_iter()
    .flatten()
    {
        set_diagnostic_control_font(control);
    }
    let Some(state) = app.diagnostic_hud_state else {
        return;
    };
    let stock = GetStockObject(DEFAULT_GUI_FONT);
    if stock.is_null() {
        return;
    }
    let mut log_font = LogFontW {
        height: 0,
        width: 0,
        escapement: 0,
        orientation: 0,
        weight: 0,
        italic: 0,
        underline: 0,
        strike_out: 0,
        charset: 0,
        out_precision: 0,
        clip_precision: 0,
        quality: 0,
        pitch_and_family: 0,
        face_name: [0; 32],
    };
    if GetObjectW(
        stock,
        size_of::<LogFontW>() as i32,
        (&mut log_font as *mut LogFontW).cast::<c_void>(),
    ) == 0
    {
        return;
    }
    log_font.height = -scale_logical(14, GetDpiForWindow(state).max(96));
    log_font.weight = 700;
    let font = CreateFontIndirectW(&log_font);
    if font.is_null() {
        return;
    }
    if let Some(previous) = app.diagnostic_state_font.replace(font) {
        let _ = DeleteObject(previous);
    }
    let _ = SendMessageW(state, WM_SETFONT, font as usize, 1);
}

fn diagnostic_presentation_message(message: u32) -> bool {
    matches!(
        message,
        WM_THEMECHANGED | WM_SYSCOLORCHANGE | WM_SETTINGCHANGE
    )
}

unsafe fn refresh_diagnostic_presentation(hwnd: *mut c_void, app: &mut App) {
    set_diagnostic_control_fonts(app);
    let background = GetSysColor(COLOR_WINDOW);
    let text = GetSysColor(COLOR_WINDOWTEXT);
    if let Some(list) = app.diagnostic_list {
        let _ = SendMessageW(list, LVM_SETBKCOLOR, 0, background as isize);
        let _ = SendMessageW(list, LVM_SETTEXTCOLOR, 0, text as isize);
        let _ = SendMessageW(list, LVM_SETTEXTBKCOLOR, 0, background as isize);
    }
    for control in [
        app.diagnostic_hud_state,
        app.diagnostic_summary,
        app.diagnostic_hud_separator,
        app.diagnostic_toolbar_separator,
        app.diagnostic_display_label,
        app.diagnostic_display_input,
        app.diagnostic_show_all_button,
        app.diagnostic_category_filter,
        app.diagnostic_search_label,
        app.diagnostic_search_input,
        app.diagnostic_toolbar_label,
        app.diagnostic_selection_summary,
        app.diagnostic_range_input,
        app.diagnostic_copy_button,
        app.diagnostic_export_button,
        app.diagnostic_export_all_button,
        app.diagnostic_message,
        app.diagnostic_list,
    ]
    .into_iter()
    .flatten()
    {
        let _ = InvalidateRect(control, std::ptr::null(), 1);
    }
    let _ = InvalidateRect(hwnd, std::ptr::null(), 1);
    let _ = UpdateWindow(hwnd);
}

pub unsafe extern "system" fn diagnostic_window_proc(
    hwnd: *mut c_void,
    message: u32,
    w_param: usize,
    l_param: isize,
) -> isize {
    let app = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
    if message == WM_CREATE {
        let create = l_param as *const CreateStruct;
        let app_ptr = app_create_params(create);
        if app_ptr.is_null() {
            return diagnostic_create_failure_result();
        }
        let app = app_ptr as *mut App;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, app_ptr as isize);
        // Edit controls fire EN_CHANGE while they are created and populated.
        // The guard keeps those creation notifications from being treated as
        // user edits until the initial population completes below.
        (*app).diagnostic_controls_initializing = true;
        let static_class = wide("STATIC");
        let hud_state = CreateWindowExW(
            0,
            static_class.as_ptr(),
            wide("Loading").as_ptr(),
            diagnostic_summary_style(),
            0,
            0,
            800,
            scale_logical(DIAGNOSTIC_HUD_STATE_HEIGHT, 96),
            hwnd,
            std::ptr::null_mut(),
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null_mut(),
        );
        let summary = CreateWindowExW(
            0,
            static_class.as_ptr(),
            wide(DIAGNOSTIC_LOADING_SUMMARY).as_ptr(),
            diagnostic_summary_style(),
            0,
            0,
            800,
            scale_logical(DIAGNOSTIC_SUMMARY_HEIGHT, 96),
            hwnd,
            std::ptr::null_mut(),
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null_mut(),
        );
        let summary_error = if summary.is_null() { GetLastError() } else { 0 };
        let hud_separator = CreateWindowExW(
            0,
            static_class.as_ptr(),
            std::ptr::null(),
            WS_CHILD | WS_VISIBLE | SS_ETCHEDHORZ,
            0,
            0,
            800,
            scale_logical(DIAGNOSTIC_SEPARATOR_HEIGHT, 96),
            hwnd,
            std::ptr::null_mut(),
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null_mut(),
        );
        let toolbar_separator = CreateWindowExW(
            0,
            static_class.as_ptr(),
            std::ptr::null(),
            WS_CHILD | WS_VISIBLE | SS_ETCHEDHORZ,
            0,
            0,
            800,
            scale_logical(DIAGNOSTIC_SEPARATOR_HEIGHT, 96),
            hwnd,
            std::ptr::null_mut(),
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null_mut(),
        );
        let display_label = CreateWindowExW(
            0,
            static_class.as_ptr(),
            wide("Show rows:").as_ptr(),
            WS_CHILD | WS_VISIBLE | SS_LEFT,
            0,
            0,
            DIAGNOSTIC_TOOLBAR_CONTROL_WIDTHS[0],
            24,
            hwnd,
            ID_DIAGNOSTIC_DISPLAY_LIMIT as *mut c_void,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null_mut(),
        );
        let display_input = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            wide("EDIT").as_ptr(),
            wide("100").as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL,
            0,
            0,
            DIAGNOSTIC_TOOLBAR_CONTROL_WIDTHS[1],
            24,
            hwnd,
            ID_DIAGNOSTIC_DISPLAY_LIMIT as *mut c_void,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null_mut(),
        );
        if !display_input.is_null() {
            SendMessageW(display_input, EM_LIMITTEXT, DIAGNOSTIC_RANGE_INPUT_LIMIT, 0);
        }
        let show_all_button = CreateWindowExW(
            0,
            wide("BUTTON").as_ptr(),
            wide("Show all").as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON,
            0,
            0,
            DIAGNOSTIC_TOOLBAR_CONTROL_WIDTHS[2],
            24,
            hwnd,
            ID_DIAGNOSTIC_SHOW_ALL as *mut c_void,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null_mut(),
        );
        let category_combo = CreateWindowExW(
            0,
            wide("COMBOBOX").as_ptr(),
            std::ptr::null(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | CBS_DROPDOWNLIST,
            0,
            0,
            DIAGNOSTIC_TOOLBAR_CONTROL_WIDTHS[3],
            200,
            hwnd,
            ID_DIAGNOSTIC_CATEGORY_FILTER as *mut c_void,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null_mut(),
        );
        if !category_combo.is_null() {
            for item in [
                "All categories",
                "Startup",
                "Timer",
                "Power",
                "UI",
                "Schedule",
                "System",
            ] {
                let item_wide = wide(item);
                let _ = SendMessageW(category_combo, CB_ADDSTRING, 0, item_wide.as_ptr() as isize);
            }
            let _ = SendMessageW(category_combo, CB_SETCURSEL, 0, 0);
        }
        let search_label = CreateWindowExW(
            0,
            static_class.as_ptr(),
            wide("|  Search:").as_ptr(),
            WS_CHILD | WS_VISIBLE | SS_LEFT,
            0,
            0,
            DIAGNOSTIC_TOOLBAR_CONTROL_WIDTHS[4],
            24,
            hwnd,
            std::ptr::null_mut(),
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null_mut(),
        );
        let search_input = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            wide("EDIT").as_ptr(),
            std::ptr::null(),
            WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL,
            0,
            0,
            DIAGNOSTIC_TOOLBAR_CONTROL_WIDTHS[5],
            24,
            hwnd,
            ID_DIAGNOSTIC_SEARCH as *mut c_void,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null_mut(),
        );
        if !search_input.is_null() {
            SendMessageW(search_input, EM_LIMITTEXT, DIAGNOSTIC_RANGE_INPUT_LIMIT, 0);
        }
        let label = CreateWindowExW(
            0,
            static_class.as_ptr(),
            wide("|  Rows to copy/export:").as_ptr(),
            WS_CHILD | WS_VISIBLE | SS_LEFT,
            0,
            0,
            DIAGNOSTIC_TOOLBAR_CONTROL_WIDTHS[6],
            24,
            hwnd,
            ID_DIAGNOSTIC_RANGE as *mut c_void,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null_mut(),
        );
        let selection_summary = CreateWindowExW(
            0,
            static_class.as_ptr(),
            wide("All rows 0").as_ptr(),
            WS_CHILD | WS_VISIBLE | SS_LEFT,
            0,
            0,
            DIAGNOSTIC_TOOLBAR_CONTROL_WIDTHS[8],
            24,
            hwnd,
            std::ptr::null_mut(),
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null_mut(),
        );
        let range_class = wide("EDIT");
        let range_input = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            range_class.as_ptr(),
            std::ptr::null(),
            WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL,
            0,
            0,
            DIAGNOSTIC_TOOLBAR_CONTROL_WIDTHS[7],
            24,
            hwnd,
            ID_DIAGNOSTIC_RANGE as *mut c_void,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null_mut(),
        );
        if !range_input.is_null() {
            SendMessageW(range_input, EM_LIMITTEXT, DIAGNOSTIC_RANGE_INPUT_LIMIT, 0);
        }
        let button_class = wide("BUTTON");
        let copy_button = CreateWindowExW(
            0,
            button_class.as_ptr(),
            wide("Copy").as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON,
            0,
            0,
            DIAGNOSTIC_TOOLBAR_CONTROL_WIDTHS[9],
            24,
            hwnd,
            ID_DIAGNOSTIC_COPY as *mut c_void,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null_mut(),
        );
        let export_button = CreateWindowExW(
            0,
            button_class.as_ptr(),
            wide("Export").as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON,
            0,
            0,
            DIAGNOSTIC_TOOLBAR_CONTROL_WIDTHS[10],
            24,
            hwnd,
            ID_DIAGNOSTIC_EXPORT as *mut c_void,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null_mut(),
        );
        let export_all_button = CreateWindowExW(
            0,
            button_class.as_ptr(),
            wide("Export all").as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON,
            0,
            0,
            DIAGNOSTIC_TOOLBAR_CONTROL_WIDTHS[11],
            24,
            hwnd,
            ID_DIAGNOSTIC_EXPORT_ALL as *mut c_void,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null_mut(),
        );
        let message = CreateWindowExW(
            0,
            static_class.as_ptr(),
            std::ptr::null(),
            WS_CHILD | WS_VISIBLE | SS_LEFT,
            0,
            0,
            480,
            24,
            hwnd,
            std::ptr::null_mut(),
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null_mut(),
        );
        let list_class = wide("SysListView32");
        let list = CreateWindowExW(
            0,
            list_class.as_ptr(),
            std::ptr::null(),
            diagnostic_list_style(),
            0,
            DIAGNOSTIC_SUMMARY_HEIGHT
                + DIAGNOSTIC_SEPARATOR_HEIGHT
                + DIAGNOSTIC_TOOLBAR_HEIGHT
                + DIAGNOSTIC_SEPARATOR_HEIGHT,
            800,
            400,
            hwnd,
            std::ptr::null_mut(),
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null_mut(),
        );
        let control_error = |control: *mut c_void| {
            if control.is_null() {
                GetLastError()
            } else {
                0
            }
        };
        let hud_state_error = control_error(hud_state);
        let hud_separator_error = control_error(hud_separator);
        let toolbar_separator_error = control_error(toolbar_separator);
        let display_label_error = control_error(display_label);
        let display_input_error = control_error(display_input);
        let show_all_error = control_error(show_all_button);
        let category_error = control_error(category_combo);
        let search_label_error = control_error(search_label);
        let search_input_error = control_error(search_input);
        let label_error = control_error(label);
        let selection_summary_error = control_error(selection_summary);
        let range_error = control_error(range_input);
        let copy_error = control_error(copy_button);
        let export_error = control_error(export_button);
        let export_all_error = control_error(export_all_button);
        let message_error = control_error(message);
        let list_error = control_error(list);
        if hud_state.is_null()
            || summary.is_null()
            || hud_separator.is_null()
            || toolbar_separator.is_null()
            || display_label.is_null()
            || display_input.is_null()
            || show_all_button.is_null()
            || category_combo.is_null()
            || search_label.is_null()
            || search_input.is_null()
            || label.is_null()
            || selection_summary.is_null()
            || range_input.is_null()
            || copy_button.is_null()
            || export_button.is_null()
            || export_all_button.is_null()
            || message.is_null()
            || list.is_null()
        {
            if !app_ptr.is_null() {
                (*(app_ptr as *mut App)).diagnostics.record(
                    "native.CreateWindowExW.diagnostic_control.error",
                    format!(
                        "hud_state_null={} hud_state_raw_status={} summary_null={} summary_raw_status={} hud_separator_null={} hud_separator_raw_status={} toolbar_separator_null={} toolbar_separator_raw_status={} display_label_null={} display_label_raw_status={} display_input_null={} display_input_raw_status={} show_all_null={} show_all_raw_status={} category_null={} category_raw_status={} search_label_null={} search_label_raw_status={} search_input_null={} search_input_raw_status={} label_null={} label_raw_status={} selection_summary_null={} selection_summary_raw_status={} range_null={} range_raw_status={} copy_null={} copy_raw_status={} export_null={} export_raw_status={} export_all_null={} export_all_raw_status={} message_null={} message_raw_status={} list_null={} list_raw_status={}",
                        hud_state.is_null(),
                        hud_state_error,
                        summary.is_null(),
                        summary_error,
                        hud_separator.is_null(),
                        hud_separator_error,
                        toolbar_separator.is_null(),
                        toolbar_separator_error,
                        display_label.is_null(),
                        display_label_error,
                        display_input.is_null(),
                        display_input_error,
                        show_all_button.is_null(),
                        show_all_error,
                        category_combo.is_null(),
                        category_error,
                        search_label.is_null(),
                        search_label_error,
                        search_input.is_null(),
                        search_input_error,
                        label.is_null(),
                        label_error,
                        selection_summary.is_null(),
                        selection_summary_error,
                        range_input.is_null(),
                        range_error,
                        copy_button.is_null(),
                        copy_error,
                        export_button.is_null(),
                        export_error,
                        export_all_button.is_null(),
                        export_all_error,
                        message.is_null(),
                        message_error,
                        list.is_null(),
                        list_error
                    ),
                );
            }
            destroy_created_diagnostic_controls(&[
                hud_state,
                summary,
                hud_separator,
                toolbar_separator,
                display_label,
                display_input,
                show_all_button,
                category_combo,
                search_label,
                search_input,
                label,
                selection_summary,
                range_input,
                copy_button,
                export_button,
                export_all_button,
                message,
                list,
            ]);
            return diagnostic_create_failure_result();
        }
        for control in [
            hud_state,
            summary,
            hud_separator,
            toolbar_separator,
            display_label,
            display_input,
            show_all_button,
            category_combo,
            search_label,
            search_input,
            label,
            selection_summary,
            range_input,
            copy_button,
            export_button,
            export_all_button,
            message,
            list,
        ] {
            set_diagnostic_control_font(control);
        }
        if let Err(raw_error) = initialize_diagnostic_list(list) {
            if !app_ptr.is_null() {
                (*(app_ptr as *mut App)).diagnostics.record(
                    "native.ListView.insert_column.error",
                    format!("raw_status={raw_error}"),
                );
            }
            destroy_created_diagnostic_controls(&[
                hud_state,
                summary,
                hud_separator,
                toolbar_separator,
                display_label,
                display_input,
                show_all_button,
                category_combo,
                search_label,
                search_input,
                label,
                selection_summary,
                range_input,
                copy_button,
                export_button,
                export_all_button,
                message,
                list,
            ]);
            return diagnostic_create_failure_result();
        }
        {
            (*app).diagnostic_hud_state = Some(hud_state);
            (*app).diagnostic_summary = Some(summary);
            (*app).diagnostic_hud_separator = Some(hud_separator);
            (*app).diagnostic_toolbar_separator = Some(toolbar_separator);
            (*app).diagnostic_display_label = Some(display_label);
            (*app).diagnostic_display_input = Some(display_input);
            (*app).diagnostic_show_all_button = Some(show_all_button);
            (*app).diagnostic_category_filter = Some(category_combo);
            (*app).diagnostic_search_label = Some(search_label);
            (*app).diagnostic_search_input = Some(search_input);
            (*app).diagnostic_toolbar_label = Some(label);
            (*app).diagnostic_selection_summary = Some(selection_summary);
            (*app).diagnostic_range_input = Some(range_input);
            (*app).diagnostic_copy_button = Some(copy_button);
            (*app).diagnostic_export_button = Some(export_button);
            (*app).diagnostic_export_all_button = Some(export_all_button);
            (*app).diagnostic_message = Some(message);
            (*app).diagnostic_list = Some(list);
            SetWindowLongPtrW(list, GWLP_USERDATA, app_ptr as isize);
            let prev_proc = SetWindowLongPtrW(
                list,
                GWLP_WNDPROC,
                crate::ui::marquee::diagnostic_list_proc as *const () as usize as isize,
            );
            if prev_proc != 0 {
                let prev_proc_fn: unsafe extern "system" fn(
                    *mut c_void,
                    u32,
                    usize,
                    isize,
                ) -> isize = std::mem::transmute(prev_proc as *const ());
                (*app).diagnostic_list_prev_proc = Some(prev_proc_fn);
            }
            refresh_diagnostic_presentation(hwnd, &mut *app);
            let _ = layout_diagnostic_controls(hwnd, &*app);
            (*app).diagnostic_controls_initializing = false;
        }
        return 0;
    }
    if !app.is_null() {
        if diagnostic_presentation_message(message) {
            refresh_diagnostic_presentation(hwnd, &mut *app);
            return 0;
        } else if message == WM_ERASEBKGND {
            let brush = GetSysColorBrush(COLOR_WINDOW);
            let mut rect = Rect {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            if !brush.is_null() && GetClientRect(hwnd, &mut rect) != 0 {
                let _ = FillRect(w_param as *mut c_void, &rect, brush);
            }
            return 1;
        } else if message == WM_PAINT {
            let mut paint = PaintStruct {
                hdc: std::ptr::null_mut(),
                erase: 0,
                paint: Rect {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                },
                restore: 0,
                inc_update: 0,
                reserved: [0; 32],
            };
            let hdc = BeginPaint(hwnd, &mut paint);
            if !hdc.is_null() {
                let brush = GetSysColorBrush(COLOR_WINDOW);
                let _ = FillRect(hdc, &paint.paint, brush);
            }
            let _ = EndPaint(hwnd, &paint);
            return 0;
        } else if message == WM_CTLCOLOREDIT || message == WM_CTLCOLORSTATIC {
            let hdc = w_param as *mut c_void;
            let brush = GetSysColorBrush(COLOR_WINDOW);
            let _ = SetTextColor(hdc, GetSysColor(COLOR_WINDOWTEXT));
            let _ = SetBkColor(hdc, GetSysColor(COLOR_WINDOW));
            return brush as isize;
        } else if message == WM_NOTIFY {
            if handle_diagnostic_notify(&mut *app, l_param) {
                return 0;
            }
        } else if message == WM_COMMAND {
            let command = w_param & 0xffff;
            let notification = (w_param >> 16) & 0xffff;
            let edit_notification = notification == EN_CHANGE
                && matches!(
                    command,
                    ID_DIAGNOSTIC_DISPLAY_LIMIT | ID_DIAGNOSTIC_SEARCH | ID_DIAGNOSTIC_RANGE
                );
            if edit_notification && (*app).diagnostic_controls_initializing {
                return 0;
            }
            if command == ID_DIAGNOSTIC_DISPLAY_LIMIT && notification == EN_CHANGE {
                diagnostic_display_changed(&mut *app);
                return 0;
            }
            if command == ID_DIAGNOSTIC_SHOW_ALL && notification == BN_CLICKED {
                diagnostic_show_all(&mut *app);
                return 0;
            }
            if command == ID_DIAGNOSTIC_CATEGORY_FILTER && notification == CBN_SELCHANGE {
                diagnostic_category_filter_changed(&mut *app);
                return 0;
            }
            if command == ID_DIAGNOSTIC_SEARCH && notification == EN_CHANGE {
                diagnostic_search_changed(&mut *app);
                return 0;
            }
            if command == ID_DIAGNOSTIC_RANGE && notification == EN_CHANGE {
                diagnostic_range_changed(&mut *app);
                return 0;
            }
            if command == ID_DIAGNOSTIC_COPY && notification == BN_CLICKED {
                diagnostic_toolbar_action(&mut *app, "copy");
                return 0;
            }
            if command == ID_DIAGNOSTIC_EXPORT && notification == BN_CLICKED {
                diagnostic_toolbar_action(&mut *app, "export");
                return 0;
            }
            if command == ID_DIAGNOSTIC_EXPORT_ALL && notification == BN_CLICKED {
                diagnostic_toolbar_action(&mut *app, "export_all");
                return 0;
            }
        } else if message == WM_DIAGNOSTIC_REFRESH {
            (*app).diagnostic_refresh_pending = false;
            refresh_diagnostic_window(hwnd, &mut *app, true);
            return 0;
        } else if message == WM_DPICHANGED {
            let suggested = l_param as *const Rect;
            if suggested.is_null() {
                (*app).diagnostics.record(
                    "diagnostic.dpi.error",
                    format!("stage=suggested_rect_missing dpi={}", w_param & 0xffff),
                );
            } else {
                let width = ((*suggested).right - (*suggested).left).max(0);
                let height = ((*suggested).bottom - (*suggested).top).max(0);
                if SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    (*suggested).left,
                    (*suggested).top,
                    width,
                    height,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                ) == 0
                {
                    (*app).diagnostics.record(
                        "diagnostic.dpi.error",
                        format!("stage=SetWindowPos raw_status={}", GetLastError()),
                    );
                }
            }
            set_diagnostic_control_fonts(&mut *app);
            let _ = layout_diagnostic_controls(hwnd, &*app);
            return 0;
        } else if message == WM_SIZE {
            let _ = layout_diagnostic_controls(hwnd, &*app);
            return 0;
        } else if message == WM_GETMINMAXINFO {
            let limits = l_param as *mut MinMaxInfo;
            if !limits.is_null() {
                let dpi = GetDpiForWindow(hwnd).max(96);
                let client_width = diagnostic_toolbar_min_width(dpi);
                let client_height = diagnostic_min_client_height(dpi);
                let outer = diagnostic_min_outer_size(hwnd, client_width, client_height, dpi);
                (*limits).minimum_track_size.x = outer.x;
                (*limits).minimum_track_size.y = outer.y;
            }
            return 0;
        } else if message == WM_CLOSE {
            DestroyWindow(hwnd);
            return 0;
        } else if message == WM_SETFOCUS {
            request_diagnostic_refresh(&mut *app);
        } else if message == WM_NCDESTROY {
            clear_diagnostic_state(&mut *app);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        }
    }
    DefWindowProcW(hwnd, message, w_param, l_param)
}
#[repr(C)]
struct MinMaxInfo {
    reserved: Point,
    maximum_size: Point,
    maximum_position: Point,
    minimum_track_size: Point,
    maximum_track_size: Point,
}

#[link(name = "user32")]
extern "system" {
    fn IsWindowVisible(hwnd: *mut c_void) -> i32;
    fn PostMessageW(hwnd: *mut c_void, message: u32, w: usize, l: isize) -> i32;
    fn EnableWindow(window: *mut c_void, enable: i32) -> i32;
    fn AdjustWindowRectEx(rect: *mut Rect, style: u32, menu: i32, ex_style: u32) -> i32;
    fn AdjustWindowRectExForDpi(
        rect: *mut Rect,
        style: u32,
        menu: i32,
        ex_style: u32,
        dpi: u32,
    ) -> i32;
    fn GetScrollPos(window: *mut c_void, bar: i32) -> i32;
    fn SetScrollPos(window: *mut c_void, bar: i32, position: i32, redraw: i32) -> i32;
    fn OpenClipboard(owner: *mut c_void) -> i32;
    fn EmptyClipboard() -> i32;
    fn SetClipboardData(format: u32, data: *mut c_void) -> *mut c_void;
    fn CloseClipboard() -> i32;
}

#[link(name = "gdi32")]
extern "system" {
    fn GetObjectW(object: *mut c_void, count: i32, object_data: *mut c_void) -> i32;
    fn CreateFontIndirectW(log_font: *const LogFontW) -> *mut c_void;
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
fn diagnostic_layout_guard_allows(rect: DiagnosticLayoutRect, is_window: bool) -> bool {
    is_window && rect.width > 0 && rect.height > 0
}

#[cfg(test)]
fn diagnostic_layout_stops_on_first_failure(position_ok: &[bool]) -> (usize, bool) {
    let mut applied = 0usize;
    for ok in position_ok {
        if !ok {
            return (applied, false);
        }
        applied += 1;
    }
    (applied, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tray::WM_TIMER;
    use tick_diagnostics::{DiagnosticStore, DEFAULT_MAX_EVENTS};

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
    fn display_limit_input_decision_preserves_blank_and_rejects_malformed() {
        assert_eq!(
            display_limit_input_decision(""),
            DisplayLimitInputDecision::PreserveEmpty
        );
        assert_eq!(
            display_limit_input_decision("   "),
            DisplayLimitInputDecision::PreserveEmpty
        );
        assert_eq!(
            display_limit_input_decision("\t\r\n"),
            DisplayLimitInputDecision::PreserveEmpty
        );
        assert_eq!(
            display_limit_input_decision("abc"),
            DisplayLimitInputDecision::Invalid
        );
        assert_eq!(
            display_limit_input_decision("12.5"),
            DisplayLimitInputDecision::Invalid
        );
        assert_eq!(
            display_limit_input_decision("-1"),
            DisplayLimitInputDecision::Invalid
        );
        assert_eq!(
            display_limit_input_decision("0"),
            DisplayLimitInputDecision::Invalid
        );
        assert_eq!(
            display_limit_input_decision("all"),
            DisplayLimitInputDecision::Invalid
        );
        assert_eq!(
            display_limit_input_decision("250"),
            DisplayLimitInputDecision::Apply(250)
        );
        assert_eq!(
            display_limit_input_decision(" 200 "),
            DisplayLimitInputDecision::Apply(200)
        );
    }

    #[test]
    fn diagnostic_toolbar_contract_uses_explicit_controls_and_tsv_events() {
        assert_ne!(ID_DIAGNOSTIC_RANGE, ID_DIAGNOSTIC_COPY);
        assert_ne!(ID_DIAGNOSTIC_COPY, ID_DIAGNOSTIC_EXPORT);
        assert_ne!(ID_DIAGNOSTIC_EXPORT, ID_DIAGNOSTIC_EXPORT_ALL);
        assert_ne!(ID_DIAGNOSTIC_DISPLAY_LIMIT, ID_DIAGNOSTIC_RANGE);
        assert_ne!(ID_DIAGNOSTIC_SHOW_ALL, ID_DIAGNOSTIC_RANGE);
        assert_ne!(ID_DIAGNOSTIC_CATEGORY_FILTER, ID_DIAGNOSTIC_RANGE);
        assert_ne!(ID_DIAGNOSTIC_SEARCH, ID_DIAGNOSTIC_RANGE);
        assert_ne!(ID_DIAGNOSTIC_SEARCH, ID_DIAGNOSTIC_EXPORT_ALL);
        assert_eq!(DIAGNOSTIC_RANGE_INPUT_LIMIT, 64);
        assert_eq!(DIAGNOSTIC_TOOLBAR_HEIGHT, 36);
        let selection = RowSelection::new(12, 24);
        assert_eq!(
            diagnostic_range_details(selection, 32),
            "selected_range=12-24 retained_rows=32 row_count=13 format=TSV"
        );
        for name in [
            "diagnostic.display_limit.changed",
            "diagnostic.display_limit.override",
            "diagnostic.display_limit.validation",
            "diagnostic.category_filter.changed",
            "diagnostic.search.changed",
            "diagnostic.range.parsed",
            "diagnostic.copy.request",
            "diagnostic.copy.result",
            "diagnostic.export.request",
            "diagnostic.export.result",
            "diagnostic.export.cancelled",
            "diagnostic.validation_failure",
            "native.clipboard.error",
            "native.GetSaveFileNameW.error",
            "native.export.file.error",
        ] {
            assert!(!name.is_empty());
        }
    }

    #[test]
    fn logs_opening_maps_native_open_failure_to_a_terminal_failed_outcome() {
        assert_eq!(
            diagnostic_open_operation_outcome(true),
            DiagnosticOutcome::Completed
        );
        assert_eq!(
            diagnostic_open_operation_outcome(false),
            DiagnosticOutcome::Failed
        );
        assert_eq!(DiagnosticPhase::Begin.to_string(), "Begin");
        assert_eq!(DiagnosticOutcome::InProgress.to_string(), "InProgress");
        assert_eq!(DiagnosticPhase::Complete.to_string(), "Complete");
    }

    #[test]
    fn diagnostic_theme_contract_refreshes_only_presentation_messages() {
        for message in [WM_THEMECHANGED, WM_SYSCOLORCHANGE, WM_SETTINGCHANGE] {
            assert!(diagnostic_presentation_message(message));
        }
        for message in [WM_SIZE, WM_DPICHANGED, WM_TIMER, WM_COMMAND] {
            assert!(!diagnostic_presentation_message(message));
        }
        assert_eq!(LVM_SETBKCOLOR, LVM_FIRST + 1);
        assert_eq!(LVM_SETTEXTCOLOR, LVM_FIRST + 36);
        assert_eq!(LVM_SETTEXTBKCOLOR, LVM_FIRST + 38);
    }

    #[test]
    fn diagnostic_summary_is_a_non_scrolling_read_only_static_contract() {
        let style = diagnostic_summary_style();
        assert_ne!(style & WS_CHILD, 0);
        assert_ne!(style & WS_VISIBLE, 0);
        assert_ne!(style & SS_NOPREFIX, 0);
        assert_eq!(style & (0x0020_0000 | 0x0040 | 0x0004), 0);
        assert_eq!(DIAGNOSTIC_SUMMARY_HEIGHT, 140);
    }

    #[test]
    fn diagnostic_refresh_follow_up_is_bounded_to_one_extra_pass() {
        assert!(diagnostic_follow_up_needed(true, false));
        assert!(!diagnostic_follow_up_needed(true, true));
        assert!(!diagnostic_follow_up_needed(false, false));
    }

    #[test]
    fn diagnostic_create_failure_returns_documented_failure_value() {
        assert_eq!(diagnostic_create_failure_result(), -1);
    }

    #[test]
    fn diagnostic_layout_guards_and_stop_on_first_failure() {
        let good = DiagnosticLayoutRect {
            left: 0,
            top: 0,
            width: 10,
            height: 10,
        };
        let flat = DiagnosticLayoutRect {
            left: 0,
            top: 0,
            width: 0,
            height: 10,
        };
        assert!(diagnostic_layout_guard_allows(good, true));
        assert!(!diagnostic_layout_guard_allows(good, false));
        assert!(!diagnostic_layout_guard_allows(flat, true));
        assert_eq!(
            diagnostic_layout_stops_on_first_failure(&[true, true, true]),
            (3, true)
        );
        assert_eq!(
            diagnostic_layout_stops_on_first_failure(&[true, false, true]),
            (1, false)
        );
        assert_eq!(
            diagnostic_layout_stops_on_first_failure(&[false]),
            (0, false)
        );
        assert_eq!(diagnostic_layout_stops_on_first_failure(&[]), (0, true));
    }
    #[test]
    fn diagnostic_toolbar_layout_is_one_row_and_derived_from_flow() {
        let layout = diagnostic_layout(1_190, 480, 96);
        let toolbar = layout.toolbar;
        let controls = [
            toolbar.display_label,
            toolbar.display_input,
            toolbar.show_all,
            toolbar.category_filter,
            toolbar.search_label,
            toolbar.search_input,
            toolbar.transfer_label,
            toolbar.range_input,
            toolbar.selection_summary,
            toolbar.copy,
            toolbar.export,
            toolbar.export_all,
            toolbar.message,
        ];
        assert!(controls
            .windows(2)
            .all(|pair| pair[0].right() <= pair[1].left));
        assert!(controls
            .iter()
            .all(|control| control.top == toolbar.display_input.top
                && control.height == toolbar.display_input.height));
        assert_eq!(
            toolbar.display_input.left - toolbar.display_label.right(),
            8
        );
        assert_eq!(toolbar.message.width, 140);
        assert_eq!(diagnostic_toolbar_min_width(96), 1_190);
        assert!(toolbar.message.width >= DIAGNOSTIC_TOOLBAR_MIN_MESSAGE_WIDTH);
        let tight = diagnostic_toolbar_layout(400, 0, 36, 96);
        assert_eq!(tight.message.width, DIAGNOSTIC_TOOLBAR_MIN_MESSAGE_WIDTH);
        let tighter = diagnostic_toolbar_layout(0, 0, 36, 96);
        assert_eq!(tighter.message.width, DIAGNOSTIC_TOOLBAR_MIN_MESSAGE_WIDTH);
    }

    #[test]
    fn compact_default_outer_size_keeps_one_row_toolbar_and_grid_usable() {
        assert_eq!(
            (DIAGNOSTIC_DEFAULT_WIDTH, DIAGNOSTIC_DEFAULT_HEIGHT),
            (1_280, 520)
        );
        let frame = Rect {
            left: -8,
            top: -31,
            right: 1_024,
            bottom: 249,
        };
        let minimum = outer_size_from_client(1_190, 240, frame);
        assert_eq!(minimum, Point { x: 1_032, y: 280 });
        assert!(DIAGNOSTIC_DEFAULT_WIDTH >= diagnostic_toolbar_min_width(96));
        assert!(DIAGNOSTIC_DEFAULT_WIDTH >= minimum.x);
        assert!(DIAGNOSTIC_DEFAULT_HEIGHT >= minimum.y);
        let layout = diagnostic_layout(1_280, 480, 96);
        assert!(layout.toolbar.message.width >= DIAGNOSTIC_TOOLBAR_MIN_MESSAGE_WIDTH);
        assert!(layout.list.height >= DIAGNOSTIC_GRID_MIN_HEIGHT);
        assert!(layout.toolbar.display_input.top == layout.toolbar.export.top);
    }

    #[test]
    fn diagnostic_minimum_size_and_dpi_scaling_keep_summary_and_grid_visible() {
        assert_eq!(diagnostic_min_client_height(96), 276);
        assert_eq!(diagnostic_toolbar_min_width(144), 1_785);
        assert_eq!(diagnostic_min_client_height(144), 414);
        let layout = diagnostic_layout(1_600, 414, 144);
        assert_eq!(
            layout.hud_state.height,
            scale_logical(DIAGNOSTIC_HUD_STATE_HEIGHT, 144)
        );
        assert_eq!(
            layout.hud_separator.top,
            scale_logical(DIAGNOSTIC_SUMMARY_HEIGHT, 144)
        );
        assert_eq!(
            layout.list.height,
            scale_logical(DIAGNOSTIC_GRID_MIN_HEIGHT, 144)
        );
        assert!(layout.summary.bottom() <= layout.toolbar.display_input.top);
        assert!(layout.toolbar.display_input.bottom() <= layout.list.top);
        assert_eq!(
            layout.hud_separator.height,
            scale_logical(DIAGNOSTIC_SEPARATOR_HEIGHT, 144)
        );
        assert_eq!(
            layout.toolbar_separator.height,
            scale_logical(DIAGNOSTIC_SEPARATOR_HEIGHT, 144)
        );
    }

    #[test]
    fn minimum_tracking_size_adds_the_native_frame_to_the_client_contract() {
        let frame = Rect {
            left: -8,
            top: -31,
            right: 838,
            bottom: 249,
        };
        assert_eq!(
            outer_size_from_client(830, 240, frame),
            Point { x: 846, y: 280 }
        );
    }

    #[test]
    fn hud_field_formatting_keeps_key_values_and_snapshot_evidence_visible() {
        let text = diagnostic_hud_field_text(DiagnosticHudFields {
            effective: "0.4966 ms (4966 HNS)",
            ownership: "True\u{2122} Tick",
            power: "AC",
            startup: "On",
            running_duration: "4m 12s",
            next_action: "None",
            history: "retained_events=12 retention_cap=512 truncated=true",
            visible: 4,
            retained: 12,
            chain_status: "OK",
            slot: "A",
            root: "C:\\Portable\\TrueTick",
        });
        for field in [
            "Effective: 0.4966 ms (4966 HNS)",
            "Ownership: True\u{2122} Tick",
            "Power: AC",
            "Startup: On",
            "Running: 4m 12s",
            "Next: None",
            "History: retained_events=12 retention_cap=512 truncated=true",
            "Showing 4 of 12 retained rows",
            "Chain: OK",
            "Slot: A",
            "Root: C:\\Portable\\TrueTick",
            "True\u{2122} Tick v",
            "by kaiiuen",
            GITHUB_URL,
        ] {
            assert!(text.contains(field), "missing HUD field: {field}");
        }
        assert!(text.contains(env!("CARGO_PKG_VERSION")));
        assert_eq!(text.lines().count(), 5);
    }

    #[test]
    fn hud_state_and_separators_are_native_control_contracts() {
        assert_ne!(diagnostic_summary_style() & SS_NOPREFIX, 0);
        assert_ne!(WS_CHILD | WS_VISIBLE | SS_ETCHEDHORZ, 0);
        assert_eq!(DIAGNOSTIC_HUD_STATE_HEIGHT, 32);
        assert_eq!(DIAGNOSTIC_SEPARATOR_HEIGHT, 2);
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
    fn diagnostic_grid_style_supports_visible_multi_row_drag_selection() {
        let style = diagnostic_list_style();
        let extended_style = diagnostic_list_extended_style();
        assert_ne!(style & LVS_REPORT, 0);
        assert_eq!(style & LVS_SINGLESEL, 0);
        assert_ne!(style & LVS_SHOWSELALWAYS, 0);
        assert_ne!(extended_style & LVS_EX_GRIDLINES, 0);
        assert_ne!(extended_style & LVS_EX_FULLROWSELECT, 0);
        assert_ne!(extended_style & LVS_EX_DOUBLEBUFFER, 0);
        assert_eq!(LVN_ITEMCHANGED, -101);
        assert_eq!(LVN_KEYDOWN, -155);
        assert_eq!(WM_NOTIFY, 0x004E);
        assert!(
            size_of::<NotifyHeader>()
                >= size_of::<*mut c_void>() + size_of::<usize>() + size_of::<i32>()
        );
        assert!(
            size_of::<ListViewNotification>()
                >= size_of::<NotifyHeader>()
                    + 3 * size_of::<i32>()
                    + size_of::<Point>()
                    + size_of::<isize>()
        );
        assert!(
            size_of::<ListViewKeyDownNotification>()
                >= size_of::<NotifyHeader>() + size_of::<u16>() + size_of::<u32>()
        );
    }
    #[test]
    fn diagnostic_grid_message_contract_uses_correct_native_messages_and_bounded_counts() {
        assert_eq!(LVM_INSERTITEMW, LVM_FIRST + 77);
        assert_eq!(LVM_SETITEMTEXTW, LVM_FIRST + 116);
        assert_eq!(
            diagnostic_grid_refresh_message(12, 11, 11, 1, 2),
            "snapshot_rows=12 inserted_rows=11 item_count=11 insert_failures=1 set_text_failures=2"
        );
    }
    #[test]
    fn diagnostic_window_contract_is_normal_and_taskbar_visible() {
        assert_eq!(DIAGNOSTIC_WINDOW_TITLE, "True™ Tick Status and Diagnostics");
        assert_eq!(
            diagnostic_window_style() & WS_OVERLAPPEDWINDOW,
            WS_OVERLAPPEDWINDOW
        );
        assert_eq!(diagnostic_window_style() & WS_CHILD, 0);
        assert_eq!(diagnostic_window_style() & WS_VISIBLE, 0);
        assert!(DIAGNOSTIC_WINDOW_PARENT.is_null());
        assert_ne!(diagnostic_window_extended_style() & WS_EX_APPWINDOW, 0);
        assert_eq!(diagnostic_window_extended_style() & WS_EX_TOOLWINDOW, 0);
        assert_eq!(diagnostic_toolbar_min_width(96), 1_190);
        assert_eq!(diagnostic_min_client_height(96), 276);
        assert_eq!(DIAGNOSTIC_TOOLBAR_HEIGHT, 36);
    }
    #[test]
    fn diagnostic_initialization_phase_renders_real_data_before_first_show() {
        assert!(diagnostic_loading_summary_is_nonblank());
        assert!(!diagnostic_summary_is_non_loading(
            DIAGNOSTIC_LOADING_SUMMARY
        ));
        assert!(diagnostic_summary_is_non_loading(
            "Effective: Unknown  |  History: retained_events=4"
        ));
        assert_eq!(diagnostic_window_style() & WS_VISIBLE, 0);
        assert_eq!(WM_DIAGNOSTIC_REFRESH, WM_APP + 2);
    }

    #[test]
    fn diagnostic_manifest_declares_per_monitor_v2_for_windows_10_and_11() {
        let manifest = include_str!("../../windows/true-tick.manifest");
        assert!(manifest.contains("supportedOS Id=\"{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}\""));
        assert!(manifest.contains(">true/pm</dpiAware>"));
        assert!(manifest.contains(">PerMonitorV2</dpiAwareness>"));
        assert!(manifest.contains("level=\"asInvoker\""));
    }

    #[test]
    fn diagnostic_child_handles_are_assigned_before_parent_is_shown() {
        let source = include_str!("diagnostic_window.rs");
        let create_start = source
            .find("unsafe extern \"system\" fn diagnostic_window_proc(")
            .expect("diagnostic window procedure must exist");
        let create_source = &source[create_start..];
        let columns_initialized = create_source
            .find("initialize_diagnostic_list(list)")
            .expect("diagnostic list columns must initialize during creation");
        let assignments_start = create_source
            .find("(*app).diagnostic_hud_state = Some(hud_state);")
            .expect("diagnostic child assignment block must exist");
        assert!(columns_initialized < assignments_start);
        let render_start = create_source[assignments_start..]
            .find("refresh_diagnostic_presentation(hwnd, &mut *app);")
            .map(|offset| assignments_start + offset)
            .expect("diagnostic presentation must follow child assignment");
        for assignment in [
            "(*app).diagnostic_hud_state = Some(hud_state);",
            "(*app).diagnostic_summary = Some(summary);",
            "(*app).diagnostic_hud_separator = Some(hud_separator);",
            "(*app).diagnostic_toolbar_separator = Some(toolbar_separator);",
            "(*app).diagnostic_display_label = Some(display_label);",
            "(*app).diagnostic_display_input = Some(display_input);",
            "(*app).diagnostic_show_all_button = Some(show_all_button);",
            "(*app).diagnostic_category_filter = Some(category_combo);",
            "(*app).diagnostic_search_label = Some(search_label);",
            "(*app).diagnostic_search_input = Some(search_input);",
            "(*app).diagnostic_toolbar_label = Some(label);",
            "(*app).diagnostic_selection_summary = Some(selection_summary);",
            "(*app).diagnostic_range_input = Some(range_input);",
            "(*app).diagnostic_copy_button = Some(copy_button);",
            "(*app).diagnostic_export_button = Some(export_button);",
            "(*app).diagnostic_message = Some(message);",
            "(*app).diagnostic_list = Some(list);",
        ] {
            let assignment_position = create_source
                .find(assignment)
                .expect("required diagnostic child assignment must exist");
            assert!(
                (assignments_start..render_start).contains(&assignment_position),
                "required diagnostic child assignment must precede presentation: {assignment}"
            );
        }

        let open_start = source
            .find("unsafe fn open_diagnostic_window(app: &mut App)")
            .expect("diagnostic open function must exist");
        let open_source = &source[open_start..];
        let parent_assignment = open_source
            .find("app.diagnostic_window = Some(window);")
            .expect("diagnostic parent assignment must exist");
        let layout = open_source[parent_assignment..]
            .find("if !layout_diagnostic_controls(window, app)")
            .map(|offset| parent_assignment + offset)
            .expect("initial diagnostic layout must succeed before rendering");
        let first_render = open_source[parent_assignment..]
            .find("refresh_diagnostic_window(window, app, false);")
            .map(|offset| parent_assignment + offset)
            .expect("hidden diagnostic window must render synchronously");
        let shown = open_source[first_render..]
            .find("ShowWindow(window, SW_SHOWNORMAL);")
            .map(|offset| first_render + offset)
            .expect("new diagnostic window must be shown after first render");
        let update = open_source[shown..]
            .find("UpdateWindow(window);")
            .map(|offset| shown + offset)
            .expect("new diagnostic window must update after showing");
        let foreground = open_source[update..]
            .find("SetForegroundWindow(window);")
            .map(|offset| update + offset)
            .expect("new diagnostic window must activate after updating");
        let later_refresh = open_source[foreground..]
            .find("request_diagnostic_refresh(app);")
            .map(|offset| foreground + offset)
            .expect("later diagnostic refresh must be posted after first visibility");
        let ready_check = open_source[parent_assignment..layout]
            .find("if !diagnostic_children_ready(app)")
            .expect("required diagnostic children must be checked before layout");
        assert!(parent_assignment < layout);
        assert!(ready_check < layout - parent_assignment);
        assert!(layout < first_render);
        assert!(first_render < shown);
        assert!(shown < update);
        assert!(update < foreground);
        assert!(foreground < later_refresh);

        let clear_start = source
            .find("fn clear_diagnostic_state(app: &mut App)")
            .expect("diagnostic cleanup function must exist");
        let clear_source = &source[clear_start..];
        for handle in ["diagnostic_hud_separator", "diagnostic_toolbar_separator"] {
            assert!(
                clear_source.contains(&format!("app.{handle} = None;")),
                "diagnostic cleanup must clear {handle}"
            );
        }
    }
    #[test]
    fn diagnostic_auto_fit_is_one_time_per_snapshot_generation() {
        assert!(diagnostic_auto_fit_once(false, false, 0, None));
        assert!(diagnostic_auto_fit_once(true, true, 3, None));
        assert!(!diagnostic_auto_fit_once(true, true, 3, Some(3)));
        assert!(!diagnostic_auto_fit_once(true, false, 4, Some(3)));
    }

    #[test]
    fn diagnostic_auto_fit_is_not_allowed_for_resize_or_scroll() {
        assert!(diagnostic_auto_fit_reason_allows(
            DiagnosticRefreshReason::Initial
        ));
        assert!(diagnostic_auto_fit_reason_allows(
            DiagnosticRefreshReason::Data
        ));
        for reason in [
            DiagnosticRefreshReason::View,
            DiagnosticRefreshReason::Resize,
            DiagnosticRefreshReason::Scroll,
        ] {
            assert!(!diagnostic_auto_fit_reason_allows(reason));
        }
    }

    #[test]
    fn diagnostic_refreshes_coalesce_and_redraw_is_batched() {
        let source = include_str!("diagnostic_window.rs");
        assert!(source.contains("native.SetWindowTextW.diagnostic_hud.error"));
        assert!(source.contains("native.SetWindowTextW.diagnostic_summary.error"));
        assert!(diagnostic_refresh_is_coalesced(true, false));
        assert!(diagnostic_refresh_is_coalesced(false, true));
        assert!(!diagnostic_refresh_is_coalesced(false, false));
        assert_eq!(WM_SETREDRAW, 0x000B);
        assert_ne!(SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOSENDCHANGING, 0);
    }

    #[test]
    fn diagnostic_post_redraw_invalidation_covers_every_child() {
        assert_eq!(
            DIAGNOSTIC_POST_REDRAW_CHILD_TARGETS,
            [
                DiagnosticChildRedrawTarget::HudState,
                DiagnosticChildRedrawTarget::Summary,
                DiagnosticChildRedrawTarget::HudSeparator,
                DiagnosticChildRedrawTarget::ToolbarSeparator,
                DiagnosticChildRedrawTarget::DisplayLabel,
                DiagnosticChildRedrawTarget::DisplayInput,
                DiagnosticChildRedrawTarget::ShowAllButton,
                DiagnosticChildRedrawTarget::CategoryFilter,
                DiagnosticChildRedrawTarget::SearchLabel,
                DiagnosticChildRedrawTarget::SearchInput,
                DiagnosticChildRedrawTarget::ToolbarLabel,
                DiagnosticChildRedrawTarget::SelectionSummary,
                DiagnosticChildRedrawTarget::RangeInput,
                DiagnosticChildRedrawTarget::CopyButton,
                DiagnosticChildRedrawTarget::ExportButton,
                DiagnosticChildRedrawTarget::ExportAllButton,
                DiagnosticChildRedrawTarget::Message,
                DiagnosticChildRedrawTarget::List,
            ]
        );
    }

    #[test]
    fn diagnostic_row_conversion_stays_within_the_retention_bound() {
        let store = DiagnosticStore::new(DEFAULT_MAX_EVENTS);
        for sequence in 0..(DEFAULT_MAX_EVENTS + 16) {
            store.record("bounded.test", sequence.to_string());
        }
        let events = store.snapshot();
        assert!(events.len() <= DEFAULT_MAX_EVENTS);
        let rows = diagnostic_grid_rows(&events, RowSelection::all(events.len()));
        assert!(rows.len() <= DEFAULT_MAX_EVENTS);
        assert!(rows
            .iter()
            .all(|row| row.cells.len() == REPORT_COLUMNS.len()));
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
