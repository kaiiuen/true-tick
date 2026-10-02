//! Diagnostic window implementation for True™ Tick.
//!
//! Provides the diagnostic and status window, covering creation, layout,
//! message handling, marquee backed selection, and DPI aware presentation.
//! Clipboard copy, CSV and TSV export, range parsing, and search and filter
//! helpers live in `diagnostic_transfer`.

use std::ffi::c_void;
use std::mem::size_of;

pub(crate) use super::diagnostic_grid::*;
pub(crate) use super::diagnostic_layout::*;

use tick_diagnostics::{
    diagnostic_grid_rows, diagnostic_grid_rows_for_sequences, format_tsv, parse_display_limit,
    parse_row_selection, retention_summary, row_selection_for_sequences, selected_event_sequences,
    selected_event_sequences_for_sequences, snapshot_is_truncated, verify_event_chain,
    DiagnosticEvent, DiagnosticGridRow, DiagnosticOutcome, DiagnosticPhase, DiagnosticRecord,
    DiagnosticSource, EventCategory, RowSelection,
};
use tick_observation_windows::ObservationSource;
use tick_policy::PowerState;

use crate::tray::list_view_native::*;
use crate::tray::{
    app_create_params, destroy_created_diagnostic_controls, diagnostic_create_failure_result,
    diagnostic_outcome, diagnostic_phase, native_outcome, scale_logical,
    set_diagnostic_control_font, wide, App, BeginPaint, CreateStruct, CreateWindowExW,
    DefWindowProcW, DeleteObject, DestroyWindow, EndPaint, FillRect, GetClientRect,
    GetDpiForSystem, GetDpiForWindow, GetLastError, GetModuleHandleW, GetStockObject, GetSysColor,
    GetSysColorBrush, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, InvalidateRect,
    IsIconic, IsWindow, PaintStruct, Point, Rect, SendMessageW, SetBkColor, SetForegroundWindow,
    SetTextColor, SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow, UpdateWindow,
    BN_CLICKED, BS_PUSHBUTTON, COLOR_WINDOW, COLOR_WINDOWTEXT, DEFAULT_GUI_FONT, EM_LIMITTEXT,
    EN_CHANGE, ES_AUTOHSCROLL, GWLP_USERDATA, GWLP_WNDPROC, SS_LEFT, SWP_NOACTIVATE, SWP_NOZORDER,
    SW_RESTORE, SW_SHOWNORMAL, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC,
    WM_DPICHANGED, WM_ERASEBKGND, WM_NCDESTROY, WM_PAINT, WM_SETFONT, WM_SETTINGCHANGE, WM_SIZE,
    WM_SYSCOLORCHANGE, WM_THEMECHANGED, WS_BORDER, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS,
    WS_EX_CLIENTEDGE, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};
use crate::tray_surface::{status_menu_items, GITHUB_URL};
use crate::ui::diagnostic_transfer::{
    choose_export_path, copy_tsv_to_clipboard, diagnostic_filtered_visible_events,
    diagnostic_range_details, diagnostic_transfer_details, diagnostic_transfer_source,
    export_format_for_path, format_export_rows, map_native_action, write_export_tsv,
    DiagnosticNativeAction, DiagnosticTransferSource,
};

pub const DIAGNOSTIC_WINDOW_CLASS: &str = "TrueTickDiagnosticClass";

const WM_APP: u32 = 0x8000;
const WM_DIAGNOSTIC_REFRESH: u32 = WM_APP + 2;

/// Window state owned by the diagnostic surface. Extracted from `App` so the
/// diagnostic control handles, marquee bookkeeping, selection state, and
/// refresh flags travel as one unit.
pub(crate) struct DiagnosticUiState {
    pub(crate) diagnostic_window: Option<*mut c_void>,
    pub(crate) diagnostic_state_font: Option<*mut c_void>,
    pub(crate) diagnostic_hud_state: Option<*mut c_void>,
    pub(crate) diagnostic_summary: Option<*mut c_void>,
    pub(crate) diagnostic_hud_separator: Option<*mut c_void>,
    pub(crate) diagnostic_toolbar_separator: Option<*mut c_void>,
    pub(crate) diagnostic_toolbar_div_filter: Option<*mut c_void>,
    pub(crate) diagnostic_toolbar_div_search: Option<*mut c_void>,
    pub(crate) diagnostic_toolbar_div_export: Option<*mut c_void>,
    pub(crate) diagnostic_display_label: Option<*mut c_void>,
    pub(crate) diagnostic_display_input: Option<*mut c_void>,
    pub(crate) diagnostic_show_all_button: Option<*mut c_void>,
    pub(crate) diagnostic_toolbar_label: Option<*mut c_void>,
    pub(crate) diagnostic_selection_summary: Option<*mut c_void>,
    pub(crate) diagnostic_range_input: Option<*mut c_void>,
    pub(crate) diagnostic_copy_button: Option<*mut c_void>,
    pub(crate) diagnostic_export_button: Option<*mut c_void>,
    pub(crate) diagnostic_export_all_button: Option<*mut c_void>,
    pub(crate) diagnostic_category_filter: Option<*mut c_void>,
    pub(crate) diagnostic_search_label: Option<*mut c_void>,
    pub(crate) diagnostic_search_input: Option<*mut c_void>,
    pub(crate) diagnostic_selected_category: Option<EventCategory>,
    pub(crate) diagnostic_message: Option<*mut c_void>,
    pub(crate) diagnostic_list: Option<*mut c_void>,
    pub(crate) diagnostic_list_prev_proc:
        Option<unsafe extern "system" fn(*mut c_void, u32, usize, isize) -> isize>,
    pub(crate) diagnostic_marquee_active: bool,
    pub(crate) diagnostic_marquee_pending: bool,
    pub(crate) diagnostic_marquee_anchor: Point,
    pub(crate) diagnostic_marquee_current: Point,
    pub(crate) diagnostic_marquee_initial_selected: Vec<usize>,
    pub(crate) diagnostic_selection: Option<RowSelection>,
    pub(crate) diagnostic_selection_sequences: Option<Vec<u64>>,
    pub(crate) diagnostic_grid_selection_sequences: Vec<u64>,
    pub(crate) diagnostic_grid_selection_reset: bool,
    pub(crate) diagnostic_selection_reset: bool,
    pub(crate) diagnostic_display_limit: usize,
    pub(crate) diagnostic_display_all: bool,
    pub(crate) diagnostic_controls_initializing: bool,
    pub(crate) diagnostic_search_text: String,
    pub(crate) diagnostic_message_text: String,
    pub(crate) diagnostic_refresh_pending: bool,
    pub(crate) diagnostic_refreshing: bool,
    pub(crate) diagnostic_refresh_direct_recorded: bool,
    pub(crate) diagnostic_refresh_follow_up_scheduled: bool,
    pub(crate) diagnostic_layout_stable: bool,
    pub(crate) diagnostic_snapshot_key: Option<(usize, u64)>,
    pub(crate) diagnostic_snapshot_generation: u64,
    pub(crate) diagnostic_auto_fit_generation: Option<u64>,
}

