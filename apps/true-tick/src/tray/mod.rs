use crate::config;
use crate::pause::{
    acquisition_is_allowed, timer_interval_ms, CoordinatorTimerEvent, DurationAction,
    DurationCoordinator, DurationPreset, PresetsManager, ScheduleRequest,
};
use crate::ui::presets_window;
use std::ffi::c_void;
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tick_core::{DesiredIntent, DesiredIntentQueue};
use tick_diagnostics::{
    truncate_utf8, DiagnosticOutcome, DiagnosticPhase, DiagnosticRecord, DiagnosticSource,
    DiagnosticStore, EventCategory, NativeOutcome, OperationContext, RowSelection,
    DEFAULT_MAX_EVENTS,
};
use tick_observation_windows::{ObservationSource, WindowsObservation};
use tick_ownership::{OwnershipState, TimerController, TimingSnapshot, Verification};
use tick_platform_windows::{TimerObservation, WindowsTimerPlatform};
use tick_policy::{decide, PolicyInput, PowerState};
use tick_startup_windows::{
    startup_operation, StartupOperation, StartupRegistration, WindowsUserStartup,
};

pub mod icon;
pub mod menu;
pub(crate) use icon::*;

pub(crate) mod list_view_native {
    use super::{c_void, Point};

    pub const LVS_REPORT: u32 = 0x0001;
    pub const LVS_SINGLESEL: u32 = 0x0004;
    pub const LVS_SHOWSELALWAYS: u32 = 0x0008;
    pub const LVS_EX_GRIDLINES: usize = 0x0000_0001;
    pub const LVS_EX_FULLROWSELECT: usize = 0x0000_0020;
    pub const LVS_EX_DOUBLEBUFFER: usize = 0x0001_0000;
    pub const LVM_FIRST: u32 = 0x1000;
    pub const LVM_SETBKCOLOR: u32 = LVM_FIRST + 1;
    pub const LVM_DELETEALLITEMS: u32 = LVM_FIRST + 9;
    pub const LVM_GETITEMCOUNT: u32 = LVM_FIRST + 4;
    pub const LVM_GETNEXTITEM: u32 = LVM_FIRST + 12;
    pub const LVM_GETCOLUMNWIDTH: u32 = LVM_FIRST + 29;
    pub const LVM_GETTOPINDEX: u32 = LVM_FIRST + 39;
    pub const LVM_GETCOUNTPERPAGE: u32 = LVM_FIRST + 40;
    pub const LVM_ENSUREVISIBLE: u32 = LVM_FIRST + 19;
    pub const LVM_SETITEMSTATE: u32 = LVM_FIRST + 43;
    pub const LVM_INSERTITEMW: u32 = LVM_FIRST + 77;
    pub const LVM_SETITEMTEXTW: u32 = LVM_FIRST + 116;
    pub const LVM_INSERTCOLUMNW: u32 = LVM_FIRST + 97;
    pub const LVM_SETTEXTCOLOR: u32 = LVM_FIRST + 36;
    pub const LVM_SETTEXTBKCOLOR: u32 = LVM_FIRST + 38;
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
    pub const LVIR_BOUNDS: i32 = 0;
    pub const LVM_GETITEMRECT: u32 = LVM_FIRST + 14;
    pub const WM_NOTIFY: u32 = 0x004E;
    pub const LVN_ITEMCHANGED: i32 = -101;
    pub const LVN_KEYDOWN: i32 = -155;
    pub const VK_SHIFT: i32 = 0x10;
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

use crate::shutdown::{
    message_loop_exit, shutdown_disposition, MessageLoopExit, ShutdownDisposition, ShutdownGate,
};
use crate::tray_surface::{
    menu_command_dispatch_allowed, power_reconciliation, release_needs_handoff,
    scheduled_display_key, tooltip_at, tray_notification_opens_menu, HandoffProgress,
    HandoffTracker, PowerReconciliation, TimingValues, TrayStatus, GITHUB_URL,
    HANDOFF_POLL_INTERVAL_MS,
};

const WM_APP: u32 = 0x8000;
const WM_TRAY: u32 = WM_APP + 1;
pub(crate) const WM_CREATE: u32 = 0x0001;
pub(crate) const WM_COMMAND: u32 = 0x0111;
const WM_DESTROY: u32 = 0x0002;
const WM_POWERBROADCAST: u32 = 0x0218;
pub(crate) const WM_TIMER: u32 = 0x0113;
const WM_MENUSELECT: u32 = 0x011F;
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
pub(crate) const EN_CHANGE: usize = 0x0300;
pub(crate) const BN_CLICKED: usize = 0;
pub(crate) const WS_VSCROLL: u32 = 0x00200000;
pub(crate) const EM_LIMITTEXT: u32 = WM_USER + 1;
pub(crate) const WS_TABSTOP: u32 = 0x00010000;
pub(crate) const ES_AUTOHSCROLL: u32 = 0x0080;
pub(crate) const BS_PUSHBUTTON: u32 = 0x00000000;
pub(crate) const SS_LEFT: u32 = 0x00000000;

pub(crate) const WM_SIZE: u32 = 0x0005;
pub(crate) const WM_SYSCOLORCHANGE: u32 = 0x0015;
pub(crate) const WM_SETTINGCHANGE: u32 = 0x001A;
pub(crate) const WM_DPICHANGED: u32 = 0x02E0;
pub(crate) const WM_THEMECHANGED: u32 = 0x031A;
pub(crate) const WM_PAINT: u32 = 0x000F;
pub(crate) const WM_CLOSE: u32 = 0x0010;
pub(crate) const WM_ERASEBKGND: u32 = 0x0014;
pub(crate) const WM_NCDESTROY: u32 = 0x0082;
pub(crate) const WM_CTLCOLOREDIT: u32 = 0x0133;
pub(crate) const WM_CTLCOLORSTATIC: u32 = 0x0138;
pub(crate) const WM_SETFONT: u32 = 0x0030;
pub(crate) const WS_VISIBLE: u32 = 0x10000000;
pub(crate) const WS_BORDER: u32 = 0x00800000;
pub(crate) const WS_CLIPCHILDREN: u32 = 0x02000000;
pub(crate) const WS_CLIPSIBLINGS: u32 = 0x04000000;
pub(crate) const SW_SHOWNORMAL: i32 = 1;
pub(crate) const SW_RESTORE: i32 = 9;
const MAX_STARTUP_STATUS_BYTES: usize = 512;
pub(crate) const COLOR_WINDOW: i32 = 5;
pub(crate) const COLOR_WINDOWTEXT: i32 = 8;
pub(crate) const DEFAULT_GUI_FONT: i32 = 17;
pub(crate) const WS_CHILD: u32 = 0x40000000;
pub(crate) const WS_EX_CLIENTEDGE: u32 = 0x00000200;
pub(crate) const SWP_NOZORDER: u32 = 0x0004;
pub(crate) const SWP_NOACTIVATE: u32 = 0x0010;

const ICC_LISTVIEW_CLASSES: u32 = 0x0000_0001;
// The standard bar-class group includes the native tooltip control.
const ICC_BAR_CLASSES: u32 = 0x0000_0004;
const REQUIRED_COMMON_CONTROL_CLASSES: u32 = ICC_LISTVIEW_CLASSES | ICC_BAR_CLASSES;

pub(crate) const GWLP_WNDPROC: i32 = -4;
pub(crate) const GWLP_USERDATA: i32 = -21;

pub(crate) const MB_ICONWARNING: u32 = 0x0000_0030;
const MB_YESNO: u32 = 0x0000_0004;
const MB_DEFBUTTON2: u32 = 0x0000_0100;
const IDYES: i32 = 6;

const ERROR_CLASS_ALREADY_EXISTS: u32 = 1410;
const ERROR_ALREADY_EXISTS: u32 = 183;
const SINGLE_INSTANCE_MUTEX_NAME: &str = "Local\\TrueTickSingleInstance";
const TASKBAR_CREATED_MESSAGE_NAME: &str = "TaskbarCreated";
pub(crate) const WS_POPUP: u32 = 0x8000_0000;
pub(crate) const WS_EX_TOPMOST: u32 = 0x0000_0008;
pub(crate) const TTS_ALWAYSTIP: u32 = 0x0001;
pub(crate) const TTS_NOPREFIX: u32 = 0x0002;
pub(crate) const TTF_IDISHWND: u32 = 0x0001;
pub(crate) const TTF_TRACK: u32 = 0x0020;
pub(crate) const TTF_ABSOLUTE: u32 = 0x0080;
const WM_USER: u32 = 0x0400;
pub(crate) const TTM_TRACKACTIVATE: u32 = WM_USER + 17;
pub(crate) const TTM_TRACKPOSITION: u32 = WM_USER + 18;
pub(crate) const TTM_ADDTOOLW: u32 = WM_USER + 50;
pub(crate) const TTM_UPDATETIPTEXTW: u32 = WM_USER + 57;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativeResult {
    Succeeded,
    Failed { raw_error: u32 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SingleInstanceDecision {
    Proceed { handle: *mut c_void },
    ExistingInstance,
    Failure { raw_error: u32 },
}

fn single_instance_decision(handle: *mut c_void, raw_error: u32) -> SingleInstanceDecision {
    if handle.is_null() {
        SingleInstanceDecision::Failure { raw_error }
    } else if raw_error == ERROR_ALREADY_EXISTS {
        SingleInstanceDecision::ExistingInstance
    } else {
        SingleInstanceDecision::Proceed { handle }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TaskbarCreatedDecision {
    RestoreTrayIcon,
    Ignore,
}

fn taskbar_created_decision(message: u32, registered_message: u32) -> TaskbarCreatedDecision {
    if registered_message != 0 && message == registered_message {
        TaskbarCreatedDecision::RestoreTrayIcon
    } else {
        TaskbarCreatedDecision::Ignore
    }
}

struct SingleInstanceGuard {
    handle: *mut c_void,
    diagnostics: Arc<DiagnosticStore>,
}

impl SingleInstanceGuard {
    fn new(handle: *mut c_void, diagnostics: Arc<DiagnosticStore>) -> Self {
        Self {
            handle,
            diagnostics,
        }
    }
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            let result = unsafe { CloseHandle(self.handle) };
            let raw_status = if result == 0 {
                unsafe { GetLastError() }
            } else {
                0
            };
            self.diagnostics.record(
                "native.CloseHandle",
                format!("result={} raw_status={}", result != 0, raw_status),
            );
            self.handle = std::ptr::null_mut();
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
pub(crate) struct Rect {
    pub(crate) left: i32,
    pub(crate) top: i32,
    pub(crate) right: i32,
    pub(crate) bottom: i32,
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
pub(crate) struct ToolInfo {
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
pub(crate) struct CreateStruct {
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

pub(crate) fn app_create_params(create: *const CreateStruct) -> *mut c_void {
    if create.is_null() {
        std::ptr::null_mut()
    } else {
        unsafe { (*create).create_params }
    }
}

#[repr(C)]
pub(crate) struct PaintStruct {
    pub(crate) hdc: *mut c_void,
    pub(crate) erase: i32,
    pub(crate) paint: Rect,
    pub(crate) restore: i32,
    pub(crate) inc_update: i32,
    pub(crate) reserved: [u8; 32],
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
pub(crate) struct PopupMenuHandles {
    pub(crate) root: *mut c_void,
    pub(crate) schedule: Option<*mut c_void>,
    pub(crate) status: Option<*mut c_void>,
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

pub(crate) struct App {
    pub(crate) controller: TimerController<WindowsTimerPlatform>,
    pub(crate) observation: WindowsObservation,
    pub(crate) config: config::Config,
    tray_status: TrayStatus,
    pub(crate) config_path: PathBuf,
    executable: PathBuf,
    startup_status: String,
    taskbar_created_message: u32,
    tray_icon: Option<NotifyIconData>,
    timing_snapshot: TimingSnapshot,
    timing_snapshot_valid: bool,
    invalid_interval: bool,
    external_timing: bool,
    desired_intent: DesiredIntentQueue,
    pub(crate) diagnostics: Arc<DiagnosticStore>,
    pub(crate) diagnostic_window: Option<*mut c_void>,
    pub(crate) diagnostic_state_font: Option<*mut c_void>,
    pub(crate) diagnostic_hud_state: Option<*mut c_void>,
    pub(crate) diagnostic_summary: Option<*mut c_void>,
    pub(crate) diagnostic_hud_separator: Option<*mut c_void>,
    pub(crate) diagnostic_toolbar_separator: Option<*mut c_void>,
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
    pub(crate) diagnostic_message_text: String,
    pub(crate) diagnostic_refresh_pending: bool,
    pub(crate) diagnostic_refreshing: bool,
    pub(crate) diagnostic_refresh_direct_recorded: bool,
    pub(crate) diagnostic_refresh_follow_up_scheduled: bool,
    pub(crate) diagnostic_layout_stable: bool,
    pub(crate) diagnostic_snapshot_key: Option<(usize, u64)>,
    pub(crate) diagnostic_snapshot_generation: u64,
    pub(crate) diagnostic_auto_fit_generation: Option<u64>,
    pub(crate) menu_active: bool,
    pub(crate) popup_menus: Option<PopupMenuHandles>,
    pub(crate) popup_refresh_timer_active: bool,
    schedule_display_timer_active: bool,
    pub(crate) handoff: Option<HandoffTracker>,
    pub(crate) menu_help: Option<*mut c_void>,
    pub(crate) menu_help_text: Vec<u16>,
    pub(crate) pause: DurationCoordinator,
    pub(crate) presets_manager: PresetsManager,
    pub(crate) presets_window: Option<*mut c_void>,
    pub(crate) presets_listbox: Option<*mut c_void>,
    pub(crate) presets_input: Option<*mut c_void>,
    duration_timer_id: Option<usize>,
    duration_timer_generation: Option<u64>,
    scheduled_operation: Option<OperationContext>,
    running_since: Option<std::time::Instant>,
    shutdown_gate: ShutdownGate,
    pub(crate) operation: Option<OperationContext>,
    operation_source: DiagnosticSource,
    handoff_operation: Option<OperationContext>,
    last_publication: Option<PublicationKey>,
    pub(crate) log_directory: PathBuf,
    pub(crate) last_persisted_event_sequence: u64,
}

pub fn run() {
    unsafe {
        let diagnostics = Arc::new(DiagnosticStore::new(DEFAULT_MAX_EVENTS));
        diagnostics.record("lifecycle.start", "application_start");
        let mutex_name = wide(SINGLE_INSTANCE_MUTEX_NAME);
        let instance_handle = CreateMutexW(std::ptr::null_mut(), 0, mutex_name.as_ptr());
        let instance_last_error = GetLastError();
        let _instance_guard = match single_instance_decision(instance_handle, instance_last_error) {
            SingleInstanceDecision::Proceed { handle } => {
                diagnostics.record("lifecycle.single_instance", "result=acquired");
                Some(SingleInstanceGuard::new(handle, diagnostics.clone()))
            }
            SingleInstanceDecision::ExistingInstance => {
                diagnostics.record(
                    "lifecycle.single_instance",
                    format!(
                        "result=existing_instance raw_status={}",
                        instance_last_error
                    ),
                );
                let result = CloseHandle(instance_handle);
                let raw_status = if result == 0 { GetLastError() } else { 0 };
                diagnostics.record(
                    "native.CloseHandle",
                    format!("result={} raw_status={}", result != 0, raw_status),
                );
                std::process::exit(0);
            }
            SingleInstanceDecision::Failure { raw_error } => {
                diagnostics.record(
                    "lifecycle.single_instance",
                    format!("result=guard_failed raw_status={raw_error}"),
                );
                std::process::exit(1);
            }
        };
        let executable = get_module_file_name_w_path_with_diagnostics(Some(&diagnostics));
        let log_directory = crate::logging::resolve_log_directory(&executable);
        diagnostics.record("lifecycle.executable_observed", "path=redacted");
        let (is_slot_layout, portable_root_str, active_slot_selection, slot_executable_target) =
            match crate::portable::portable_root_from_slot_executable(&executable) {
                Ok(root) => {
                    let selection = crate::portable::select(&root);
                    diagnostics.record(
                        "portable.active_slot.selection",
                        format!("result={}", selection.description()),
                    );
                    let (selection_str, target_str) = match &selection {
                        crate::portable::Selection::Selected { slot, path } => (
                            format!("Selected({slot:?})"),
                            path.join("true-tick.exe").display().to_string(),
                        ),
                        crate::portable::Selection::RepairRequired(reason) => {
                            (format!("RepairRequired({reason})"), "none".to_string())
                        }
                    };
                    (true, root.display().to_string(), selection_str, target_str)
                }
                Err(_) => {
                    diagnostics.record(
                        "portable.active_slot.selection",
                        "result=not_portable_slot_layout",
                    );
                    let selection_str = if cfg!(debug_assertions)
                        && tick_startup_windows::is_development_executable(&executable)
                    {
                        "DevelopmentTarget".to_string()
                    } else {
                        "none".to_string()
                    };
                    let target_str = if cfg!(debug_assertions)
                        && tick_startup_windows::is_development_executable(&executable)
                    {
                        executable.display().to_string()
                    } else {
                        "none".to_string()
                    };
                    (false, "none".to_string(), selection_str, target_str)
                }
            };
        diagnostics.record(
            "lifecycle.portable_environment",
            format!(
                "is_slot_layout={is_slot_layout} portable_root={portable_root_str} active_slot_selection={active_slot_selection} slot_executable_target={slot_executable_target}"
            ),
        );
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
                    "power.observation.raw",
                    format!(
                        "power_state={:?} battery_saver={:?}",
                        snapshot.state, snapshot.battery_saver
                    ),
                );
                diagnostics.record(
                    "power.initial_observation",
                    format!("result=success state={:?}", snapshot.state),
                );
                None
            }
            Err(error) => {
                let snapshot = observation.power();
                diagnostics.record(
                    "power.observation.raw",
                    format!(
                        "power_state={:?} battery_saver={:?}",
                        snapshot.state, snapshot.battery_saver
                    ),
                );
                diagnostics.record(
                    "power.initial_observation",
                    format!("result=error reason={error}"),
                );
                Some(error)
            }
        };
        let startup_status = bounded_startup_status(startup_status);
        let taskbar_created_name = wide(TASKBAR_CREATED_MESSAGE_NAME);
        let taskbar_created_message = RegisterWindowMessageW(taskbar_created_name.as_ptr());
        let taskbar_created_error = if taskbar_created_message == 0 {
            GetLastError()
        } else {
            0
        };
        diagnostics.record(
            "native.RegisterWindowMessageW.TaskbarCreated",
            format!("message_id={taskbar_created_message} raw_status={taskbar_created_error}"),
        );
        let presets_manager = PresetsManager::from_seconds_list(&loaded.schedule_presets_seconds);
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
            taskbar_created_message,
            tray_icon: None,
            timing_snapshot: TimingSnapshot::default(),
            timing_snapshot_valid: false,
            invalid_interval: false,
            external_timing: false,
            desired_intent: DesiredIntentQueue::new(),
            diagnostics,
            diagnostic_window: None,
            diagnostic_state_font: None,
            diagnostic_hud_state: None,
            diagnostic_summary: None,
            diagnostic_hud_separator: None,
            diagnostic_toolbar_separator: None,
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
            diagnostic_message_text: String::new(),
            diagnostic_refresh_pending: false,
            diagnostic_refreshing: false,
            diagnostic_refresh_direct_recorded: false,
            diagnostic_refresh_follow_up_scheduled: false,
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
            presets_manager,
            presets_window: None,
            presets_listbox: None,
            presets_input: None,
            duration_timer_id: None,
            duration_timer_generation: None,
            scheduled_operation: None,
            running_since: None,
            shutdown_gate: ShutdownGate::new(),
            operation: None,
            operation_source: DiagnosticSource::Internal,
            handoff_operation: None,
            last_publication: None,
            log_directory,
            last_persisted_event_sequence: 0,
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
        let class_context = app.operation.map_or_else(
            || app.diagnostics.begin_operation(DiagnosticSource::Startup),
            |root| {
                app.diagnostics
                    .child_operation(root, DiagnosticSource::Startup)
            },
        );
        let class_outcome = if tray_class_atom != 0 {
            DiagnosticOutcome::Completed
        } else {
            DiagnosticOutcome::Failed
        };
        app.diagnostics.record_verification(
            class_context,
            DiagnosticSource::Startup,
            class_outcome,
            "window.class.verify",
            format!("atom={tray_class_atom} raw_status={tray_class_error}"),
        );
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
        let diagnostic_class_name = wide(crate::ui::diagnostic_window::DIAGNOSTIC_WINDOW_CLASS);
        let diagnostic_class = WndClass {
            style: 0,
            wnd_proc: Some(crate::ui::diagnostic_window::diagnostic_window_proc),
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
        let presets_class_name = wide(presets_window::PRESETS_WINDOW_CLASS);
        let presets_class = WndClass {
            style: 0,
            wnd_proc: Some(presets_window::presets_window_proc),
            cls_extra: 0,
            wnd_extra: 0,
            instance: wnd_class.instance,
            icon: wnd_class.icon,
            cursor: std::ptr::null_mut(),
            background: GetSysColorBrush(COLOR_WINDOW),
            menu_name: std::ptr::null(),
            class_name: presets_class_name.as_ptr(),
        };
        let presets_class_atom = RegisterClassW(&presets_class);
        let presets_class_error = if presets_class_atom == 0 {
            GetLastError()
        } else {
            0
        };
        if !matches!(
            class_registration_result(presets_class_atom, presets_class_error),
            NativeResult::Succeeded
        ) {
            app.record(
                "native.RegisterClassW.presets.error",
                format!("raw_status={presets_class_error}"),
            );
            abort_startup(
                app_ptr,
                "True™ Tick could not create its presets window class.",
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
        let host_hwnd_valid = !hwnd.is_null();
        let host_is_window = IsWindow(hwnd);
        let host_outcome = if !host_hwnd_valid {
            DiagnosticOutcome::Failed
        } else if host_is_window == 0 {
            DiagnosticOutcome::Unverified
        } else {
            DiagnosticOutcome::Completed
        };
        // The diagnostics crate has no Lifecycle source variant. Store level checks
        // land on Internal while neighboring records in this startup path carry the
        // active root source, so the verification reuses Startup for parity.
        let host_context = app.operation.map_or_else(
            || app.diagnostics.begin_operation(DiagnosticSource::Startup),
            |root| {
                app.diagnostics
                    .child_operation(root, DiagnosticSource::Startup)
            },
        );
        app.diagnostics.record_verification(
            host_context,
            DiagnosticSource::Startup,
            host_outcome,
            "window.host.verify",
            format!(
                "hwnd_valid={host_hwnd_valid} raw_status={hwnd_error} is_window={host_is_window}"
            ),
        );
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
        let icon_context = app.operation.map_or_else(
            || app.diagnostics.begin_operation(DiagnosticSource::Startup),
            |root| {
                app.diagnostics
                    .child_operation(root, DiagnosticSource::Startup)
            },
        );
        let icon_outcome = if add_result != 0 {
            DiagnosticOutcome::Completed
        } else {
            DiagnosticOutcome::Failed
        };
        app.diagnostics.record_verification(
            icon_context,
            DiagnosticSource::Startup,
            icon_outcome,
            "tray.icon.verify",
            format!("result={} raw_status={add_error}", add_result != 0),
        );
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
        crate::ui::diagnostic_window::destroy_diagnostic_window(app);
        if !hwnd.is_null() && IsWindow(hwnd) != 0 {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            DestroyWindow(hwnd);
        }
        app.record(
            "lifecycle.shutdown.resources",
            "result=destroyed_before_app_drop",
        );
        crate::logging::flush_diagnostic_events_to_disk(
            &app.diagnostics,
            &mut app.last_persisted_event_sequence,
            &app.log_directory,
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

pub(crate) unsafe fn remove_tray_icon(app: &mut App) {
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

pub(crate) unsafe fn restore_tray_icon(hwnd: *mut c_void, app: &mut App) {
    app.record("tray.taskbar_created", "action=restore");
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
        return;
    }
    app.tray_icon = Some(icon);
    app.last_publication = None;
    publish_tray_icon(app);
}

pub(crate) fn publish_tray_icon(app: &mut App) {
    app.publish();
}

pub(crate) unsafe fn destroy_created_diagnostic_controls(controls: [*mut c_void; 16]) {
    for control in controls {
        if !control.is_null() {
            let _ = DestroyWindow(control);
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

fn schedule_duration_action(app: &mut App, action: DurationAction, duration: DurationPreset) {
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
            "action={} duration_seconds={} generation={} deadline_monotonic_ms={} remaining_ms={}",
            action.label(),
            duration.seconds(),
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
            "outcome=scheduled action={} duration_seconds={} generation={} remaining_ms={}",
            action.label(),
            duration.seconds(),
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

fn schedule_preset_action(app: &mut App, action: DurationAction, preset_index: usize) {
    let Some(preset) = app.presets_manager.presets().get(preset_index).copied() else {
        app.record(
            "duration.schedule.preset",
            format!(
                "result=missing action={} index={preset_index}",
                action.label()
            ),
        );
        return;
    };
    schedule_duration_action(app, action, preset);
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
    if matches!(action.action, DurationAction::Start | DurationAction::Stop) {
        let timing_verified = match action.action {
            DurationAction::Start => {
                app.controller.ownership() == OwnershipState::Owned
                    && matches!(
                        app.controller.verification(),
                        Verification::Verified | Verification::FinerThanRequested
                    )
            }
            DurationAction::Stop => {
                app.controller.ownership() == OwnershipState::Released && app.handoff.is_none()
            }
            DurationAction::Pause => false,
        };
        let action_outcome = if timing_verified {
            DiagnosticOutcome::Completed
        } else {
            DiagnosticOutcome::Unverified
        };
        let action_context = app.operation.map_or_else(
            || app.diagnostics.begin_operation(DiagnosticSource::Timer),
            |root| {
                app.diagnostics
                    .child_operation(root, DiagnosticSource::Timer)
            },
        );
        app.diagnostics.record_verification(
            action_context,
            DiagnosticSource::Timer,
            action_outcome,
            "schedule.action.verify",
            format!(
                "action={} generation={} timing_verified={timing_verified}",
                action.action.label(),
                action.generation
            ),
        );
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
    let power_snapshot = app.observation.power();
    app.record(
        "power.observation.raw",
        format!(
            "power_state={:?} battery_saver={:?}",
            power_snapshot.state, power_snapshot.battery_saver
        ),
    );
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

pub(crate) fn native_outcome(name: &str, details: &str) -> NativeOutcome {
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

pub(crate) fn diagnostic_phase(name: &str) -> DiagnosticPhase {
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

pub(crate) fn diagnostic_outcome(name: &str, details: &str) -> DiagnosticOutcome {
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

    pub(crate) fn running_duration(&self) -> Option<std::time::Duration> {
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
        self.diagnostics.record_with_context(
            DiagnosticRecord {
                context,
                phase: DiagnosticPhase::Begin,
                source,
                outcome: DiagnosticOutcome::InProgress,
                native: NativeOutcome::default(),
            },
            "operation.begin",
            format!(
                "source={source:?} phase=begin outcome=in_progress parent_operation_id={:?}",
                context.parent_operation_id
            ),
        );
        context
    }

    pub(crate) fn lifecycle_status(&self) -> TrayStatus {
        crate::tray_surface::scheduled_lifecycle_status(
            self.tray_status,
            self.handoff.is_some(),
            self.pause.current().map(|action| action.action),
        )
    }

    pub(crate) fn timing_values(&self) -> TimingValues {
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

    pub(crate) fn begin_operation(&mut self, source: DiagnosticSource) -> OperationContext {
        let context = self.diagnostics.begin_operation(source);
        self.operation = Some(context);
        self.operation_source = source;
        // Feed the controller a child of the root so native platform calls recorded
        // during this operation carry the root as their parent operation ID.
        let native_context = self.diagnostics.child_operation(context, source);
        self.controller.set_operation_context(Some((
            native_context.operation_id,
            native_context.parent_operation_id,
            native_context.correlation_id,
        )));
        self.diagnostics.record_with_context(
            DiagnosticRecord {
                context,
                phase: DiagnosticPhase::Begin,
                source,
                outcome: DiagnosticOutcome::InProgress,
                native: NativeOutcome::default(),
            },
            "operation.begin",
            format!("source={source:?} phase=begin outcome=in_progress"),
        );
        context
    }

    pub(crate) fn finish_operation(&mut self, outcome: DiagnosticOutcome) {
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
        crate::ui::diagnostic_window::request_diagnostic_refresh(self);
    }

    pub(crate) fn record(&mut self, name: &str, details: impl AsRef<str>) {
        let details = details.as_ref();
        let source = self
            .operation
            .map_or_else(|| diagnostic_source(name), |_| self.operation_source);
        let context = self.operation.map_or_else(
            || self.diagnostics.begin_operation(source),
            |root| self.diagnostics.child_operation(root, source),
        );
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
        crate::ui::diagnostic_window::request_diagnostic_refresh(self);
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
            msg if matches!(
                taskbar_created_decision(msg, app.taskbar_created_message),
                TaskbarCreatedDecision::RestoreTrayIcon
            ) =>
            {
                restore_tray_icon(hwnd, app);
            }
            WM_TRAY if tray_notification_opens_menu(l_param as usize, app.menu_active) => {
                show_menu(hwnd, app)
            }
            WM_COMMAND => {
                handle_menu_command(hwnd, app, w_param & 0xffff);
            }
            WM_MENUSELECT => {
                menu::update_menu_help(
                    app,
                    w_param & 0xffff,
                    (w_param >> 16) as u32,
                    l_param as *mut c_void,
                );
            }
            WM_TIMER if w_param == POPUP_REFRESH_TIMER_ID => {
                menu::refresh_popup_menu(app);
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
                let power_snapshot = app.observation.power();
                app.record(
                    "power.observation.raw",
                    format!(
                        "power_state={:?} battery_saver={:?}",
                        power_snapshot.state, power_snapshot.battery_saver
                    ),
                );
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
    menu::show_menu(hwnd, app);
}

unsafe fn refresh_popup_menu(app: &mut App) {
    menu::refresh_popup_menu(app);
}

unsafe fn handle_menu_command(hwnd: *mut c_void, app: &mut App, command: usize) -> bool {
    menu::handle_menu_command(hwnd, app, command)
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

pub(crate) fn scale_logical(value: i32, dpi: u32) -> i32 {
    value.saturating_mul(dpi as i32).saturating_add(95) / 96
}

pub(crate) const fn diagnostic_create_failure_result() -> isize {
    -1
}

pub(crate) unsafe fn set_diagnostic_control_font(control: *mut c_void) {
    if control.is_null() {
        return;
    }
    let font = GetStockObject(DEFAULT_GUI_FONT);
    if !font.is_null() {
        let _ = SendMessageW(control, WM_SETFONT, font as usize, 1);
    }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StartupRollback {
    /// Leaving the written value in place is the only non-destructive inverse after an enable.
    SkipDestructiveInverse,
    /// Re-registering restores the state that the persisted configuration still describes.
    RestoreRegistration,
}

/// A rollback must never delete a current-user Run value that this process did not create. The
/// inverse of a successful enable is therefore deliberately not a removal. The persisted
/// configuration still records the previous state, so the next launch reconciles the registry
/// through the ordinary startup decision path.
const fn startup_rollback(enabled: bool) -> StartupRollback {
    if enabled {
        StartupRollback::SkipDestructiveInverse
    } else {
        StartupRollback::RestoreRegistration
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
        let rollback = match startup_rollback(enabled) {
            StartupRollback::SkipDestructiveInverse => {
                app.record(
                    "startup.registration.rollback_skipped",
                    "reason=non_destructive_policy value=TrueTick",
                );
                Ok(())
            }
            StartupRollback::RestoreRegistration => match startup_target(&app.executable) {
                Ok(target) => register_startup_target(&target),
                Err(_error) => Err(tick_startup_windows::StartupError::InvalidExecutablePath),
            },
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
        app.startup_status = bounded_startup_status(if enabled {
            "startup config persistence failed, registry left enabled and repair is required"
        } else if rollback.is_ok() {
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

pub(crate) fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
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
#[derive(Clone, Copy, Default, Debug, Eq, PartialEq)]
pub(crate) struct Point {
    pub(crate) x: i32,
    pub(crate) y: i32,
}

#[link(name = "user32")]
extern "system" {
    fn RegisterWindowMessageW(string: *const u16) -> u32;
    fn RegisterClassW(class: *const WndClass) -> u16;
    pub(crate) fn CreateWindowExW(
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
    pub(crate) fn DefWindowProcW(hwnd: *mut c_void, message: u32, w: usize, l: isize) -> isize;
    pub(crate) fn CallWindowProcW(
        prev_wnd_proc: unsafe extern "system" fn(*mut c_void, u32, usize, isize) -> isize,
        hwnd: *mut c_void,
        message: u32,
        w: usize,
        l: isize,
    ) -> isize;
    pub(crate) fn GetWindowLongPtrW(hwnd: *mut c_void, index: i32) -> isize;
    pub(crate) fn SetWindowLongPtrW(hwnd: *mut c_void, index: i32, value: isize) -> isize;
    pub(crate) fn IsWindow(window: *mut c_void) -> i32;
    pub(crate) fn IsIconic(window: *mut c_void) -> i32;
    pub(crate) fn PostQuitMessage(code: i32);
    pub(crate) fn SetTimer(
        hwnd: *mut c_void,
        event_id: usize,
        interval_ms: u32,
        callback: *mut c_void,
    ) -> usize;
    pub(crate) fn KillTimer(hwnd: *mut c_void, event_id: usize) -> i32;
    pub(crate) fn MessageBoxW(
        hwnd: *mut c_void,
        text: *const u16,
        title: *const u16,
        flags: u32,
    ) -> i32;
    pub(crate) fn CreatePopupMenu() -> *mut c_void;
    pub(crate) fn GetMenuItemCount(menu: *mut c_void) -> i32;
    pub(crate) fn GetMenuStringW(
        menu: *mut c_void,
        item: usize,
        text: *mut u16,
        maximum: i32,
        flags: u32,
    ) -> i32;
    pub(crate) fn AppendMenuW(menu: *mut c_void, flags: u32, id: usize, text: *const u16) -> i32;
    pub(crate) fn ModifyMenuW(
        menu: *mut c_void,
        item: usize,
        flags: u32,
        new_item: usize,
        text: *const u16,
    ) -> i32;
    pub(crate) fn SetForegroundWindow(hwnd: *mut c_void) -> i32;
    pub(crate) fn TrackPopupMenu(
        menu: *mut c_void,
        flags: u32,
        x: i32,
        y: i32,
        reserved: i32,
        hwnd: *mut c_void,
        rect: *const c_void,
    ) -> i32;
    pub(crate) fn DestroyMenu(menu: *mut c_void) -> i32;
    pub(crate) fn DestroyWindow(window: *mut c_void) -> i32;
    pub(crate) fn ShowWindow(window: *mut c_void, command: i32) -> i32;
    pub(crate) fn UpdateWindow(window: *mut c_void) -> i32;
    pub(crate) fn BeginPaint(window: *mut c_void, paint: *mut PaintStruct) -> *mut c_void;
    pub(crate) fn EndPaint(window: *mut c_void, paint: *const PaintStruct) -> i32;
    pub(crate) fn GetSysColor(index: i32) -> u32;
    pub(crate) fn GetSysColorBrush(index: i32) -> *mut c_void;
    pub(crate) fn SetTextColor(hdc: *mut c_void, color: u32) -> u32;
    pub(crate) fn SetBkColor(hdc: *mut c_void, color: u32) -> u32;

    pub(crate) fn SetWindowTextW(window: *mut c_void, text: *const u16) -> i32;
    pub(crate) fn GetWindowTextLengthW(window: *mut c_void) -> i32;
    pub(crate) fn GetWindowTextW(window: *mut c_void, text: *mut u16, maximum: i32) -> i32;
    pub(crate) fn GetKeyState(key: i32) -> i16;
    pub(crate) fn GetClientRect(window: *mut c_void, rect: *mut Rect) -> i32;
    pub(crate) fn SendMessageW(hwnd: *mut c_void, message: u32, w: usize, l: isize) -> isize;
    pub(crate) fn SetWindowPos(
        window: *mut c_void,
        insert_after: *mut c_void,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        flags: u32,
    ) -> i32;
    pub(crate) fn InvalidateRect(window: *mut c_void, rect: *const Rect, erase: i32) -> i32;
    pub(crate) fn GetCursorPos(point: *mut Point) -> i32;
    pub(crate) fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    pub(crate) fn GetDpiForWindow(window: *mut c_void) -> u32;
    pub(crate) fn GetDpiForSystem() -> u32;
    pub(crate) fn GetSystemMetrics(index: i32) -> i32;
    pub(crate) fn FillRect(hdc: *mut c_void, rect: *const Rect, brush: *mut c_void) -> i32;
    pub(crate) fn DrawFocusRect(hdc: *mut c_void, rect: *const Rect) -> i32;
    pub(crate) fn GetDC(hwnd: *mut c_void) -> *mut c_void;
    pub(crate) fn ReleaseDC(hwnd: *mut c_void, hdc: *mut c_void) -> i32;
    pub(crate) fn SetCapture(hwnd: *mut c_void) -> *mut c_void;
    pub(crate) fn ReleaseCapture() -> i32;
    pub(crate) fn GetCapture() -> *mut c_void;
}

#[link(name = "comctl32")]
extern "system" {
    fn InitCommonControlsEx(init: *const InitCommonControlsEx) -> i32;
}

#[link(name = "shell32")]
extern "system" {
    pub(crate) fn ShellExecuteW(
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
    pub(crate) fn GetStockObject(index: i32) -> *mut c_void;
    pub(crate) fn DeleteObject(object: *mut c_void) -> i32;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ModulePathGrowthDecision {
    Accept(usize),
    Grow(usize),
    GiveUp,
}

const MODULE_PATH_INITIAL_CAPACITY: usize = 260;
const MODULE_PATH_MAX_ATTEMPTS: usize = 8;
const MODULE_PATH_MAX_CAPACITY: usize = 32_768;

fn module_path_growth_decision(
    length: u32,
    buffer_capacity: usize,
    attempts: usize,
) -> ModulePathGrowthDecision {
    if length == 0 {
        return ModulePathGrowthDecision::GiveUp;
    }
    let length_usize = length as usize;
    if length_usize < buffer_capacity {
        ModulePathGrowthDecision::Accept(length_usize)
    } else if attempts >= MODULE_PATH_MAX_ATTEMPTS || buffer_capacity >= MODULE_PATH_MAX_CAPACITY {
        ModulePathGrowthDecision::GiveUp
    } else {
        let next_capacity = buffer_capacity
            .saturating_mul(2)
            .min(MODULE_PATH_MAX_CAPACITY);
        if next_capacity > buffer_capacity {
            ModulePathGrowthDecision::Grow(next_capacity)
        } else {
            ModulePathGrowthDecision::GiveUp
        }
    }
}

unsafe fn get_module_file_name_w_path_with_diagnostics(
    diagnostics: Option<&DiagnosticStore>,
) -> PathBuf {
    let mut capacity = MODULE_PATH_INITIAL_CAPACITY;
    let mut attempts = 0usize;

    loop {
        let mut buffer = vec![0u16; capacity];
        let length = GetModuleFileNameW(
            std::ptr::null_mut(),
            buffer.as_mut_ptr(),
            buffer.len() as u32,
        );
        match module_path_growth_decision(length, capacity, attempts) {
            ModulePathGrowthDecision::Accept(valid_length) => {
                return String::from_utf16_lossy(&buffer[..valid_length]).into();
            }
            ModulePathGrowthDecision::Grow(next_capacity) => {
                capacity = next_capacity;
                attempts = attempts.saturating_add(1);
            }
            ModulePathGrowthDecision::GiveUp => {
                let raw_status = GetLastError();
                if let Some(store) = diagnostics {
                    store.record(
                        "native.GetModuleFileNameW.error",
                        format!("raw_status={raw_status}"),
                    );
                }
                return PathBuf::new();
            }
        }
    }
}

#[allow(dead_code)]
unsafe fn get_module_file_name_w_path() -> PathBuf {
    get_module_file_name_w_path_with_diagnostics(None)
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleFileNameW(module: *mut c_void, filename: *mut u16, size: u32) -> u32;
    pub(crate) fn GetLastError() -> u32;
    fn CreateMutexW(
        security_attributes: *mut c_void,
        initial_owner: i32,
        name: *const u16,
    ) -> *mut c_void;
    fn CloseHandle(handle: *mut c_void) -> i32;
}

#[link(name = "comdlg32")]
extern "system" {}

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
    fn startup_rollback_never_deletes_a_value_after_an_enable() {
        assert_eq!(
            startup_rollback(true),
            StartupRollback::SkipDestructiveInverse
        );
        assert_eq!(
            startup_rollback(false),
            StartupRollback::RestoreRegistration
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

    #[test]
    fn module_path_growth_decision_handles_adequate_insufficient_and_give_up() {
        assert_eq!(
            module_path_growth_decision(100, 260, 0),
            ModulePathGrowthDecision::Accept(100)
        );
        assert_eq!(
            module_path_growth_decision(259, 260, 0),
            ModulePathGrowthDecision::Accept(259)
        );
        assert_eq!(
            module_path_growth_decision(260, 260, 0),
            ModulePathGrowthDecision::Grow(520)
        );
        assert_eq!(
            module_path_growth_decision(300, 260, 0),
            ModulePathGrowthDecision::Grow(520)
        );
        assert_eq!(
            module_path_growth_decision(0, 260, 0),
            ModulePathGrowthDecision::GiveUp
        );
        assert_eq!(
            module_path_growth_decision(260, 260, MODULE_PATH_MAX_ATTEMPTS),
            ModulePathGrowthDecision::GiveUp
        );
        assert_eq!(
            module_path_growth_decision(32_768, 32_768, 0),
            ModulePathGrowthDecision::GiveUp
        );
    }

    #[test]
    fn single_instance_decision_maps_create_result_and_last_error() {
        assert_eq!(
            single_instance_decision(std::ptr::null_mut(), 5),
            SingleInstanceDecision::Failure { raw_error: 5 }
        );
        let existing_handle = std::ptr::dangling_mut::<c_void>();
        assert_eq!(
            single_instance_decision(existing_handle, 183),
            SingleInstanceDecision::ExistingInstance
        );
        assert_eq!(
            single_instance_decision(existing_handle, 0),
            SingleInstanceDecision::Proceed {
                handle: existing_handle
            }
        );
    }

    #[test]
    fn taskbar_created_decision_matches_registered_message_only_when_valid() {
        assert_eq!(
            taskbar_created_decision(0xC000, 0xC000),
            TaskbarCreatedDecision::RestoreTrayIcon
        );
        assert_eq!(
            taskbar_created_decision(0xC000, 0xC001),
            TaskbarCreatedDecision::Ignore
        );
        assert_eq!(
            taskbar_created_decision(0, 0),
            TaskbarCreatedDecision::Ignore
        );
        assert_eq!(
            taskbar_created_decision(0x0001, 0),
            TaskbarCreatedDecision::Ignore
        );
    }

    #[test]
    fn taskbar_created_registered_message_name_is_exact() {
        assert_eq!(TASKBAR_CREATED_MESSAGE_NAME, "TaskbarCreated");
        let wide_name = wide(TASKBAR_CREATED_MESSAGE_NAME);
        assert_eq!(wide_name.last(), Some(&0));
        assert_eq!(wide_name.len(), "TaskbarCreated".len() + 1);
    }

    #[test]
    fn register_window_message_taskbar_created_returns_non_zero_id() {
        let name = wide(TASKBAR_CREATED_MESSAGE_NAME);
        let id = unsafe { RegisterWindowMessageW(name.as_ptr()) };
        assert_ne!(id, 0);
        assert_eq!(
            taskbar_created_decision(id, id),
            TaskbarCreatedDecision::RestoreTrayIcon
        );
    }

    #[test]
    fn active_operation_lineage_populates_parent_operation_id() {
        let store = Arc::new(DiagnosticStore::new(16));
        let root = store.begin_operation(DiagnosticSource::TrayCommand);
        let child = store.child_operation(root, DiagnosticSource::Native);

        assert_ne!(root.operation_id, child.operation_id);
        assert_eq!(child.parent_operation_id, Some(root.operation_id));
        assert_eq!(child.correlation_id, root.correlation_id);

        store.record_with_context(
            DiagnosticRecord {
                context: child,
                phase: DiagnosticPhase::Observe,
                source: DiagnosticSource::Native,
                outcome: DiagnosticOutcome::Completed,
                native: NativeOutcome::default(),
            },
            "native.NtQueryTimerResolution.call",
            "status=success",
        );

        let events = store.snapshot();
        let last_event = events.last().expect("event recorded");
        assert_eq!(last_event.parent_operation_id, Some(root.operation_id));
        assert_eq!(last_event.operation_id, child.operation_id);
    }
}
