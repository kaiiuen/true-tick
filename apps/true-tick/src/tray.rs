use crate::config;
use crate::pause::{
    acquisition_is_allowed, timer_interval_ms, CoordinatorTimerEvent, DurationAction,
    DurationChoice, DurationCoordinator, ScheduleRequest,
};
use std::ffi::c_void;
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tick_core::{DesiredIntent, DesiredIntentQueue};
use tick_diagnostics::{
    diagnostic_grid_rows, diagnostic_grid_rows_for_sequences, format_tsv, latest_row_selection,
    parse_display_limit, parse_row_selection, row_selection_for_sequences,
    selected_event_sequences, selected_event_sequences_for_positions,
    selected_event_sequences_for_sequences, truncate_utf8, visible_positions_for_sequences,
    DiagnosticOutcome, DiagnosticPhase, DiagnosticRecord, DiagnosticSource, DiagnosticStore,
    NativeOutcome, OperationContext, RowSelection, DEFAULT_MAX_EVENTS, REPORT_COLUMNS,
};
use tick_observation_windows::{ObservationSource, WindowsObservation};
use tick_ownership::{OwnershipState, TimerController, TimingSnapshot, Verification};
use tick_platform_windows::{TimerObservation, WindowsTimerPlatform};
use tick_policy::{decide, PolicyInput, PowerState};
use tick_startup_windows::{
    startup_operation, StartupOperation, StartupRegistration, WindowsUserStartup,
};

mod list_view_native {
    use super::{c_void, Point};

    pub const LVS_REPORT: u32 = 0x0001;
    pub const LVS_SINGLESEL: u32 = 0x0004;
    pub const LVS_SHOWSELALWAYS: u32 = 0x0008;
    pub const LVS_EX_GRIDLINES: usize = 0x0000_0001;
    pub const LVS_EX_FULLROWSELECT: usize = 0x0000_0020;
    pub const LVS_EX_DOUBLEBUFFER: usize = 0x0001_0000;
    pub const LVM_FIRST: u32 = 0x1000;
    pub const LVM_DELETEALLITEMS: u32 = LVM_FIRST + 9;
    pub const LVM_GETITEMCOUNT: u32 = LVM_FIRST + 4;
    pub const LVM_GETNEXTITEM: u32 = LVM_FIRST + 12;
    pub const LVM_GETCOLUMNWIDTH: u32 = LVM_FIRST + 29;
    pub const LVM_SETITEMSTATE: u32 = LVM_FIRST + 43;
    pub const LVM_INSERTITEMW: u32 = LVM_FIRST + 77;
    pub const LVM_SETITEMTEXTW: u32 = LVM_FIRST + 116;
    pub const LVM_INSERTCOLUMNW: u32 = LVM_FIRST + 97;
    pub const LVM_SETEXTENDEDLISTVIEWSTYLE: u32 = LVM_FIRST + 54;
    pub const LVM_SETCOLUMNWIDTH: u32 = LVM_FIRST + 30;
    pub const LVSCW_AUTOSIZE: i32 = -1;
    pub const LVSCW_AUTOSIZE_USEHEADER: i32 = -2;
    pub const LVIF_TEXT: u32 = 0x0001;
    pub const LVIF_STATE: u32 = 0x0008;
    pub const LVIS_SELECTED: u32 = 0x0002;
    pub const LVNI_SELECTED: u32 = 0x0002;
    pub const LVCF_WIDTH: u32 = 0x0002;
    pub const LVCF_TEXT: u32 = 0x0004;
    pub const LVCFMT_LEFT: i32 = 0x0000;
    pub const WM_NOTIFY: u32 = 0x004E;
    pub const LVN_ITEMCHANGED: i32 = -101;
    pub const LVN_KEYDOWN: i32 = -155;
    pub const VK_CONTROL: i32 = 0x11;

    #[repr(C)]
    pub struct ListViewColumn {
        pub mask: u32,
        pub format: i32,
        pub width: i32,
        pub text: *mut u16,
        pub text_maximum: i32,
        pub subitem: i32,
        pub image: i32,
        pub order: i32,
        pub minimum_width: i32,
        pub default_width: i32,
        pub ideal_width: i32,
    }

    #[repr(C)]
    pub struct ListViewItem {
        pub mask: u32,
        pub item: i32,
        pub subitem: i32,
        pub state: u32,
        pub state_mask: u32,
        pub text: *mut u16,
        pub text_maximum: i32,
        pub image: i32,
        pub parameter: isize,
        pub indent: i32,
        pub group_id: i32,
        pub columns: u32,
        pub column_indices: *mut u32,
        pub column_formats: *mut i32,
        pub group: i32,
    }

    #[repr(C)]
    pub struct NotifyHeader {
        pub hwnd_from: *mut c_void,
        pub id_from: usize,
        pub code: i32,
    }

    #[repr(C)]
    pub struct ListViewNotification {
        pub header: NotifyHeader,
        pub item: i32,
        pub subitem: i32,
        pub new_state: u32,
        pub old_state: u32,
        pub changed: u32,
        pub action_point: Point,
        pub parameter: isize,
    }

    #[repr(C)]
    pub struct ListViewKeyDownNotification {
        pub header: NotifyHeader,
        pub virtual_key: u16,
        pub flags: u32,
    }
}

use list_view_native::*;

use crate::shutdown::{
    message_loop_exit, shutdown_disposition, MessageLoopExit, ShutdownDisposition, ShutdownGate,
};
use crate::tray_surface::{
    dpi_to_icon_canvas, duration_choices, duration_command, duration_menu_items, icon_pixel_color,
    menu_action_keeps_open, menu_command_dispatch_allowed, menu_command_is_enabled_with_pause,
    menu_description, menu_items_with_duration, power_reconciliation, release_needs_handoff,
    scheduled_display_key, status_menu_items, tooltip_at, tray_notification_opens_menu,
    HandoffProgress, HandoffTracker, PowerReconciliation, TimingValues, TrayStatus,
    CANCEL_PAUSE_COMMAND_ID, CANCEL_SCHEDULED_COMMAND_ID, GITHUB_COMMAND_ID, GITHUB_URL,
    HANDOFF_POLL_INTERVAL_MS, LOGS_COMMAND_ID, PAUSE_FOR_15_COMMAND_ID, PAUSE_FOR_30_COMMAND_ID,
    PAUSE_FOR_5_COMMAND_ID, PAUSE_FOR_60_COMMAND_ID, START_IN_15_COMMAND_ID, START_IN_1_COMMAND_ID,
    START_IN_30_COMMAND_ID, START_IN_5_COMMAND_ID, START_IN_60_COMMAND_ID, STOP_IN_15_COMMAND_ID,
    STOP_IN_1_COMMAND_ID, STOP_IN_30_COMMAND_ID, STOP_IN_5_COMMAND_ID, STOP_IN_60_COMMAND_ID,
};

const WM_APP: u32 = 0x8000;
const WM_TRAY: u32 = WM_APP + 1;
const WM_CREATE: u32 = 0x0001;
const WM_COMMAND: u32 = 0x0111;
const WM_DESTROY: u32 = 0x0002;
const WM_POWERBROADCAST: u32 = 0x0218;
const WM_TIMER: u32 = 0x0113;
const WM_MENUSELECT: u32 = 0x011F;
const WM_DIAGNOSTIC_REFRESH: u32 = WM_APP + 2;
const PBT_APMPOWERSTATUSCHANGE: usize = 0x000A;
const HANDOFF_TIMER_ID: usize = 0x5449;
const DURATION_TIMER_ID_BASE: usize = 0x6000;
const POPUP_REFRESH_TIMER_ID: usize = 0x7000;
/// Popup-only UI cadence for live status refresh.
const POPUP_REFRESH_INTERVAL_MS: u32 = 500;
const SCHEDULE_DISPLAY_TIMER_ID: usize = 0x7100;
/// Bounded tray countdown cadence. The publication key suppresses redundant updates.
const SCHEDULE_DISPLAY_INTERVAL_MS: u32 = 1_000;
const ID_START: usize = 1001;
const ID_STOP: usize = 1002;
const ID_QUIT: usize = 1004;
const ID_STARTUP_ON: usize = 1005;
const ID_STARTUP_OFF: usize = 1006;
const ID_AUTOMATIC_ON: usize = 1007;
const ID_AUTOMATIC_OFF: usize = 1008;
const ID_DIAGNOSTIC_RANGE: usize = 1201;
const ID_DIAGNOSTIC_COPY: usize = 1202;
const ID_DIAGNOSTIC_EXPORT: usize = 1203;
const ID_DIAGNOSTIC_DISPLAY_LIMIT: usize = 1204;
const ID_DIAGNOSTIC_SHOW_ALL: usize = 1205;
const EN_CHANGE: usize = 0x0300;
const BN_CLICKED: usize = 0;
const EM_LIMITTEXT: u32 = WM_USER + 1;
const WS_TABSTOP: u32 = 0x00010000;
const ES_AUTOHSCROLL: u32 = 0x0080;
const BS_PUSHBUTTON: u32 = 0x00000000;
const SS_LEFT: u32 = 0x00000000;

const WM_SIZE: u32 = 0x0005;
const WM_DPICHANGED: u32 = 0x02E0;
const WM_SETREDRAW: u32 = 0x000B;
const WM_PAINT: u32 = 0x000F;
const WM_CLOSE: u32 = 0x0010;
const WM_ERASEBKGND: u32 = 0x0014;
const WM_NCDESTROY: u32 = 0x0082;
const WM_SETFOCUS: u32 = 0x0007;
const WM_GETMINMAXINFO: u32 = 0x0024;
const WM_CTLCOLOREDIT: u32 = 0x0133;
const WM_CTLCOLORSTATIC: u32 = 0x0138;
const WM_SETFONT: u32 = 0x0030;
const GWL_STYLE: i32 = -16;
const GWL_EXSTYLE: i32 = -20;
const WS_OVERLAPPEDWINDOW: u32 = 0x00cf0000;
const WS_VISIBLE: u32 = 0x10000000;
const WS_BORDER: u32 = 0x00800000;
const WS_CLIPCHILDREN: u32 = 0x02000000;
const WS_CLIPSIBLINGS: u32 = 0x04000000;
const WS_EX_TOOLWINDOW: u32 = 0x00000080;
const WS_EX_APPWINDOW: u32 = 0x00040000;
const SW_SHOWNORMAL: i32 = 1;
const SW_RESTORE: i32 = 9;
const DIAGNOSTIC_WINDOW_TITLE: &str = "True™ Tick Status and Diagnostics";
const DIAGNOSTIC_LOADING_SUMMARY: &str = "Loading True™ Tick diagnostics...";
const MAX_STARTUP_STATUS_BYTES: usize = 512;
const DIAGNOSTIC_WINDOW_PARENT: *mut c_void = std::ptr::null_mut();
const DIAGNOSTIC_SUMMARY_HEIGHT: i32 = 220;
const DIAGNOSTIC_TOOLBAR_HEIGHT: i32 = 36;
const DIAGNOSTIC_GRID_MIN_HEIGHT: i32 = 96;
const DIAGNOSTIC_TOOLBAR_MARGIN: i32 = 8;
const DIAGNOSTIC_TOOLBAR_GAP: i32 = 8;
const DIAGNOSTIC_TOOLBAR_MIN_MESSAGE_WIDTH: i32 = 140;
const DIAGNOSTIC_TOOLBAR_CONTROL_WIDTHS: [i32; 8] = [64, 52, 72, 132, 88, 160, 64, 64];
const DIAGNOSTIC_COLUMN_WIDTHS: [i32; 11] = [54, 70, 78, 78, 70, 86, 82, 90, 86, 160, 240];
const DIAGNOSTIC_COLUMN_MIN_WIDTHS: [i32; 11] = [36, 52, 64, 64, 52, 64, 60, 68, 64, 84, 240];
const DIAGNOSTIC_COLUMN_MAX_WIDTHS: [i32; 11] =
    [84, 120, 140, 144, 120, 144, 112, 124, 124, 260, 640];
const DIAGNOSTIC_DEFAULT_WIDTH: i32 = 980;
const DIAGNOSTIC_DEFAULT_HEIGHT: i32 = 600;
const COLOR_WINDOW: i32 = 5;
const COLOR_WINDOWTEXT: i32 = 8;
const DEFAULT_GUI_FONT: i32 = 17;
const DIAGNOSTIC_RANGE_INPUT_LIMIT: usize = 64;
const WS_CHILD: u32 = 0x40000000;
const WS_HSCROLL: u32 = 0x00100000;
const WS_EX_CLIENTEDGE: u32 = 0x00000200;
const SWP_NOZORDER: u32 = 0x0004;
const SWP_NOACTIVATE: u32 = 0x0010;
const SWP_NOSENDCHANGING: u32 = 0x0400;
const SB_HORZ: i32 = 0;

const SS_NOPREFIX: u32 = 0x0000_0080;

const ICC_LISTVIEW_CLASSES: u32 = 0x0000_0001;
// The standard bar-class group includes the native tooltip control.
const ICC_BAR_CLASSES: u32 = 0x0000_0004;
const REQUIRED_COMMON_CONTROL_CLASSES: u32 = ICC_LISTVIEW_CLASSES | ICC_BAR_CLASSES;

const TPM_RIGHTBUTTON: u32 = 0x0002;
const TPM_RETURNCMD: u32 = 0x0100;
const MF_STRING: u32 = 0x0000;
const MF_SEPARATOR: u32 = 0x0800;
const MF_GRAYED: u32 = 0x0001;
const MF_POPUP: u32 = 0x0010;
const MF_BYPOSITION: u32 = 0x0400;

const NIF_MESSAGE: u32 = 0x0001;
const NIF_ICON: u32 = 0x0002;
const NIF_TIP: u32 = 0x0004;
const NIM_ADD: u32 = 0x0000;
const NIM_DELETE: u32 = 0x0002;
const NIM_MODIFY: u32 = 0x0001;
const GWLP_USERDATA: i32 = -21;

const IDI_APPLICATION: usize = 32512;
const MB_ICONWARNING: u32 = 0x0000_0030;
const MB_YESNO: u32 = 0x0000_0004;
const MB_DEFBUTTON2: u32 = 0x0000_0100;
const IDYES: i32 = 6;
const CF_UNICODETEXT: u32 = 13;
const GMEM_MOVEABLE: u32 = 0x0002;
const MOVEFILE_REPLACE_EXISTING: u32 = 0x00000001;
const MOVEFILE_WRITE_THROUGH: u32 = 0x00000008;
const OFN_OVERWRITEPROMPT: u32 = 0x00000002;
const OFN_HIDEREADONLY: u32 = 0x00000004;
const OFN_NOCHANGEDIR: u32 = 0x00000008;
const OFN_PATHMUSTEXIST: u32 = 0x00000800;
const OFN_EXPLORER: u32 = 0x00080000;
const MAX_EXPORT_PATH_UTF16: usize = 32_768;

const ERROR_CLASS_ALREADY_EXISTS: u32 = 1410;
const WS_POPUP: u32 = 0x8000_0000;
const WS_EX_TOPMOST: u32 = 0x0000_0008;
const TTS_ALWAYSTIP: u32 = 0x0001;
const TTS_NOPREFIX: u32 = 0x0002;
const TTF_IDISHWND: u32 = 0x0001;
const TTF_TRACK: u32 = 0x0020;
const TTF_ABSOLUTE: u32 = 0x0080;
const WM_USER: u32 = 0x0400;
const TTM_TRACKACTIVATE: u32 = WM_USER + 17;
const TTM_TRACKPOSITION: u32 = WM_USER + 18;
const TTM_ADDTOOLW: u32 = WM_USER + 50;
const TTM_UPDATETIPTEXTW: u32 = WM_USER + 57;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativeResult {
    Succeeded,
    Failed { raw_error: u32 },
}

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

fn native_bool_result(result: i32, raw_error: u32) -> NativeResult {
    if result == 0 {
        NativeResult::Failed { raw_error }
    } else {
        NativeResult::Succeeded
    }
}

fn native_handle_result(is_null: bool, raw_error: u32) -> NativeResult {
    if is_null {
        NativeResult::Failed { raw_error }
    } else {
        NativeResult::Succeeded
    }
}