impl DiagnosticUiState {
    pub(crate) fn new() -> Self {
        Self {
            diagnostic_window: None,
            diagnostic_state_font: None,
            diagnostic_hud_state: None,
            diagnostic_summary: None,
            diagnostic_hud_separator: None,
            diagnostic_toolbar_separator: None,
            diagnostic_toolbar_div_filter: None,
            diagnostic_toolbar_div_search: None,
            diagnostic_toolbar_div_export: None,
            diagnostic_display_label: None,
            diagnostic_display_input: None,
            diagnostic_show_all_button: None,
            diagnostic_toolbar_label: None,
            diagnostic_selection_summary: None,
            diagnostic_range_input: None,
            diagnostic_copy_button: None,
            diagnostic_export_button: None,
            diagnostic_export_all_button: None,
            diagnostic_category_filter: None,
            diagnostic_search_label: None,
            diagnostic_search_input: None,
            diagnostic_selected_category: None,
            diagnostic_message: None,
            diagnostic_list: None,
            diagnostic_list_prev_proc: None,
            diagnostic_marquee_active: false,
            diagnostic_marquee_pending: false,
            diagnostic_marquee_anchor: Point::default(),
            diagnostic_marquee_current: Point::default(),
            diagnostic_marquee_initial_selected: Vec::new(),
            diagnostic_selection: None,
            diagnostic_selection_sequences: None,
            diagnostic_grid_selection_sequences: Vec::new(),
            diagnostic_grid_selection_reset: false,
            diagnostic_selection_reset: false,
            diagnostic_display_limit: 100,
            diagnostic_display_all: false,
            diagnostic_controls_initializing: false,
            diagnostic_search_text: String::new(),
            diagnostic_message_text: String::new(),
            diagnostic_refresh_pending: false,
            diagnostic_refreshing: false,
            diagnostic_refresh_direct_recorded: false,
            diagnostic_refresh_follow_up_scheduled: false,
            diagnostic_layout_stable: false,
            diagnostic_snapshot_key: None,
            diagnostic_snapshot_generation: 0,
            diagnostic_auto_fit_generation: None,
        }
    }
}

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
const DIAGNOSTIC_RANGE_INPUT_LIMIT: usize = 64;
const SWP_NOSENDCHANGING: u32 = 0x0400;
const SS_NOPREFIX: u32 = 0x0000_0080;
const SS_ETCHEDHORZ: u32 = 0x0000_0010;

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
fn clear_diagnostic_state(state: &mut DiagnosticUiState) {
    unsafe {
        // SAFETY: the object is a font created by this window and is deleted exactly once
        if let Some(font) = state.diagnostic_state_font.take() {
            let _ = DeleteObject(font);
        }
    }
    state.diagnostic_window = None;
    state.diagnostic_hud_state = None;
    state.diagnostic_summary = None;
    state.diagnostic_hud_separator = None;
    state.diagnostic_toolbar_separator = None;
    state.diagnostic_toolbar_div_filter = None;
    state.diagnostic_toolbar_div_search = None;
    state.diagnostic_toolbar_div_export = None;
    state.diagnostic_display_label = None;
    state.diagnostic_display_input = None;
    state.diagnostic_show_all_button = None;
    state.diagnostic_category_filter = None;
    state.diagnostic_search_label = None;
    state.diagnostic_search_input = None;
    state.diagnostic_search_text.clear();
    state.diagnostic_toolbar_label = None;
    state.diagnostic_selection_summary = None;
    state.diagnostic_range_input = None;
    state.diagnostic_copy_button = None;
    state.diagnostic_export_button = None;
    state.diagnostic_export_all_button = None;
    state.diagnostic_selected_category = None;
    state.diagnostic_message = None;
    state.diagnostic_list = None;
    state.diagnostic_list_prev_proc = None;
    state.diagnostic_marquee_active = false;
    state.diagnostic_marquee_pending = false;
    state.diagnostic_marquee_anchor = Point::default();
    state.diagnostic_marquee_current = Point::default();
    state.diagnostic_marquee_initial_selected.clear();
    state.diagnostic_selection = None;
    state.diagnostic_selection_sequences = None;
    state.diagnostic_grid_selection_sequences.clear();
    state.diagnostic_grid_selection_reset = false;
    state.diagnostic_selection_reset = false;
    state.diagnostic_message_text.clear();
    state.diagnostic_controls_initializing = false;
    state.diagnostic_refresh_pending = false;
    state.diagnostic_refreshing = false;
    state.diagnostic_refresh_direct_recorded = false;
    state.diagnostic_refresh_follow_up_scheduled = false;
    state.diagnostic_layout_stable = false;
    state.diagnostic_snapshot_key = None;
    state.diagnostic_snapshot_generation = 0;
    state.diagnostic_auto_fit_generation = None;
    let _ = diagnostic_owner_buffer_with(|buffer| buffer.clear());
}
// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
unsafe fn diagnostic_children_ready(state: &DiagnosticUiState) -> bool {
    [
        state.diagnostic_hud_state,
        state.diagnostic_summary,
        state.diagnostic_hud_separator,
        state.diagnostic_toolbar_separator,
        state.diagnostic_display_label,
        state.diagnostic_display_input,
        state.diagnostic_show_all_button,
        state.diagnostic_category_filter,
        state.diagnostic_search_label,
        state.diagnostic_search_input,
        state.diagnostic_toolbar_label,
        state.diagnostic_selection_summary,
        state.diagnostic_range_input,
        state.diagnostic_copy_button,
        state.diagnostic_export_button,
        state.diagnostic_export_all_button,
        state.diagnostic_message,
        state.diagnostic_list,
    ]
    .into_iter()
    .all(|control| control.is_some_and(|value| IsWindow(value) != 0))
}
// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
pub(crate) unsafe fn destroy_diagnostic_window(app: &mut App) {
    if let Some(window) = app.diagnostic_ui.diagnostic_window.take() {
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
    clear_diagnostic_state(&mut app.diagnostic_ui);
}
// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
pub unsafe fn open_diagnostic_window(app: &mut App) -> bool {
    if let Some(window) = app.diagnostic_ui.diagnostic_window {
        if IsWindow(window) == 0 {
            record_diagnostic_event_with_context(
                app,
                "diagnostic.window.invalid",
                "result=cleared reason=parent_not_a_window",
                DiagnosticPhase::Render,
                DiagnosticOutcome::Failed,
            );
            clear_diagnostic_state(&mut app.diagnostic_ui);
        } else if !diagnostic_children_ready(&app.diagnostic_ui) {
            record_diagnostic_event_with_context(
                app,
                "diagnostic.window.invalid",
                "result=destroyed reason=required_child_missing",
                DiagnosticPhase::Render,
                DiagnosticOutcome::Failed,
            );
            let _ = DestroyWindow(window);
            clear_diagnostic_state(&mut app.diagnostic_ui);
        } else {
            if IsIconic(window) != 0 {
                ShowWindow(window, SW_RESTORE);
            } else {
                ShowWindow(window, SW_SHOWNORMAL);
            }
            UpdateWindow(window);
            SetForegroundWindow(window);
            request_diagnostic_refresh(&mut app.diagnostic_ui);
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
            app.record_with_outcome(
                "native.SetWindowTextW.diagnostic.error",
                format!("raw_status={}", GetLastError()),
                DiagnosticOutcome::Failed,
            );
            DestroyWindow(window);
            return false;
        }
        app.diagnostic_ui.diagnostic_window = Some(window);
        if !diagnostic_children_ready(&app.diagnostic_ui) {
            record_diagnostic_event_with_context(
                app,
                "diagnostic.window.invalid",
                "result=destroyed reason=create_returned_without_required_children",
                DiagnosticPhase::Render,
                DiagnosticOutcome::Failed,
            );
            let _ = DestroyWindow(window);
            clear_diagnostic_state(&mut app.diagnostic_ui);
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
            clear_diagnostic_state(&mut app.diagnostic_ui);
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
        request_diagnostic_refresh(&mut app.diagnostic_ui);
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

fn diagnostic_summary_style() -> u32 {
    WS_CHILD | WS_VISIBLE | SS_LEFT | SS_NOPREFIX
}

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
unsafe fn diagnostic_display_text(app: &App) -> String {
    let Some(input) = app.diagnostic_ui.diagnostic_display_input else {
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

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
unsafe fn diagnostic_search_text(app: &App) -> String {
    let Some(input) = app.diagnostic_ui.diagnostic_search_input else {
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

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
unsafe fn diagnostic_range_text(app: &App) -> String {
    let Some(input) = app.diagnostic_ui.diagnostic_range_input else {
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

pub(crate) fn record_diagnostic_refresh_event(app: &mut App, name: &str, details: impl AsRef<str>) {
    if app.diagnostic_ui.diagnostic_refreshing {
        app.diagnostic_ui.diagnostic_refresh_direct_recorded = true;
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

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
pub(crate) unsafe fn set_diagnostic_message(app: &mut App, message: impl Into<String>) {
    app.diagnostic_ui.diagnostic_message_text = message.into();
    if let Some(control) = app.diagnostic_ui.diagnostic_message {
        let text = wide(&app.diagnostic_ui.diagnostic_message_text);
        if SetWindowTextW(control, text.as_ptr()) == 0 {
            app.diagnostics.record_with_outcome(
                "native.SetWindowTextW.diagnostic_message.error",
                format!("raw_status={}", GetLastError()),
                DiagnosticOutcome::Failed,
            );
        }
    }
}

pub(crate) fn diagnostic_selection_summary(app: &App, retained_rows: usize) -> String {
    if !app
        .diagnostic_ui
        .diagnostic_grid_selection_sequences
        .is_empty()
    {
        if app.diagnostic_ui.diagnostic_grid_selection_reset {
            format!(
                "Selected: {} rows (some no longer retained)",
                app.diagnostic_ui.diagnostic_grid_selection_sequences.len()
            )
        } else {
            format!(
                "Selected: {} rows",
                app.diagnostic_ui.diagnostic_grid_selection_sequences.len()
            )
        }
    } else if app.diagnostic_ui.diagnostic_grid_selection_reset {
        "Selection reset: selected rows are no longer retained".to_owned()
    } else {
        match app.diagnostic_ui.diagnostic_selection {
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

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
pub(crate) unsafe fn set_diagnostic_selection_summary(app: &mut App, text: impl AsRef<str>) {
    if let Some(summary) = app.diagnostic_ui.diagnostic_selection_summary {
        let text = wide(text.as_ref());
        let _ = SetWindowTextW(summary, text.as_ptr());
    }
}

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
pub(crate) unsafe fn set_diagnostic_action_enabled(app: &App, enabled: bool) {
    if let Some(copy) = app.diagnostic_ui.diagnostic_copy_button {
        EnableWindow(copy, i32::from(enabled));
    }
    if let Some(export) = app.diagnostic_ui.diagnostic_export_button {
        EnableWindow(export, i32::from(enabled));
    }
}

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
unsafe fn refresh_diagnostic_controls(
    app: &mut App,
    events: &[tick_diagnostics::DiagnosticEvent],
    preserve_selection: bool,
) {
    let retained_rows = events.len();
    let input = diagnostic_range_text(app);
    let input_is_all = input.trim().is_empty() || input.trim().eq_ignore_ascii_case("all");
    if preserve_selection && app.diagnostic_ui.diagnostic_selection_reset {
        set_diagnostic_selection_summary(app, diagnostic_selection_summary(app, retained_rows));
        set_diagnostic_action_enabled(
            app,
            !app.diagnostic_ui
                .diagnostic_grid_selection_sequences
                .is_empty(),
        );
        return;
    }
    if preserve_selection && !input_is_all {
        if let Some(sequences) = app.diagnostic_ui.diagnostic_selection_sequences.clone() {
            if let Some(selection) = row_selection_for_sequences(events, &sequences) {
                app.diagnostic_ui.diagnostic_selection = Some(selection);
                set_diagnostic_selection_summary(
                    app,
                    diagnostic_selection_summary(app, retained_rows),
                );
                set_diagnostic_action_enabled(
                    app,
                    !app.diagnostic_ui
                        .diagnostic_grid_selection_sequences
                        .is_empty()
                        || !selection.is_empty(),
                );
                return;
            }
            app.diagnostic_ui.diagnostic_selection = None;
            app.diagnostic_ui.diagnostic_selection_sequences = None;
            app.diagnostic_ui.diagnostic_selection_reset = true;
            set_diagnostic_selection_summary(app, diagnostic_selection_summary(app, retained_rows));
            set_diagnostic_action_enabled(
                app,
                !app.diagnostic_ui
                    .diagnostic_grid_selection_sequences
                    .is_empty(),
            );
            set_diagnostic_message(
                app,
                "Selection reset: selected rows are no longer retained.",
            );
            record_diagnostic_refresh_event(
                app,
                "diagnostic.selection.reset",
                format!("reason=range_rows_not_retained retained_rows={retained_rows}"),
            );
            request_diagnostic_refresh(&mut app.diagnostic_ui);
            return;
        }
    }
    match parse_row_selection(&input, retained_rows) {
        Ok(selection) => {
            app.diagnostic_ui.diagnostic_selection = Some(selection);
            app.diagnostic_ui.diagnostic_selection_sequences =
                Some(selected_event_sequences(events, selection));
            app.diagnostic_ui.diagnostic_selection_reset = false;
            if app
                .diagnostic_ui
                .diagnostic_message_text
                .starts_with("Invalid range:")
            {
                set_diagnostic_message(app, "");
            }
        }
        Err(error) => {
            app.diagnostic_ui.diagnostic_selection = None;
            app.diagnostic_ui.diagnostic_selection_sequences = None;
            app.diagnostic_ui.diagnostic_selection_reset = false;
            set_diagnostic_message(app, format!("Invalid range: {error}"));
        }
    }
    set_diagnostic_selection_summary(app, diagnostic_selection_summary(app, retained_rows));
    set_diagnostic_action_enabled(
        app,
        !app.diagnostic_ui
            .diagnostic_grid_selection_sequences
            .is_empty()
            || app
                .diagnostic_ui
                .diagnostic_selection
                .is_some_and(|selection| !selection.is_empty()),
    );
}

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
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

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
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

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
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
        (app.diagnostic_ui.diagnostic_hud_state, layout.hud_state),
        (app.diagnostic_ui.diagnostic_summary, layout.summary),
        (
            app.diagnostic_ui.diagnostic_hud_separator,
            layout.hud_separator,
        ),
        (
            app.diagnostic_ui.diagnostic_toolbar_separator,
            layout.toolbar_separator,
        ),
        (
            app.diagnostic_ui.diagnostic_display_label,
            toolbar.display_label,
        ),
        (
            app.diagnostic_ui.diagnostic_display_input,
            toolbar.display_input,
        ),
        (
            app.diagnostic_ui.diagnostic_show_all_button,
            toolbar.show_all,
        ),
        (
            app.diagnostic_ui.diagnostic_category_filter,
            toolbar.category_filter,
        ),
        (
            app.diagnostic_ui.diagnostic_search_label,
            toolbar.search_label,
        ),
        (
            app.diagnostic_ui.diagnostic_search_input,
            toolbar.search_input,
        ),
        (
            app.diagnostic_ui.diagnostic_toolbar_label,
            toolbar.transfer_label,
        ),
        (
            app.diagnostic_ui.diagnostic_range_input,
            toolbar.range_input,
        ),
        (
            app.diagnostic_ui.diagnostic_selection_summary,
            toolbar.selection_summary,
        ),
        (app.diagnostic_ui.diagnostic_copy_button, toolbar.copy),
        (app.diagnostic_ui.diagnostic_export_button, toolbar.export),
        (
            app.diagnostic_ui.diagnostic_export_all_button,
            toolbar.export_all,
        ),
        (app.diagnostic_ui.diagnostic_message, toolbar.message),
        (app.diagnostic_ui.diagnostic_list, layout.list),
    ];
    let positioned = apply_diagnostic_layout(window, app, &controls);
    if let Some(list) = app.diagnostic_ui.diagnostic_list {
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

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
unsafe fn set_diagnostic_redraw(app: &App, enabled: bool) {
    let redraw = usize::from(enabled);
    if let Some(window) = app.diagnostic_ui.diagnostic_window {
        let _ = SendMessageW(window, WM_SETREDRAW, redraw, 0);
    }
    for control in [
        app.diagnostic_ui.diagnostic_hud_state,
        app.diagnostic_ui.diagnostic_summary,
        app.diagnostic_ui.diagnostic_hud_separator,
        app.diagnostic_ui.diagnostic_toolbar_separator,
        app.diagnostic_ui.diagnostic_display_label,
        app.diagnostic_ui.diagnostic_display_input,
        app.diagnostic_ui.diagnostic_show_all_button,
        app.diagnostic_ui.diagnostic_category_filter,
        app.diagnostic_ui.diagnostic_search_label,
        app.diagnostic_ui.diagnostic_search_input,
        app.diagnostic_ui.diagnostic_toolbar_label,
        app.diagnostic_ui.diagnostic_selection_summary,
        app.diagnostic_ui.diagnostic_range_input,
        app.diagnostic_ui.diagnostic_copy_button,
        app.diagnostic_ui.diagnostic_export_button,
        app.diagnostic_ui.diagnostic_export_all_button,
        app.diagnostic_ui.diagnostic_message,
        app.diagnostic_ui.diagnostic_list,
    ]
    .into_iter()
    .flatten()
    {
        let _ = SendMessageW(control, WM_SETREDRAW, redraw, 0);
    }
}

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
unsafe fn finish_diagnostic_redraw(window: *mut c_void, app: &App) {
    set_diagnostic_redraw(app, true);
    for target in DIAGNOSTIC_POST_REDRAW_CHILD_TARGETS {
        let control = match target {
            DiagnosticChildRedrawTarget::HudState => app.diagnostic_ui.diagnostic_hud_state,
            DiagnosticChildRedrawTarget::Summary => app.diagnostic_ui.diagnostic_summary,
            DiagnosticChildRedrawTarget::HudSeparator => app.diagnostic_ui.diagnostic_hud_separator,
            DiagnosticChildRedrawTarget::ToolbarSeparator => {
                app.diagnostic_ui.diagnostic_toolbar_separator
            }
            DiagnosticChildRedrawTarget::DisplayLabel => app.diagnostic_ui.diagnostic_display_label,
            DiagnosticChildRedrawTarget::DisplayInput => app.diagnostic_ui.diagnostic_display_input,
            DiagnosticChildRedrawTarget::ShowAllButton => {
                app.diagnostic_ui.diagnostic_show_all_button
            }
            DiagnosticChildRedrawTarget::CategoryFilter => {
                app.diagnostic_ui.diagnostic_category_filter
            }
            DiagnosticChildRedrawTarget::SearchLabel => app.diagnostic_ui.diagnostic_search_label,
            DiagnosticChildRedrawTarget::SearchInput => app.diagnostic_ui.diagnostic_search_input,
            DiagnosticChildRedrawTarget::ToolbarLabel => app.diagnostic_ui.diagnostic_toolbar_label,
            DiagnosticChildRedrawTarget::SelectionSummary => {
                app.diagnostic_ui.diagnostic_selection_summary
            }
            DiagnosticChildRedrawTarget::RangeInput => app.diagnostic_ui.diagnostic_range_input,
            DiagnosticChildRedrawTarget::CopyButton => app.diagnostic_ui.diagnostic_copy_button,
            DiagnosticChildRedrawTarget::ExportButton => app.diagnostic_ui.diagnostic_export_button,
            DiagnosticChildRedrawTarget::ExportAllButton => {
                app.diagnostic_ui.diagnostic_export_all_button
            }
            DiagnosticChildRedrawTarget::Message => app.diagnostic_ui.diagnostic_message,
            DiagnosticChildRedrawTarget::List => app.diagnostic_ui.diagnostic_list,
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

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
unsafe fn refresh_diagnostic_window(
    window: *mut c_void,
    app: &mut App,
    allow_follow_up_post: bool,
) {
    if app.diagnostic_ui.diagnostic_refreshing {
        return;
    }
    app.diagnostic_ui.diagnostic_refreshing = true;
    let follow_up_refresh = app.diagnostic_ui.diagnostic_refresh_follow_up_scheduled;
    app.diagnostic_ui.diagnostic_refresh_follow_up_scheduled = false;
    app.diagnostic_ui.diagnostic_refresh_direct_recorded = false;
    set_diagnostic_redraw(app, false);
    let events = app.diagnostics.snapshot();
    let retained_rows = events.len();
    let snapshot_key = diagnostic_snapshot_key(&events);
    let snapshot_changed =
        diagnostic_data_snapshot_changed(app.diagnostic_ui.diagnostic_snapshot_key, snapshot_key);
    if snapshot_changed {
        app.diagnostic_ui.diagnostic_snapshot_generation = app
            .diagnostic_ui
            .diagnostic_snapshot_generation
            .saturating_add(1);
        app.diagnostic_ui.diagnostic_snapshot_key = snapshot_key;
    }
    let auto_fit = diagnostic_auto_fit_once(
        app.diagnostic_ui.diagnostic_layout_stable,
        snapshot_changed,
        app.diagnostic_ui.diagnostic_snapshot_generation,
        app.diagnostic_ui.diagnostic_auto_fit_generation,
    );
    if auto_fit {
        app.diagnostic_ui.diagnostic_auto_fit_generation =
            Some(app.diagnostic_ui.diagnostic_snapshot_generation);
    }
    let horizontal_scroll = app
        .diagnostic_ui
        .diagnostic_layout_stable
        .then(|| {
            app.diagnostic_ui
                .diagnostic_list
                .map(|list| GetScrollPos(list, SB_HORZ))
        })
        .flatten();
    preserve_diagnostic_grid_selection(app, &events);
    refresh_diagnostic_controls(app, &events, true);
    let Some(list) = app.diagnostic_ui.diagnostic_list else {
        app.diagnostics.record_with_outcome(
            "diagnostic.grid.refresh.error",
            "list_handle_null",
            DiagnosticOutcome::Failed,
        );
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
    let visible_events = diagnostic_filtered_visible_events(app, &events);
    let rows: Vec<DiagnosticGridRow> = visible_events
        .iter()
        .enumerate()
        .map(|(index, event)| tick_diagnostics::diagnostic_grid_row(index.saturating_add(1), event))
        .collect();
    let displayed_rows = rows.len();
    let overflow = displayed_rows.saturating_sub(EVENT_BUFFER_CAP);
    let (buffer_len, evicted_rows) = diagnostic_owner_buffer_with(|buffer| {
        buffer.clear();
        for row in rows {
            buffer.push(row);
        }
        (buffer.len(), overflow)
    })
    .unwrap_or((0, 0));
    let _ = SendMessageW(
        list,
        LVM_SETITEMCOUNT,
        buffer_len,
        LVSICF_NOINVALIDATEALL as isize,
    );
    let _ = InvalidateRect(list, std::ptr::null(), 1);
    apply_diagnostic_grid_selection(app, list, &events);
    let item_count = SendMessageW(list, LVM_GETITEMCOUNT, 0, 0);
    if !follow_up_refresh {
        record_diagnostic_refresh_event(
            app,
            "diagnostic.grid.refresh",
            diagnostic_grid_refresh_message(
                snapshot_rows,
                displayed_rows,
                item_count,
                evicted_rows,
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
    if let Some(state) = app.diagnostic_ui.diagnostic_hud_state {
        let text = wide(&diagnostic_state_text(app));
        if SetWindowTextW(state, text.as_ptr()) == 0 {
            app.diagnostics.record_with_outcome(
                "native.SetWindowTextW.diagnostic_hud.error",
                format!("raw_status={}", GetLastError()),
                DiagnosticOutcome::Failed,
            );
        }
    }
    if let Some(summary) = app.diagnostic_ui.diagnostic_summary {
        let text = wide(&diagnostic_summary_text(app, retained_rows));
        if SetWindowTextW(summary, text.as_ptr()) == 0 {
            app.diagnostics.record_with_outcome(
                "native.SetWindowTextW.diagnostic_summary.error",
                format!("raw_status={}", GetLastError()),
                DiagnosticOutcome::Failed,
            );
        }
    }
    app.diagnostic_ui.diagnostic_layout_stable = true;
    let generated_direct_records = app.diagnostic_ui.diagnostic_refresh_direct_recorded;
    finish_diagnostic_redraw(window, app);
    finish_diagnostic_refresh(
        app,
        allow_follow_up_post
            && diagnostic_follow_up_needed(generated_direct_records, follow_up_refresh),
    );
}

pub(crate) fn diagnostic_toolbar_action(app: &mut App, action: &str) {
    let events = app.diagnostics.snapshot();
    let retained_rows = events.len();
    app.begin_operation(DiagnosticSource::Diagnostic);
    let grid_sequences = selected_event_sequences_for_sequences(
        &events,
        &app.diagnostic_ui.diagnostic_grid_selection_sequences,
    );
    let (source, rows, range) = if !grid_sequences.is_empty() {
        (
            diagnostic_transfer_source(grid_sequences.len(), 0).expect("grid selection has rows"),
            diagnostic_grid_rows_for_sequences(&events, &grid_sequences),
            None,
        )
    } else {
        let input = unsafe {
            // SAFETY: reads a live edit control owned by the diagnostic window
            diagnostic_range_text(app)
        };
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
                app.record_with_outcome(
                    "diagnostic.validation_failure",
                    format!("{details} result=failed reason=no_retained_events"),
                    DiagnosticOutcome::Failed,
                );
                unsafe {
                    // SAFETY: updates in-memory text then touches a live control owned by the diagnostic window
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
                app.record_with_outcome(
                    "diagnostic.range.parsed",
                    format!("{details} result=invalid error={error}"),
                    DiagnosticOutcome::Failed,
                );
                app.record_with_outcome(
                    "diagnostic.validation_failure",
                    format!("{details} result=failed error={error}"),
                    DiagnosticOutcome::Failed,
                );
                unsafe {
                    // SAFETY: updates in-memory text then touches a live control owned by the diagnostic window
                    set_diagnostic_message(app, format!("Invalid range: {error}"));
                }
                app.finish_operation(DiagnosticOutcome::Failed);
                return;
            }
        };
        app.diagnostic_ui.diagnostic_selection = Some(selection);
        app.diagnostic_ui.diagnostic_selection_sequences =
            Some(selected_event_sequences(&events, selection));
        app.diagnostic_ui.diagnostic_selection_reset = false;
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
                // SAFETY: inputs are validated before the call and callee notes cover the clipboard contract
                copy_tsv_to_clipboard(
                    app.diagnostic_ui
                        .diagnostic_window
                        .unwrap_or(std::ptr::null_mut()),
                    &tsv,
                )
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
                        // SAFETY: updates in-memory text then touches a live control owned by the diagnostic window
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
                    app.record_with_outcome(
                        "native.clipboard.error",
                        format!(
                            "stage={} {details} result=failed raw_status={raw_error}",
                            error.stage
                        ),
                        DiagnosticOutcome::Failed,
                    );
                    unsafe {
                        // SAFETY: updates in-memory text then touches a live control owned by the diagnostic window
                        set_diagnostic_message(
                            app,
                            format!("Copy failed (native error {}).", error.raw_error),
                        );
                    }
                    app.record_with_outcome(
                        "diagnostic.copy.result",
                        format!("result=failed {details} raw_status={}", error.raw_error),
                        DiagnosticOutcome::Failed,
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
                // SAFETY: the owner is null or a live window and buffers live for the dialog
                choose_export_path(
                    app.diagnostic_ui
                        .diagnostic_window
                        .unwrap_or(std::ptr::null_mut()),
                )
            } {
                Ok(Some(path)) => path,
                Ok(None) => {
                    unsafe {
                        // SAFETY: updates in-memory text then touches a live control owned by the diagnostic window
                        set_diagnostic_message(app, "Export cancelled.");
                    }
                    app.record_with_outcome(
                        "diagnostic.export.cancelled",
                        format!("result=cancelled {details}"),
                        DiagnosticOutcome::Cancelled,
                    );
                    app.record_with_outcome(
                        "diagnostic.export.result",
                        format!("result=cancelled {details}"),
                        DiagnosticOutcome::Cancelled,
                    );
                    app.finish_operation(DiagnosticOutcome::Cancelled);
                    return;
                }
                Err(error) => {
                    app.record_with_outcome(
                        "native.GetSaveFileNameW.error",
                        format!("{details} result=failed raw_status={}", error.raw_error),
                        DiagnosticOutcome::Failed,
                    );
                    unsafe {
                        // SAFETY: updates in-memory text then touches a live control owned by the diagnostic window
                        set_diagnostic_message(
                            app,
                            format!("Export dialog failed (native error {}).", error.raw_error),
                        );
                    }
                    app.record_with_outcome(
                        "diagnostic.export.result",
                        format!("result=failed {details} raw_status={}", error.raw_error),
                        DiagnosticOutcome::Failed,
                    );
                    app.finish_operation(DiagnosticOutcome::Failed);
                    return;
                }
            };
            let format = export_format_for_path(&path);
            let export_text = format_export_rows(&rows, format);
            let write_result = unsafe {
                // SAFETY: the path was validated by the save dialog flow
                write_export_tsv(&path, &export_text)
            };
            match write_result {
                Ok(()) => {
                    unsafe {
                        // SAFETY: updates in-memory text then touches a live control owned by the diagnostic window
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
                    app.record_with_outcome(
                        "native.export.file.error",
                        format!(
                            "stage={} {details} result=failed raw_status={}",
                            error.stage, error.raw_error
                        ),
                        DiagnosticOutcome::Failed,
                    );
                    unsafe {
                        // SAFETY: updates in-memory text then touches a live control owned by the diagnostic window
                        set_diagnostic_message(
                            app,
                            format!("Export failed (native error {}).", error.raw_error),
                        );
                    }
                    app.record_with_outcome(
                        "diagnostic.export.result",
                        format!(
                            "result=failed {details} path=redacted raw_status={}",
                            error.raw_error
                        ),
                        DiagnosticOutcome::Failed,
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
                // SAFETY: the owner is null or a live window and buffers live for the dialog
                choose_export_path(
                    app.diagnostic_ui
                        .diagnostic_window
                        .unwrap_or(std::ptr::null_mut()),
                )
            } {
                Ok(Some(path)) => path,
                Ok(None) => {
                    unsafe {
                        // SAFETY: updates in-memory text then touches a live control owned by the diagnostic window
                        set_diagnostic_message(app, "Export cancelled.");
                    }
                    app.record_with_outcome(
                        "diagnostic.export.cancelled",
                        format!("result=cancelled {all_details}"),
                        DiagnosticOutcome::Cancelled,
                    );
                    app.record_with_outcome(
                        "diagnostic.export.result",
                        format!("result=cancelled {all_details}"),
                        DiagnosticOutcome::Cancelled,
                    );
                    app.finish_operation(DiagnosticOutcome::Cancelled);
                    return;
                }
                Err(error) => {
                    app.record_with_outcome(
                        "native.GetSaveFileNameW.error",
                        format!("{all_details} result=failed raw_status={}", error.raw_error),
                        DiagnosticOutcome::Failed,
                    );
                    unsafe {
                        // SAFETY: updates in-memory text then touches a live control owned by the diagnostic window
                        set_diagnostic_message(
                            app,
                            format!("Export dialog failed (native error {}).", error.raw_error),
                        );
                    }
                    app.record_with_outcome(
                        "diagnostic.export.result",
                        format!("result=failed {all_details} raw_status={}", error.raw_error),
                        DiagnosticOutcome::Failed,
                    );
                    app.finish_operation(DiagnosticOutcome::Failed);
                    return;
                }
            };
            let format = export_format_for_path(&path);
            let export_text = format_export_rows(&all_rows, format);
            let write_result = unsafe {
                // SAFETY: the path was validated by the save dialog flow
                write_export_tsv(&path, &export_text)
            };
            match write_result {
                Ok(()) => {
                    unsafe {
                        // SAFETY: updates in-memory text then touches a live control owned by the diagnostic window
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
                    app.record_with_outcome(
                        "native.export.file.error",
                        format!(
                            "stage={} {all_details} result=failed raw_status={}",
                            error.stage, error.raw_error
                        ),
                        DiagnosticOutcome::Failed,
                    );
                    unsafe {
                        // SAFETY: updates in-memory text then touches a live control owned by the diagnostic window
                        set_diagnostic_message(
                            app,
                            format!("Export failed (native error {}).", error.raw_error),
                        );
                    }
                    app.record_with_outcome(
                        "diagnostic.export.result",
                        format!(
                            "result=failed {all_details} path=redacted raw_status={}",
                            error.raw_error
                        ),
                        DiagnosticOutcome::Failed,
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
    let input = unsafe {
        // SAFETY: reads a live edit control owned by the diagnostic window
        diagnostic_range_text(app)
    };
    app.begin_operation(DiagnosticSource::Diagnostic);
    match parse_row_selection(&input, retained_rows) {
        Ok(selection) => {
            app.diagnostic_ui.diagnostic_selection = Some(selection);
            app.diagnostic_ui.diagnostic_selection_sequences =
                Some(selected_event_sequences(&events, selection));
            app.diagnostic_ui.diagnostic_selection_reset = false;
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
            app.record_with_outcome(
                "diagnostic.range.parsed",
                format!(
                    "selected_range=none retained_rows={retained_rows} row_count=0 format=TSV result=invalid error={error}"
                )
            , DiagnosticOutcome::Failed);
            app.record_with_outcome(
                "diagnostic.validation_failure",
                format!(
                    "selected_range=none retained_rows={retained_rows} row_count=0 format=TSV result=failed error={error}"
                )
            , DiagnosticOutcome::Failed);
            app.diagnostic_ui.diagnostic_selection = None;
            app.diagnostic_ui.diagnostic_selection_sequences = None;
            app.diagnostic_ui.diagnostic_selection_reset = false;
            app.finish_operation(DiagnosticOutcome::Failed);
        }
    }
    unsafe {
        // SAFETY: runs on the UI thread while the app is exclusively borrowed
        refresh_diagnostic_controls(app, &events, false);
    }
}

fn diagnostic_display_changed(app: &mut App) {
    let input = unsafe {
        // SAFETY: reads a live edit control owned by the diagnostic window
        diagnostic_display_text(app)
    };
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
            app.record_with_outcome(
                "diagnostic.display_limit.validation",
                format!(
                    "result=invalid input_bytes={} retained_cap={}",
                    input.len(),
                    app.diagnostics.maximum_events()
                ),
                DiagnosticOutcome::Failed,
            );
            unsafe {
                // SAFETY: updates in-memory text then touches a live control owned by the diagnostic window
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
            app.diagnostic_ui.diagnostic_display_limit = limit;
            app.diagnostic_ui.diagnostic_display_all = false;
            app.begin_operation(DiagnosticSource::Diagnostic);
            app.record(
                "diagnostic.display_limit.changed",
                format!("result=success limit={limit} override=none"),
            );
            unsafe {
                // SAFETY: updates in-memory text then touches a live control owned by the diagnostic window
                if app
                    .diagnostic_ui
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
    app.diagnostic_ui.diagnostic_display_all = true;
    app.begin_operation(DiagnosticSource::Diagnostic);
    app.record(
        "diagnostic.display_limit.override",
        format!(
            "result=success mode=all retained_cap={}",
            app.diagnostics.maximum_events()
        ),
    );
    unsafe {
        // SAFETY: updates in-memory text then touches a live control owned by the diagnostic window
        if app
            .diagnostic_ui
            .diagnostic_message_text
            .starts_with("Invalid Show rows value:")
        {
            set_diagnostic_message(app, "");
        }
    }
    app.finish_operation(DiagnosticOutcome::Completed);
}

fn diagnostic_category_filter_changed(app: &mut App) {
    let Some(combo) = app.diagnostic_ui.diagnostic_category_filter else {
        return;
    };
    let index = unsafe {
        // SAFETY: the target is a live control created by this window and wparam plus lparam match the message contract
        SendMessageW(combo, CB_GETCURSEL, 0, 0)
    };
    let category = match index {
        1 => Some(EventCategory::Startup),
        2 => Some(EventCategory::Timer),
        3 => Some(EventCategory::Power),
        4 => Some(EventCategory::UI),
        5 => Some(EventCategory::Schedule),
        6 => Some(EventCategory::System),
        _ => None,
    };
    app.diagnostic_ui.diagnostic_selected_category = category;
    app.begin_operation(DiagnosticSource::Diagnostic);
    app.record(
        "diagnostic.category_filter.changed",
        format!("category={}", category.map_or("All", |c| c.as_str())),
    );
    app.finish_operation(DiagnosticOutcome::Completed);
    request_diagnostic_refresh(&mut app.diagnostic_ui);
}

fn diagnostic_search_changed(app: &mut App) {
    let search = unsafe {
        // SAFETY: reads a live edit control owned by the diagnostic window
        diagnostic_search_text(app)
    };
    if search == app.diagnostic_ui.diagnostic_search_text {
        return;
    }
    app.diagnostic_ui.diagnostic_search_text = search;
    app.begin_operation(DiagnosticSource::Diagnostic);
    app.record(
        "diagnostic.search.changed",
        format!(
            "query_bytes={}",
            app.diagnostic_ui.diagnostic_search_text.len()
        ),
    );
    app.finish_operation(DiagnosticOutcome::Completed);
    request_diagnostic_refresh(&mut app.diagnostic_ui);
}

pub(crate) fn request_diagnostic_refresh(state: &mut DiagnosticUiState) {
    if diagnostic_refresh_is_coalesced(
        state.diagnostic_refresh_pending,
        state.diagnostic_refreshing,
    ) {
        if state.diagnostic_refreshing {
            state.diagnostic_refresh_pending = true;
        }
        return;
    }
    let Some(window) = state.diagnostic_window else {
        return;
    };
    state.diagnostic_refresh_pending = true;
    unsafe {
        // SAFETY: hwnd is a live window and the message id was registered by this module
        if PostMessageW(window, WM_DIAGNOSTIC_REFRESH, 0, 0) == 0 {
            state.diagnostic_refresh_pending = false;
        }
    }
}

fn finish_diagnostic_refresh(app: &mut App, follow_up_needed: bool) {
    app.diagnostic_ui.diagnostic_refreshing = false;
    crate::logging::flush_diagnostic_events_to_disk(
        &app.diagnostics,
        &app.last_persisted_event_sequence,
        &app.log_directory,
    );
    if follow_up_needed {
        app.diagnostic_ui.diagnostic_refresh_follow_up_scheduled = true;
    }
    if app.diagnostic_ui.diagnostic_refresh_pending || follow_up_needed {
        app.diagnostic_ui.diagnostic_refresh_pending = false;
        request_diagnostic_refresh(&mut app.diagnostic_ui);
    }
}

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
unsafe fn set_diagnostic_control_fonts(app: &mut App) {
    for control in [
        app.diagnostic_ui.diagnostic_hud_state,
        app.diagnostic_ui.diagnostic_summary,
        app.diagnostic_ui.diagnostic_hud_separator,
        app.diagnostic_ui.diagnostic_toolbar_separator,
        app.diagnostic_ui.diagnostic_display_label,
        app.diagnostic_ui.diagnostic_display_input,
        app.diagnostic_ui.diagnostic_show_all_button,
        app.diagnostic_ui.diagnostic_category_filter,
        app.diagnostic_ui.diagnostic_search_label,
        app.diagnostic_ui.diagnostic_search_input,
        app.diagnostic_ui.diagnostic_toolbar_label,
        app.diagnostic_ui.diagnostic_selection_summary,
        app.diagnostic_ui.diagnostic_range_input,
        app.diagnostic_ui.diagnostic_copy_button,
        app.diagnostic_ui.diagnostic_export_button,
        app.diagnostic_ui.diagnostic_export_all_button,
        app.diagnostic_ui.diagnostic_message,
        app.diagnostic_ui.diagnostic_list,
    ]
    .into_iter()
    .flatten()
    {
        set_diagnostic_control_font(control);
    }
    let Some(state) = app.diagnostic_ui.diagnostic_hud_state else {
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
    if let Some(previous) = app.diagnostic_ui.diagnostic_state_font.replace(font) {
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

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
unsafe fn refresh_diagnostic_presentation(hwnd: *mut c_void, app: &mut App) {
    set_diagnostic_control_fonts(app);
    let background = GetSysColor(COLOR_WINDOW);
    let text = GetSysColor(COLOR_WINDOWTEXT);
    if let Some(list) = app.diagnostic_ui.diagnostic_list {
        let _ = SendMessageW(list, LVM_SETBKCOLOR, 0, background as isize);
        let _ = SendMessageW(list, LVM_SETTEXTCOLOR, 0, text as isize);
        let _ = SendMessageW(list, LVM_SETTEXTBKCOLOR, 0, background as isize);
    }
    for control in [
        app.diagnostic_ui.diagnostic_hud_state,
        app.diagnostic_ui.diagnostic_summary,
        app.diagnostic_ui.diagnostic_hud_separator,
        app.diagnostic_ui.diagnostic_toolbar_separator,
        app.diagnostic_ui.diagnostic_display_label,
        app.diagnostic_ui.diagnostic_display_input,
        app.diagnostic_ui.diagnostic_show_all_button,
        app.diagnostic_ui.diagnostic_category_filter,
        app.diagnostic_ui.diagnostic_search_label,
        app.diagnostic_ui.diagnostic_search_input,
        app.diagnostic_ui.diagnostic_toolbar_label,
        app.diagnostic_ui.diagnostic_selection_summary,
        app.diagnostic_ui.diagnostic_range_input,
        app.diagnostic_ui.diagnostic_copy_button,
        app.diagnostic_ui.diagnostic_export_button,
        app.diagnostic_ui.diagnostic_export_all_button,
        app.diagnostic_ui.diagnostic_message,
        app.diagnostic_ui.diagnostic_list,
    ]
    .into_iter()
    .flatten()
    {
        let _ = InvalidateRect(control, std::ptr::null(), 1);
    }
    let _ = InvalidateRect(hwnd, std::ptr::null(), 1);
    let _ = UpdateWindow(hwnd);
}

// SAFETY: the OS calls this with a valid hwnd and message parameters and the app pointer comes from GWLP_USERDATA set at creation
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
        (*app).diagnostic_ui.diagnostic_controls_initializing = true;
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
                (*(app_ptr as *mut App)).diagnostics.record_with_outcome(
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
                    )
                , DiagnosticOutcome::Failed);
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
                (*(app_ptr as *mut App)).diagnostics.record_with_outcome(
                    "native.ListView.insert_column.error",
                    format!("raw_status={raw_error}"),
                    DiagnosticOutcome::Failed,
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
            (*app).diagnostic_ui.diagnostic_hud_state = Some(hud_state);
            (*app).diagnostic_ui.diagnostic_summary = Some(summary);
            (*app).diagnostic_ui.diagnostic_hud_separator = Some(hud_separator);
            (*app).diagnostic_ui.diagnostic_toolbar_separator = Some(toolbar_separator);
            (*app).diagnostic_ui.diagnostic_display_label = Some(display_label);
            (*app).diagnostic_ui.diagnostic_display_input = Some(display_input);
            (*app).diagnostic_ui.diagnostic_show_all_button = Some(show_all_button);
            (*app).diagnostic_ui.diagnostic_category_filter = Some(category_combo);
            (*app).diagnostic_ui.diagnostic_search_label = Some(search_label);
            (*app).diagnostic_ui.diagnostic_search_input = Some(search_input);
            (*app).diagnostic_ui.diagnostic_toolbar_label = Some(label);
            (*app).diagnostic_ui.diagnostic_selection_summary = Some(selection_summary);
            (*app).diagnostic_ui.diagnostic_range_input = Some(range_input);
            (*app).diagnostic_ui.diagnostic_copy_button = Some(copy_button);
            (*app).diagnostic_ui.diagnostic_export_button = Some(export_button);
            (*app).diagnostic_ui.diagnostic_export_all_button = Some(export_all_button);
            (*app).diagnostic_ui.diagnostic_message = Some(message);
            (*app).diagnostic_ui.diagnostic_list = Some(list);
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
                (*app).diagnostic_ui.diagnostic_list_prev_proc = Some(prev_proc_fn);
            }
            refresh_diagnostic_presentation(hwnd, &mut *app);
            let _ = layout_diagnostic_controls(hwnd, &*app);
            (*app).diagnostic_ui.diagnostic_controls_initializing = false;
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
            if edit_notification && (*app).diagnostic_ui.diagnostic_controls_initializing {
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
            (*app).diagnostic_ui.diagnostic_refresh_pending = false;
            refresh_diagnostic_window(hwnd, &mut *app, true);
            return 0;
        } else if message == WM_DPICHANGED {
            let suggested = l_param as *const Rect;
            if suggested.is_null() {
                (*app).diagnostics.record_with_outcome(
                    "diagnostic.dpi.error",
                    format!("stage=suggested_rect_missing dpi={}", w_param & 0xffff),
                    DiagnosticOutcome::Failed,
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
                    (*app).diagnostics.record_with_outcome(
                        "diagnostic.dpi.error",
                        format!("stage=SetWindowPos raw_status={}", GetLastError()),
                        DiagnosticOutcome::Failed,
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
            request_diagnostic_refresh(&mut (*app).diagnostic_ui);
        } else if message == WM_NCDESTROY {
            clear_diagnostic_state(&mut (*app).diagnostic_ui);
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
    fn GetScrollPos(window: *mut c_void, bar: i32) -> i32;
    fn SetScrollPos(window: *mut c_void, bar: i32, position: i32, redraw: i32) -> i32;
}

#[link(name = "gdi32")]
extern "system" {
    fn GetObjectW(object: *mut c_void, count: i32, object_data: *mut c_void) -> i32;
    fn CreateFontIndirectW(log_font: *const LogFontW) -> *mut c_void;
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
    use tick_diagnostics::{DiagnosticStore, DEFAULT_MAX_EVENTS, REPORT_COLUMNS};

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
            .find("(*app).diagnostic_ui.diagnostic_hud_state = Some(hud_state);")
            .expect("diagnostic child assignment block must exist");
        assert!(columns_initialized < assignments_start);
        let render_start = create_source[assignments_start..]
            .find("refresh_diagnostic_presentation(hwnd, &mut *app);")
            .map(|offset| assignments_start + offset)
            .expect("diagnostic presentation must follow child assignment");
        for assignment in [
            "(*app).diagnostic_ui.diagnostic_hud_state = Some(hud_state);",
            "(*app).diagnostic_ui.diagnostic_summary = Some(summary);",
            "(*app).diagnostic_ui.diagnostic_hud_separator = Some(hud_separator);",
            "(*app).diagnostic_ui.diagnostic_toolbar_separator = Some(toolbar_separator);",
            "(*app).diagnostic_ui.diagnostic_display_label = Some(display_label);",
            "(*app).diagnostic_ui.diagnostic_display_input = Some(display_input);",
            "(*app).diagnostic_ui.diagnostic_show_all_button = Some(show_all_button);",
            "(*app).diagnostic_ui.diagnostic_category_filter = Some(category_combo);",
            "(*app).diagnostic_ui.diagnostic_search_label = Some(search_label);",
            "(*app).diagnostic_ui.diagnostic_search_input = Some(search_input);",
            "(*app).diagnostic_ui.diagnostic_toolbar_label = Some(label);",
            "(*app).diagnostic_ui.diagnostic_selection_summary = Some(selection_summary);",
            "(*app).diagnostic_ui.diagnostic_range_input = Some(range_input);",
            "(*app).diagnostic_ui.diagnostic_copy_button = Some(copy_button);",
            "(*app).diagnostic_ui.diagnostic_export_button = Some(export_button);",
            "(*app).diagnostic_ui.diagnostic_message = Some(message);",
            "(*app).diagnostic_ui.diagnostic_list = Some(list);",
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
            .find("app.diagnostic_ui.diagnostic_window = Some(window);")
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
            .find("request_diagnostic_refresh(&mut app.diagnostic_ui);")
            .map(|offset| foreground + offset)
            .expect("later diagnostic refresh must be posted after first visibility");
        let ready_check = open_source[parent_assignment..layout]
            .find("if !diagnostic_children_ready(&app.diagnostic_ui)")
            .expect("required diagnostic children must be checked before layout");
        assert!(parent_assignment < layout);
        assert!(ready_check < layout - parent_assignment);
        assert!(layout < first_render);
        assert!(first_render < shown);
        assert!(shown < update);
        assert!(update < foreground);
        assert!(foreground < later_refresh);

        let clear_start = source
            .find("fn clear_diagnostic_state(state: &mut DiagnosticUiState)")
            .expect("diagnostic cleanup function must exist");
        let clear_source = &source[clear_start..];
        for handle in ["diagnostic_hud_separator", "diagnostic_toolbar_separator"] {
            assert!(
                clear_source.contains(&format!("state.{handle} = None;")),
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
}