fn class_registration_result(atom: u16, raw_error: u32) -> NativeResult {
    if atom != 0 || raw_error == ERROR_CLASS_ALREADY_EXISTS {
        NativeResult::Succeeded
    } else {
        NativeResult::Failed { raw_error }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QuitDecision {
    ExitNormally,
    RequireSafetyDialog { reason: QuitSafetyReason },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QuitDialogDecision {
    StopAndQuit,
    Cancel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct QuitWarningResult {
    decision: QuitDialogDecision,
    message_box_result: Option<i32>,
    dialog_shown: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QuitSafetyReason {
    OwnedActive,
    OwnershipUncertain,
    TimingNotSettled,
}

#[repr(C)]
struct NotifyIconData {
    cb_size: u32,
    h_wnd: *mut c_void,
    u_id: u32,
    u_flags: u32,
    u_callback_message: u32,
    h_icon: *mut c_void,
    sz_tip: [u16; 128],
    dw_state: u32,
    dw_state_mask: u32,
    sz_info: [u16; 256],
    u_timeout_or_version: u32,
    sz_info_title: [u16; 64],
    dw_info_flags: u32,
    guid: [u8; 16],
    h_balloon_icon: *mut c_void,
}

#[repr(C)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct InitCommonControlsEx {
    size: u32,
    classes: u32,
}

fn common_controls_initialization_contract() -> InitCommonControlsEx {
    InitCommonControlsEx {
        size: size_of::<InitCommonControlsEx>() as u32,
        classes: REQUIRED_COMMON_CONTROL_CLASSES,
    }
}

#[repr(C)]
struct ToolInfo {
    cb_size: u32,
    flags: u32,
    hwnd: *mut c_void,
    id: usize,
    rect: Rect,
    instance: *mut c_void,
    text: *const u16,
    l_param: isize,
    reserved: *mut c_void,
}

#[repr(C)]
struct CreateStruct {
    create_params: *mut c_void,
    instance: *mut c_void,
    menu: *mut c_void,
    parent: *mut c_void,
    height: i32,
    width: i32,
    y: i32,
    x: i32,
    style: u32,
    name: *const u16,
    class_name: *const u16,
    extended_style: u32,
}

fn app_create_params(create: *const CreateStruct) -> *mut c_void {
    if create.is_null() {
        std::ptr::null_mut()
    } else {
        unsafe { (*create).create_params }
    }
}

const fn diagnostic_window_style() -> u32 {
    WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN | WS_CLIPSIBLINGS
}

const fn diagnostic_window_extended_style() -> u32 {
    WS_EX_APPWINDOW
}

#[repr(C)]
struct PaintStruct {
    hdc: *mut c_void,
    erase: i32,
    paint: Rect,
    restore: i32,
    inc_update: i32,
    reserved: [u8; 32],
}

#[repr(C)]
struct WndClass {
    style: u32,
    wnd_proc: Option<unsafe extern "system" fn(*mut c_void, u32, usize, isize) -> isize>,
    cls_extra: i32,
    wnd_extra: i32,
    instance: *mut c_void,
    icon: *mut c_void,
    cursor: *mut c_void,
    background: *mut c_void,
    menu_name: *const u16,
    class_name: *const u16,
}

enum StartupTarget {
    PortableLauncher(PathBuf),
    DevelopmentExecutable(PathBuf),
}

impl StartupTarget {
    fn description(&self) -> &'static str {
        match self {
            Self::PortableLauncher(_) => "portable Launcher.exe entry point",
            Self::DevelopmentExecutable(_) => "debug true-tick.exe development fallback",
        }
    }
}

fn startup_target(executable: &Path) -> Result<StartupTarget, String> {
    match crate::portable::launcher_path_from_slot_executable(executable) {
        Ok(launcher) => Ok(StartupTarget::PortableLauncher(launcher)),
        Err(_error)
            if cfg!(debug_assertions)
                && tick_startup_windows::is_development_executable(executable) =>
        {
            Ok(StartupTarget::DevelopmentExecutable(
                executable.to_path_buf(),
            ))
        }
        Err(error) => Err(format!("portable launcher path unavailable {error:?}")),
    }
}

fn register_startup_target(
    target: &StartupTarget,
) -> Result<(), tick_startup_windows::StartupError> {
    let mut startup = WindowsUserStartup;
    match target {
        StartupTarget::PortableLauncher(path) => startup.register(path),
        StartupTarget::DevelopmentExecutable(path) => startup.register_development(path),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PopupMenuHandles {
    root: *mut c_void,
    duration: Option<*mut c_void>,
    status: Option<*mut c_void>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PublicationKey {
    status: TrayStatus,
    ownership: OwnershipState,
    effective: Option<tick_core::Hns>,
    requested: Option<tick_core::Hns>,
    handoff: bool,
    scheduled: Option<(DurationAction, u64, u64)>,
}

struct App {
    controller: TimerController<WindowsTimerPlatform>,
    observation: WindowsObservation,
    config: config::Config,
    tray_status: TrayStatus,
    config_path: PathBuf,
    executable: PathBuf,
    startup_status: String,
    tray_icon: Option<NotifyIconData>,
    timing_snapshot: TimingSnapshot,
    timing_snapshot_valid: bool,
    invalid_interval: bool,
    external_timing: bool,
    desired_intent: DesiredIntentQueue,
    diagnostics: Arc<DiagnosticStore>,
    diagnostic_window: Option<*mut c_void>,
    diagnostic_summary: Option<*mut c_void>,
    diagnostic_display_label: Option<*mut c_void>,
    diagnostic_display_input: Option<*mut c_void>,
    diagnostic_show_all_button: Option<*mut c_void>,
    diagnostic_toolbar_label: Option<*mut c_void>,
    diagnostic_selection_summary: Option<*mut c_void>,
    diagnostic_range_input: Option<*mut c_void>,
    diagnostic_copy_button: Option<*mut c_void>,
    diagnostic_export_button: Option<*mut c_void>,
    diagnostic_message: Option<*mut c_void>,
    diagnostic_list: Option<*mut c_void>,
    diagnostic_selection: Option<RowSelection>,
    diagnostic_selection_sequences: Option<Vec<u64>>,
    diagnostic_grid_selection_sequences: Vec<u64>,
    diagnostic_grid_selection_reset: bool,
    diagnostic_selection_reset: bool,
    diagnostic_display_limit: usize,
    diagnostic_display_all: bool,
    diagnostic_message_text: String,
    diagnostic_refresh_pending: bool,
    diagnostic_refreshing: bool,
    diagnostic_layout_stable: bool,
    diagnostic_snapshot_key: Option<(usize, u64)>,
    diagnostic_snapshot_generation: u64,
    diagnostic_auto_fit_generation: Option<u64>,
    menu_active: bool,
    popup_menus: Option<PopupMenuHandles>,
    popup_refresh_timer_active: bool,
    schedule_display_timer_active: bool,
    handoff: Option<HandoffTracker>,
    menu_help: Option<*mut c_void>,
    menu_help_text: Vec<u16>,
    pause: DurationCoordinator,
    duration_timer_id: Option<usize>,
    duration_timer_generation: Option<u64>,
    scheduled_operation: Option<OperationContext>,
    running_since: Option<std::time::Instant>,
    shutdown_gate: ShutdownGate,
    operation: Option<OperationContext>,
    operation_source: DiagnosticSource,
    handoff_operation: Option<OperationContext>,
    last_publication: Option<PublicationKey>,
}

pub fn run() {
    unsafe {
        let diagnostics = Arc::new(DiagnosticStore::new(DEFAULT_MAX_EVENTS));
        diagnostics.record("lifecycle.start", "application_start");
        let executable = get_module_file_name_w_path();
        diagnostics.record("lifecycle.executable_observed", "path=redacted");
        if let Ok(root) = crate::portable::portable_root_from_slot_executable(&executable) {
            let selection = crate::portable::select(&root);
            diagnostics.record(
                "portable.active_slot.selection",
                format!("result={}", selection.description()),
            );
        } else {
            diagnostics.record(
                "portable.active_slot.selection",
                "result=not_portable_slot_layout",
            );
        }
        let config_path = config::path_from_executable(&executable);
        let (loaded, config_status, config_migrated) =
            match config::load_with_migration(&config_path) {
                Ok((config, migrated)) => {
                    diagnostics.record(
                        "config.load.result",
                        format!("result=success migrated={migrated}"),
                    );
                    (config, None, migrated)
                }
                Err(error) => {
                    diagnostics.record("config.load.result", format!("result=error error={error}"));
                    (
                        config::Config::default(),
                        Some(format!("Red: {error}")),
                        false,
                    )
                }
            };
        if config_migrated {
            diagnostics.record(
                "config.migration",
                format!(
                    "legacy_request_hns={} result=automatic_selection",
                    config::LEGACY_ONE_MILLISECOND_REQUEST_INTERVAL.value()
                ),
            );
        }
        diagnostics.record(
            "startup.decision",
            format!(
                "enabled={} operation={:?}",
                loaded.startup_enabled,
                startup_operation(loaded.startup_enabled)
            ),
        );
        let startup_status = match (config_status, startup_operation(loaded.startup_enabled)) {
            (Some(error), _) => error,
            (None, StartupOperation::Register) => match startup_target(&executable) {
                Ok(target) => {
                    diagnostics
                        .record("native.RegSetValueExW.call", "value=TrueTick path=redacted");
                    match register_startup_target(&target) {
                        Ok(()) => {
                            diagnostics.record(
                                "startup.registration.result",
                                format!("result=success target={}", target.description()),
                            );
                            format!(
                                "boot startup registered for current-user {}",
                                target.description()
                            )
                        }
                        Err(error) => {
                            diagnostics.record(
                                "startup.registration.result",
                                format!("result=error error={error:?}"),
                            );
                            format!("Red: boot startup registration error {error:?}")
                        }
                    }
                }
                Err(error) => {
                    diagnostics.record(
                        "startup.launcher_slot_selection",
                        format!("result=error error={error}"),
                    );
                    format!("Red: {error}")
                }
            },
            (None, StartupOperation::Remove) => {
                diagnostics.record("native.RegDeleteValueW.call", "value=TrueTick");
                match WindowsUserStartup.remove() {
                    Ok(()) => {
                        diagnostics.record("startup.registration.result", "result=removed");
                        "boot startup registration disabled by config".to_owned()
                    }
                    Err(error) => {
                        diagnostics.record(
                            "startup.registration.result",
                            format!("result=error error={error:?}"),
                        );
                        format!("Red: boot startup removal error {error:?}")
                    }
                }
            }
        };

        let mut observation = WindowsObservation::default();
        let power_observation_error = match observation.refresh_power() {
            Ok(snapshot) => {
                diagnostics.record(
                    "power.initial_observation",
                    format!("result=success state={:?}", snapshot.state),
                );
                None
            }
            Err(error) => {
                diagnostics.record(
                    "power.initial_observation",
                    format!("result=error reason={error}"),
                );
                Some(error)
            }
        };
        let startup_status = bounded_startup_status(startup_status);
        let app = Box::new(App {
            controller: TimerController::new(
                WindowsTimerPlatform::with_diagnostics(diagnostics.clone()),
                loaded.request_interval,
            ),
            observation,
            config: loaded,
            tray_status: if power_observation_error.is_some() {
                TrayStatus::Degraded
            } else {
                TrayStatus::Stopped
            },
            config_path,
            executable,
            startup_status,
            tray_icon: None,
            timing_snapshot: TimingSnapshot::default(),
            timing_snapshot_valid: false,
            invalid_interval: false,
            external_timing: false,
            desired_intent: DesiredIntentQueue::new(),
            diagnostics,
            diagnostic_window: None,
            diagnostic_summary: None,
            diagnostic_display_label: None,
            diagnostic_display_input: None,
            diagnostic_show_all_button: None,
            diagnostic_toolbar_label: None,
            diagnostic_selection_summary: None,
            diagnostic_range_input: None,
            diagnostic_copy_button: None,
            diagnostic_export_button: None,
            diagnostic_message: None,
            diagnostic_list: None,
            diagnostic_selection: None,
            diagnostic_selection_sequences: None,
            diagnostic_grid_selection_sequences: Vec::new(),
            diagnostic_grid_selection_reset: false,
            diagnostic_selection_reset: false,
            diagnostic_display_limit: 100,
            diagnostic_display_all: false,
            diagnostic_message_text: String::new(),
            diagnostic_refresh_pending: false,
            diagnostic_refreshing: false,
            diagnostic_layout_stable: false,
            diagnostic_snapshot_key: None,
            diagnostic_snapshot_generation: 0,
            diagnostic_auto_fit_generation: None,
            menu_active: false,
            popup_menus: None,
            popup_refresh_timer_active: false,
            schedule_display_timer_active: false,
            handoff: None,
            menu_help: None,
            menu_help_text: Vec::new(),
            pause: DurationCoordinator::new(),
            duration_timer_id: None,
            duration_timer_generation: None,
            scheduled_operation: None,
            running_since: None,
            shutdown_gate: ShutdownGate::new(),
            operation: None,
            operation_source: DiagnosticSource::Internal,
            handoff_operation: None,
            last_publication: None,
        });
        let app_ptr = Box::into_raw(app);
        let app = &mut *app_ptr;
        app.begin_operation(DiagnosticSource::Startup);
        let class_name = wide("TrueTickTrayClass");
        let instance = GetModuleHandleW(std::ptr::null());
        if instance.is_null() {
            app.record(
                "native.GetModuleHandleW.error",
                format!("raw_status={}", GetLastError()),
            );
            abort_startup(
                app_ptr,
                "True™ Tick could not obtain its native module handle.",
            );
            return;
        }
        match initialize_common_controls() {
            Ok(()) => app.record(
                "native.InitCommonControlsEx.result",
                format!("result=success classes=list-view+tooltip flags={REQUIRED_COMMON_CONTROL_CLASSES:#x}"),
            ),
            Err(raw_error) => {
                app.record(
                    "native.InitCommonControlsEx.error",
                    format!("classes=list-view+tooltip flags={REQUIRED_COMMON_CONTROL_CLASSES:#x} raw_status={raw_error}"),
                );
                app.record(
                    "diagnostic.window.result",
                    format!("result=common_controls_initialization_failed raw_status={raw_error}"),
                );
                abort_startup(
                    app_ptr,
                    &format!(
                        "True™ Tick could not initialize Windows Common Controls for the diagnostic window. Native error code: {raw_error}."
                    ),
                );
                return;
            }
        }
        let wnd_class = WndClass {
            style: 0,
            wnd_proc: Some(window_proc),
            cls_extra: 0,
            wnd_extra: 0,
            instance,
            icon: LoadIconW(std::ptr::null_mut(), IDI_APPLICATION as *const u16),
            cursor: std::ptr::null_mut(),
            background: std::ptr::null_mut(),
            menu_name: std::ptr::null(),
            class_name: class_name.as_ptr(),
        };
        let tray_class_atom = RegisterClassW(&wnd_class);
        let tray_class_error = if tray_class_atom == 0 {
            GetLastError()
        } else {
            0
        };
        if !matches!(
            class_registration_result(tray_class_atom, tray_class_error),
            NativeResult::Succeeded
        ) {
            app.record(
                "native.RegisterClassW.tray.error",
                format!("raw_status={tray_class_error}"),
            );
            abort_startup(
                app_ptr,
                "True™ Tick could not create its tray window class.",
            );
            return;
        }
        if wnd_class.icon.is_null() {
            app.record(
                "native.LoadIconW.error",
                format!("raw_status={}", GetLastError()),
            );
            abort_startup(
                app_ptr,
                "True™ Tick could not create its tray icon resource.",
            );
            return;
        }
        let diagnostic_class_name = wide("TrueTickDiagnosticClass");
        let diagnostic_class = WndClass {
            style: 0,
            wnd_proc: Some(diagnostic_window_proc),
            cls_extra: 0,
            wnd_extra: 0,
            instance: wnd_class.instance,
            icon: wnd_class.icon,
            cursor: std::ptr::null_mut(),
            background: GetSysColorBrush(COLOR_WINDOW),
            menu_name: std::ptr::null(),
            class_name: diagnostic_class_name.as_ptr(),
        };
        let diagnostic_class_atom = RegisterClassW(&diagnostic_class);
        let diagnostic_class_error = if diagnostic_class_atom == 0 {
            GetLastError()
        } else {
            0
        };
        if !matches!(
            class_registration_result(diagnostic_class_atom, diagnostic_class_error),
            NativeResult::Succeeded
        ) {
            app.record(
                "native.RegisterClassW.diagnostic.error",
                format!("raw_status={diagnostic_class_error}"),
            );
            abort_startup(
                app_ptr,
                "True™ Tick could not create its diagnostic window class.",
            );
            return;
        }
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            class_name.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            wnd_class.instance,
            app_ptr as *mut c_void,
        );
        let hwnd_error = if hwnd.is_null() { GetLastError() } else { 0 };
        if !matches!(
            native_handle_result(hwnd.is_null(), hwnd_error),
            NativeResult::Succeeded
        ) {
            app.record(
                "native.CreateWindowExW.tray.error",
                format!("raw_status={hwnd_error}"),
            );
            abort_startup(app_ptr, "True™ Tick could not create its tray window.");
            return;
        }
        let mut icon = match NotifyIconData::new(
            hwnd,
            app.lifecycle_status(),
            app.timing_values(),
            app.pause.current(),
        ) {
            Ok(icon) => icon,
            Err(raw_error) => {
                app.record(
                    "native.tray_icon.create.error",
                    format!("raw_status={raw_error}"),
                );
                abort_after_window(app_ptr, hwnd, "True™ Tick could not create its tray icon.");
                return;
            }
        };
        let add_result = Shell_NotifyIconW(NIM_ADD, &mut icon);
        let add_error = if add_result == 0 { GetLastError() } else { 0 };
        if matches!(
            native_bool_result(add_result, add_error),
            NativeResult::Failed { .. }
        ) {
            app.record(
                "native.Shell_NotifyIconW.add.error",
                format!("raw_status={add_error}"),
            );
            drop(icon);
            abort_after_window(app_ptr, hwnd, "True™ Tick could not add its tray icon.");
            return;
        }
        app.tray_icon = Some(icon);
        app.record("policy.recalculate", "trigger=startup");
        refresh_timing_observation(app);
        if app.config.automatic {
            app.record("policy.startup_automatic", "enabled=true");
        }
        apply_power_reconciliation(app);
        app.finish_operation(DiagnosticOutcome::Completed);
        loop {
            let message_loop_exit = run_message_loop();
            let cleanup_result = app.cleanup_normal_shutdown();
            let cleanup_verified = cleanup_result.is_ok();
            let cleanup_error = cleanup_result.err();
            let ui_usable = !hwnd.is_null() && IsWindow(hwnd) != 0;
            match shutdown_disposition(message_loop_exit, cleanup_verified, ui_usable) {
                ShutdownDisposition::KeepAliveForRetry => {
                    let error = cleanup_error.unwrap_or_else(|| "cleanup unresolved".to_owned());
                    app.tray_status = TrayStatus::Unverified;
                    app.record(
                        "shutdown.retry_required",
                        format!("reason=cleanup_unresolved error={error}"),
                    );
                    app.publish();
                    show_shutdown_warning(
                        hwnd,
                        &format!(
                            "True™ Tick could not verify timer cleanup. The app remains open so cleanup can be retried.\n\n{error}"
                        ),
                    );
                }
                ShutdownDisposition::Complete => {
                    app.record("lifecycle.shutdown", "result=normal_cleanup_verified");
                    break;
                }
                ShutdownDisposition::ExitAfterMessageLoopError => {
                    let MessageLoopExit::GetMessageFailed { raw_error } = message_loop_exit else {
                        unreachable!()
                    };
                    app.record(
                        "lifecycle.message_loop.error",
                        format!("native.GetMessageW raw_error={raw_error}"),
                    );
                    app.record(
                        "lifecycle.shutdown",
                        "result=message_loop_error cleanup_verified",
                    );
                    show_shutdown_warning(
                        hwnd,
                        &format!(
                            "True™ Tick message handling failed and the app must exit. Native error code: {raw_error}."
                        ),
                    );
                    break;
                }
                ShutdownDisposition::ExitWithUnresolvedCleanup => {
                    let error = cleanup_error.unwrap_or_else(|| "cleanup unresolved".to_owned());
                    if let MessageLoopExit::GetMessageFailed { raw_error } = message_loop_exit {
                        app.record(
                            "lifecycle.message_loop.error",
                            format!("native.GetMessageW raw_error={raw_error}"),
                        );
                    }
                    app.record(
                        "lifecycle.shutdown",
                        format!("result=exit_with_unresolved_cleanup error={error}"),
                    );
                    show_shutdown_warning(
                        hwnd,
                        &format!(
                            "True™ Tick must exit before cleanup could be verified. Native cleanup state is unresolved.\n\n{error}"
                        ),
                    );
                    break;
                }
            }
        }
        remove_tray_icon(app);
        destroy_diagnostic_window(app);
        if !hwnd.is_null() && IsWindow(hwnd) != 0 {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            DestroyWindow(hwnd);
        }
        app.record(
            "lifecycle.shutdown.resources",
            "result=destroyed_before_app_drop",
        );
        drop(Box::from_raw(app_ptr));
    }
}

unsafe fn abort_startup(app_ptr: *mut App, message: &str) {
    let app = Box::from_raw(app_ptr);
    app.diagnostics.record("lifecycle.startup.abort", message);
    drop(app);
    show_shutdown_warning(std::ptr::null_mut(), message);
}

unsafe fn abort_after_window(app_ptr: *mut App, hwnd: *mut c_void, message: &str) {
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
    if IsWindow(hwnd) != 0 {
        DestroyWindow(hwnd);
    }
    let app = Box::from_raw(app_ptr);
    app.diagnostics.record("lifecycle.startup.abort", message);
    drop(app);
    show_shutdown_warning(std::ptr::null_mut(), message);
}

unsafe fn run_message_loop() -> MessageLoopExit {
    let mut message = Message::default();
    loop {
        let result = GetMessageW(&mut message, std::ptr::null_mut(), 0, 0);
        if result > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
            continue;
        }
        let raw_error = if result < 0 { GetLastError() } else { 0 };
        return message_loop_exit(result, raw_error)
            .expect("GetMessageW returned an invalid result");
    }
}

unsafe fn remove_tray_icon(app: &mut App) {
    if let Some(mut icon) = app.tray_icon.take() {
        let result = Shell_NotifyIconW(NIM_DELETE, &mut icon);
        app.record(
            "native.Shell_NotifyIconW.delete",
            format!(
                "result={} raw_status={}",
                result != 0,
                if result == 0 { GetLastError() } else { 0 }
            ),
        );
    }
}

unsafe fn destroy_diagnostic_window(app: &mut App) {
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
}

unsafe fn show_shutdown_warning(hwnd: *mut c_void, message: &str) {
    let text = wide(message);
    let title = wide("True™ Tick shutdown warning");
    MessageBoxW(hwnd, text.as_ptr(), title.as_ptr(), MB_ICONWARNING);
}

fn bounded_startup_status(value: impl AsRef<str>) -> String {
    truncate_utf8(value.as_ref(), MAX_STARTUP_STATUS_BYTES)
}

fn reconcile(app: &mut App) {
    app.record("policy.recalculate", "trigger=reconcile");
    refresh_timing_observation(app);
    apply_policy(app);
}

fn apply_policy(app: &mut App) {
    let power = app.observation.power().state;
    let decision = decide(PolicyInput {
        enabled: true,
        eligible_profile: true,
        power,
    });
    app.record(
        "policy.evaluation",
        format!(
            "power={power:?} status={:?} reason={:?}",
            decision.status, decision.reason
        ),
    );
    let intent = if app.pause.pause_active() {
        app.record(
            "policy.pause_suppressed",
            format!("requested_status={:?} reason=pause_active", decision.status),
        );
        DesiredIntent::Release
    } else if let Some(scheduled) = app.pause.current() {
        match scheduled.action {
            DurationAction::Start | DurationAction::Stop
                if decision.status != tick_core::Status::Requested =>
            {
                app.record(
                    "policy.duration.suppressed",
                    format!(
                        "action={} reason={:?} result=release_safe timing_snapshot={}",
                        scheduled.action.label(),
                        decision.reason,
                        app.timing_snapshot_details()
                    ),
                );
                return release_for_policy(app, decision.reason);
            }
            DurationAction::Start | DurationAction::Stop => {
                app.record(
                    "policy.duration.deferred",
                    format!(
                        "action={} reason=scheduled_action deadline_remaining_ms={}",
                        scheduled.action.label(),
                        scheduled.remaining(std::time::Instant::now()).as_millis()
                    ),
                );
                show_ownership_status(app, TrayStatus::Stopped);
                return;
            }
            DurationAction::Pause => DesiredIntent::Release,
        }
    } else if acquisition_is_allowed(false, app.config.automatic, power) {
        DesiredIntent::Acquire
    } else {
        DesiredIntent::Release
    };
    if app.handoff.is_some() {
        app.record(
            "policy.recalculate.deferred",
            format!("reason=handoff_pending queued_intent={intent:?}"),
        );
    }
    queue_intent(app, intent, format!("policy reason={:?}", decision.reason));
}

fn queue_intent(app: &mut App, intent: DesiredIntent, source: impl AsRef<str>) {
    app.desired_intent.request(intent);
    app.record(
        "lifecycle.desired_intent",
        format!("intent={intent:?} source={}", source.as_ref()),
    );
    if app.handoff.is_some() {
        app.tray_status = if app.pause.pause_active() {
            TrayStatus::Pausing
        } else {
            TrayStatus::Stopping
        };
        app.publish();
        return;
    }
    process_desired_intent(app);
}

fn process_desired_intent(app: &mut App) {
    if app.handoff.is_some() {
        app.tray_status = if app.pause.pause_active() {
            TrayStatus::Pausing
        } else {
            TrayStatus::Stopping
        };
        app.publish();
        return;
    }
    let Some(intent) = app.desired_intent.take() else {
        return;
    };
    match intent {
        DesiredIntent::Acquire => {
            if app.pause.pause_active() {
                app.record(
                    "lifecycle.acquire.suppressed",
                    "reason=pause_active result=suppressed",
                );
                if app.controller.ownership() == OwnershipState::Released {
                    app.tray_status = TrayStatus::Paused;
                    app.publish();
                    arm_duration_timer(app);
                }
                return;
            }
            let power = app.observation.power().state;
            let decision = decide(PolicyInput {
                enabled: true,
                eligible_profile: true,
                power,
            });
            if decision.status == tick_core::Status::Requested {
                acquire_timer(app);
            } else {
                release_for_policy(app, decision.reason);
            }
        }
        DesiredIntent::Release => {
            if app.tray_status == TrayStatus::Starting {
                app.desired_intent.request(DesiredIntent::Release);
                app.record(
                    "lifecycle.release_queued",
                    "reason=acquisition_verification_pending",
                );
            } else if matches!(
                app.controller.ownership(),
                OwnershipState::Owned | OwnershipState::Uncertain
            ) {
                let _ = guarded_release(
                    app,
                    "desired intent",
                    if app.pause.pause_active() {
                        TrayStatus::Paused
                    } else {
                        TrayStatus::Stopped
                    },
                );
            } else {
                app.tray_status =
                    if app.pause.pause_active() && app.tray_status != TrayStatus::Unverified {
                        TrayStatus::Paused
                    } else {
                        TrayStatus::Stopped
                    };
                app.publish();
                if app.pause.pause_active() {
                    arm_duration_timer(app);
                }
            }
        }
    }
}

fn acquire_timer(app: &mut App) {
    if app.controller.ownership() != OwnershipState::Released {
        show_ownership_status(app, TrayStatus::Stopped);
        return;
    }
    app.tray_status = TrayStatus::Starting;
    app.publish();
    app.record("ownership.acquire.request", "source=desired_intent");
    let start_result = app.controller.start();
    app.sync_timing_snapshot();
    app.timing_snapshot_valid = matches!(
        start_result,
        Ok(Verification::Verified | Verification::FinerThanRequested)
    );
    app.invalid_interval = matches!(
        start_result,
        Err(tick_platform_windows::TimerError::InvalidInterval)
    );
    let status = match start_result {
        Ok(Verification::Verified | Verification::FinerThanRequested) => {
            app.record("verification.result", format!("result={start_result:?}"));
            app.record("ownership.changed", "state=owned");
            TrayStatus::Running
        }
        Ok(verification) => {
            app.record("verification.result", format!("result={verification:?}"));
            TrayStatus::Unverified
        }
        Err(error) => {
            app.record("ownership.acquire.error", format!("error={error:?}"));
            match error {
                tick_platform_windows::TimerError::Unsupported => TrayStatus::Unsupported,
                _ => TrayStatus::Error,
            }
        }
    };
    if status == TrayStatus::Running {
        app.running_since = Some(std::time::Instant::now());
    } else {
        app.running_since = None;
    }
    app.tray_status = status;
    app.publish();
    if app.desired_intent.pending().is_some() {
        process_desired_intent(app);
    }
}

fn release_for_policy(app: &mut App, reason: tick_policy::PolicyReason) {
    if app.handoff.is_some() {
        queue_intent(
            app,
            DesiredIntent::Release,
            format!("policy reason={reason:?}"),
        );
        return;
    }
    let released_status = match reason {
        tick_policy::PolicyReason::BatteryRestricted
        | tick_policy::PolicyReason::BatterySaverRestricted
        | tick_policy::PolicyReason::PowerUnknown
        | tick_policy::PolicyReason::GloballyDisabled => TrayStatus::Blocked,
        tick_policy::PolicyReason::NoEligibleProfile
        | tick_policy::PolicyReason::EligibleProfile => TrayStatus::Stopped,
    };
    if matches!(
        app.controller.ownership(),
        OwnershipState::Owned | OwnershipState::Uncertain
    ) {
        let _ = guarded_release(app, format!("policy reason={reason:?}"), released_status);
    } else {
        app.tray_status = released_status;
        app.publish();
    }
}

fn guarded_release(
    app: &mut App,
    source: impl AsRef<str>,
    released_status: TrayStatus,
) -> Result<bool, String> {
    if app.handoff.is_some() {
        app.record(
            "ownership.release.repeated",
            format!("source={} result=handoff_already_pending", source.as_ref()),
        );
        return Err("handoff already pending".to_owned());
    }
    app.record(
        "ownership.release.request",
        format!("source={}", source.as_ref()),
    );
    app.external_timing = false;
    app.running_since = None;
    app.tray_status = if app.pause.pause_active() {
        TrayStatus::Pausing
    } else {
        TrayStatus::Stopping
    };
    app.publish();
    let stop_result = app.controller.stop();
    app.sync_timing_snapshot();
    app.timing_snapshot_valid = stop_result.is_ok();
    match stop_result {
        Ok(released) => {
            app.record(
                "ownership.changed",
                format!("state=released changed={released}"),
            );
            let effective = app.timing_snapshot.effective;
            let boundary = app.timing_snapshot.requested;
            if released && boundary.is_some_and(|value| release_needs_handoff(value, effective)) {
                let boundary = boundary.expect("boundary checked above");
                if begin_handoff(
                    app,
                    boundary,
                    if app.pause.pause_active() {
                        TrayStatus::Paused
                    } else {
                        TrayStatus::Stopped
                    },
                ) {
                    return Ok(released);
                }
            }
            app.record(
                "ownership.result",
                format!("state=released effective_system={effective:?} handoff=not_required"),
            );
            app.controller.clear_release_boundary();
            app.sync_timing_snapshot();
            app.timing_snapshot_valid = true;
            app.tray_status = released_status;
            app.publish();
            if app.pause.pause_active() {
                arm_duration_timer(app);
            }
            Ok(released)
        }
        Err(error) => {
            app.timing_snapshot_valid = false;
            app.running_since = None;
            app.record("ownership.release.error", format!("error={error:?}"));
            app.tray_status = TrayStatus::Unverified;
            app.publish();
            Err(format!("{error:?}"))
        }
    }
}

fn manual_start(app: &mut App) {
    app.record("tray.command", "command=start");
    app.record("lifecycle.start_request", "source=manual");
    queue_intent(app, DesiredIntent::Acquire, "manual");
}

fn manual_stop(app: &mut App) {
    app.record("tray.command", "command=stop");
    app.record("lifecycle.stop_request", "source=manual");
    queue_intent(app, DesiredIntent::Release, "manual");
}

fn schedule_duration_action(app: &mut App, action: DurationAction, duration: DurationChoice) {
    let now = std::time::Instant::now();
    let replacing_pause = app.pause.pause_active();
    if let Some(previous) = app.pause.current() {
        app.record(
            "duration.schedule.replacement",
            format!(
                "previous_action={} previous_duration_minutes={} previous_remaining_ms={} result=replacing",
                previous.action.label(),
                previous.duration.minutes(),
                previous.remaining(now).as_millis()
            ),
        );
        app.record(
            "duration.schedule.replacement.timing",
            app.timing_snapshot_details(),
        );
    }
    cancel_duration_timer(app);
    let request = app.pause.schedule(action, duration, now);
    let current = match request {
        ScheduleRequest::Started(current) => current,
        ScheduleRequest::Replaced { current, .. } => current,
    };
    app.scheduled_operation = app.operation;
    app.record(
        "duration.schedule",
        format!(
            "action={} duration_minutes={} generation={} deadline_monotonic_ms={} remaining_ms={}",
            action.label(),
            duration.minutes(),
            current.generation,
            duration.duration().as_millis(),
            current.remaining(now).as_millis()
        ),
    );
    app.record("duration.schedule.timing", app.timing_snapshot_details());
    app.record(
        "duration.schedule.policy",
        "policy_result=deferred safety_recheck=deadline",
    );
    if !arm_duration_timer(app) {
        let _ = app.pause.cancel();
        app.scheduled_operation = None;
        app.record(
            "duration.schedule.result",
            "outcome=failed reason=coordinator_timer_unavailable",
        );
        app.tray_status = TrayStatus::Error;
        app.publish();
        return;
    }
    begin_schedule_display_timer(app);
    app.record(
        "duration.schedule.result",
        format!(
            "outcome=scheduled action={} duration_minutes={} generation={} remaining_ms={}",
            action.label(),
            duration.minutes(),
            current.generation,
            current.remaining(std::time::Instant::now()).as_millis()
        ),
    );
    if replacing_pause && action != DurationAction::Pause {
        show_ownership_status(app, TrayStatus::Stopped);
    }
    if action == DurationAction::Pause {
        app.tray_status = TrayStatus::Pausing;
        app.publish();
        queue_intent(app, DesiredIntent::Release, "duration pause");
    } else {
        app.publish();
    }
    if app.menu_active {
        unsafe { refresh_popup_menu(app) };
    }
}

fn cancel_scheduled_action(app: &mut App) {
    let now = std::time::Instant::now();
    let Some(previous) = app.pause.current() else {
        app.record(
            "duration.cancel",
            "outcome=noop reason=no_scheduled_action remaining_ms=0",
        );
        if app.menu_active {
            unsafe { refresh_popup_menu(app) };
        }
        return;
    };
    app.record(
        "duration.cancel.request",
        format!(
            "action={} duration_minutes={} generation={} remaining_ms={}",
            previous.action.label(),
            previous.duration.minutes(),
            previous.generation,
            previous.remaining(now).as_millis()
        ),
    );
    app.record("duration.cancel.timing", app.timing_snapshot_details());
    cancel_duration_timer(app);
    kill_schedule_display_timer(app);
    let cancelled = app.pause.cancel().expect("scheduled action was checked");
    app.scheduled_operation = None;
    app.record(
        "duration.cancel",
        format!(
            "outcome=cancelled action={} duration_minutes={} generation={} remaining_ms=0",
            cancelled.action.label(),
            cancelled.duration.minutes(),
            cancelled.generation
        ),
    );
    refresh_power_for_duration(app, "cancel");
    apply_power_reconciliation(app);
    if app.menu_active {
        unsafe { refresh_popup_menu(app) };
    }
}

fn duration_timer_id(generation: u64) -> usize {
    DURATION_TIMER_ID_BASE.saturating_add(generation as usize)
}

fn cancel_duration_timer(app: &mut App) {
    let Some(timer_id) = app.duration_timer_id.take() else {
        app.duration_timer_generation = None;
        return;
    };
    app.duration_timer_generation = None;
    if let Some(hwnd) = app.tray_icon.as_ref().map(|icon| icon.h_wnd) {
        if unsafe { KillTimer(hwnd, timer_id) } == 0 {
            app.record(
                "native.KillTimer.duration.error",
                format!("timer_id={timer_id} raw_status={}", unsafe {
                    GetLastError()
                }),
            );
        }
    }
}

fn begin_schedule_display_timer(app: &mut App) {
    if app.schedule_display_timer_active {
        return;
    }
    let Some(hwnd) = app.tray_icon.as_ref().map(|icon| icon.h_wnd) else {
        app.record(
            "duration.display_timer",
            "outcome=unavailable reason=tray_window_missing",
        );
        return;
    };
    let result = unsafe {
        SetTimer(
            hwnd,
            SCHEDULE_DISPLAY_TIMER_ID,
            SCHEDULE_DISPLAY_INTERVAL_MS,
            std::ptr::null_mut(),
        )
    };
    if result == 0 {
        app.record(
            "native.SetTimer.duration_display.error",
            format!("raw_status={}", unsafe { GetLastError() }),
        );
    } else {
        app.schedule_display_timer_active = true;
    }
}

fn kill_schedule_display_timer(app: &mut App) {
    if !app.schedule_display_timer_active {
        return;
    }
    app.schedule_display_timer_active = false;
    if let Some(hwnd) = app.tray_icon.as_ref().map(|icon| icon.h_wnd) {
        unsafe {
            let _ = KillTimer(hwnd, SCHEDULE_DISPLAY_TIMER_ID);
        }
    }
}

fn handle_schedule_display_timer(app: &mut App) {
    if app.pause.active() {
        app.publish();
    } else {
        kill_schedule_display_timer(app);
    }
}

fn arm_duration_timer(app: &mut App) -> bool {
    let Some(deadline) = app.pause.deadline() else {
        return false;
    };
    let Some(hwnd) = app.tray_icon.as_ref().map(|icon| icon.h_wnd) else {
        app.record(
            "duration.timer",
            "outcome=unavailable reason=tray_window_missing",
        );
        return false;
    };
    let generation = app.pause.generation();
    let timer_id = duration_timer_id(generation);
    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
    let result = unsafe {
        SetTimer(
            hwnd,
            timer_id,
            timer_interval_ms(remaining),
            std::ptr::null_mut(),
        )
    };
    if result == 0 {
        app.record(
            "native.SetTimer.duration.error",
            format!("generation={generation} raw_status={}", unsafe {
                GetLastError()
            }),
        );
        let _ = app.pause.cancel();
        kill_schedule_display_timer(app);
        app.duration_timer_id = None;
        app.duration_timer_generation = None;
        app.tray_status = TrayStatus::Error;
        app.publish();
        return false;
    }
    app.duration_timer_id = Some(timer_id);
    app.duration_timer_generation = Some(generation);
    app.record(
        "duration.timer.armed",
        format!(
            "generation={generation} remaining_ms={} interval_ms={}",
            remaining.as_millis(),
            timer_interval_ms(remaining)
        ),
    );
    true
}

fn execute_scheduled_action(app: &mut App, action: crate::pause::ScheduledAction) {
    if let Some(parent) = app.scheduled_operation.take() {
        app.begin_child_operation(parent, DiagnosticSource::Timer);
    } else {
        app.begin_operation(DiagnosticSource::Timer);
    }
    app.record(
        "duration.deadline",
        format!(
            "action={} duration_minutes={} generation={} remaining_ms=0",
            action.action.label(),
            action.duration.minutes(),
            action.generation
        ),
    );
    app.record("duration.deadline.timing", app.timing_snapshot_details());
    match action.action {
        DurationAction::Start => {
            refresh_power_for_duration(app, "start_deadline");
            let power = app.observation.power().state;
            let decision = decide(PolicyInput {
                enabled: true,
                eligible_profile: true,
                power,
            });
            app.record(
                "duration.start.policy",
                format!(
                    "power={power:?} result={:?} reason={:?} selected_duration_minutes={} deadline_remaining_ms=0",
                    decision.status,
                    decision.reason,
                    action.duration.minutes()
                ),
            );
            app.record(
                "duration.start.policy.timing",
                app.timing_snapshot_details(),
            );
            if decision.status == tick_core::Status::Requested {
                if app.controller.ownership() == OwnershipState::Released {
                    app.record(
                        "duration.start.result",
                        "outcome=acquire_intent_queued policy_result=allowed",
                    );
                    queue_intent(app, DesiredIntent::Acquire, "duration start deadline");
                } else {
                    app.record(
                        "duration.start.result",
                        format!(
                            "outcome=noop reason=ownership_{:?} policy_result=allowed",
                            app.controller.ownership()
                        ),
                    );
                    show_ownership_status(app, TrayStatus::Stopped);
                }
            } else {
                app.record(
                    "duration.start.suppressed",
                    format!(
                        "outcome=suppressed reason={:?} policy_result=blocked",
                        decision.reason
                    ),
                );
                app.record(
                    "duration.start.suppressed.timing",
                    app.timing_snapshot_details(),
                );
                release_for_policy(app, decision.reason);
            }
        }
        DurationAction::Stop => {
            if app.controller.ownership() == OwnershipState::Released {
                app.record(
                    "duration.stop.result",
                    "outcome=noop reason=already_released policy_result=not_needed remaining_ms=0",
                );
                show_ownership_status(app, TrayStatus::Stopped);
            } else {
                let result = guarded_release(app, "duration stop deadline", TrayStatus::Stopped);
                app.record(
                    "duration.stop.result",
                    format!("outcome={:?} remaining_ms=0", result),
                );
            }
            app.record("duration.stop.timing", app.timing_snapshot_details());
        }
        DurationAction::Pause => {
            app.record(
                "duration.pause.expired",
                format!(
                    "outcome=resume_and_reconcile duration_minutes={} remaining_ms=0",
                    action.duration.minutes()
                ),
            );
            refresh_power_for_duration(app, "pause_expiry");
            reconcile(app);
        }
    }
    if app.handoff_operation.is_none() {
        app.finish_operation(DiagnosticOutcome::Completed);
    }
}

fn handle_duration_timer(app: &mut App, timer_id: usize) {
    if app.duration_timer_id != Some(timer_id) {
        app.record(
            "duration.timer",
            format!("outcome=ignored reason=stale_timer timer_id={timer_id}"),
        );
        return;
    }
    let generation = app.duration_timer_generation.unwrap_or_default();
    match app.pause.timer_event(generation, std::time::Instant::now()) {
        CoordinatorTimerEvent::Early {
            remaining, action, ..
        } => {
            app.record(
                "duration.timer",
                format!(
                    "outcome=early action={} generation={generation} remaining_ms={}",
                    action.label(),
                    remaining.as_millis()
                ),
            );
            arm_duration_timer(app);
        }
        CoordinatorTimerEvent::Expired(action) => {
            cancel_duration_timer(app);
            kill_schedule_display_timer(app);
            let _ = app.pause.cancel();
            execute_scheduled_action(app, action);
        }
        CoordinatorTimerEvent::Stale => {
            app.record(
                "duration.timer",
                format!("outcome=ignored reason=stale_generation generation={generation}"),
            );
        }
    }
}

fn refresh_power_for_duration(app: &mut App, source: &str) {
    let previous = app.observation.power().state;
    app.record(
        "native.GetSystemPowerStatus.call",
        format!("source={source} fields=sanitized"),
    );
    let result = app.observation.refresh_power();
    app.record(
        "duration.power_observation",
        format!(
            "source={source} previous={previous:?} result={result:?} current={:?} policy=authoritative",
            app.observation.power().state
        ),
    );
}

fn release_for_power_change(app: &mut App) {
    app.record(
        "power.transition",
        format!("state={:?}", app.observation.power().state),
    );
    refresh_timing_observation(app);
    apply_power_reconciliation(app);
}

fn apply_power_reconciliation(app: &mut App) {
    if app.pause.pause_active() {
        app.record(
            "policy.power_reconciliation.suppressed",
            "reason=pause_active result=release_only",
        );
        queue_intent(app, DesiredIntent::Release, "pause power reconciliation");
        return;
    }
    let power = app.observation.power().state;
    let action = power_reconciliation(app.config.automatic, power, app.controller.ownership());
    app.record(
        "policy.power_reconciliation",
        format!("power={power:?} action={action:?}"),
    );
    match action {
        PowerReconciliation::Acquire => apply_policy(app),
        PowerReconciliation::ReleaseBlocked => release_for_policy(
            app,
            match power {
                PowerState::Battery => tick_policy::PolicyReason::BatteryRestricted,
                PowerState::BatterySaver => tick_policy::PolicyReason::BatterySaverRestricted,
                PowerState::Unknown => tick_policy::PolicyReason::PowerUnknown,
                PowerState::Ac => unreachable!(),
            },
        ),
        PowerReconciliation::ShowStopped => show_ownership_status(app, TrayStatus::Stopped),
        PowerReconciliation::PreserveOwned => show_ownership_status(app, TrayStatus::Stopped),
        PowerReconciliation::PreserveUncertain => show_ownership_status(app, TrayStatus::Stopped),
    }
}

fn show_ownership_status(app: &mut App, released_status: TrayStatus) {
    if app.handoff.is_some() {
        app.tray_status = if app.pause.pause_active() {
            TrayStatus::Pausing
        } else {
            TrayStatus::Stopping
        };
        app.publish();
        return;
    }
    app.tray_status = match app.controller.ownership() {
        OwnershipState::Released if app.pause.pause_active() => TrayStatus::Paused,
        OwnershipState::Released => released_status,
        OwnershipState::Owned => TrayStatus::Running,
        OwnershipState::Uncertain => TrayStatus::Unverified,
    };
    app.publish();
}

fn numeric_detail(details: &str, key: &str) -> Option<i64> {
    details.split_whitespace().find_map(|field| {
        let (field_key, value) = field.split_once('=')?;
        (field_key == key)
            .then(|| value.parse::<i64>().ok())
            .flatten()
    })
}

fn native_outcome(name: &str, details: &str) -> NativeOutcome {
    let raw_status = numeric_detail(details, "raw_status");
    let raw_error = numeric_detail(details, "raw_error");
    let ntstatus = if name.contains("Nt") || name.starts_with("timer.") {
        raw_status.and_then(|value| i32::try_from(value).ok())
    } else {
        None
    };
    let win32_last_error = if name.contains("GetLastError")
        || raw_error.is_some()
        || (name.starts_with("native.") && ntstatus.is_none())
    {
        raw_error
            .or_else(|| {
                name.contains("GetLastError")
                    .then_some(raw_status)
                    .flatten()
            })
            .or_else(|| name.starts_with("native.").then_some(raw_status).flatten())
            .and_then(|value| u32::try_from(value).ok())
    } else {
        None
    };
    NativeOutcome {
        ntstatus,
        win32_last_error,
        requested_hns: numeric_detail(details, "requested_hns").map(|value| value as u64),
        selected_hns: numeric_detail(details, "selected_hns").map(|value| value as u64),
        effective_hns: numeric_detail(details, "effective_hns").map(|value| value as u64),
    }
}

fn diagnostic_phase(name: &str) -> DiagnosticPhase {
    if name.contains("handoff") {
        DiagnosticPhase::Handoff
    } else if name.contains("policy") {
        DiagnosticPhase::Decide
    } else if name.contains("duration") || name.contains("pause") {
        DiagnosticPhase::Timer
    } else if name.contains("shutdown") || name.contains("quit") {
        DiagnosticPhase::Shutdown
    } else if name.contains("query") || name.contains("observation") {
        DiagnosticPhase::Observe
    } else if name.contains("acquire") || name.contains("request") {
        DiagnosticPhase::Acquire
    } else if name.contains("release") || name.contains("stop") {
        DiagnosticPhase::Release
    } else if name.contains("verification") {
        DiagnosticPhase::Verify
    } else if name.contains("config") || name.contains("startup") {
        DiagnosticPhase::Persist
    } else if name.contains("window") || name.contains("tray.status") {
        DiagnosticPhase::Render
    } else {
        DiagnosticPhase::Complete
    }
}

fn diagnostic_outcome(name: &str, details: &str) -> DiagnosticOutcome {
    let value = format!("{name} {details}").to_ascii_lowercase();
    if value.contains("timedout") || value.contains("timeout") {
        DiagnosticOutcome::TimedOut
    } else if value.contains("suppressed") {
        DiagnosticOutcome::Suppressed
    } else if value.contains("cancel") {
        DiagnosticOutcome::Cancelled
    } else if value.contains("unverified") || value.contains("uncertain") {
        DiagnosticOutcome::Unverified
    } else if value.contains("error") || value.contains("failed") {
        DiagnosticOutcome::Failed
    } else if value.contains("started") || value.contains("pending") {
        DiagnosticOutcome::InProgress
    } else {
        DiagnosticOutcome::Completed
    }
}

fn diagnostic_source(name: &str) -> DiagnosticSource {
    if name.contains("power") {
        DiagnosticSource::PowerEvent
    } else if name.contains("startup") {
        DiagnosticSource::Startup
    } else if name.contains("pause") {
        DiagnosticSource::Pause
    } else if name.contains("resume") {
        DiagnosticSource::Resume
    } else if name.contains("handoff") {
        DiagnosticSource::Handoff
    } else if name.contains("duration") {
        DiagnosticSource::Timer
    } else if name.contains("shutdown") || name.contains("quit") {
        DiagnosticSource::Shutdown
    } else if name.contains("policy") {
        DiagnosticSource::Policy
    } else if name.contains("ownership") {
        DiagnosticSource::Ownership
    } else if name.starts_with("native.") || name.starts_with("timer.") {
        DiagnosticSource::Native
    } else if name.contains("tray.command") {
        DiagnosticSource::TrayCommand
    } else {
        DiagnosticSource::Internal
    }
}

impl App {
    fn sync_timing_snapshot(&mut self) {
        self.timing_snapshot = self.controller.snapshot();
    }

    fn timing_snapshot_details(&self) -> String {
        format!(
            "valid={} requested_hns={} selected_hns={} effective_hns={} minimum_hns={} maximum_hns={} raw_status={}",
            self.timing_snapshot_valid,
            self.timing_snapshot
                .requested
                .map_or_else(|| "unknown".to_owned(), |value| value.value().to_string()),
            self.timing_snapshot
                .selected
                .map_or_else(|| "unknown".to_owned(), |value| value.value().to_string()),
            self.timing_snapshot
                .effective
                .map_or_else(|| "unknown".to_owned(), |value| value.value().to_string()),
            self.timing_snapshot
                .minimum_interval
                .map_or_else(|| "unknown".to_owned(), |value| value.value().to_string()),
            self.timing_snapshot
                .maximum_interval
                .map_or_else(|| "unknown".to_owned(), |value| value.value().to_string()),
            self.timing_snapshot
                .raw_status
                .map_or_else(|| "unknown".to_owned(), |value| value.to_string()),
        )
    }

    fn running_duration(&self) -> Option<std::time::Duration> {
        (self.lifecycle_status() == TrayStatus::Running
            && self.controller.ownership() == OwnershipState::Owned)
            .then(|| self.running_since.map(|since| since.elapsed()))
            .flatten()
    }

    fn begin_child_operation(
        &mut self,
        parent: OperationContext,
        source: DiagnosticSource,
    ) -> OperationContext {
        let context = self.diagnostics.child_operation(parent, source);
        self.operation = Some(context);
        self.operation_source = source;
        self.controller.set_operation_context(Some((
            context.operation_id,
            context.parent_operation_id,
            context.correlation_id,
        )));
        self.record(
            "operation.begin",
            format!(
                "source={source:?} parent_operation_id={:?}",
                context.parent_operation_id
            ),
        );
        context
    }

    fn lifecycle_status(&self) -> TrayStatus {
        crate::tray_surface::scheduled_lifecycle_status(
            self.tray_status,
            self.handoff.is_some(),
            self.pause.current().map(|action| action.action),
        )
    }

    fn timing_values(&self) -> TimingValues {
        let external = self.external_timing
            || (self.controller.ownership() == OwnershipState::Released
                && matches!(
                    (self.timing_snapshot.requested, self.timing_snapshot.effective),
                    (Some(requested), Some(effective)) if effective < requested
                ));
        TimingValues::from_snapshot(
            self.timing_snapshot,
            self.handoff.is_some(),
            external,
            self.invalid_interval,
            self.timing_snapshot_valid,
        )
    }

    fn cleanup_normal_shutdown(&mut self) -> Result<(), String> {
        if self.shutdown_gate.verified() {
            return Ok(());
        }
        if let Some(scheduled) = self.pause.cancel() {
            self.record(
                "duration.cleared",
                format!(
                    "reason=shutdown action={} duration_minutes={} generation={}",
                    scheduled.action.label(),
                    scheduled.duration.minutes(),
                    scheduled.generation
                ),
            );
        }
        cancel_duration_timer(self);
        kill_schedule_display_timer(self);
        self.scheduled_operation = None;
        self.begin_operation(DiagnosticSource::Shutdown);
        self.record(
            "shutdown.cleanup",
            format!(
                "attempt=guarded number={}",
                self.shutdown_gate.attempts() + 1
            ),
        );
        if self.handoff.is_some() && self.controller.ownership() == OwnershipState::Released {
            finish_handoff_timer(self);
            self.handoff = None;
            self.handoff_operation = None;
            self.controller.clear_release_boundary();
            self.sync_timing_snapshot();
            self.external_timing = true;
            self.tray_status = TrayStatus::Stopped;
            self.record(
                "handoff.quit",
                "ownership=released external_timing_remains=true watcher=stopped",
            );
            self.publish();
            self.shutdown_gate
                .attempt(|| Ok::<(), String>(()))
                .expect("cleanup gate bookkeeping cannot fail");
            self.record(
                "shutdown.cleanup.result",
                "result=verified ownership=released external_timing_remains=true",
            );
            self.finish_operation(DiagnosticOutcome::Completed);
            return Ok(());
        }
        let result = guarded_release(self, "shutdown", TrayStatus::Stopped);
        if result.is_ok() && self.handoff.is_some() {
            let error = "handoff remains pending after ownership release".to_owned();
            self.shutdown_gate
                .attempt(|| Err::<(), String>(error.clone()))
                .expect_err("pending handoff must remain unresolved");
            self.record(
                "shutdown.cleanup.result",
                format!("result=handoff_pending error={error}"),
            );
            return Err(error);
        }
        match result {
            Ok(released) => {
                self.shutdown_gate
                    .attempt(|| Ok::<(), String>(()))
                    .expect("cleanup gate bookkeeping cannot fail");
                self.record(
                    "shutdown.cleanup.result",
                    format!("result=verified released={released}"),
                );
                self.finish_operation(DiagnosticOutcome::Completed);
                Ok(())
            }
            Err(error) => {
                self.shutdown_gate
                    .attempt(|| Err::<(), String>(error.clone()))
                    .expect_err("failed cleanup must remain unresolved");
                self.record(
                    "shutdown.cleanup.result",
                    format!("result=unverified error={error}"),
                );
                self.finish_operation(DiagnosticOutcome::Unverified);
                Err(error)
            }
        }
    }

    fn begin_operation(&mut self, source: DiagnosticSource) -> OperationContext {
        let context = self.diagnostics.begin_operation(source);
        self.operation = Some(context);
        self.operation_source = source;
        self.controller.set_operation_context(Some((
            context.operation_id,
            context.parent_operation_id,
            context.correlation_id,
        )));
        self.record("operation.begin", format!("source={source:?}"));
        context
    }

    fn finish_operation(&mut self, outcome: DiagnosticOutcome) {
        let Some(context) = self.operation.take() else {
            return;
        };
        self.diagnostics.record_with_context(
            DiagnosticRecord {
                context,
                phase: DiagnosticPhase::Complete,
                source: self.operation_source,
                outcome,
                native: NativeOutcome::default(),
            },
            "operation.complete",
            format!("outcome={outcome:?}"),
        );
        self.controller.set_operation_context(None);
        request_diagnostic_refresh(self);
    }

    fn record(&mut self, name: &str, details: impl AsRef<str>) {
        let details = details.as_ref();
        let source = self
            .operation
            .map_or_else(|| diagnostic_source(name), |_| self.operation_source);
        let context = self
            .operation
            .unwrap_or_else(|| self.diagnostics.begin_operation(source));
        if self.operation.is_some() {
            self.controller.set_operation_context(Some((
                context.operation_id,
                context.parent_operation_id,
                context.correlation_id,
            )));
        }
        self.diagnostics.record_with_context(
            DiagnosticRecord {
                context,
                phase: diagnostic_phase(name),
                source,
                outcome: diagnostic_outcome(name, details),
                native: native_outcome(name, details),
            },
            name,
            details,
        );
        request_diagnostic_refresh(self);
    }

    fn publish(&mut self) {
        let status = self.lifecycle_status();
        if status != TrayStatus::Running || self.controller.ownership() != OwnershipState::Owned {
            self.running_since = None;
        }
        self.tray_status = status;
        let timing = self.timing_values();
        let now = std::time::Instant::now();
        let key = PublicationKey {
            status,
            ownership: self.controller.ownership(),
            effective: timing.effective,
            requested: timing.requested,
            handoff: timing.handoff_pending,
            scheduled: self
                .pause
                .current()
                .map(|action| scheduled_display_key(action, now)),
        };
        if self.last_publication == Some(key) {
            return;
        }
        self.last_publication = Some(key);
        let status_text = tooltip_at(status, timing, self.pause.current(), now);
        self.record(
            "tray.status.changed",
            format!("status={status:?} tooltip={status_text}"),
        );
        self.record(
            "ownership.state",
            format!("state={:?}", self.controller.ownership()),
        );
        self.record(
            "effective.system.state",
            format!(
                "effective_hns={}",
                timing
                    .effective
                    .map_or_else(|| "unknown".to_owned(), |value| value.value().to_string())
            ),
        );
        if let Some(icon) = self.tray_icon.as_mut() {
            if let Err(raw_error) = update_icon(icon, status, timing, self.pause.current()) {
                self.record(
                    "native.Shell_NotifyIconW.modify.error",
                    format!("raw_status={raw_error}"),
                );
            }
        }
        if self.menu_active {
            unsafe { refresh_popup_menu(self) };
        }
    }
}

fn refresh_timing_observation(app: &mut App) -> Option<TimerObservation> {
    match app.controller.query() {
        Ok(observation) => {
            app.sync_timing_snapshot();
            app.timing_snapshot_valid = true;
            app.record(
                "timer.query.observation",
                format!(
                    "requested_hns={} effective_hns={} raw_status={} effective_relation={}",
                    observation.requested.value(),
                    observation.reported_current.value(),
                    observation.raw_status,
                    observation.effective_relation()
                ),
            );
            Some(observation)
        }
        Err(error) => {
            app.sync_timing_snapshot();
            app.timing_snapshot_valid = false;
            app.record(
                "timer.query.error",
                format!("error={error:?} effective=unknown"),
            );
            None
        }
    }
}

fn begin_handoff(app: &mut App, boundary: tick_core::Hns, released_status: TrayStatus) -> bool {
    let tracker = HandoffTracker::new(boundary, released_status);
    let handoff_operation = app.begin_operation(DiagnosticSource::Handoff);
    app.handoff_operation = Some(handoff_operation);
    cancel_duration_timer(app);
    let Some(hwnd) = app.tray_icon.as_ref().map(|icon| icon.h_wnd) else {
        app.record(
            "handoff.timeout",
            "reason=watcher_unavailable ownership=released effective_system=finer",
        );
        app.external_timing = true;
        app.controller.clear_release_boundary();
        app.sync_timing_snapshot();
        app.tray_status = if released_status == TrayStatus::Paused {
            TrayStatus::Unverified
        } else {
            TrayStatus::Stopped
        };
        app.publish();
        process_desired_intent(app);
        if app.pause.pause_active() {
            arm_duration_timer(app);
        }
        app.finish_operation(DiagnosticOutcome::TimedOut);
        app.handoff_operation = None;
        return false;
    };
    let timer = unsafe {
        SetTimer(
            hwnd,
            HANDOFF_TIMER_ID,
            HANDOFF_POLL_INTERVAL_MS,
            std::ptr::null_mut(),
        )
    };
    if timer == 0 {
        app.record(
            "handoff.timeout",
            format!(
                "reason=watcher_start_failed ownership=released boundary_hns={} effective_system=finer raw_status={}",
                boundary.value(),
                unsafe { GetLastError() }
            ),
        );
        app.record(
            "ownership.result",
            "state=released effective_system=finer due_to=external_or_unknown_client",
        );
        app.external_timing = true;
        app.controller.clear_release_boundary();
        app.sync_timing_snapshot();
        app.tray_status = if released_status == TrayStatus::Paused {
            TrayStatus::Unverified
        } else {
            TrayStatus::Stopped
        };
        app.publish();
        process_desired_intent(app);
        if app.pause.pause_active() {
            arm_duration_timer(app);
        }
        app.finish_operation(DiagnosticOutcome::TimedOut);
        app.handoff_operation = None;
        return false;
    }
    app.handoff = Some(tracker);
    app.tray_status = if released_status == TrayStatus::Paused {
        TrayStatus::Pausing
    } else {
        TrayStatus::Stopping
    };
    app.record(
        "handoff.started",
        format!(
            "boundary_hns={} interval_ms={} max_polls={} ownership=released effective_system=finer",
            boundary.value(),
            HANDOFF_POLL_INTERVAL_MS,
            crate::tray_surface::HANDOFF_MAX_POLLS
        ),
    );
    app.publish();
    true
}

fn finish_handoff_timer(app: &mut App) {
    if let Some(hwnd) = app.tray_icon.as_ref().map(|icon| icon.h_wnd) {
        let result = unsafe { KillTimer(hwnd, HANDOFF_TIMER_ID) };
        if result == 0 {
            app.record(
                "native.KillTimer.handoff.error",
                format!("raw_status={}", unsafe { GetLastError() }),
            );
        }
    }
}

fn handle_handoff_timer(app: &mut App) {
    if let Some(context) = app.handoff_operation {
        app.operation = Some(context);
        app.operation_source = DiagnosticSource::Handoff;
    }
    let Some(tracker) = app.handoff.take() else {
        return;
    };
    app.handoff = Some(tracker);
    let boundary = tracker.boundary();
    let observation = match app.controller.observe_current(boundary) {
        Ok(observation) => {
            app.sync_timing_snapshot();
            app.timing_snapshot_valid = true;
            app.record(
                "timer.handoff.observation",
                format!(
                    "requested_hns={} effective_hns={} raw_status={} effective_relation={}",
                    observation.requested.value(),
                    observation.reported_current.value(),
                    observation.raw_status,
                    observation.effective_relation()
                ),
            );
            Some(observation)
        }
        Err(error) => {
            app.sync_timing_snapshot();
            app.timing_snapshot_valid = false;
            app.record(
                "timer.handoff.query.error",
                format!("error={error:?} effective=unknown"),
            );
            None
        }
    };
    let effective = observation.map(|value| value.reported_current);
    let mut tracker = app.handoff.take().expect("handoff tracker remains active");
    let progress = tracker.observe(effective);
    app.record(
        "handoff.observation",
        format!(
            "boundary_hns={} effective_hns={} poll={} result={progress:?}",
            tracker.boundary().value(),
            effective.map_or_else(|| "unknown".to_owned(), |value| value.value().to_string()),
            tracker.polls()
        ),
    );
    match progress {
        HandoffProgress::Pending => {
            app.handoff = Some(tracker);
            app.tray_status = if tracker.released_status() == TrayStatus::Paused {
                TrayStatus::Pausing
            } else {
                TrayStatus::Stopping
            };
            app.publish();
        }
        HandoffProgress::Completed => {
            finish_handoff_timer(app);
            app.record(
                "handoff.completed",
                format!(
                    "boundary_hns={} effective_hns={} ownership=released",
                    tracker.boundary().value(),
                    effective
                        .map_or_else(|| "unknown".to_owned(), |value| value.value().to_string())
                ),
            );
            app.record(
                "ownership.result",
                "state=released effective_system=non_finer handoff=completed",
            );
            app.controller.clear_release_boundary();
            app.sync_timing_snapshot();
            app.tray_status = tracker.released_status();
            app.external_timing = false;
            app.publish();
            process_desired_intent(app);
            if app.pause.pause_active() {
                arm_duration_timer(app);
            }
            app.finish_operation(DiagnosticOutcome::Completed);
            app.handoff_operation = None;
        }
        HandoffProgress::TimedOut => {
            finish_handoff_timer(app);
            app.record(
                "handoff.timeout",
                format!(
                    "boundary_hns={} effective_hns={} ownership=released reason=external_or_unknown_client_remains_finer",
                    tracker.boundary().value(),
                    effective.map_or_else(|| "unknown".to_owned(), |value| value.value().to_string())
                ),
            );
            app.record(
                "ownership.result",
                "state=released effective_system=finer due_to=external_or_unknown_client",
            );
            app.controller.clear_release_boundary();
            app.sync_timing_snapshot();
            app.tray_status = if tracker.released_status() == TrayStatus::Paused {
                TrayStatus::Unverified
            } else {
                TrayStatus::Stopped
            };
            app.external_timing = true;
            app.publish();
            process_desired_intent(app);
            if app.pause.pause_active() {
                arm_duration_timer(app);
            }
            app.finish_operation(DiagnosticOutcome::TimedOut);
            app.handoff_operation = None;
        }
    }
}

fn update_icon(
    icon: &mut NotifyIconData,
    status: TrayStatus,
    timing: TimingValues,
    scheduled: Option<crate::pause::ScheduledAction>,
) -> Result<(), u32> {
    let replacement = unsafe { status_icon(status, dpi_for_window(icon.h_wnd)) }?;
    let old_icon = icon.h_icon;
    let old_tip = icon.sz_tip;
    icon.h_icon = replacement;
    icon.sz_tip = [0; 128];
    for (target, source) in icon
        .sz_tip
        .iter_mut()
        .zip(tooltip_at(status, timing, scheduled, std::time::Instant::now()).encode_utf16())
    {
        *target = source;
    }
    let result = unsafe { Shell_NotifyIconW(NIM_MODIFY, icon) };
    let raw_error = if result == 0 {
        unsafe { GetLastError() }
    } else {
        0
    };
    if matches!(
        native_bool_result(result, raw_error),
        NativeResult::Failed { .. }
    ) {
        icon.h_icon = old_icon;
        icon.sz_tip = old_tip;
        unsafe { destroy_icon(replacement) };
        Err(raw_error)
    } else {
        unsafe { destroy_icon(old_icon) };
        Ok(())
    }
}

unsafe fn status_icon(status: TrayStatus, dpi: u32) -> Result<*mut c_void, u32> {
    let color = icon_pixel_color(status);
    let canvas = dpi_to_icon_canvas(dpi);
    let pixel_count = (canvas * canvas) as usize;
    let pixels = vec![color; pixel_count];
    let mask = vec![0u8; pixel_count / 8];
    let bitmap = CreateBitmap(canvas, canvas, 1, 32, pixels.as_ptr() as *const c_void);
    if bitmap.is_null() {
        return Err(GetLastError());
    }
    let mask_bitmap = CreateBitmap(canvas, canvas, 1, 1, mask.as_ptr() as *const c_void);
    if mask_bitmap.is_null() {
        let raw_error = GetLastError();
        DeleteObject(bitmap);
        return Err(raw_error);
    }
    let info = IconInfo {
        f_icon: 1,
        x_hotspot: 0,
        y_hotspot: 0,
        h_bm_mask: mask_bitmap,
        h_bm_color: bitmap,
    };
    let icon = CreateIconIndirect(&info);
    if icon.is_null() {
        let raw_error = GetLastError();
        DeleteObject(bitmap);
        DeleteObject(mask_bitmap);
        return Err(raw_error);
    }
    DeleteObject(bitmap);
    DeleteObject(mask_bitmap);
    Ok(icon)
}

fn dpi_for_window(window: *mut c_void) -> u32 {
    let dpi = unsafe { GetDpiForWindow(window) };
    if dpi == 0 {
        96
    } else {
        dpi
    }
}

unsafe fn destroy_icon(icon: *mut c_void) {
    if !icon.is_null() {
        DestroyIcon(icon);
    }
}

unsafe extern "system" fn window_proc(
    hwnd: *mut c_void,
    message: u32,
    w_param: usize,
    l_param: isize,
) -> isize {
    let app = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
    if message == WM_CREATE {
        let create = l_param as *const CreateStruct;
        let app_ptr = app_create_params(create);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, app_ptr as isize);
        return 0;
    }
    if !app.is_null()
        && (*app).menu_active
        && matches!(message, WM_TRAY | WM_COMMAND)
        && !menu_command_dispatch_allowed(true, false)
    {
        return 0;
    }
    if !app.is_null() {
        let app = &mut *app;
        match message {
            WM_TRAY if tray_notification_opens_menu(l_param as usize, app.menu_active) => {
                show_menu(hwnd, app)
            }
            WM_COMMAND => {
                handle_menu_command(hwnd, app, w_param & 0xffff);
            }
            WM_MENUSELECT => {
                update_menu_help(
                    app,
                    w_param & 0xffff,
                    (w_param >> 16) as u32,
                    l_param as *mut c_void,
                );
            }
            WM_TIMER if w_param == POPUP_REFRESH_TIMER_ID => {
                refresh_popup_menu(app);
            }
            WM_TIMER if w_param == SCHEDULE_DISPLAY_TIMER_ID => {
                handle_schedule_display_timer(app);
            }
            WM_TIMER if w_param == HANDOFF_TIMER_ID => {
                handle_handoff_timer(app);
            }
            WM_TIMER if app.duration_timer_id == Some(w_param) => {
                handle_duration_timer(app, w_param);
            }
            WM_POWERBROADCAST if w_param == PBT_APMPOWERSTATUSCHANGE => {
                app.begin_operation(DiagnosticSource::PowerEvent);
                app.record("power.broadcast", "event=APMPOWERSTATUSCHANGE");
                let previous = app.observation.power().state;
                app.record("native.GetSystemPowerStatus.call", "fields=sanitized");
                let result = app.observation.refresh_power();
                app.record(
                    "power.observation",
                    format!(
                        "previous={previous:?} result={result:?} current={:?}",
                        app.observation.power().state
                    ),
                );
                release_for_power_change(app);
                app.finish_operation(DiagnosticOutcome::Completed);
            }
            WM_DESTROY => PostQuitMessage(0),
            _ => {}
        }
    }
    DefWindowProcW(hwnd, message, w_param, l_param)
}

unsafe fn show_menu(hwnd: *mut c_void, app: &mut App) {
    if app.menu_active {
        return;
    }
    app.menu_active = true;
    let mut anchor = Point { x: 0, y: 0 };
    if GetCursorPos(&mut anchor) == 0 {
        app.record(
            "native.GetCursorPos.error",
            format!("raw_status={}", GetLastError()),
        );
        app.menu_active = false;
        return;
    }
    loop {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            app.record(
                "native.CreatePopupMenu.error",
                format!("raw_status={}", GetLastError()),
            );
            break;
        }
        app.popup_menus = Some(PopupMenuHandles {
            root: menu,
            duration: None,
            status: None,
        });
        let items = menu_items_with_duration(
            app.lifecycle_status(),
            app.config.startup_enabled,
            app.config.automatic,
            app.timing_values(),
            app.pause.pause_active(),
        );
        let header_flags = if items[0].enabled {
            MF_STRING
        } else {
            MF_STRING | MF_GRAYED
        };
        let start_flags = if items[1].enabled {
            MF_STRING
        } else {
            MF_STRING | MF_GRAYED
        };
        let stop_flags = if items[2].enabled {
            MF_STRING
        } else {
            MF_STRING | MF_GRAYED
        };
        let startup_id = if app.config.startup_enabled {
            ID_STARTUP_OFF
        } else {
            ID_STARTUP_ON
        };
        let automatic_id = if app.config.automatic {
            ID_AUTOMATIC_OFF
        } else {
            ID_AUTOMATIC_ON
        };
        let menu_ok = append_menu_checked(
            app,
            menu,
            header_flags,
            GITHUB_COMMAND_ID,
            wide(&items[0].label).as_ptr(),
        ) && append_menu_checked(app, menu, MF_SEPARATOR, 0, std::ptr::null())
            && append_menu_checked(
                app,
                menu,
                start_flags,
                ID_START,
                wide(&items[1].label).as_ptr(),
            )
            && append_menu_checked(
                app,
                menu,
                stop_flags,
                ID_STOP,
                wide(&items[2].label).as_ptr(),
            )
            && append_duration_choice_submenu(app, menu, DurationAction::Pause)
            && append_duration_submenu(app, menu)
            && append_menu_checked(app, menu, MF_SEPARATOR, 0, std::ptr::null())
            && append_menu_checked(
                app,
                menu,
                MF_STRING,
                startup_id,
                wide(&items[5].label).as_ptr(),
            )
            && append_menu_checked(
                app,
                menu,
                MF_STRING,
                automatic_id,
                wide(&items[6].label).as_ptr(),
            )
            && append_menu_checked(app, menu, MF_SEPARATOR, 0, std::ptr::null())
            && append_status_submenu(app, menu)
            && append_menu_checked(
                app,
                menu,
                MF_STRING,
                LOGS_COMMAND_ID,
                wide(&items[8].label).as_ptr(),
            )
            && append_menu_checked(app, menu, MF_SEPARATOR, 0, std::ptr::null())
            && append_menu_checked(
                app,
                menu,
                MF_STRING,
                ID_QUIT,
                wide(&items[9].label).as_ptr(),
            );
        if !menu_ok {
            app.popup_menus = None;
            if DestroyMenu(menu) == 0 {
                app.record(
                    "native.DestroyMenu.error",
                    format!("raw_status={}", GetLastError()),
                );
            }
            break;
        }
        let _ = create_menu_help(hwnd, app);
        begin_popup_refresh_timer(app);
        if SetForegroundWindow(hwnd) == 0 {
            app.record(
                "native.SetForegroundWindow.error",
                format!("raw_status={}", GetLastError()),
            );
        }
        let command = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_RETURNCMD,
            anchor.x,
            anchor.y,
            0,
            hwnd,
            std::ptr::null(),
        );
        destroy_menu_help(app);
        kill_popup_refresh_timer(app);
        app.popup_menus = None;
        if DestroyMenu(menu) == 0 {
            app.record(
                "native.DestroyMenu.error",
                format!("raw_status={}", GetLastError()),
            );
        }
        if command == 0 {
            app.record(
                "native.TrackPopupMenu.result",
                format!("result=empty raw_status={}", GetLastError()),
            );
        }
        let Some(command) = returned_menu_command(command) else {
            break;
        };
        app.record(
            "tray.command.dispatch",
            format!("source=TPM_RETURNCMD id={command}"),
        );
        if !menu_command_dispatch_allowed(app.menu_active, true)
            || !handle_menu_command(hwnd, app, command)
        {
            break;
        }
    }
    destroy_menu_help(app);
    kill_popup_refresh_timer(app);
    app.popup_menus = None;
    app.menu_active = false;
}

fn begin_popup_refresh_timer(app: &mut App) {
    if app.popup_refresh_timer_active {
        return;
    }
    let Some(hwnd) = app.tray_icon.as_ref().map(|icon| icon.h_wnd) else {
        return;
    };
    let result = unsafe {
        SetTimer(
            hwnd,
            POPUP_REFRESH_TIMER_ID,
            POPUP_REFRESH_INTERVAL_MS,
            std::ptr::null_mut(),
        )
    };
    if result == 0 {
        app.record(
            "native.SetTimer.popup_refresh.error",
            format!("raw_status={}", unsafe { GetLastError() }),
        );
    } else {
        app.popup_refresh_timer_active = true;
    }
}

fn kill_popup_refresh_timer(app: &mut App) {
    if !app.popup_refresh_timer_active {
        return;
    }
    app.popup_refresh_timer_active = false;
    if let Some(hwnd) = app.tray_icon.as_ref().map(|icon| icon.h_wnd) {
        unsafe {
            let _ = KillTimer(hwnd, POPUP_REFRESH_TIMER_ID);
        }
    }
}

unsafe fn refresh_popup_menu(app: &mut App) {
    let Some(handles) = app.popup_menus else {
        return;
    };
    let timing = app.timing_values();
    let start_enabled = menu_command_is_enabled_with_pause(
        ID_START,
        app.lifecycle_status(),
        app.config.startup_enabled,
        app.config.automatic,
        app.pause.active(),
    );
    let stop_enabled = menu_command_is_enabled_with_pause(
        ID_STOP,
        app.lifecycle_status(),
        app.config.startup_enabled,
        app.config.automatic,
        app.pause.active(),
    );
    let _ = ModifyMenuW(
        handles.root,
        2,
        MF_BYPOSITION | MF_STRING | if start_enabled { 0 } else { MF_GRAYED },
        ID_START,
        wide("Start").as_ptr(),
    );
    let _ = ModifyMenuW(
        handles.root,
        3,
        MF_BYPOSITION | MF_STRING | if stop_enabled { 0 } else { MF_GRAYED },
        ID_STOP,
        wide("Stop").as_ptr(),
    );
    let startup_id = if app.config.startup_enabled {
        ID_STARTUP_OFF
    } else {
        ID_STARTUP_ON
    };
    let automatic_id = if app.config.automatic {
        ID_AUTOMATIC_OFF
    } else {
        ID_AUTOMATIC_ON
    };
    let _ = ModifyMenuW(
        handles.root,
        7,
        MF_BYPOSITION | MF_STRING,
        startup_id,
        wide(crate::tray_surface::auto_start_label(
            app.config.startup_enabled,
        ))
        .as_ptr(),
    );
    let _ = ModifyMenuW(
        handles.root,
        8,
        MF_BYPOSITION | MF_STRING,
        automatic_id,
        wide(crate::tray_surface::automatic_label(app.config.automatic)).as_ptr(),
    );
    if let Some(duration) = handles.duration {
        let cancel_enabled = app
            .pause
            .current()
            .is_some_and(|action| action.action != DurationAction::Pause);
        let _ = ModifyMenuW(
            duration,
            2,
            MF_BYPOSITION | MF_STRING | if cancel_enabled { 0 } else { MF_GRAYED },
            CANCEL_SCHEDULED_COMMAND_ID,
            wide("Cancel scheduled action").as_ptr(),
        );
        let pause_active = app.pause.pause_active();
        let _ = ModifyMenuW(
            duration,
            3,
            MF_BYPOSITION | MF_STRING | if pause_active { 0 } else { MF_GRAYED },
            CANCEL_PAUSE_COMMAND_ID,
            wide("Resume now").as_ptr(),
        );
    }
    if let Some(status_menu) = handles.status {
        let items = status_menu_items(
            app.lifecycle_status(),
            timing,
            app.controller.ownership(),
            app.pause.current(),
            app.running_duration(),
            std::time::Instant::now(),
        );
        for (index, item) in items.iter().enumerate() {
            let _ = ModifyMenuW(
                status_menu,
                index,
                MF_BYPOSITION | MF_STRING | MF_GRAYED,
                item.command_id.unwrap_or(0),
                wide(&item.label).as_ptr(),
            );
        }
    }
}

unsafe fn append_duration_choice_submenu(
    app: &mut App,
    parent: *mut c_void,
    action: DurationAction,
) -> bool {
    let submenu = CreatePopupMenu();
    if submenu.is_null() {
        app.record(
            "native.CreatePopupMenu.duration_choice.error",
            format!("action={} raw_status={}", action.label(), GetLastError()),
        );
        return false;
    }
    let command_ids: &[usize] = match action {
        DurationAction::Start => &[
            crate::tray_surface::START_IN_1_COMMAND_ID,
            crate::tray_surface::START_IN_5_COMMAND_ID,
            crate::tray_surface::START_IN_15_COMMAND_ID,
            crate::tray_surface::START_IN_30_COMMAND_ID,
            crate::tray_surface::START_IN_60_COMMAND_ID,
        ],
        DurationAction::Stop => &[
            crate::tray_surface::STOP_IN_1_COMMAND_ID,
            crate::tray_surface::STOP_IN_5_COMMAND_ID,
            crate::tray_surface::STOP_IN_15_COMMAND_ID,
            crate::tray_surface::STOP_IN_30_COMMAND_ID,
            crate::tray_surface::STOP_IN_60_COMMAND_ID,
        ],
        DurationAction::Pause => &[
            PAUSE_FOR_5_COMMAND_ID,
            PAUSE_FOR_15_COMMAND_ID,
            PAUSE_FOR_30_COMMAND_ID,
            PAUSE_FOR_60_COMMAND_ID,
        ],
    };
    let choices = duration_choices(action);
    let mut ok = true;
    for (command, item) in command_ids.iter().zip(choices.iter()) {
        let enabled = match action {
            DurationAction::Start => !app.pause.pause_active(),
            DurationAction::Stop | DurationAction::Pause => true,
        };
        let flags = if enabled {
            MF_STRING
        } else {
            MF_STRING | MF_GRAYED
        };
        ok &= append_menu_checked(app, submenu, flags, *command, wide(&item.label).as_ptr());
    }
    if !ok {
        DestroyMenu(submenu);
        return false;
    }
    let label = match action {
        DurationAction::Start => "Start in >",
        DurationAction::Stop => "Stop in >",
        DurationAction::Pause => "Pause for >",
    };
    if !append_menu_checked(
        app,
        parent,
        MF_STRING | MF_POPUP,
        submenu as usize,
        wide(label).as_ptr(),
    ) {
        DestroyMenu(submenu);
        return false;
    }
    true
}

unsafe fn append_duration_submenu(app: &mut App, menu: *mut c_void) -> bool {
    let submenu = CreatePopupMenu();
    if submenu.is_null() {
        app.record(
            "native.CreatePopupMenu.duration.error",
            format!("raw_status={}", GetLastError()),
        );
        return false;
    }
    let mut ok = append_duration_choice_submenu(app, submenu, DurationAction::Start);
    ok &= append_duration_choice_submenu(app, submenu, DurationAction::Stop);
    let items = duration_menu_items(app.pause.current());
    let cancel = items[2].clone();
    let cancel_flags = if cancel.enabled {
        MF_STRING
    } else {
        MF_STRING | MF_GRAYED
    };
    ok &= append_menu_checked(
        app,
        submenu,
        cancel_flags,
        CANCEL_SCHEDULED_COMMAND_ID,
        wide(&cancel.label).as_ptr(),
    );
    let resume = items[3].clone();
    let resume_flags = if resume.enabled {
        MF_STRING
    } else {
        MF_STRING | MF_GRAYED
    };
    ok &= append_menu_checked(
        app,
        submenu,
        resume_flags,
        CANCEL_PAUSE_COMMAND_ID,
        wide(&resume.label).as_ptr(),
    );
    if !ok {
        DestroyMenu(submenu);
        return false;
    }
    if !append_menu_checked(
        app,
        menu,
        MF_STRING | MF_POPUP,
        submenu as usize,
        wide("Schedule >").as_ptr(),
    ) {
        DestroyMenu(submenu);
        return false;
    }
    if let Some(handles) = app.popup_menus.as_mut() {
        handles.duration = Some(submenu);
    }
    true
}

unsafe fn append_status_submenu(app: &mut App, menu: *mut c_void) -> bool {
    let submenu = CreatePopupMenu();
    if submenu.is_null() {
        app.record(
            "native.CreatePopupMenu.status.error",
            format!("raw_status={}", GetLastError()),
        );
        return false;
    }
    let items = status_menu_items(
        app.lifecycle_status(),
        app.timing_values(),
        app.controller.ownership(),
        app.pause.current(),
        app.running_duration(),
        std::time::Instant::now(),
    );
    let mut ok = true;
    for (index, item) in items.iter().enumerate() {
        let flags = if item.enabled {
            MF_STRING
        } else {
            MF_STRING | MF_GRAYED
        };
        ok &= append_menu_checked(
            app,
            submenu,
            flags,
            item.command_id.unwrap_or(0),
            wide(&item.label).as_ptr(),
        );
        if index == 4 {
            app.record(
                "status.submenu.render",
                format!(
                    "rows=state,timing,running_for,next_action,ownership state={:?} ownership={:?} next_action={}",
                    app.lifecycle_status(),
                    app.controller.ownership(),
                    app.pause.current().map_or_else(
                        || "none".to_owned(),
                        |action| format!(
                            "{} remaining_ms={}",
                            action.action.label(),
                            action.remaining(std::time::Instant::now()).as_millis()
                        )
                    )
                ),
            );
            app.record("status.submenu.timing", app.timing_snapshot_details());
        }
    }
    if !ok {
        DestroyMenu(submenu);
        return false;
    }
    if !append_menu_checked(
        app,
        menu,
        MF_STRING | MF_POPUP,
        submenu as usize,
        wide("Status >").as_ptr(),
    ) {
        DestroyMenu(submenu);
        return false;
    }
    if let Some(handles) = app.popup_menus.as_mut() {
        handles.status = Some(submenu);
    }
    true
}

unsafe fn create_menu_help(hwnd: *mut c_void, app: &mut App) -> bool {
    let class_name = wide("tooltips_class32");
    let tooltip = CreateWindowExW(
        WS_EX_TOPMOST,
        class_name.as_ptr(),
        std::ptr::null(),
        WS_POPUP | TTS_ALWAYSTIP | TTS_NOPREFIX,
        0,
        0,
        0,
        0,
        hwnd,
        std::ptr::null_mut(),
        GetModuleHandleW(std::ptr::null()),
        std::ptr::null_mut(),
    );
    if tooltip.is_null() {
        app.record(
            "native.CreateWindowExW.menu_help.error",
            format!("raw_status={}", GetLastError()),
        );
        return false;
    }
    app.menu_help_text = wide("");
    let tool = menu_tool_info(hwnd, app);
    if SendMessageW(
        tooltip,
        TTM_ADDTOOLW,
        0,
        (&tool as *const ToolInfo).cast::<c_void>() as isize,
    ) == 0
    {
        app.record(
            "native.SendMessageW.menu_help_add.error",
            format!("raw_status={}", GetLastError()),
        );
        DestroyWindow(tooltip);
        return false;
    }
    app.menu_help = Some(tooltip);
    true
}

fn menu_tool_info(hwnd: *mut c_void, app: &App) -> ToolInfo {
    ToolInfo {
        cb_size: size_of::<ToolInfo>() as u32,
        flags: TTF_IDISHWND | TTF_TRACK | TTF_ABSOLUTE,
        hwnd,
        id: hwnd as usize,
        rect: Rect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        instance: std::ptr::null_mut(),
        text: app.menu_help_text.as_ptr(),
        l_param: 0,
        reserved: std::ptr::null_mut(),
    }
}

fn track_position_lparam(x: i32, y: i32) -> isize {
    let packed = (x as u32 & 0xffff) | ((y as u32 & 0xffff) << 16);
    packed as usize as isize
}

fn submenu_description(menu: *mut c_void) -> Option<&'static str> {
    if menu.is_null() {
        return None;
    }
    let count = unsafe { GetMenuItemCount(menu) };
    if count < 0 {
        return None;
    }
    for position in 0..count {
        let mut buffer = [0u16; 64];
        let length = unsafe {
            GetMenuStringW(
                menu,
                position as usize,
                buffer.as_mut_ptr(),
                buffer.len() as i32,
                MF_BYPOSITION,
            )
        };
        if length <= 0 {
            continue;
        }
        let label = String::from_utf16_lossy(&buffer[..length as usize]);
        let label = label.trim_end_matches('&');
        let description = match label {
            "Schedule >" => Some("Schedule a bounded timing action"),
            "Start in >" => Some("Schedule a future guarded acquire"),
            "Stop in >" => Some("Schedule a future guarded release"),
            "Pause for >" => Some("Suppress acquisition for a fixed duration"),
            "Resume now" => Some("Resume timing and cancel the pause"),
            "Status >" => Some("View read-only lifecycle details"),
            _ => None,
        };
        if description.is_some() {
            return description;
        }
    }
    None
}

fn update_menu_help(app: &mut App, command: usize, flags: u32, menu: *mut c_void) {
    let Some(tooltip) = app.menu_help else {
        return;
    };
    let description = menu_description(command).or_else(|| {
        if flags & MF_POPUP != 0 {
            submenu_description(menu)
        } else {
            None
        }
    });
    let Some(description) = description else {
        unsafe {
            let tool = menu_tool_info(
                app.tray_icon
                    .as_ref()
                    .map_or(std::ptr::null_mut(), |icon| icon.h_wnd),
                app,
            );
            SendMessageW(
                tooltip,
                TTM_TRACKACTIVATE,
                0,
                (&tool as *const ToolInfo).cast::<c_void>() as isize,
            );
        }
        return;
    };
    app.menu_help_text = wide(description);
    let tool = menu_tool_info(
        app.tray_icon
            .as_ref()
            .map_or(std::ptr::null_mut(), |icon| icon.h_wnd),
        app,
    );
    unsafe {
        SendMessageW(
            tooltip,
            TTM_UPDATETIPTEXTW,
            0,
            (&tool as *const ToolInfo).cast::<c_void>() as isize,
        );
        let mut cursor = Point::default();
        if GetCursorPos(&mut cursor) == 0 {
            app.record(
                "native.GetCursorPos.menu_help.error",
                format!("raw_status={}", GetLastError()),
            );
            SendMessageW(
                tooltip,
                TTM_TRACKACTIVATE,
                0,
                (&tool as *const ToolInfo).cast::<c_void>() as isize,
            );
            return;
        }
        SendMessageW(
            tooltip,
            TTM_TRACKPOSITION,
            0,
            track_position_lparam(cursor.x, cursor.y),
        );
        SendMessageW(
            tooltip,
            TTM_TRACKACTIVATE,
            1,
            (&tool as *const ToolInfo).cast::<c_void>() as isize,
        );
    }
}

fn destroy_menu_help(app: &mut App) {
    if let Some(tooltip) = app.menu_help.take() {
        unsafe {
            let tool = menu_tool_info(
                app.tray_icon
                    .as_ref()
                    .map_or(std::ptr::null_mut(), |icon| icon.h_wnd),
                app,
            );
            SendMessageW(
                tooltip,
                TTM_TRACKACTIVATE,
                0,
                (&tool as *const ToolInfo).cast::<c_void>() as isize,
            );
            if DestroyWindow(tooltip) == 0 {
                app.record(
                    "native.DestroyWindow.menu_help.error",
                    format!("raw_status={}", GetLastError()),
                );
            }
        }
    }
    app.menu_help_text.clear();
}

unsafe fn append_menu_checked(
    app: &mut App,
    menu: *mut c_void,
    flags: u32,
    command: usize,
    text: *const u16,
) -> bool {
    let result = AppendMenuW(menu, flags, command, text);
    let raw_error = if result == 0 { GetLastError() } else { 0 };
    if matches!(
        native_bool_result(result, raw_error),
        NativeResult::Succeeded
    ) {
        true
    } else {
        app.record(
            "native.AppendMenuW.error",
            format!("command={command} raw_status={raw_error}"),
        );
        false
    }
}

unsafe fn handle_menu_command(hwnd: *mut c_void, app: &mut App, command: usize) -> bool {
    if !menu_command_is_enabled_with_pause(
        command,
        app.lifecycle_status(),
        app.config.startup_enabled,
        app.config.automatic,
        app.pause.active(),
    ) {
        let reason = if command == ID_START && app.pause.pause_active() {
            "start_disabled_while_paused"
        } else {
            "disabled_or_unknown"
        };
        app.record(
            "tray.command.rejected",
            format!("id={command} reason={reason}"),
        );
        return false;
    }
    app.begin_operation(DiagnosticSource::TrayCommand);
    app.record("tray.command.id", format!("id={command}"));
    match command {
        ID_START => manual_start(app),
        ID_STOP => manual_stop(app),
        LOGS_COMMAND_ID => {
            app.record("tray.command", "command=logs");
            open_diagnostic_window(app);
        }
        GITHUB_COMMAND_ID => open_github_page(hwnd, app),
        CANCEL_SCHEDULED_COMMAND_ID | CANCEL_PAUSE_COMMAND_ID => cancel_scheduled_action(app),
        START_IN_1_COMMAND_ID
        | START_IN_5_COMMAND_ID
        | START_IN_15_COMMAND_ID
        | START_IN_30_COMMAND_ID
        | START_IN_60_COMMAND_ID
        | STOP_IN_1_COMMAND_ID
        | STOP_IN_5_COMMAND_ID
        | STOP_IN_15_COMMAND_ID
        | STOP_IN_30_COMMAND_ID
        | STOP_IN_60_COMMAND_ID
        | PAUSE_FOR_5_COMMAND_ID
        | PAUSE_FOR_15_COMMAND_ID
        | PAUSE_FOR_30_COMMAND_ID
        | PAUSE_FOR_60_COMMAND_ID => {
            if let Some((action, duration)) = duration_command(command) {
                schedule_duration_action(app, action, duration);
            }
        }
        ID_STARTUP_ON => set_startup(app, true),
        ID_STARTUP_OFF => set_startup(app, false),
        ID_AUTOMATIC_ON => set_automatic(app, true),
        ID_AUTOMATIC_OFF => set_automatic(app, false),
        ID_QUIT => {
            app.record("tray.command", "command=quit");
            app.record("lifecycle.shutdown_request", "source=tray");
            app.record("quit.requested", "source=tray");
            let decision = quit_decision(app);
            app.record(
                "quit.active_state",
                format!(
                    "decision={decision:?} status={:?} ownership={:?} verification={:?}",
                    app.tray_status,
                    app.controller.ownership(),
                    app.controller.verification()
                ),
            );
            match decision {
                QuitDecision::ExitNormally => {
                    app.record("quit.dialog.result", "result=not_shown");
                    match app.cleanup_normal_shutdown() {
                        Ok(()) => {
                            app.record("quit.cleanup.result", "result=verified");
                            app.record(
                                "quit.exit.allowed",
                                "result=allowed reason=already_stopped",
                            );
                            PostQuitMessage(0);
                            return false;
                        }
                        Err(error) => {
                            app.record(
                                "quit.cleanup.result",
                                format!("result=unverified error={error}"),
                            );
                            app.record("quit.blocked.uncertain_cleanup", format!("error={error}"));
                            app.record("quit.exit.allowed", "result=denied");
                            show_shutdown_warning(
                                hwnd,
                                &format!(
                                    "True™ Tick could not verify a safe stop. The app remains open.\n\n{error}"
                                ),
                            );
                            return true;
                        }
                    }
                }
                QuitDecision::RequireSafetyDialog { reason } => {
                    app.record("quit.warning.shown", format!("reason={reason:?}"));
                    let warning = show_quit_warning(hwnd, reason);
                    app.record(
                        "quit.dialog.result",
                        format!(
                            "dialog_path=message_box reason={reason:?} message_box_result={:?} final_decision={:?} dialog_shown={}",
                            warning.message_box_result,
                            warning.decision,
                            warning.dialog_shown
                        ),
                    );
                    match warning.decision {
                        QuitDialogDecision::StopAndQuit => {
                            app.record("quit.dialog.result", "result=stop_and_quit");
                            app.record("quit.stop_and_quit.selected", "result=selected");
                            match app.cleanup_normal_shutdown() {
                                Ok(()) => {
                                    app.record("quit.cleanup.result", "result=verified");
                                    app.record("quit.release.result", "result=verified");
                                    app.record("quit.exit.allowed", "result=allowed");
                                    PostQuitMessage(0);
                                    return false;
                                }
                                Err(error) => {
                                    app.record(
                                        "quit.cleanup.result",
                                        format!("result=unverified error={error}"),
                                    );
                                    app.record(
                                        "quit.blocked.uncertain_cleanup",
                                        format!("error={error}"),
                                    );
                                    app.record("quit.exit.allowed", "result=denied");
                                    let message = format!(
                                        "True™ Tick could not verify a safe stop. The app remains open.\n\n{error}"
                                    );
                                    MessageBoxW(
                                        hwnd,
                                        wide(&message).as_ptr(),
                                        wide("True™ Tick quit warning").as_ptr(),
                                        MB_ICONWARNING,
                                    );
                                    return true;
                                }
                            }
                        }
                        QuitDialogDecision::Cancel => {
                            app.record(
                                "quit.cancel.selected",
                                format!("result=cancelled dialog_shown={}", warning.dialog_shown),
                            );
                            app.record("quit.cleanup.result", "result=not_attempted");
                            app.record("quit.exit.allowed", "result=denied");
                            return warning.dialog_shown;
                        }
                    }
                }
            }
        }
        _ => {}
    }
    let keeps_open = menu_action_keeps_open(command);
    if command != ID_QUIT {
        app.finish_operation(DiagnosticOutcome::Completed);
    }
    keeps_open
}

unsafe fn open_github_page(hwnd: *mut c_void, app: &mut App) {
    let operation = wide("open");
    let url = wide(GITHUB_URL);
    let result = ShellExecuteW(
        hwnd,
        operation.as_ptr(),
        url.as_ptr(),
        std::ptr::null(),
        std::ptr::null(),
        SW_SHOWNORMAL,
    );
    if (result as isize) <= 32 {
        app.record(
            "native.ShellExecuteW.github.error",
            format!("result={} raw_status={}", result as isize, GetLastError()),
        );
    } else {
        app.record(
            "native.ShellExecuteW.github.result",
            format!("result=success url={GITHUB_URL}"),
        );
    }
}

unsafe fn open_diagnostic_window(app: &mut App) {
    if let Some(window) = app.diagnostic_window {
        if IsWindow(window) == 0 {
            app.diagnostic_window = None;
        } else {
            if IsIconic(window) != 0 {
                ShowWindow(window, SW_RESTORE);
            } else {
                ShowWindow(window, SW_SHOWNORMAL);
            }
            UpdateWindow(window);
            SetForegroundWindow(window);
            request_diagnostic_refresh(app);
            return;
        }
    }
    let class_name = wide("TrueTickDiagnosticClass");
    let title = wide(DIAGNOSTIC_WINDOW_TITLE);
    let window = CreateWindowExW(
        diagnostic_window_extended_style(),
        class_name.as_ptr(),
        title.as_ptr(),
        diagnostic_window_style(),
        120,
        120,
        DIAGNOSTIC_DEFAULT_WIDTH,
        DIAGNOSTIC_DEFAULT_HEIGHT,
        DIAGNOSTIC_WINDOW_PARENT,
        std::ptr::null_mut(),
        GetModuleHandleW(std::ptr::null()),
        app as *mut App as *mut c_void,
    );
    if window.is_null() {
        app.record(
            "diagnostic.window.result",
            format!("result=create_failed raw_status={}", GetLastError()),
        );
    } else {
        if SetWindowTextW(window, title.as_ptr()) == 0 {
            app.record(
                "native.SetWindowTextW.diagnostic.error",
                format!("raw_status={}", GetLastError()),
            );
            DestroyWindow(window);
            return;
        }
        app.diagnostic_window = Some(window);
        layout_diagnostic_controls(window, app);
        ShowWindow(window, SW_SHOWNORMAL);
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

fn diagnostic_visible_selection(app: &App, retained_rows: usize) -> RowSelection {
    if app.diagnostic_display_all {
        RowSelection::all(retained_rows)
    } else {
        latest_row_selection(app.diagnostic_display_limit, retained_rows)
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
    let visible = diagnostic_visible_selection(app, retained).row_count();
    format!(
        "{}\r\n{}\r\n{}\r\n{}\r\n{}\r\nPower: {}\r\nStartup: {}\r\nShowing {visible} of {retained} retained rows\r\nTransfer selection is independent of Show rows. Hidden retained rows can be copied or exported intentionally.\r\nRetention cap: {} events. Newest retained events are shown in chronological order. Timing is current only when the latest observation is valid, otherwise it is Unknown.\r\n",
        status[0].label,
        status[1].label,
        status[2].label,
        status[3].label,
        status[4].label,
        power_state_label(app.observation.power().state),
        app.startup_status,
        app.diagnostics.maximum_events(),
    )
}

fn scale_logical(value: i32, dpi: u32) -> i32 {
    value.saturating_mul(dpi as i32).saturating_add(95) / 96
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DiagnosticLayoutRect {
    left: i32,
    top: i32,
    width: i32,
    height: i32,
}

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
    transfer_label: DiagnosticLayoutRect,
    range_input: DiagnosticLayoutRect,
    selection_summary: DiagnosticLayoutRect,
    copy: DiagnosticLayoutRect,
    export: DiagnosticLayoutRect,
    message: DiagnosticLayoutRect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DiagnosticLayout {
    summary: DiagnosticLayoutRect,
    toolbar: DiagnosticToolbarLayout,
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
        DIAGNOSTIC_SUMMARY_HEIGHT + DIAGNOSTIC_TOOLBAR_HEIGHT + DIAGNOSTIC_GRID_MIN_HEIGHT,
        dpi.max(96),
    )
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
    let transfer_label = next(widths[3]);
    let range_input = next(widths[4]);
    let selection_summary = next(widths[5]);
    let copy = next(widths[6]);
    let export = next(widths[7]);
    let message_left = cursor;
    let message_width = width
        .saturating_sub(message_left)
        .saturating_sub(margin)
        .max(0);
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
        transfer_label,
        range_input,
        selection_summary,
        copy,
        export,
        message,
    }
}

fn diagnostic_layout(width: i32, height: i32, dpi: u32) -> DiagnosticLayout {
    let dpi = dpi.max(96);
    let toolbar_height = scale_logical(DIAGNOSTIC_TOOLBAR_HEIGHT, dpi);
    let grid_minimum = scale_logical(DIAGNOSTIC_GRID_MIN_HEIGHT, dpi);
    let summary_height = scale_logical(DIAGNOSTIC_SUMMARY_HEIGHT, dpi).min(
        height
            .saturating_sub(toolbar_height)
            .saturating_sub(grid_minimum)
            .max(0),
    );
    let list_top = summary_height.saturating_add(toolbar_height);
    DiagnosticLayout {
        summary: DiagnosticLayoutRect {
            left: 0,
            top: 0,
            width: width.max(0),
            height: summary_height,
        },
        toolbar: diagnostic_toolbar_layout(width, summary_height, toolbar_height, dpi),
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
            app.diagnostics.record(
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

unsafe fn list_selected_item_positions(list: *mut c_void) -> Vec<usize> {
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

unsafe fn update_diagnostic_grid_selection(app: &mut App) {
    if app.diagnostic_refreshing {
        return;
    }
    let Some(list) = app.diagnostic_list else {
        return;
    };
    let events = app.diagnostics.snapshot();
    let positions = list_selected_item_positions(list);
    let selected = selected_event_sequences_for_positions(
        &events,
        diagnostic_visible_selection(app, events.len()),
        &positions,
    );
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
        app.diagnostics.record(
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
    let positions = visible_positions_for_sequences(
        events,
        diagnostic_visible_selection(app, events.len()),
        &app.diagnostic_grid_selection_sequences,
    );
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
    app.diagnostics.record(
        "diagnostic.layout.error",
        format!(
            "stage={stage} control_index={} raw_status={raw_status}",
            index.map_or_else(|| "none".to_owned(), |value| value.to_string())
        ),
    );
}

unsafe fn fallback_diagnostic_layout(
    window: *mut c_void,
    app: &App,
    controls: &[(Option<*mut c_void>, DiagnosticLayoutRect)],
) -> bool {
    let mut success = true;
    for (index, (control, rect)) in controls.iter().enumerate() {
        let Some(control) = *control else {
            record_diagnostic_layout_failure(app, "missing_control", Some(index), 0);
            success = false;
            continue;
        };
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
        }
    }
    let _ = InvalidateRect(window, std::ptr::null(), 1);
    success
}

unsafe fn apply_diagnostic_layout(
    window: *mut c_void,
    app: &App,
    controls: &[(Option<*mut c_void>, DiagnosticLayoutRect)],
) -> bool {
    let mut defer = BeginDeferWindowPos(controls.len() as i32);
    if defer.is_null() {
        record_diagnostic_layout_failure(app, "BeginDeferWindowPos", None, GetLastError());
        return fallback_diagnostic_layout(window, app, controls);
    }
    let mut deferred_failure = false;
    for (index, (control, rect)) in controls.iter().enumerate() {
        let Some(control) = *control else {
            record_diagnostic_layout_failure(app, "missing_control", Some(index), 0);
            deferred_failure = true;
            continue;
        };
        if deferred_failure {
            continue;
        }
        let next = DeferWindowPos(
            defer,
            control,
            std::ptr::null_mut(),
            rect.left,
            rect.top,
            rect.width,
            rect.height,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOSENDCHANGING,
        );
        if next.is_null() {
            record_diagnostic_layout_failure(app, "DeferWindowPos", Some(index), GetLastError());
            deferred_failure = true;
        } else {
            defer = next;
        }
    }
    let end_result = EndDeferWindowPos(defer);
    if end_result == 0 {
        record_diagnostic_layout_failure(app, "EndDeferWindowPos", None, GetLastError());
        deferred_failure = true;
    }
    if deferred_failure {
        fallback_diagnostic_layout(window, app, controls)
    } else {
        let _ = InvalidateRect(window, std::ptr::null(), 1);
        true
    }
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
        (app.diagnostic_summary, layout.summary),
        (app.diagnostic_display_label, toolbar.display_label),
        (app.diagnostic_display_input, toolbar.display_input),
        (app.diagnostic_show_all_button, toolbar.show_all),
        (app.diagnostic_toolbar_label, toolbar.transfer_label),
        (app.diagnostic_range_input, toolbar.range_input),
        (app.diagnostic_selection_summary, toolbar.selection_summary),
        (app.diagnostic_copy_button, toolbar.copy),
        (app.diagnostic_export_button, toolbar.export),
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

unsafe fn set_diagnostic_redraw(app: &App, enabled: bool) {
    let redraw = usize::from(enabled);
    if let Some(window) = app.diagnostic_window {
        let _ = SendMessageW(window, WM_SETREDRAW, redraw, 0);
    }
    for control in [
        app.diagnostic_summary,
        app.diagnostic_display_label,
        app.diagnostic_display_input,
        app.diagnostic_show_all_button,
        app.diagnostic_toolbar_label,
        app.diagnostic_selection_summary,
        app.diagnostic_range_input,
        app.diagnostic_copy_button,
        app.diagnostic_export_button,
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
    if let Some(list) = app.diagnostic_list {
        let _ = InvalidateRect(list, std::ptr::null(), 1);
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

unsafe fn refresh_diagnostic_window(window: *mut c_void, app: &mut App) {
    if app.diagnostic_refreshing {
        return;
    }
    app.diagnostic_refreshing = true;
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
        finish_diagnostic_refresh(app);
        return;
    };
    let snapshot_rows = events.len();
    let _ = SendMessageW(list, LVM_DELETEALLITEMS, 0, 0);
    let rows = diagnostic_grid_rows(&events, diagnostic_visible_selection(app, snapshot_rows));
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
            app.diagnostics.record(
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
                app.diagnostics.record(
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
    app.diagnostics.record(
        "diagnostic.grid.refresh",
        diagnostic_grid_refresh_message(
            snapshot_rows,
            inserted_rows,
            item_count,
            insert_failures,
            set_text_failures,
        ),
    );
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
    if let Some(position) = horizontal_scroll {
        let _ = SetScrollPos(list, SB_HORZ, position, 0);
    }
    if let Some(summary) = app.diagnostic_summary {
        let text = wide(&diagnostic_summary_text(app, retained_rows));
        let _ = SetWindowTextW(summary, text.as_ptr());
    }
    app.diagnostic_layout_stable = true;
    finish_diagnostic_redraw(window, app);
    finish_diagnostic_refresh(app);
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
    let filter = wide("TSV files (*.tsv)\0*.tsv\0All files (*.*)\0*.*\0\0");
    let title = wide("Export True™ Tick diagnostic log");
    let default_extension = wide("tsv");
    let mut file = wide("true-tick-log.tsv");
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
    let details = diagnostic_transfer_details(source, selected_row_count, retained_rows, range);
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
            let write_result = unsafe { write_export_tsv(&path, &tsv) };
            match write_result {
                Ok(()) => {
                    unsafe {
                        set_diagnostic_message(
                            app,
                            format!("Exported {} rows as TSV.", rows.len()),
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
    let Ok(limit) = parse_display_limit(&input) else {
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
        app.finish_operation(DiagnosticOutcome::Failed);
        return;
    };
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

fn request_diagnostic_refresh(app: &mut App) {
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

fn finish_diagnostic_refresh(app: &mut App) {
    app.diagnostic_refreshing = false;
    if app.diagnostic_refresh_pending {
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

unsafe fn set_diagnostic_control_font(control: *mut c_void) {
    if control.is_null() {
        return;
    }
    let font = GetStockObject(DEFAULT_GUI_FONT);
    if !font.is_null() {
        let _ = SendMessageW(control, WM_SETFONT, font as usize, 1);
    }
}

unsafe extern "system" fn diagnostic_window_proc(
    hwnd: *mut c_void,
    message: u32,
    w_param: usize,
    l_param: isize,
) -> isize {
    let app = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
    if message == WM_CREATE {
        let create = l_param as *const CreateStruct;
        let app_ptr = app_create_params(create);
        let app = app_ptr as *mut App;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, app_ptr as isize);
        let static_class = wide("STATIC");
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
        let display_label = CreateWindowExW(
            0,
            static_class.as_ptr(),
            wide("Show rows:").as_ptr(),
            WS_CHILD | WS_VISIBLE | SS_LEFT,
            0,
            0,
            76,
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
            64,
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
            76,
            24,
            hwnd,
            ID_DIAGNOSTIC_SHOW_ALL as *mut c_void,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null_mut(),
        );
        let label = CreateWindowExW(
            0,
            static_class.as_ptr(),
            wide("Rows to copy/export:").as_ptr(),
            WS_CHILD | WS_VISIBLE | SS_LEFT,
            0,
            0,
            132,
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
            136,
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
            120,
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
            68,
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
            68,
            24,
            hwnd,
            ID_DIAGNOSTIC_EXPORT as *mut c_void,
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
            DIAGNOSTIC_SUMMARY_HEIGHT + DIAGNOSTIC_TOOLBAR_HEIGHT,
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
        let display_label_error = control_error(display_label);
        let display_input_error = control_error(display_input);
        let show_all_error = control_error(show_all_button);
        let label_error = control_error(label);
        let selection_summary_error = control_error(selection_summary);
        let range_error = control_error(range_input);
        let copy_error = control_error(copy_button);
        let export_error = control_error(export_button);
        let message_error = control_error(message);
        let list_error = control_error(list);
        if summary.is_null()
            || display_label.is_null()
            || display_input.is_null()
            || show_all_button.is_null()
            || label.is_null()
            || selection_summary.is_null()
            || range_input.is_null()
            || copy_button.is_null()
            || export_button.is_null()
            || message.is_null()
            || list.is_null()
        {
            if !app_ptr.is_null() {
                (*(app_ptr as *mut App)).diagnostics.record(
                    "native.CreateWindowExW.diagnostic_control.error",
                    format!(
                        "summary_null={} summary_raw_status={} display_label_null={} display_label_raw_status={} display_input_null={} display_input_raw_status={} show_all_null={} show_all_raw_status={} label_null={} label_raw_status={} selection_summary_null={} selection_summary_raw_status={} range_null={} range_raw_status={} copy_null={} copy_raw_status={} export_null={} export_raw_status={} message_null={} message_raw_status={} list_null={} list_raw_status={}",
                        summary.is_null(),
                        summary_error,
                        display_label.is_null(),
                        display_label_error,
                        display_input.is_null(),
                        display_input_error,
                        show_all_button.is_null(),
                        show_all_error,
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
                        message.is_null(),
                        message_error,
                        list.is_null(),
                        list_error
                    ),
                );
            }
            for control in [
                summary,
                display_label,
                display_input,
                show_all_button,
                label,
                selection_summary,
                range_input,
                copy_button,
                export_button,
                message,
                list,
            ] {
                if !control.is_null() {
                    DestroyWindow(control);
                }
            }
            return 1;
        }
        for control in [
            summary,
            display_label,
            display_input,
            show_all_button,
            label,
            selection_summary,
            range_input,
            copy_button,
            export_button,
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
            DestroyWindow(summary);
            DestroyWindow(list);
            return 1;
        }
        if !app_ptr.is_null() {
            (*app).diagnostic_summary = Some(summary);
            (*app).diagnostic_display_label = Some(display_label);
            (*app).diagnostic_display_input = Some(display_input);
            (*app).diagnostic_show_all_button = Some(show_all_button);
            (*app).diagnostic_toolbar_label = Some(label);
            (*app).diagnostic_selection_summary = Some(selection_summary);
            (*app).diagnostic_range_input = Some(range_input);
            (*app).diagnostic_copy_button = Some(copy_button);
            (*app).diagnostic_export_button = Some(export_button);
            (*app).diagnostic_message = Some(message);
            (*app).diagnostic_list = Some(list);
            layout_diagnostic_controls(hwnd, &*app);
        }
        return 0;
    }
    if !app.is_null() {
        if message == WM_ERASEBKGND {
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
            if command == ID_DIAGNOSTIC_DISPLAY_LIMIT && notification == EN_CHANGE {
                diagnostic_display_changed(&mut *app);
                return 0;
            }
            if command == ID_DIAGNOSTIC_SHOW_ALL && notification == BN_CLICKED {
                diagnostic_show_all(&mut *app);
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
        } else if message == WM_DIAGNOSTIC_REFRESH {
            (*app).diagnostic_refresh_pending = false;
            refresh_diagnostic_window(hwnd, &mut *app);
            return 0;
        } else if message == WM_SIZE {
            layout_diagnostic_controls(hwnd, &*app);
        } else if message == WM_GETMINMAXINFO {
            let limits = l_param as *mut MinMaxInfo;
            if !limits.is_null() {
                let dpi = GetDpiForWindow(hwnd).max(96);
                (*limits).minimum_track_size.x = diagnostic_toolbar_min_width(dpi);
                (*limits).minimum_track_size.y = diagnostic_min_client_height(dpi);
            }
            return 0;
        } else if message == WM_CLOSE {
            DestroyWindow(hwnd);
            return 0;
        } else if message == WM_SETFOCUS {
            request_diagnostic_refresh(&mut *app);
        } else if message == WM_NCDESTROY {
            (*app).diagnostic_window = None;
            (*app).diagnostic_summary = None;
            (*app).diagnostic_display_label = None;
            (*app).diagnostic_display_input = None;
            (*app).diagnostic_show_all_button = None;
            (*app).diagnostic_toolbar_label = None;
            (*app).diagnostic_selection_summary = None;
            (*app).diagnostic_range_input = None;
            (*app).diagnostic_copy_button = None;
            (*app).diagnostic_export_button = None;
            (*app).diagnostic_message = None;
            (*app).diagnostic_list = None;
            (*app).diagnostic_selection = None;
            (*app).diagnostic_selection_sequences = None;
            (*app).diagnostic_grid_selection_sequences.clear();
            (*app).diagnostic_grid_selection_reset = false;
            (*app).diagnostic_selection_reset = false;
            (*app).diagnostic_message_text.clear();
            (*app).diagnostic_refresh_pending = false;
            (*app).diagnostic_refreshing = false;
            (*app).diagnostic_layout_stable = false;
            (*app).diagnostic_snapshot_key = None;
            (*app).diagnostic_snapshot_generation = 0;
            (*app).diagnostic_auto_fit_generation = None;
        }
    }
    DefWindowProcW(hwnd, message, w_param, l_param)
}

fn set_automatic(app: &mut App, enabled: bool) {
    app.record(
        "tray.command",
        format!("command=automatic enabled={enabled}"),
    );
    app.record(
        "toggle.requested",
        format!("setting=automatic requested={enabled}"),
    );
    app.record(
        "policy.automatic_setting_changed",
        format!("enabled={enabled}"),
    );
    let mut next = app.config.clone();
    next.automatic = enabled;
    if let Err(error) = config::save_atomic(&app.config_path, &next) {
        app.record(
            "config.save.result",
            format!("result=error setting=automatic error={error}"),
        );
        app.record(
            "toggle.result",
            format!(
                "setting=automatic value={} result=unchanged",
                app.config.automatic
            ),
        );
        app.publish();
        return;
    }
    app.record(
        "config.save.result",
        format!("result=success setting=automatic value={enabled}"),
    );
    app.config = next;
    app.record(
        "toggle.result",
        format!(
            "setting=automatic value={} result=applied",
            app.config.automatic
        ),
    );
    if enabled {
        reconcile(app);
    } else {
        manual_stop(app);
    }
}

fn set_startup(app: &mut App, enabled: bool) {
    app.record("tray.command", format!("command=startup enabled={enabled}"));
    app.record(
        "toggle.requested",
        format!("setting=startup_enabled requested={enabled}"),
    );

    let target = if enabled {
        match startup_target(&app.executable) {
            Ok(target) => Some(target),
            Err(error) => {
                app.record(
                    "startup.registration.result",
                    format!("result=unavailable error={error}"),
                );
                let mut next = app.config.clone();
                next.startup_enabled = enabled;
                if let Err(save_error) = config::save_atomic(&app.config_path, &next) {
                    app.record(
                        "config.save.result",
                        format!("result=error setting=startup_enabled error={save_error}"),
                    );
                    app.record(
                        "toggle.result",
                        format!(
                            "setting=startup_enabled value={} result=unchanged",
                            app.config.startup_enabled
                        ),
                    );
                    app.startup_status = bounded_startup_status(
                        "startup registration unavailable and config persistence failed",
                    );
                } else {
                    app.record(
                        "config.save.result",
                        "result=success setting=startup_enabled value=true",
                    );
                    app.config = next;
                    app.record(
                        "toggle.result",
                        "setting=startup_enabled value=true result=applied",
                    );
                    app.startup_status = bounded_startup_status(format!(
                        "auto-start enabled in config, current-user registration unavailable: {error}"
                    ));
                }
                app.publish();
                return;
            }
        }
    } else {
        None
    };

    let registration = if let Some(target) = target.as_ref() {
        app.record("native.RegSetValueExW.call", "value=TrueTick path=redacted");
        register_startup_target(target)
    } else {
        app.record("native.RegDeleteValueW.call", "value=TrueTick");
        let mut startup = WindowsUserStartup;
        startup.remove()
    };
    if let Err(error) = registration {
        app.record(
            "startup.registration.result",
            format!("result=error error={error:?}"),
        );
        app.record(
            "toggle.result",
            format!(
                "setting=startup_enabled value={} result=unchanged",
                app.config.startup_enabled
            ),
        );
        app.startup_status = bounded_startup_status("startup registration error");
        app.publish();
        return;
    }

    let mut next = app.config.clone();
    next.startup_enabled = enabled;
    if let Err(error) = config::save_atomic(&app.config_path, &next) {
        app.record(
            "config.save.result",
            format!("result=error setting=startup_enabled error={error}"),
        );
        let rollback = if enabled {
            let mut startup = WindowsUserStartup;
            startup.remove()
        } else {
            match startup_target(&app.executable) {
                Ok(target) => register_startup_target(&target),
                Err(_error) => Err(tick_startup_windows::StartupError::InvalidExecutablePath),
            }
        };
        app.record(
            "startup.registration.rollback",
            format!("result={rollback:?}"),
        );
        app.record(
            "toggle.result",
            format!(
                "setting=startup_enabled value={} result=unchanged",
                app.config.startup_enabled
            ),
        );
        app.startup_status = bounded_startup_status(if rollback.is_ok() {
            "startup config persistence failed, registry change rolled back"
        } else {
            "startup config persistence failed, repair required"
        });
        app.publish();
        return;
    }
    app.record(
        "config.save.result",
        format!("result=success setting=startup_enabled value={enabled}"),
    );
    app.config = next;
    app.record(
        "toggle.result",
        format!(
            "setting=startup_enabled value={} result=applied",
            app.config.startup_enabled
        ),
    );
    app.record(
        "startup.registration.result",
        format!("result=success enabled={enabled}"),
    );
    app.startup_status = bounded_startup_status(if let Some(target) = target {
        format!(
            "boot startup registered for current-user {}",
            target.description()
        )
    } else {
        "boot startup registration disabled by config".to_owned()
    });
    app.publish();
}

fn quit_decision(app: &App) -> QuitDecision {
    quit_decision_for_handoff(
        app.lifecycle_status(),
        app.controller.ownership(),
        app.handoff.is_some(),
    )
}

fn quit_decision_for_handoff(
    status: TrayStatus,
    ownership: OwnershipState,
    handoff_active: bool,
) -> QuitDecision {
    if handoff_active && ownership == OwnershipState::Released {
        return QuitDecision::ExitNormally;
    }
    if handoff_active {
        return QuitDecision::RequireSafetyDialog {
            reason: QuitSafetyReason::TimingNotSettled,
        };
    }
    quit_decision_for(status, ownership)
}

fn quit_decision_for(status: TrayStatus, ownership: OwnershipState) -> QuitDecision {
    match ownership {
        OwnershipState::Owned => {
            return QuitDecision::RequireSafetyDialog {
                reason: QuitSafetyReason::OwnedActive,
            };
        }
        OwnershipState::Uncertain => {
            return QuitDecision::RequireSafetyDialog {
                reason: QuitSafetyReason::OwnershipUncertain,
            };
        }
        OwnershipState::Released => {}
    }
    if matches!(
        status,
        TrayStatus::Running
            | TrayStatus::Starting
            | TrayStatus::Stopping
            | TrayStatus::Pending
            | TrayStatus::Degraded
            | TrayStatus::Unverified
    ) {
        return QuitDecision::RequireSafetyDialog {
            reason: QuitSafetyReason::TimingNotSettled,
        };
    }
    QuitDecision::ExitNormally
}

fn quit_warning_text(reason: QuitSafetyReason) -> &'static str {
    match reason {
        QuitSafetyReason::OwnershipUncertain => {
            "True™ Tick could not verify that timing is fully released. Keep the app open and retry cleanup?"
        }
        QuitSafetyReason::OwnedActive | QuitSafetyReason::TimingNotSettled => {
            "True™ Tick is currently controlling timer resolution. Stop timing and quit?"
        }
    }
}

unsafe fn show_quit_warning(hwnd: *mut c_void, reason: QuitSafetyReason) -> QuitWarningResult {
    let title = wide("True™ Tick");
    let text = wide(quit_warning_text(reason));
    let result = MessageBoxW(
        hwnd,
        text.as_ptr(),
        title.as_ptr(),
        MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
    );
    quit_warning_result(Some(result))
}

fn quit_warning_result(message_box_result: Option<i32>) -> QuitWarningResult {
    let result = message_box_result.unwrap_or(0);
    QuitWarningResult {
        decision: message_box_decision(result),
        message_box_result,
        dialog_shown: result != 0,
    }
}

fn message_box_decision(result: i32) -> QuitDialogDecision {
    if result == IDYES {
        QuitDialogDecision::StopAndQuit
    } else {
        QuitDialogDecision::Cancel
    }
}

fn returned_menu_command(result: i32) -> Option<usize> {
    (result > 0).then_some(result as usize)
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

impl Drop for NotifyIconData {
    fn drop(&mut self) {
        unsafe { destroy_icon(self.h_icon) };
        self.h_icon = std::ptr::null_mut();
    }
}

impl NotifyIconData {
    fn new(
        hwnd: *mut c_void,
        status: TrayStatus,
        timing: TimingValues,
        scheduled: Option<crate::pause::ScheduledAction>,
    ) -> Result<Self, u32> {
        let mut value = Self {
            cb_size: size_of::<Self>() as u32,
            h_wnd: hwnd,
            u_id: 1,
            u_flags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            u_callback_message: WM_TRAY,
            h_icon: unsafe { status_icon(status, dpi_for_window(hwnd)) }?,
            sz_tip: [0; 128],
            dw_state: 0,
            dw_state_mask: 0,
            sz_info: [0; 256],
            u_timeout_or_version: 0,
            sz_info_title: [0; 64],
            dw_info_flags: 0,
            guid: [0; 16],
            h_balloon_icon: std::ptr::null_mut(),
        };
        for (target, source) in value
            .sz_tip
            .iter_mut()
            .zip(tooltip_at(status, timing, scheduled, std::time::Instant::now()).encode_utf16())
        {
            *target = source;
        }
        Ok(value)
    }
}

#[repr(C)]
#[derive(Default)]
struct Message {
    hwnd: *mut c_void,
    message: u32,
    w_param: usize,
    l_param: isize,
    time: u32,
    point: Point,
}
#[repr(C)]
#[derive(Default)]
struct Point {
    x: i32,
    y: i32,
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
    fn RegisterClassW(class: *const WndClass) -> u16;
    fn CreateWindowExW(
        ex: u32,
        class: *const u16,
        title: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: *mut c_void,
        menu: *mut c_void,
        instance: *mut c_void,
        param: *mut c_void,
    ) -> *mut c_void;
    fn GetMessageW(message: *mut Message, hwnd: *mut c_void, min: u32, max: u32) -> i32;
    fn TranslateMessage(message: *const Message) -> i32;
    fn DispatchMessageW(message: *const Message) -> isize;
    fn DefWindowProcW(hwnd: *mut c_void, message: u32, w: usize, l: isize) -> isize;
    fn GetWindowLongPtrW(hwnd: *mut c_void, index: i32) -> isize;
    fn SetWindowLongPtrW(hwnd: *mut c_void, index: i32, value: isize) -> isize;
    fn IsWindow(window: *mut c_void) -> i32;
    fn IsIconic(window: *mut c_void) -> i32;
    fn PostQuitMessage(code: i32);
    fn PostMessageW(hwnd: *mut c_void, message: u32, w: usize, l: isize) -> i32;
    fn SetTimer(
        hwnd: *mut c_void,
        event_id: usize,
        interval_ms: u32,
        callback: *mut c_void,
    ) -> usize;
    fn KillTimer(hwnd: *mut c_void, event_id: usize) -> i32;
    fn MessageBoxW(hwnd: *mut c_void, text: *const u16, title: *const u16, flags: u32) -> i32;
    fn CreatePopupMenu() -> *mut c_void;
    fn GetMenuItemCount(menu: *mut c_void) -> i32;
    fn GetMenuStringW(
        menu: *mut c_void,
        item: usize,
        text: *mut u16,
        maximum: i32,
        flags: u32,
    ) -> i32;
    fn AppendMenuW(menu: *mut c_void, flags: u32, id: usize, text: *const u16) -> i32;
    fn ModifyMenuW(
        menu: *mut c_void,
        item: usize,
        flags: u32,
        new_item: usize,
        text: *const u16,
    ) -> i32;
    fn SetForegroundWindow(hwnd: *mut c_void) -> i32;
    fn TrackPopupMenu(
        menu: *mut c_void,
        flags: u32,
        x: i32,
        y: i32,
        reserved: i32,
        hwnd: *mut c_void,
        rect: *const c_void,
    ) -> i32;
    fn DestroyMenu(menu: *mut c_void) -> i32;
    fn DestroyWindow(window: *mut c_void) -> i32;
    fn ShowWindow(window: *mut c_void, command: i32) -> i32;
    fn UpdateWindow(window: *mut c_void) -> i32;
    fn BeginPaint(window: *mut c_void, paint: *mut PaintStruct) -> *mut c_void;
    fn EndPaint(window: *mut c_void, paint: *const PaintStruct) -> i32;
    fn GetSysColor(index: i32) -> u32;
    fn GetSysColorBrush(index: i32) -> *mut c_void;
    fn SetTextColor(hdc: *mut c_void, color: u32) -> u32;
    fn SetBkColor(hdc: *mut c_void, color: u32) -> u32;

    fn SetWindowTextW(window: *mut c_void, text: *const u16) -> i32;
    fn OpenClipboard(owner: *mut c_void) -> i32;
    fn EmptyClipboard() -> i32;
    fn SetClipboardData(format: u32, data: *mut c_void) -> *mut c_void;
    fn CloseClipboard() -> i32;
    fn GetWindowTextLengthW(window: *mut c_void) -> i32;
    fn GetWindowTextW(window: *mut c_void, text: *mut u16, maximum: i32) -> i32;
    fn GetKeyState(key: i32) -> i16;
    fn EnableWindow(window: *mut c_void, enable: i32) -> i32;
    fn GetClientRect(window: *mut c_void, rect: *mut Rect) -> i32;
    fn SendMessageW(hwnd: *mut c_void, message: u32, w: usize, l: isize) -> isize;
    fn BeginDeferWindowPos(number: i32) -> *mut c_void;
    fn SetWindowPos(
        window: *mut c_void,
        insert_after: *mut c_void,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        flags: u32,
    ) -> i32;
    fn DeferWindowPos(
        defer: *mut c_void,
        window: *mut c_void,
        insert_after: *mut c_void,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        flags: u32,
    ) -> *mut c_void;
    fn EndDeferWindowPos(defer: *mut c_void) -> i32;
    fn InvalidateRect(window: *mut c_void, rect: *const Rect, erase: i32) -> i32;
    fn GetScrollPos(window: *mut c_void, bar: i32) -> i32;
    fn SetScrollPos(window: *mut c_void, bar: i32, position: i32, redraw: i32) -> i32;
    fn GetCursorPos(point: *mut Point) -> i32;
    fn LoadIconW(instance: *mut c_void, name: *const u16) -> *mut c_void;
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    fn GetDpiForWindow(window: *mut c_void) -> u32;
    fn FillRect(hdc: *mut c_void, rect: *const Rect, brush: *mut c_void) -> i32;
    fn CreateIconIndirect(info: *const IconInfo) -> *mut c_void;
    fn DestroyIcon(icon: *mut c_void) -> i32;
}

#[link(name = "comctl32")]
extern "system" {
    fn InitCommonControlsEx(init: *const InitCommonControlsEx) -> i32;
}

#[link(name = "shell32")]
extern "system" {
    fn Shell_NotifyIconW(message: u32, data: *mut NotifyIconData) -> i32;
    fn ShellExecuteW(
        hwnd: *mut c_void,
        operation: *const u16,
        file: *const u16,
        parameters: *const u16,
        directory: *const u16,
        show_command: i32,
    ) -> *mut c_void;
}

#[link(name = "gdi32")]
extern "system" {
    fn GetStockObject(index: i32) -> *mut c_void;
    fn CreateBitmap(
        width: i32,
        height: i32,
        planes: u32,
        bits: u32,
        bits_data: *const c_void,
    ) -> *mut c_void;
    fn DeleteObject(object: *mut c_void) -> i32;
}

#[repr(C)]
struct IconInfo {
    f_icon: i32,
    x_hotspot: u32,
    y_hotspot: u32,
    h_bm_mask: *mut c_void,
    h_bm_color: *mut c_void,
}

unsafe fn get_module_file_name_w_path() -> PathBuf {
    let mut buffer = [0u16; 260];
    let length = GetModuleFileNameW(
        std::ptr::null_mut(),
        buffer.as_mut_ptr(),
        buffer.len() as u32,
    );
    String::from_utf16_lossy(&buffer[..length as usize]).into()
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleFileNameW(module: *mut c_void, filename: *mut u16, size: u32) -> u32;
    fn GetLastError() -> u32;
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

unsafe fn initialize_common_controls() -> Result<(), u32> {
    let init = common_controls_initialization_contract();
    if InitCommonControlsEx(&init) == 0 {
        Err(GetLastError())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn diagnostic_toolbar_contract_uses_explicit_controls_and_tsv_events() {
        assert_ne!(ID_DIAGNOSTIC_RANGE, ID_DIAGNOSTIC_COPY);
        assert_ne!(ID_DIAGNOSTIC_COPY, ID_DIAGNOSTIC_EXPORT);
        assert_ne!(ID_DIAGNOSTIC_DISPLAY_LIMIT, ID_DIAGNOSTIC_RANGE);
        assert_ne!(ID_DIAGNOSTIC_SHOW_ALL, ID_DIAGNOSTIC_RANGE);
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
    fn diagnostic_summary_is_a_non_scrolling_read_only_static_contract() {
        let style = diagnostic_summary_style();
        assert_ne!(style & WS_CHILD, 0);
        assert_ne!(style & WS_VISIBLE, 0);
        assert_ne!(style & SS_NOPREFIX, 0);
        assert_eq!(style & (0x0020_0000 | 0x0040 | 0x0004), 0);
        assert!(DIAGNOSTIC_SUMMARY_HEIGHT >= 10 * 18);
    }

    #[test]
    fn diagnostic_toolbar_layout_is_one_row_and_derived_from_flow() {
        let layout = diagnostic_layout(980, 600, 96);
        let toolbar = layout.toolbar;
        let controls = [
            toolbar.display_label,
            toolbar.display_input,
            toolbar.show_all,
            toolbar.transfer_label,
            toolbar.range_input,
            toolbar.selection_summary,
            toolbar.copy,
            toolbar.export,
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
        assert_eq!(toolbar.message.width, 204);
        assert_eq!(diagnostic_toolbar_min_width(96), 916);
        assert!(toolbar.message.width >= DIAGNOSTIC_TOOLBAR_MIN_MESSAGE_WIDTH);
    }

    #[test]
    fn diagnostic_minimum_size_and_dpi_scaling_keep_summary_and_grid_visible() {
        assert_eq!(diagnostic_min_client_height(96), 352);
        assert_eq!(diagnostic_toolbar_min_width(144), 1_374);
        assert_eq!(diagnostic_min_client_height(144), 528);
        let layout = diagnostic_layout(1_374, 528, 144);
        assert_eq!(
            layout.summary.height,
            scale_logical(DIAGNOSTIC_SUMMARY_HEIGHT, 144)
        );
        assert_eq!(
            layout.list.height,
            scale_logical(DIAGNOSTIC_GRID_MIN_HEIGHT, 144)
        );
        assert!(layout.summary.bottom() <= layout.toolbar.display_input.top);
        assert!(layout.toolbar.display_input.bottom() <= layout.list.top);
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
    fn common_controls_initialization_uses_list_view_and_tooltip_classes() {
        let init = common_controls_initialization_contract();
        assert_eq!(init.size as usize, size_of::<InitCommonControlsEx>());
        assert_eq!(init.classes, ICC_LISTVIEW_CLASSES | ICC_BAR_CLASSES);
        assert_ne!(init.classes & ICC_LISTVIEW_CLASSES, 0);
        assert_ne!(init.classes & ICC_BAR_CLASSES, 0);
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
    fn native_result_mapping_preserves_success_and_raw_failure_codes() {
        assert_eq!(native_bool_result(1, 0), NativeResult::Succeeded);
        assert_eq!(
            native_bool_result(0, 5),
            NativeResult::Failed { raw_error: 5 }
        );
        assert_eq!(native_handle_result(false, 0), NativeResult::Succeeded);
        assert_eq!(
            native_handle_result(true, 6),
            NativeResult::Failed { raw_error: 6 }
        );
    }

    #[test]
    fn class_registration_accepts_existing_class_and_rejects_other_failures() {
        assert_eq!(class_registration_result(1, 0), NativeResult::Succeeded);
        assert_eq!(
            class_registration_result(0, ERROR_CLASS_ALREADY_EXISTS),
            NativeResult::Succeeded
        );
        assert_eq!(
            class_registration_result(0, 5),
            NativeResult::Failed { raw_error: 5 }
        );
    }

    #[test]
    fn signed_screen_coordinates_keep_their_low_32_bits() {
        assert_eq!(track_position_lparam(-1, -2) as u32, 0xfffe_ffff);
        assert_eq!(track_position_lparam(-1920, 1080) as u32, 0x0438_f880);
    }

    #[test]
    fn returned_popup_command_is_dispatched_once_as_an_optional_id() {
        assert_eq!(returned_menu_command(0), None);
        assert_eq!(returned_menu_command(-1), None);
        assert_eq!(returned_menu_command(ID_START as i32), Some(ID_START));
        assert_eq!(
            returned_menu_command(ID_AUTOMATIC_OFF as i32),
            Some(ID_AUTOMATIC_OFF)
        );
    }

    #[test]
    fn built_in_quit_warning_uses_concise_active_and_uncertain_messages() {
        assert_eq!(
            quit_warning_text(QuitSafetyReason::OwnedActive),
            "True™ Tick is currently controlling timer resolution. Stop timing and quit?"
        );
        assert_eq!(
            quit_warning_text(QuitSafetyReason::OwnershipUncertain),
            "True™ Tick could not verify that timing is fully released. Keep the app open and retry cleanup?"
        );
    }

    #[test]
    fn message_box_maps_only_yes_to_stop_and_quit_and_close_to_cancel() {
        assert_eq!(message_box_decision(IDYES), QuitDialogDecision::StopAndQuit);
        for result in [7, 0, 1, -1, 9999] {
            assert_eq!(message_box_decision(result), QuitDialogDecision::Cancel);
        }
        let failed = quit_warning_result(Some(0));
        assert_eq!(failed.decision, QuitDialogDecision::Cancel);
        assert!(!failed.dialog_shown);
        let accepted = quit_warning_result(Some(IDYES));
        assert_eq!(accepted.decision, QuitDialogDecision::StopAndQuit);
        assert!(accepted.dialog_shown);
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
        assert_eq!(diagnostic_toolbar_min_width(96), 916);
        assert_eq!(diagnostic_min_client_height(96), 352);
        assert_eq!(DIAGNOSTIC_TOOLBAR_HEIGHT, 36);
    }

    #[test]
    fn quit_decision_allows_only_settled_released_state_to_exit() {
        assert_eq!(
            quit_decision_for(TrayStatus::Stopped, OwnershipState::Released),
            QuitDecision::ExitNormally
        );
        assert_eq!(
            quit_decision_for_handoff(TrayStatus::Stopping, OwnershipState::Released, true),
            QuitDecision::ExitNormally
        );
        assert_eq!(
            quit_decision_for_handoff(TrayStatus::Stopping, OwnershipState::Owned, true),
            QuitDecision::RequireSafetyDialog {
                reason: QuitSafetyReason::TimingNotSettled
            }
        );
        assert_eq!(
            quit_decision_for(TrayStatus::Blocked, OwnershipState::Released),
            QuitDecision::ExitNormally
        );
        assert_eq!(
            quit_decision_for(TrayStatus::Running, OwnershipState::Owned),
            QuitDecision::RequireSafetyDialog {
                reason: QuitSafetyReason::OwnedActive
            }
        );
    }

    #[test]
    fn quit_decision_requires_safety_for_running_transition_and_unverified_states() {
        for status in [
            TrayStatus::Running,
            TrayStatus::Starting,
            TrayStatus::Stopping,
        ] {
            assert_eq!(
                quit_decision_for(status, OwnershipState::Released),
                QuitDecision::RequireSafetyDialog {
                    reason: QuitSafetyReason::TimingNotSettled
                }
            );
        }
        for status in [
            TrayStatus::Unverified,
            TrayStatus::Degraded,
            TrayStatus::Pending,
        ] {
            assert_eq!(
                quit_decision_for(status, OwnershipState::Released),
                QuitDecision::RequireSafetyDialog {
                    reason: QuitSafetyReason::TimingNotSettled
                }
            );
        }
    }

    #[test]
    fn quit_decision_requires_safety_when_ownership_is_uncertain() {
        for status in [TrayStatus::Stopped, TrayStatus::Error] {
            assert_eq!(
                quit_decision_for(status, OwnershipState::Uncertain),
                QuitDecision::RequireSafetyDialog {
                    reason: QuitSafetyReason::OwnershipUncertain
                }
            );
        }
    }

    #[test]
    fn diagnostic_initialization_phase_has_visible_loading_before_deferred_render() {
        assert!(diagnostic_loading_summary_is_nonblank());
        assert_eq!(diagnostic_window_style() & WS_VISIBLE, 0);
        assert_eq!(WM_DIAGNOSTIC_REFRESH, WM_APP + 2);
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
        assert!(diagnostic_refresh_is_coalesced(true, false));
        assert!(diagnostic_refresh_is_coalesced(false, true));
        assert!(!diagnostic_refresh_is_coalesced(false, false));
        assert_eq!(WM_SETREDRAW, 0x000B);
        assert_ne!(SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOSENDCHANGING, 0);
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
    fn create_params_are_taken_from_create_struct() {
        let marker = 7u8;
        let create = CreateStruct {
            create_params: (&marker as *const u8).cast_mut().cast(),
            instance: std::ptr::null_mut(),
            menu: std::ptr::null_mut(),
            parent: std::ptr::null_mut(),
            height: 0,
            width: 0,
            y: 0,
            x: 0,
            style: 0,
            name: std::ptr::null(),
            class_name: std::ptr::null(),
            extended_style: 0,
        };
        assert_eq!(app_create_params(&create), create.create_params);
        assert!(app_create_params(std::ptr::null()).is_null());
    }
}
