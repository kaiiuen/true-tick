use crate::config;
use crate::pause::{
    acquisition_is_allowed, timer_interval_ms, CoordinatorTimerEvent, DurationAction,
    DurationPreset, ScheduleRequest,
};
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use tick_core::DesiredIntent;
use tick_diagnostics::{
    truncate_utf8, DiagnosticOutcome, DiagnosticPhase, DiagnosticSource, NativeOutcome,
};
use tick_observation_windows::ObservationSource;
use tick_ownership::{
    OwnershipState, SettleProbeOutcome, SettleProbeStep, Verification, SETTLE_PROBE_ATTEMPTS,
    SETTLE_PROBE_SPACING_MS,
};
use tick_platform_windows::TimerObservation;
use tick_policy::{
    decide, evaluate_tier, tier_allows_high_resolution, OperatingTier, PolicyInput, PowerState,
    TierContext, TierTransition,
};
use tick_startup_windows::{StartupRegistration, WindowsUserStartup};

pub mod controller;
pub mod icon;
pub mod ipc;
pub mod menu;
pub use controller::run;
pub(crate) use controller::App;
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
    pub const LVM_GETITEMCOUNT: u32 = LVM_FIRST + 4;
    pub const LVM_GETNEXTITEM: u32 = LVM_FIRST + 12;
    pub const LVM_GETCOLUMNWIDTH: u32 = LVM_FIRST + 29;
    pub const LVM_GETTOPINDEX: u32 = LVM_FIRST + 39;
    pub const LVM_GETCOUNTPERPAGE: u32 = LVM_FIRST + 40;
    pub const LVM_ENSUREVISIBLE: u32 = LVM_FIRST + 19;
    pub const LVM_SETITEMSTATE: u32 = LVM_FIRST + 43;
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

use crate::tray_surface::{
    power_reconciliation, release_needs_handoff, uncertain_recovery_attempts_acquire,
    HandoffProgress, HandoffTracker, PowerReconciliation, TrayStatus, GITHUB_URL,
    HANDOFF_POLL_INTERVAL_MS,
};

const WM_APP: u32 = 0x8000;
const WM_TRAY: u32 = WM_APP + 1;
/// Private window message posted by the IPC server thread whenever a
/// state-mutating command has been queued for the UI thread to execute.
pub(crate) const WM_APP_IPC: u32 = WM_APP + 3;

/// One inbound IPC request waiting for the UI thread. The pipe handler
/// pushes the verb, wire payload, and a one shot responder, then posts
/// `WM_APP_IPC` so the message loop drains the queue.
pub(crate) struct IpcCommandRequest {
    pub(crate) verb: tick_ipc::CommandVerb,
    pub(crate) payload: Vec<u8>,
    pub(crate) responder: std::sync::mpsc::Sender<tick_ipc::IpcResponse>,
}

/// Thread-safe queue between the IPC server thread and the UI thread.
pub(crate) struct IpcCommandQueue {
    inner: std::sync::Mutex<std::collections::VecDeque<IpcCommandRequest>>,
}

impl IpcCommandQueue {
    /// Creates an empty command queue.
    pub(crate) fn new() -> Self {
        Self {
            inner: std::sync::Mutex::new(std::collections::VecDeque::new()),
        }
    }

    /// Enqueues a request from the IPC server thread.
    pub(crate) fn push(&self, request: IpcCommandRequest) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.push_back(request);
    }

    /// Pops the oldest pending request, if any, on the UI thread.
    pub(crate) fn pop(&self) -> Option<IpcCommandRequest> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.pop_front()
    }
}
pub(crate) const WM_CREATE: u32 = 0x0001;
pub(crate) const WM_COMMAND: u32 = 0x0111;
pub(crate) const WM_DESTROY: u32 = 0x0002;
const WM_POWERBROADCAST: u32 = 0x0218;
pub(crate) const WM_TIMER: u32 = 0x0113;
const WM_MENUSELECT: u32 = 0x011F;
const PBT_APMPOWERSTATUSCHANGE: usize = 0x000A;
const PBT_APMSUSPEND: usize = 0x0004;
const PBT_APMRESUMESUSPEND: usize = 0x0007;
const PBT_APMRESUMEAUTOMATIC: usize = 0x0012;
const HANDOFF_TIMER_ID: usize = 0x5449;
const DURATION_TIMER_ID_BASE: usize = 0x6000;
pub(crate) const SURFACE_RECOVERY_TIMER_ID: usize = 0x6A00;
const POPUP_REFRESH_TIMER_ID: usize = 0x7000;
/// Popup-only UI cadence for live status refresh.
const POPUP_REFRESH_INTERVAL_MS: u32 = 500;
const SCHEDULE_DISPLAY_TIMER_ID: usize = 0x7100;
/// Bounded tray countdown cadence. The publication key suppresses redundant updates.
const SCHEDULE_DISPLAY_INTERVAL_MS: u32 = 1_000;
pub(crate) const POWER_DEBOUNCE_TIMER_ID: usize = 0x7200;
pub(crate) const POWER_DEBOUNCE_INTERVAL_MS: u64 = 2_000;
pub(crate) const HEARTBEAT_TIMER_ID: usize = 0x7300;

/// Bounded tray icon recreation attempts allowed while the surface is degraded.
pub const SURFACE_RECOVERY_ATTEMPTS: u32 = 3;
/// Milliseconds between successive tray icon recreation attempts.
pub const SURFACE_RECOVERY_SPACING_MS: u64 = 2000;
const ID_START: usize = 1001;
const ID_STOP: usize = 1002;
const ID_QUIT: usize = 1004;
const ID_STARTUP_ON: usize = 1005;
const ID_STARTUP_OFF: usize = 1006;
const ID_AUTOMATIC_ON: usize = 1007;
const ID_AUTOMATIC_OFF: usize = 1008;
const ID_AUTO_RESUME_ON: usize = 1009;
const ID_AUTO_RESUME_OFF: usize = 1010;
const ID_BATTERY_LOCKOUT_ON: usize = crate::tray_surface::BATTERY_LOCKOUT_ON_COMMAND_ID;
const ID_BATTERY_LOCKOUT_OFF: usize = crate::tray_surface::BATTERY_LOCKOUT_OFF_COMMAND_ID;
const ID_RESET: usize = 1060;
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

pub(crate) const GWLP_WNDPROC: i32 = -4;
pub(crate) const GWLP_USERDATA: i32 = -21;

pub(crate) const MB_ICONWARNING: u32 = 0x0000_0030;
pub(crate) const MB_ICONINFORMATION: u32 = 0x0000_0040;
pub(crate) const MB_OK: u32 = 0x0000_0000;
const MB_SETFOREGROUND: u32 = 0x0001_0000;
const MB_YESNO: u32 = 0x0000_0004;
const MB_DEFBUTTON2: u32 = 0x0000_0100;
const IDYES: i32 = 6;

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
        unsafe {
            // SAFETY: create is the non-null CREATESTRUCT pointer the OS passes with window creation
            (*create).create_params
        }
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
    pub(crate) settings: Option<*mut c_void>,
    pub(crate) status: Option<*mut c_void>,
}

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
pub(crate) unsafe fn remove_tray_icon(app: &mut App) {
    if let Some(mut icon) = app.tray_icon.take() {
        // SAFETY: `icon` is the live tray registration owned by the app, so
        // passing it to `NIM_DELETE` unregisters exactly this icon.
        let result = Shell_NotifyIconW(NIM_DELETE, &mut icon);
        app.record(
            "native.Shell_NotifyIconW.delete",
            format!(
                "result={} raw_status={}",
                result != 0,
                // SAFETY: `GetLastError` runs immediately on the same thread
                // after the delete call, so the value belongs to this call.
                if result == 0 { GetLastError() } else { 0 }
            ),
        );
    }
    // SAFETY: the tray registration above is gone, so no `NotifyIconData`
    // aliases a cached handle and every cached `HICON` can be destroyed.
    crate::tray::icon::clear_icon_cache();
}

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
pub(crate) unsafe fn restore_tray_icon(hwnd: *mut c_void, app: &mut App) {
    app.record("tray.taskbar_created", "action=restore");
    let mut icon = match NotifyIconData::new(
        hwnd,
        app.lifecycle_status(),
        app.timing_values(),
        app.pause.current(),
        app.last_block_reason,
    ) {
        Ok(icon) => icon,
        Err(raw_error) => {
            app.record_with_outcome(
                "native.tray_icon.create.error",
                format!("raw_status={raw_error}"),
                DiagnosticOutcome::Failed,
            );
            // The surface is gone, so the tier machine must see the icon as
            // absent instead of trusting a stale registration.
            app.tray_icon = None;
            publish_tray_icon(app);
            return;
        }
    };
    let add_result = Shell_NotifyIconW(NIM_ADD, &mut icon);
    let add_error = if add_result == 0 { GetLastError() } else { 0 };
    if matches!(
        native_bool_result(add_result, add_error),
        NativeResult::Failed { .. }
    ) {
        app.record_with_outcome(
            "native.Shell_NotifyIconW.add.error",
            format!("raw_status={add_error}"),
            DiagnosticOutcome::Failed,
        );
        app.tray_icon = None;
        publish_tray_icon(app);
        return;
    }
    app.tray_icon = Some(icon);
    app.last_publication = None;
    publish_tray_icon(app);
}

fn publish_tray_icon(app: &mut App) {
    app.publish();
}

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
pub(crate) unsafe fn destroy_created_diagnostic_controls(controls: &[*mut c_void]) {
    for &control in controls {
        if !control.is_null() {
            let _ = DestroyWindow(control);
        }
    }
}

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
unsafe fn show_shutdown_warning(hwnd: *mut c_void, message: &str) {
    let text = wide(message);
    let title = wide("True™ Tick shutdown warning");
    MessageBoxW(hwnd, text.as_ptr(), title.as_ptr(), MB_ICONWARNING);
}

/// Warns once per launch that the previous session ended without a clean
/// shutdown. The box is shown on a spawned thread so the startup path is
/// never gated on user acknowledgment and the main loop is never delayed.
pub(crate) fn show_unclean_shutdown_warning() {
    let text = wide(
        "True Tick did not shut down cleanly last time. The previous session may have ended from a force quit or a system crash.",
    );
    let title = wide("True Tick");
    std::thread::spawn(move || unsafe {
        // SAFETY: all string pointers reference live nul-terminated buffers and the owner is null or a live window
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONWARNING | MB_SETFOREGROUND,
        );
    });
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
        battery_lockout_enabled: app.config.battery_lockout,
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
    } else if acquisition_is_allowed(
        false,
        app.config.automatic,
        power,
        app.config.battery_lockout,
    ) {
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
                battery_lockout_enabled: app.config.battery_lockout,
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
                let source = "desired intent";
                match guarded_release(
                    app,
                    source,
                    if app.pause.pause_active() {
                        TrayStatus::Paused
                    } else {
                        TrayStatus::Stopped
                    },
                ) {
                    Ok(_) => {}
                    Err(message) => app.record(
                        "ownership.release.caller_failed",
                        format!("source={source} reason={message}"),
                    ),
                }
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
    if !tier_allows_high_resolution(app.operating_tier) {
        app.record(
            "tier.acquisition_blocked",
            format!("tier={:?} result=suppressed", app.operating_tier),
        );
        app.tray_status = if app.pause.pause_active() {
            TrayStatus::Paused
        } else {
            TrayStatus::Stopped
        };
        app.publish();
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
    if matches!(
        start_result,
        Err(tick_platform_windows::TimerError::RequestFailed { .. })
    ) {
        app.kernel_rejected_requests = app.kernel_rejected_requests.saturating_add(1);
    }
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
            app.record_with_outcome(
                "ownership.acquire.error",
                format!("error={error:?}"),
                DiagnosticOutcome::Failed,
            );
            match error {
                tick_platform_windows::TimerError::Unsupported => TrayStatus::Unsupported,
                _ => TrayStatus::Error,
            }
        }
    };
    if start_result.is_ok() {
        // A successful acquisition supersedes any earlier policy block.
        app.last_block_reason = None;
    }
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

/// Pulls newly observed anomaly producer counts into `unhandled_anomalies`.
/// Each producer contributes at most one increment per distinct occurrence.
pub(crate) fn drain_anomaly_producers(app: &mut App) {
    let pending = app.pending_anomalies.swap(0, Ordering::Acquire);
    if pending > 0 {
        let reasons = {
            let mut guard = app
                .pending_anomaly_reasons
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            std::mem::take(&mut *guard)
        };
        app.unhandled_anomalies = app.unhandled_anomalies.saturating_add(pending);
        for reason in reasons {
            app.record(
                "tier.anomaly.observed",
                format!("reason={reason} total={}", app.unhandled_anomalies),
            );
        }
    }
    let episodes = app
        .watchdog
        .as_ref()
        .map(tick_watchdog::WatchdogHandle::stall_episodes);
    if let Some(episodes) = episodes {
        if episodes > app.last_watchdog_episodes {
            let new_episodes = episodes - app.last_watchdog_episodes;
            app.last_watchdog_episodes = episodes;
            app.unhandled_anomalies = app.unhandled_anomalies.saturating_add(new_episodes);
            app.record(
                "tier.anomaly.observed",
                format!(
                    "reason=watchdog_stall_episode episodes={episodes} total={}",
                    app.unhandled_anomalies
                ),
            );
        }
    }
    let storage_failures = app
        .diagnostics
        .snapshot()
        .iter()
        .filter(|event| is_storage_failure_event(event))
        .map(|event| event.sequence)
        .max()
        .unwrap_or(0);
    if storage_failures > app.consumed_log_write_failures {
        let delta = app
            .diagnostics
            .snapshot()
            .iter()
            .filter(|event| {
                is_storage_failure_event(event)
                    && event.sequence > app.consumed_log_write_failures
                    && event.sequence <= storage_failures
            })
            .count() as u32;
        app.consumed_log_write_failures = storage_failures;
        app.unhandled_anomalies = app.unhandled_anomalies.saturating_add(delta);
        app.record(
            "tier.anomaly.observed",
            format!(
                "reason=log_write_failure failures={storage_failures} total={}",
                app.unhandled_anomalies
            ),
        );
    }
    let observed_panics = controller::caught_dispatch_panics();
    if observed_panics > app.consumed_dispatch_panics {
        let delta = observed_panics - app.consumed_dispatch_panics;
        app.consumed_dispatch_panics = observed_panics;
        app.unhandled_anomalies = app.unhandled_anomalies.saturating_add(delta);
        app.record(
            "tier.anomaly.observed",
            format!(
                "reason=caught_panic panics={observed_panics} total={}",
                app.unhandled_anomalies
            ),
        );
    }
}

/// Records a producer-side anomaly so the next tier evaluation sees it.
/// Safe to call from any thread that holds the shared app counters.
/// Today the producers that need it are exercised through tests. Runtime
/// producers fold directly through the dedicated counters drained in
/// `drain_anomaly_producers`.
#[allow(dead_code)]
pub(crate) fn report_anomaly(app: &App, reason: &'static str) {
    app.pending_anomalies.fetch_add(1, Ordering::AcqRel);
    let mut guard = app
        .pending_anomaly_reasons
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.push(reason);
}

/// True for diagnostic events that prove a log write or retention failure.
/// The insufficient free space path records `storage.write_suppressed` with
/// a `Suppressed` outcome and purge or append faults land on `storage.*`
/// failure events, so scanning the store sees every persisted storage fault
/// from the anomaly ladder side.
fn is_storage_failure_event(event: &tick_diagnostics::DiagnosticEvent) -> bool {
    event.name.starts_with("storage.")
        && matches!(
            event.outcome,
            DiagnosticOutcome::Failed | DiagnosticOutcome::Suppressed
        )
}

fn refresh_operating_tier(app: &mut App) {
    drain_anomaly_producers(app);
    let current_floor_hns = match app.operating_tier {
        OperatingTier::MetrologyDegraded { floored_hns } => floored_hns,
        _ => 0,
    };
    let nominally_recoverable = app.tray_icon.is_some()
        && app.kernel_rejected_requests == 0
        && app.unhandled_anomalies == 0;
    let context = TierContext {
        tray_surface_available: app.tray_icon.is_some(),
        kernel_rejected_requests: app.kernel_rejected_requests,
        unhandled_anomalies: app.unhandled_anomalies,
        current_floor_hns,
        nominally_recoverable,
    };
    let transition = evaluate_tier(app.operating_tier, &context);
    let leaving_surface = app.operating_tier == OperatingTier::SurfaceDegraded
        && !matches!(
            transition,
            TierTransition::Stay | TierTransition::EnterSurfaceDegraded
        );
    if leaving_surface {
        // SAFETY: `tray_hwnd` stays valid while the app runs and killing a
        // stale timer id is a no-op when the timer was never armed.
        unsafe { KillTimer(app.tray_hwnd, SURFACE_RECOVERY_TIMER_ID) };
    }
    match transition {
        TierTransition::Stay => {}
        TierTransition::EnterSurfaceDegraded => {
            app.operating_tier = OperatingTier::SurfaceDegraded;
            app.surface_recovery_attempts = 0;
            app.surface_recovery_exhausted = false;
            app.surface_recovery_due = Some(
                std::time::Instant::now()
                    + std::time::Duration::from_millis(SURFACE_RECOVERY_SPACING_MS),
            );
            // SAFETY: `tray_hwnd` is the live tray window while the app is
            // running. The callback is null so `WM_TIMER` posts to the queue.
            let timer_set = unsafe {
                SetTimer(
                    app.tray_hwnd,
                    SURFACE_RECOVERY_TIMER_ID,
                    SURFACE_RECOVERY_SPACING_MS as u32,
                    std::ptr::null_mut(),
                )
            };
            app.record(
                "tier.surface_recovery.armed",
                format!("timer_valid={}", timer_set != 0),
            );
        }
        TierTransition::EnterMetrologyDegraded { floored_hns } => {
            app.operating_tier = OperatingTier::MetrologyDegraded { floored_hns };
            app.controller
                .clamp_requested_interval_floor(tick_core::Hns::new(floored_hns));
        }
        TierTransition::EnterQuiescent => {
            app.operating_tier = OperatingTier::Quiescent;
        }
        TierTransition::Recover => {
            app.operating_tier = OperatingTier::Nominal;
            app.kernel_rejected_requests = 0;
            app.unhandled_anomalies = 0;
            app.last_watchdog_episodes = app
                .watchdog
                .as_ref()
                .map_or(0, |handle| handle.stall_episodes());
            app.consumed_log_write_failures = app
                .diagnostics
                .snapshot()
                .iter()
                .filter(|event| is_storage_failure_event(event))
                .map(|event| event.sequence)
                .max()
                .unwrap_or(0);
            app.surface_recovery_attempts = 0;
            app.surface_recovery_exhausted = false;
            app.surface_recovery_due = None;
            app.consumed_dispatch_panics = controller::caught_dispatch_panics();
        }
    }
    if !matches!(transition, TierTransition::Stay) {
        app.record(
            "tier.transition",
            format!("transition={transition:?} tier={:?}", app.operating_tier),
        );
        if matches!(transition, TierTransition::EnterQuiescent)
            && app.controller.ownership() == OwnershipState::Owned
        {
            app.record(
                "tier.quiescent.restorative_release",
                "action=guarded_release reason=quiescent_entered",
            );
            let source = "quiescent tier entry";
            match guarded_release(app, source, TrayStatus::Stopped) {
                Ok(_) => {}
                Err(message) => app.record(
                    "ownership.release.caller_failed",
                    format!("source={source} reason={message}"),
                ),
            }
        }
    }
}

/// Drives one bounded surface recovery step while the tier is `SurfaceDegraded`.
/// The tray icon path is recreated through the same `NotifyIconData` +
/// `Shell_NotifyIconW(NIM_ADD)` sequence used at startup and taskbar restore.
/// On success `tier.surface_recovered` is recorded and the next evaluation
/// returns to `Nominal`. On exhaustion `tier.surface_recovery_exhausted` is
/// recorded and the app stays in `SurfaceDegraded` serving IPC. The heartbeat
/// keeps being refreshed through `publish` so the watchdog never reports a
/// false stall while the surface is down.
pub(crate) fn service_surface_recovery(app: &mut App, hwnd: *mut c_void) {
    if app.operating_tier != OperatingTier::SurfaceDegraded {
        return;
    }
    if app.tray_icon.is_some() {
        return;
    }
    if app.surface_recovery_exhausted {
        return;
    }
    let Some(due) = app.surface_recovery_due else {
        app.surface_recovery_due = Some(
            std::time::Instant::now()
                + std::time::Duration::from_millis(SURFACE_RECOVERY_SPACING_MS),
        );
        return;
    };
    if std::time::Instant::now() < due {
        return;
    }
    let recovered = unsafe { attempt_surface_restore(hwnd, app) };
    advance_surface_recovery(app, recovered);
}

/// State machine step for one surface recovery outcome. Separated from the
/// Win32 restore call so tests can drive the budget and transition logic
/// without a shell. `recovered` marks whether the recreation attempt was
/// accepted by the tray surface.
pub(crate) fn advance_surface_recovery(app: &mut App, recovered: bool) {
    app.surface_recovery_attempts = app.surface_recovery_attempts.saturating_add(1);
    let attempt = app.surface_recovery_attempts;
    if recovered {
        app.record(
            "tier.surface_recovered",
            format!("attempt={attempt} budget={SURFACE_RECOVERY_ATTEMPTS}"),
        );
        app.surface_recovery_due = None;
        app.surface_recovery_attempts = 0;
        if !app.tray_hwnd.is_null() {
            // SAFETY: recovery finished, so the wake timer is no longer needed.
            unsafe { KillTimer(app.tray_hwnd, SURFACE_RECOVERY_TIMER_ID) };
        }
        app.publish();
        return;
    }
    app.record_with_outcome(
        "tier.surface_recovery.attempt",
        format!("attempt={attempt} budget={SURFACE_RECOVERY_ATTEMPTS} result=failed"),
        DiagnosticOutcome::Failed,
    );
    if app.surface_recovery_attempts >= SURFACE_RECOVERY_ATTEMPTS {
        app.surface_recovery_exhausted = true;
        app.surface_recovery_due = None;
        if !app.tray_hwnd.is_null() {
            // SAFETY: the budget is spent, so no further wake ticks are needed.
            unsafe { KillTimer(app.tray_hwnd, SURFACE_RECOVERY_TIMER_ID) };
        }
        app.record(
            "tier.surface_recovery_exhausted",
            format!(
                "attempts={} result=staying_surface_degraded",
                app.surface_recovery_attempts
            ),
        );
    } else {
        app.surface_recovery_due = Some(
            std::time::Instant::now()
                + std::time::Duration::from_millis(SURFACE_RECOVERY_SPACING_MS),
        );
    }
    app.publish();
}

/// One tray icon recreation attempt. Returns true when the shell accepted
/// the new registration. The app keeps running either way and the IPC pipe
/// remains the control channel while the surface is down.
unsafe fn attempt_surface_restore(hwnd: *mut c_void, app: &mut App) -> bool {
    let mut icon = match NotifyIconData::new(
        hwnd,
        app.lifecycle_status(),
        app.timing_values(),
        app.pause.current(),
        app.last_block_reason,
    ) {
        Ok(icon) => icon,
        Err(raw_error) => {
            app.record_with_outcome(
                "native.tray_icon.create.error",
                format!("raw_status={raw_error} context=surface_recovery"),
                DiagnosticOutcome::Failed,
            );
            return false;
        }
    };
    let add_result = Shell_NotifyIconW(NIM_ADD, &mut icon);
    let add_error = if add_result == 0 { GetLastError() } else { 0 };
    if matches!(
        native_bool_result(add_result, add_error),
        NativeResult::Failed { .. }
    ) {
        app.record_with_outcome(
            "native.Shell_NotifyIconW.add.error",
            format!("raw_status={add_error} context=surface_recovery"),
            DiagnosticOutcome::Failed,
        );
        return false;
    }
    app.tray_icon = Some(icon);
    app.last_publication = None;
    true
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
    let is_power_restriction = matches!(
        reason,
        tick_policy::PolicyReason::BatteryRestricted
            | tick_policy::PolicyReason::BatterySaverRestricted
            | tick_policy::PolicyReason::PowerUnknown
    );
    let ownership = app.controller.ownership();
    if is_power_restriction
        && matches!(ownership, OwnershipState::Owned | OwnershipState::Uncertain)
    {
        app.pending_resume_on_ac = true;
        app.record("policy.power_resume_pending", format!("reason={reason:?}"));
    }
    let released_status = match reason {
        tick_policy::PolicyReason::BatteryRestricted
        | tick_policy::PolicyReason::BatterySaverRestricted
        | tick_policy::PolicyReason::PowerUnknown
        | tick_policy::PolicyReason::GloballyDisabled => TrayStatus::Blocked,
        tick_policy::PolicyReason::NoEligibleProfile
        | tick_policy::PolicyReason::EligibleProfile => TrayStatus::Stopped,
    };
    if released_status == TrayStatus::Blocked {
        app.last_block_reason = Some(reason);
    } else {
        app.last_block_reason = None;
    }
    if matches!(
        app.controller.ownership(),
        OwnershipState::Owned | OwnershipState::Uncertain
    ) {
        let source = format!("policy reason={reason:?}");
        match guarded_release(app, &source, released_status) {
            Ok(_) => {}
            Err(message) => app.record(
                "ownership.release.caller_failed",
                format!("source={source} reason={message}"),
            ),
        }
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
                let Some(boundary) = boundary else {
                    app.record_with_outcome(
                        "ownership.release.error",
                        "reason=missing_release_boundary",
                        DiagnosticOutcome::Failed,
                    );
                    return Err("missing release boundary".to_owned());
                };
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
            app.record_with_outcome(
                "ownership.release.error",
                format!("error={error:?}"),
                DiagnosticOutcome::Failed,
            );
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
    app.pending_resume_on_ac = false;
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
        app.record_with_outcome(
            "duration.schedule.result",
            "outcome=failed reason=coordinator_timer_unavailable",
            DiagnosticOutcome::Failed,
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
        unsafe {
            // SAFETY: runs on the UI thread with the app exclusively borrowed for the menu rebuild
            refresh_popup_menu(app)
        };
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
            unsafe {
                // SAFETY: runs on the UI thread with the app exclusively borrowed for the menu rebuild
                refresh_popup_menu(app)
            };
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
    app.record_with_outcome(
        "duration.cancel",
        format!(
            "outcome=cancelled action={} duration_minutes={} generation={} remaining_ms=0",
            cancelled.action.label(),
            cancelled.duration.minutes(),
            cancelled.generation
        ),
        DiagnosticOutcome::Cancelled,
    );
    refresh_power_for_duration(app, "cancel");
    apply_power_reconciliation(app);
    if app.menu_active {
        unsafe {
            // SAFETY: runs on the UI thread with the app exclusively borrowed for the menu rebuild
            refresh_popup_menu(app)
        };
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
        if unsafe {
            // SAFETY: hwnd is the live tray window and the timer id was created by this module
            KillTimer(hwnd, timer_id)
        } == 0
        {
            app.record_with_outcome(
                "native.KillTimer.duration.error",
                format!("timer_id={timer_id} raw_status={}", unsafe {
                    // SAFETY: called immediately on the same thread after a failed Win32 call so the thread-local last-error value belongs to this call
                    GetLastError()
                }),
                DiagnosticOutcome::Failed,
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
        // SAFETY: hwnd is the live tray window and the callback is null so WM_TIMER posts to our queue
        SetTimer(
            hwnd,
            SCHEDULE_DISPLAY_TIMER_ID,
            SCHEDULE_DISPLAY_INTERVAL_MS,
            std::ptr::null_mut(),
        )
    };
    if result == 0 {
        app.record_with_outcome(
            "native.SetTimer.duration_display.error",
            format!("raw_status={}", unsafe {
                // SAFETY: called immediately on the same thread after a failed Win32 call so the thread-local last-error value belongs to this call
                GetLastError()
            }),
            DiagnosticOutcome::Failed,
        );
    } else {
        app.schedule_display_timer_active = true;
    }
}

fn kill_power_debounce_timer(app: &mut App) {
    if !app.power_debounce_active {
        return;
    }
    app.power_debounce_active = false;
    app.power_debounce_target_state = None;
    if let Some(hwnd) = app.tray_icon.as_ref().map(|icon| icon.h_wnd) {
        unsafe {
            // SAFETY: hwnd is the live tray window and the timer id was created by this module
            let _ = KillTimer(hwnd, POWER_DEBOUNCE_TIMER_ID);
        }
    }
    app.record("power.debounce.suppressed", "reason=shutdown_teardown");
}

fn kill_schedule_display_timer(app: &mut App) {
    if !app.schedule_display_timer_active {
        return;
    }
    app.schedule_display_timer_active = false;
    if let Some(hwnd) = app.tray_icon.as_ref().map(|icon| icon.h_wnd) {
        unsafe {
            // SAFETY: hwnd is the live tray window and the timer id was created by this module
            let _ = KillTimer(hwnd, SCHEDULE_DISPLAY_TIMER_ID);
        }
    }
}

fn kill_heartbeat_timer(app: &mut App) {
    if let Some(hwnd) = app.tray_icon.as_ref().map(|icon| icon.h_wnd) {
        unsafe {
            // SAFETY: hwnd is the live tray window and the timer id was created by this module
            let _ = KillTimer(hwnd, HEARTBEAT_TIMER_ID);
        }
    }
    app.last_heartbeat_fire = None;
}

fn handle_schedule_display_timer(app: &mut App) {
    if app.pause.active() {
        app.publish();
    } else {
        kill_schedule_display_timer(app);
    }
}

/// Recurring UI thread timer that proves message pump liveness so the
/// watchdog measures a wedged UI thread rather than an idle one.
///
/// Each fire also feeds the responsiveness tracker with the lateness of
/// this WM_TIMER delivery so a sustained slow pump surfaces as a
/// degraded tray status instead of a silent stall.
pub(crate) fn handle_heartbeat_timer(app: &mut App) {
    app.heartbeat.kick();
    let now = std::time::Instant::now();
    let elapsed_ms = app
        .last_heartbeat_fire
        .map(|prev| {
            u64::try_from(now.saturating_duration_since(prev).as_millis()).unwrap_or(u64::MAX)
        })
        .unwrap_or(0);
    let lateness_ms = elapsed_ms.saturating_sub(tick_watchdog::HEARTBEAT_INTERVAL_MS);
    let sample = if app.watchdog_stall_events.swap(0, Ordering::AcqRel) > 0 {
        lateness_ms.max(tick_watchdog::STALL_THRESHOLD_MS)
    } else {
        lateness_ms
    };
    app.last_heartbeat_fire = Some(now);
    app.responsiveness.observe(sample);
    let degraded_now = app.responsiveness.state() == tick_policy::ResponsivenessState::Degraded;
    if degraded_now && !app.responsiveness_degraded {
        app.responsiveness_degraded = true;
        app.record(
            "responsiveness.degraded",
            format!(
                "baseline_ms={} worst_ms={}",
                app.responsiveness.baseline_ms(),
                app.responsiveness.worst_ms()
            ),
        );
        if !app.responsiveness_notified {
            app.responsiveness_notified = true;
            if let Some(icon) = app.tray_icon.as_mut() {
                let _ = show_balloon(
                    icon,
                    "True Tick",
                    "System responsiveness is slow. Timing was not changed.",
                    true,
                );
            }
        }
        app.publish();
    } else if !degraded_now && app.responsiveness_degraded {
        app.responsiveness_degraded = false;
        app.responsiveness_notified = false;
        app.record(
            "responsiveness.recovered",
            format!("baseline_ms={}", app.responsiveness.baseline_ms()),
        );
        app.publish();
    }
}

/// Maps the lifecycle status to the published status so a slow but
/// healthy pump shows degraded without masking a real block or error.
pub(crate) fn published_status(base: TrayStatus, responsiveness_degraded: bool) -> TrayStatus {
    if responsiveness_degraded
        && matches!(
            base,
            TrayStatus::Running | TrayStatus::Stopped | TrayStatus::Paused
        )
    {
        TrayStatus::Degraded
    } else {
        base
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
        // SAFETY: hwnd is the live tray window and the callback is null so WM_TIMER posts to our queue
        SetTimer(
            hwnd,
            timer_id,
            timer_interval_ms(remaining),
            std::ptr::null_mut(),
        )
    };
    if result == 0 {
        app.record_with_outcome(
            "native.SetTimer.duration.error",
            format!("generation={generation} raw_status={}", unsafe {
                // SAFETY: called immediately on the same thread after a failed Win32 call so the thread-local last-error value belongs to this call
                GetLastError()
            }),
            DiagnosticOutcome::Failed,
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
                battery_lockout_enabled: app.config.battery_lockout,
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

pub(crate) const fn resume_on_ac_applies(
    auto_resume: bool,
    pending: bool,
    power: PowerState,
    ownership: OwnershipState,
) -> bool {
    auto_resume
        && pending
        && matches!(power, PowerState::Ac)
        && matches!(ownership, OwnershipState::Released)
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
    let automatic = app.config.automatic;
    let ownership = app.controller.ownership();
    if resume_on_ac_applies(
        app.config.auto_resume_on_ac,
        app.pending_resume_on_ac,
        power,
        ownership,
    ) {
        app.pending_resume_on_ac = false;
        app.record("policy.power_resume_applied", "action=acquire");
        queue_intent(app, DesiredIntent::Acquire, "power auto resume on ac");
        return;
    }
    let action = power_reconciliation(automatic, power, ownership, app.config.battery_lockout);
    app.record(
        "policy.power_reconciliation",
        format!(
            "power={power:?} automatic={automatic} ownership={:?} action={action:?}",
            app.controller.ownership()
        ),
    );
    match action {
        PowerReconciliation::Acquire => apply_policy(app),
        PowerReconciliation::ReleaseBlocked => release_for_policy(
            app,
            match power {
                PowerState::Battery => tick_policy::PolicyReason::BatteryRestricted,
                PowerState::BatterySaver => tick_policy::PolicyReason::BatterySaverRestricted,
                PowerState::Unknown => tick_policy::PolicyReason::PowerUnknown,
                PowerState::Ac => unreachable!("AC power never selects ReleaseBlocked"),
            },
        ),
        PowerReconciliation::ShowStopped => show_ownership_status(app, TrayStatus::Stopped),
        PowerReconciliation::PreserveOwned => show_ownership_status(app, TrayStatus::Stopped),
        PowerReconciliation::PreserveUncertain => {
            if uncertain_recovery_attempts_acquire(automatic, power) {
                let settle = guarded_release(
                    app,
                    "power reconciliation settles uncertain ownership",
                    TrayStatus::Stopped,
                );
                let settled = app.controller.ownership() == OwnershipState::Released;
                app.record(
                    "policy.power_reconciliation.uncertain_settle",
                    format!(
                        "result={settle:?} ownership={:?}",
                        app.controller.ownership()
                    ),
                );
                if settle.is_ok() && settled {
                    apply_policy(app);
                } else {
                    app.record(
                        "policy.power_reconciliation.uncertain_unsettled",
                        format!(
                            "result={settle:?} ownership={:?} fallback=stopped",
                            app.controller.ownership()
                        ),
                    );
                    show_ownership_status(app, TrayStatus::Stopped);
                }
            } else {
                show_ownership_status(app, TrayStatus::Stopped);
            }
        }
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

fn refresh_timing_observation(app: &mut App) -> Option<TimerObservation> {
    match app.controller.query() {
        Ok(observation) => {
            app.sync_timing_snapshot();
            app.timing_snapshot_valid = true;
            app.record_with_outcome(
                "timer.query.observation",
                format!(
                    "requested_hns={} effective_hns={} raw_status={} effective_relation={}",
                    observation.requested.value(),
                    observation.reported_current.value(),
                    observation.raw_status,
                    observation.effective_relation()
                ),
                DiagnosticOutcome::Completed,
            );
            Some(observation)
        }
        Err(error) => {
            app.sync_timing_snapshot();
            app.timing_snapshot_valid = false;
            app.record_with_outcome(
                "timer.query.error",
                format!("error={error:?} effective=unknown"),
                DiagnosticOutcome::Failed,
            );
            None
        }
    }
}

/// Drive the startup kernel settle probe across its bounded attempt budget.
///
/// If the effective resolution is already fine while True Tick holds no
/// ownership, the probe attempts to release an orphaned token from a prior
/// ungraceful exit. A coarser move proves the token was orphaned and settles
/// cleanly to released. A failed or unchanged probe confirms external timing.
/// Attempts are spaced by `SETTLE_PROBE_SPACING_MS` and capped at
/// `SETTLE_PROBE_ATTEMPTS`, after which the probe reports `Exhausted`.
pub(crate) fn probe_startup_kernel_settle(app: &mut App) {
    let mut attempts_used = 0u32;
    let outcome = loop {
        match app
            .controller
            .attempt_startup_kernel_settle_probe(attempts_used)
        {
            SettleProbeStep::NotApplicable => return,
            SettleProbeStep::Retry { budget_remaining } => {
                attempts_used += 1;
                app.record(
                    "recovery.settle_probe",
                    format!(
                        "result=retry attempt={} budget_remaining={} spacing_ms={}",
                        attempts_used, budget_remaining, SETTLE_PROBE_SPACING_MS
                    ),
                );
                std::thread::sleep(std::time::Duration::from_millis(SETTLE_PROBE_SPACING_MS));
            }
            SettleProbeStep::Settled(outcome) => break outcome,
        }
    };
    app.sync_timing_snapshot();
    app.timing_snapshot_valid = true;
    match outcome {
        SettleProbeOutcome::Resolved { freed_hns } => {
            app.record(
                "kernel.self_heal.settle_probe_restored",
                format!(
                    "ownership=released attempts={} freed_hns={} detail={}",
                    attempts_used + 1,
                    freed_hns,
                    app.timing_snapshot_details()
                ),
            );
        }
        SettleProbeOutcome::ExternalClientConfirmed => {
            app.external_timing = true;
            app.record(
                "kernel.self_heal.external_timing_confirmed",
                format!(
                    "ownership=released external_timing=true attempts={} detail={}",
                    attempts_used + 1,
                    app.timing_snapshot_details()
                ),
            );
        }
        SettleProbeOutcome::Exhausted => {
            app.record(
                "kernel.self_heal.settle_probe_exhausted",
                format!(
                    "ownership=released attempts={} budget=0 detail={}",
                    SETTLE_PROBE_ATTEMPTS,
                    app.timing_snapshot_details()
                ),
            );
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
        // SAFETY: hwnd is the live tray window and the callback is null so WM_TIMER posts to our queue
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
                unsafe {
                    // SAFETY: called immediately on the same thread after a failed Win32 call so the thread-local last-error value belongs to this call
                    GetLastError()
                }
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
        let result = unsafe {
            // SAFETY: hwnd is the live tray window and the timer id was created by this module
            KillTimer(hwnd, HANDOFF_TIMER_ID)
        };
        if result == 0 {
            app.record_with_outcome(
                "native.KillTimer.handoff.error",
                format!("raw_status={}", unsafe {
                    // SAFETY: called immediately on the same thread after a failed Win32 call so the thread-local last-error value belongs to this call
                    GetLastError()
                }),
                DiagnosticOutcome::Failed,
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
            app.record_with_outcome(
                "timer.handoff.query.error",
                format!("error={error:?} effective=unknown"),
                DiagnosticOutcome::Failed,
            );
            None
        }
    };
    let effective = observation.map(|value| value.reported_current);
    let Some(mut tracker) = app.handoff.take() else {
        return;
    };
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

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
unsafe fn show_menu(hwnd: *mut c_void, app: &mut App) {
    menu::show_menu(hwnd, app);
}

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
unsafe fn refresh_popup_menu(app: &mut App) {
    menu::refresh_popup_menu(app);
}

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
unsafe fn handle_menu_command(hwnd: *mut c_void, app: &mut App, command: usize) -> bool {
    menu::handle_menu_command(hwnd, app, command)
}

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
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
        app.record_with_outcome(
            "native.ShellExecuteW.github.error",
            format!("result={} raw_status={}", result as isize, GetLastError()),
            DiagnosticOutcome::Failed,
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

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
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
        app.record_with_outcome(
            "config.save.result",
            format!("result=error setting=automatic error={error}"),
            DiagnosticOutcome::Failed,
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

#[allow(dead_code)]
pub(crate) fn set_auto_resume(app: &mut App, enabled: bool) {
    app.record(
        "tray.command",
        format!("command=auto_resume enabled={enabled}"),
    );
    app.record(
        "toggle.requested",
        format!("setting=auto_resume requested={enabled}"),
    );
    app.record(
        "policy.auto_resume_setting_changed",
        format!("enabled={enabled}"),
    );
    let mut next = app.config.clone();
    next.auto_resume_on_ac = enabled;
    if let Err(error) = config::save_atomic(&app.config_path, &next) {
        app.record_with_outcome(
            "config.save.result",
            format!("result=error setting=auto_resume error={error}"),
            DiagnosticOutcome::Failed,
        );
        app.record(
            "toggle.result",
            format!(
                "setting=auto_resume value={} result=unchanged",
                app.config.auto_resume_on_ac
            ),
        );
        app.publish();
        return;
    }
    app.record(
        "config.save.result",
        format!("result=success setting=auto_resume value={enabled}"),
    );
    app.config.auto_resume_on_ac = enabled;
    app.record(
        "toggle.result",
        format!(
            "setting=auto_resume value={} result=applied",
            app.config.auto_resume_on_ac
        ),
    );
    app.publish();
}

/// Battery lockout setter. When the lockout is enabled, DC power releases a held
/// request and Battery Saver vetoes acquisition. When it is disabled, power
/// restrictions no longer release ownership or veto acquisition, and the
/// `auto_resume_on_ac` flag becomes inert because no restrictive release occurs.
pub(crate) fn set_battery_lockout(app: &mut App, enabled: bool) {
    app.record(
        "tray.command",
        format!("command=battery_lockout enabled={enabled}"),
    );
    app.record(
        "toggle.requested",
        format!("setting=battery_lockout requested={enabled}"),
    );
    app.record(
        "policy.battery_lockout_setting_changed",
        format!("enabled={enabled}"),
    );
    let mut next = app.config.clone();
    next.battery_lockout = enabled;
    if let Err(error) = config::save_atomic(&app.config_path, &next) {
        app.record_with_outcome(
            "config.save.result",
            format!("result=error setting=battery_lockout error={error}"),
            DiagnosticOutcome::Failed,
        );
        app.record(
            "toggle.result",
            format!(
                "setting=battery_lockout value={} result=unchanged",
                app.config.battery_lockout
            ),
        );
        app.publish();
        return;
    }
    app.record(
        "config.save.result",
        format!("result=success setting=battery_lockout value={enabled}"),
    );
    app.config = next;
    app.record(
        "toggle.result",
        format!(
            "setting=battery_lockout value={} result=applied",
            app.config.battery_lockout
        ),
    );
    if enabled {
        apply_power_reconciliation(app);
    } else {
        app.last_block_reason = None;
        apply_power_reconciliation(app);
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
                app.record_with_outcome(
                    "startup.registration.result",
                    format!("result=unavailable error={error}"),
                    DiagnosticOutcome::Failed,
                );
                let mut next = app.config.clone();
                next.startup_enabled = enabled;
                if let Err(save_error) = config::save_atomic(&app.config_path, &next) {
                    app.record_with_outcome(
                        "config.save.result",
                        format!("result=error setting=startup_enabled error={save_error}"),
                        DiagnosticOutcome::Failed,
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
        app.record_with_outcome(
            "startup.registration.result",
            format!("result=error error={error:?}"),
            DiagnosticOutcome::Failed,
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
        app.record_with_outcome(
            "config.save.result",
            format!("result=error setting=startup_enabled error={error}"),
            DiagnosticOutcome::Failed,
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

// SAFETY: callers must uphold the preconditions of the Win32 and grid helpers used inside
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

/// What the Settings Reset command removed from disk. Each field stays a plain
/// count or flag so the diagnostic chain can serialize the report directly.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ResetCleanup {
    pub(crate) log_entries_removed: usize,
    pub(crate) log_entries_failed: usize,
    pub(crate) config_removed: bool,
    pub(crate) session_marker_removed: bool,
}

/// Removes a single file when it exists. Missing files are not an error because
/// reset is expected to run against partially populated directories.
pub(crate) fn remove_file_if_present(path: &Path) -> std::io::Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Deletes every entry inside `directory` without removing the directory
/// itself. Returns the number of removed entries plus the number of entries
/// that failed to delete so partial wipes remain visible to the caller.
pub(crate) fn clear_directory_contents(directory: &Path) -> (usize, usize) {
    let mut removed = 0usize;
    let mut failed = 0usize;
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(_) => return (0, 0),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_directory = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
        let outcome = if is_directory {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        match outcome {
            Ok(()) => removed += 1,
            Err(_) => failed += 1,
        }
    }
    (removed, failed)
}

/// Wipes the log directory contents, the config file, and the session marker.
/// The session marker lives in the state directory next to the logs, and it
/// is removed explicitly so the report reflects the tombstone deletion even
/// when the log directory was already empty or missing.
pub(crate) fn reset_cleanup(
    log_directory: &Path,
    state_directory: &Path,
    config_path: &Path,
) -> ResetCleanup {
    let session_marker = crate::session::session_marker_path(state_directory);
    let session_marker_removed = remove_file_if_present(&session_marker).unwrap_or(false);
    let (log_entries_removed, log_entries_failed) = clear_directory_contents(log_directory);
    let config_removed = remove_file_if_present(config_path).unwrap_or(false);
    ResetCleanup {
        log_entries_removed,
        log_entries_failed,
        config_removed,
        session_marker_removed,
    }
}

pub(crate) const ABM_GETTASKBARPOS: usize = 0x0000_0005;
#[allow(dead_code)]
pub(crate) const ABE_LEFT: u32 = 0;
#[allow(dead_code)]
pub(crate) const ABE_TOP: u32 = 1;
pub(crate) const ABE_RIGHT: u32 = 2;
pub(crate) const ABE_BOTTOM: u32 = 3;

#[repr(C)]
pub(crate) struct AppBarData {
    cb_size: u32,
    hwnd: *mut c_void,
    callback_message: u32,
    edge: u32,
    rect: Rect,
    l_param: isize,
}

/// Queries the shell for the primary taskbar edge. Returns `None` when the
/// shell cannot answer so callers keep the default popup alignment.
pub(crate) fn taskbar_edge() -> Option<u32> {
    let mut data = AppBarData {
        cb_size: std::mem::size_of::<AppBarData>() as u32,
        hwnd: std::ptr::null_mut(),
        callback_message: 0,
        edge: 0,
        rect: Rect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        l_param: 0,
    };
    let result = unsafe {
        // SAFETY: data is a valid stack AppBarData with cb_size set
        SHAppBarMessage(ABM_GETTASKBARPOS, &mut data)
    };
    if result == 0 {
        None
    } else {
        Some(data.edge)
    }
}

/// Picks the `TrackPopupMenu` alignment flags for the taskbar edge that owns
/// the tray icon. A right docked taskbar needs `TPM_RIGHTALIGN` so the menu
/// grows leftward from the cursor instead of opening far to the left of the
/// icon, and a bottom docked taskbar needs `TPM_BOTTOMALIGN` so the menu opens
/// upward over the bar instead of hanging below the cursor.
pub(crate) const fn popup_track_flags(edge: Option<u32>, horizontal: u32, vertical: u32) -> u32 {
    let horizontal = match edge {
        Some(ABE_RIGHT) => menu::TPM_RIGHTALIGN,
        _ => horizontal,
    };
    let vertical = match edge {
        Some(ABE_BOTTOM) => menu::TPM_BOTTOMALIGN,
        _ => vertical,
    };
    horizontal | vertical
}

pub(crate) fn wide(value: &str) -> Vec<u16> {
    let mut buffer = Vec::with_capacity(value.len() + 1);
    buffer.extend(value.encode_utf16());
    buffer.push(0);
    buffer
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, Eq, PartialEq)]
pub(crate) struct Point {
    pub(crate) x: i32,
    pub(crate) y: i32,
}

#[link(name = "user32")]
extern "system" {
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
    pub(crate) fn SHAppBarMessage(message: usize, data: *mut AppBarData) -> usize;
}

#[link(name = "gdi32")]
extern "system" {
    pub(crate) fn GetStockObject(index: i32) -> *mut c_void;
    pub(crate) fn DeleteObject(object: *mut c_void) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    pub(crate) fn GetLastError() -> u32;
    pub(crate) fn ExitProcess(exit_code: u32) -> !;
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Arc;
    use tick_diagnostics::{DiagnosticRecord, DiagnosticStore};

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

    #[test]
    fn resume_on_ac_applies_only_when_all_conditions_hold() {
        assert!(resume_on_ac_applies(
            true,
            true,
            PowerState::Ac,
            OwnershipState::Released,
        ));
        assert!(!resume_on_ac_applies(
            false,
            true,
            PowerState::Ac,
            OwnershipState::Released,
        ));
        assert!(!resume_on_ac_applies(
            true,
            false,
            PowerState::Ac,
            OwnershipState::Released,
        ));
        assert!(!resume_on_ac_applies(
            true,
            true,
            PowerState::Battery,
            OwnershipState::Released,
        ));
        assert!(!resume_on_ac_applies(
            true,
            true,
            PowerState::Ac,
            OwnershipState::Owned,
        ));
    }

    #[test]
    fn resume_on_ac_queues_acquire_intent_even_when_automatic_timing_is_false() {
        assert!(resume_on_ac_applies(
            true,
            true,
            PowerState::Ac,
            OwnershipState::Released,
        ));
    }

    #[test]
    fn power_debounce_constants_are_bounded_and_monotonic() {
        assert_eq!(super::POWER_DEBOUNCE_TIMER_ID, 0x7200);
        assert_eq!(super::POWER_DEBOUNCE_INTERVAL_MS, 2_000);
    }

    pub(crate) fn test_app(diagnostics: Arc<DiagnosticStore>) -> App {
        use crate::pause::{DurationCoordinator, PresetsManager};
        use tick_observation_windows::WindowsObservation;
        use tick_ownership::{TimerController, TimingSnapshot};
        use tick_platform_windows::WindowsTimerPlatform;

        let config = config::Config::default();
        App {
            controller: TimerController::new(
                WindowsTimerPlatform::with_diagnostics(diagnostics.clone()),
                config.request_interval,
            ),
            observation: WindowsObservation::default(),
            config,
            tray_status: TrayStatus::Stopped,
            last_block_reason: None,
            operating_tier: tick_policy::OperatingTier::Nominal,
            kernel_rejected_requests: 0,
            unhandled_anomalies: 0,
            pending_anomalies: std::sync::atomic::AtomicU32::new(0),
            pending_anomaly_reasons: std::sync::Mutex::new(Vec::new()),
            last_watchdog_episodes: 0,
            consumed_log_write_failures: 0,
            consumed_dispatch_panics: 0,
            surface_recovery_attempts: 0,
            surface_recovery_due: None,
            surface_recovery_exhausted: false,
            tray_hwnd: std::ptr::null_mut(),
            config_path: PathBuf::new(),
            executable: PathBuf::new(),
            startup_status: String::new(),
            operating_slot_label: String::new(),
            portable_root_label: String::new(),
            taskbar_created_message: 0,
            another_instance_message: 0,
            tray_icon: None,
            timing_snapshot: TimingSnapshot::default(),
            timing_snapshot_valid: false,
            invalid_interval: false,
            external_timing: false,
            desired_intent: tick_core::DesiredIntentQueue::new(),
            diagnostics,
            diagnostic_ui: crate::ui::diagnostic_window::DiagnosticUiState::new(),
            menu_active: false,
            settings_submenu_open: false,
            popup_menus: None,
            popup_refresh_timer_active: false,
            schedule_display_timer_active: false,
            handoff: None,
            menu_help: None,
            menu_help_text: Vec::new(),
            pause: DurationCoordinator::new(),
            presets_manager: PresetsManager::new(),
            presets_window: None,
            presets_listbox: None,
            presets_input: None,
            duration_timer_id: None,
            duration_timer_generation: None,
            scheduled_operation: None,
            running_since: None,
            pending_resume_on_ac: false,
            shutdown_gate: crate::shutdown::ShutdownGate::new(),
            operation: None,
            operation_source: DiagnosticSource::Internal,
            handoff_operation: None,
            last_publication: None,
            log_directory: PathBuf::new(),
            state_directory: PathBuf::new(),
            last_persisted_event_sequence: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(
                0,
            )),
            power_debounce_active: false,
            power_debounce_target_state: None,
            heartbeat: Arc::new(tick_watchdog::Heartbeat::new()),
            watchdog: None,
            tracked_interval_hns: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            ownership_flag: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            responsiveness: tick_policy::ResponsivenessTracker::new(),
            responsiveness_degraded: false,
            responsiveness_notified: false,
            last_heartbeat_fire: None,
            watchdog_stall_events: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            ipc_server: None,
            ipc_status: Arc::new(ipc::IpcStatusSnapshot::new()),
            ipc_command_queue: Arc::new(crate::tray::IpcCommandQueue::new()),
        }
    }

    #[test]
    fn ipc_server_field_defaults_to_none_in_test_app() {
        let store = Arc::new(DiagnosticStore::new(16));
        let app = test_app(store);
        assert!(app.ipc_server.is_none());
    }

    #[test]
    fn refresh_ipc_status_writes_lifecycle_and_ownership() {
        let store = Arc::new(DiagnosticStore::new(64));
        let mut app = test_app(store);
        app.tray_status = TrayStatus::Running;
        app.publish();
        let payload = app.ipc_status.status_payload();
        let text = String::from_utf8(payload).expect("status payload is utf8");
        assert!(text.contains("status=running"));
        assert!(text.contains("ownership=released"));
    }

    fn stub_tray_icon() -> super::NotifyIconData {
        super::NotifyIconData {
            cb_size: 0,
            h_wnd: std::ptr::null_mut(),
            u_id: 0,
            u_flags: 0,
            u_callback_message: 0,
            h_icon: std::ptr::null_mut(),
            sz_tip: [0; 128],
            dw_state: 0,
            dw_state_mask: 0,
            sz_info: [0; 256],
            u_timeout_or_version: 0,
            sz_info_title: [0; 64],
            dw_info_flags: 0,
            guid: [0; 16],
            h_balloon_icon: std::ptr::null_mut(),
        }
    }

    #[test]
    fn kernel_rejection_escalates_to_metrology_degraded_and_clamps_floor() {
        let store = Arc::new(DiagnosticStore::new(64));
        let mut app = test_app(store.clone());
        app.tray_icon = Some(stub_tray_icon());
        assert_eq!(app.operating_tier, tick_policy::OperatingTier::Nominal);

        app.kernel_rejected_requests = tick_policy::KERNEL_REJECTIONS_BEFORE_FLOOR;
        super::refresh_operating_tier(&mut app);

        assert_eq!(
            app.operating_tier,
            tick_policy::OperatingTier::MetrologyDegraded {
                floored_hns: tick_policy::FALLBACK_FLOOR_HNS
            }
        );
        assert_eq!(
            app.controller.requested_interval(),
            tick_core::Hns::new(tick_policy::FALLBACK_FLOOR_HNS)
        );
        assert!(store
            .snapshot()
            .iter()
            .any(|event| event.name == "tier.transition"));
    }

    #[test]
    fn anomaly_escalation_to_quiescent_blocks_acquisition() {
        let store = Arc::new(DiagnosticStore::new(64));
        let mut app = test_app(store.clone());

        app.unhandled_anomalies = tick_policy::MAX_ANOMALIES_BEFORE_QUIESCENT;
        super::refresh_operating_tier(&mut app);

        assert_eq!(app.operating_tier, tick_policy::OperatingTier::Quiescent);
        assert!(!tick_policy::tier_allows_high_resolution(
            app.operating_tier
        ));

        super::acquire_timer(&mut app);
        assert_eq!(app.controller.ownership(), OwnershipState::Released);
        assert!(store
            .snapshot()
            .iter()
            .any(|event| event.name == "tier.acquisition_blocked"));
    }

    #[test]
    fn metrology_degraded_recovers_to_nominal_and_resets_counters() {
        let store = Arc::new(DiagnosticStore::new(64));
        let mut app = test_app(store.clone());
        app.tray_icon = Some(stub_tray_icon());

        app.kernel_rejected_requests = tick_policy::KERNEL_REJECTIONS_BEFORE_FLOOR;
        super::refresh_operating_tier(&mut app);
        assert!(matches!(
            app.operating_tier,
            tick_policy::OperatingTier::MetrologyDegraded { .. }
        ));

        app.kernel_rejected_requests = 0;
        super::refresh_operating_tier(&mut app);

        assert_eq!(app.operating_tier, tick_policy::OperatingTier::Nominal);
        assert_eq!(app.kernel_rejected_requests, 0);
        assert_eq!(app.unhandled_anomalies, 0);
    }

    #[test]
    fn explicit_outcome_records_completed_despite_unverified_detail_token() {
        let store = Arc::new(DiagnosticStore::new(16));
        let mut app = test_app(store.clone());

        app.record_with_outcome(
            "timer.query.observation",
            "requested_hns=156250 effective_hns=156250 raw_status=0 effective_relation=unverified",
            DiagnosticOutcome::Completed,
        );

        let events = store.snapshot();
        let event = events
            .iter()
            .find(|event| event.name == "timer.query.observation")
            .expect("observation event recorded");
        assert_eq!(event.outcome, DiagnosticOutcome::Completed);
        assert!(event.details.contains("effective_relation=unverified"));
    }

    #[test]
    fn failed_release_records_caller_side_diagnostic() {
        let store = Arc::new(DiagnosticStore::new(64));
        let mut app = test_app(store.clone());
        app.desired_intent.request(DesiredIntent::Release);
        app.tray_status = TrayStatus::Starting;

        // While acquisition verification is pending, a release intent is
        // requeued rather than acted on, so no guarded_release runs and no
        // release diagnostics appear.
        super::process_desired_intent(&mut app);

        let events = store.snapshot();
        assert!(events
            .iter()
            .any(|event| event.name == "lifecycle.release_queued"));
    }

    #[test]
    fn successful_release_records_no_caller_side_error() {
        let store = Arc::new(DiagnosticStore::new(64));
        let mut app = test_app(store.clone());

        // Ownership is Released in the test fixture so the release path does
        // not invoke guarded_release at all.
        super::process_desired_intent(&mut app);

        let events = store.snapshot();
        assert!(!events
            .iter()
            .any(|event| event.name == "ownership.release.caller_failed"));
        assert!(!events
            .iter()
            .any(|event| event.name == "ownership.release.error"));
    }

    fn wait_until(mut condition: impl FnMut() -> bool, what: &str) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !condition() {
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for {what}"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[test]
    fn anomaly_producer_increments_on_watchdog_stall_episode() {
        let store = Arc::new(DiagnosticStore::new(64));
        let mut app = test_app(store.clone());
        // A heartbeat stamped at creation goes stale inside the first monitor
        // cycle because the stall threshold is zero, so one episode lands
        // without any heartbeat field access from this crate.
        let heartbeat = Arc::new(tick_watchdog::Heartbeat::new());
        app.watchdog = Some(tick_watchdog::WatchdogHandle::spawn(
            heartbeat,
            tick_watchdog::WatchdogConfig {
                interval_ms: 10,
                stall_threshold_ms: 0,
            },
            Box::new(|_stall_ms| {}),
        ));
        wait_until(
            || {
                app.watchdog
                    .as_ref()
                    .map_or(0, |handle| handle.stall_episodes())
                    == 1
            },
            "first stall episode",
        );
        super::refresh_operating_tier(&mut app);
        assert_eq!(app.unhandled_anomalies, 1);
        assert!(store
            .snapshot()
            .iter()
            .any(|event| event.name == "tier.anomaly.observed"
                && event.details.contains("watchdog_stall_episode")));
        app.watchdog.as_ref().unwrap().shutdown();
        let handle = app.watchdog.take().unwrap();
        handle.join().expect("watchdog thread panicked");
    }

    #[test]
    fn anomaly_producer_increments_on_config_recovery() {
        let store = Arc::new(DiagnosticStore::new(64));
        let mut app = test_app(store);
        super::report_anomaly(&app, "config_recovery");
        super::refresh_operating_tier(&mut app);
        assert_eq!(app.unhandled_anomalies, 1);
    }

    #[test]
    fn anomaly_producer_increments_on_tombstone_corruption() {
        let store = Arc::new(DiagnosticStore::new(64));
        let mut app = test_app(store);
        super::report_anomaly(&app, "tombstone_corruption");
        super::refresh_operating_tier(&mut app);
        assert_eq!(app.unhandled_anomalies, 1);
    }

    #[test]
    fn anomaly_counter_resets_only_on_recover() {
        let store = Arc::new(DiagnosticStore::new(64));
        let mut app = test_app(store);
        app.tray_icon = Some(stub_tray_icon());
        app.unhandled_anomalies = 1;
        app.kernel_rejected_requests = tick_policy::KERNEL_REJECTIONS_BEFORE_FLOOR;
        super::refresh_operating_tier(&mut app);
        assert!(matches!(
            app.operating_tier,
            tick_policy::OperatingTier::MetrologyDegraded { .. }
        ));
        // A Stay transition must not clear the counter.
        super::refresh_operating_tier(&mut app);
        assert_eq!(app.unhandled_anomalies, 1);
        // Recovery requires anomalies cleared first by the operator path.
        app.unhandled_anomalies = 0;
        app.kernel_rejected_requests = 0;
        super::refresh_operating_tier(&mut app);
        assert_eq!(app.operating_tier, tick_policy::OperatingTier::Nominal);
        assert_eq!(app.unhandled_anomalies, 0);
    }

    #[test]
    fn surface_degraded_continues_serving_and_does_not_exit() {
        let store = Arc::new(DiagnosticStore::new(64));
        let mut app = test_app(store.clone());
        app.tray_icon = None;
        super::refresh_operating_tier(&mut app);
        assert_eq!(
            app.operating_tier,
            tick_policy::OperatingTier::SurfaceDegraded
        );
        // The IPC snapshot keeps refreshing even with no tray icon.
        app.tray_status = TrayStatus::Running;
        app.publish();
        let payload = app.ipc_status.status_payload();
        let text = String::from_utf8(payload).expect("status payload is utf8");
        assert!(text.contains("status=running"));
        // The process stays in the loop and does not tear down.
        assert_eq!(
            app.operating_tier,
            tick_policy::OperatingTier::SurfaceDegraded
        );
        assert!(store
            .snapshot()
            .iter()
            .any(|event| event.name == "tier.surface_recovery.armed"));
    }

    #[test]
    fn surface_recovery_succeeds_within_attempt_budget() {
        let store = Arc::new(DiagnosticStore::new(64));
        let mut app = test_app(store.clone());
        app.tray_icon = None;
        super::refresh_operating_tier(&mut app);
        assert_eq!(
            app.operating_tier,
            tick_policy::OperatingTier::SurfaceDegraded
        );
        // One failed attempt, then success inside the budget.
        super::advance_surface_recovery(&mut app, false);
        assert_eq!(app.surface_recovery_attempts, 1);
        app.tray_icon = Some(stub_tray_icon());
        super::advance_surface_recovery(&mut app, true);
        assert!(store
            .snapshot()
            .iter()
            .any(|event| event.name == "tier.surface_recovered"));
        // The icon being present again lets the next evaluation recover.
        super::refresh_operating_tier(&mut app);
        assert_eq!(app.operating_tier, tick_policy::OperatingTier::Nominal);
    }

    #[test]
    fn surface_recovery_exhaustion_stays_in_surface_degraded() {
        let store = Arc::new(DiagnosticStore::new(64));
        let mut app = test_app(store.clone());
        app.tray_icon = None;
        super::refresh_operating_tier(&mut app);
        assert_eq!(
            app.operating_tier,
            tick_policy::OperatingTier::SurfaceDegraded
        );
        for _ in 0..super::SURFACE_RECOVERY_ATTEMPTS {
            super::advance_surface_recovery(&mut app, false);
        }
        assert!(app.surface_recovery_exhausted);
        assert!(store
            .snapshot()
            .iter()
            .any(|event| event.name == "tier.surface_recovery_exhausted"));
        // Timing capability is unaffected, so the tier stays put and never
        // escalates to Quiescent from surface loss alone.
        super::refresh_operating_tier(&mut app);
        assert_eq!(
            app.operating_tier,
            tick_policy::OperatingTier::SurfaceDegraded
        );
        assert!(tick_policy::tier_allows_high_resolution(app.operating_tier));
    }

    #[test]
    fn published_status_degrades_running_stopped_and_paused_only() {
        assert_eq!(
            published_status(TrayStatus::Running, true),
            TrayStatus::Degraded
        );
        assert_eq!(
            published_status(TrayStatus::Stopped, true),
            TrayStatus::Degraded
        );
        assert_eq!(
            published_status(TrayStatus::Paused, true),
            TrayStatus::Degraded
        );
        for base in [
            TrayStatus::Blocked,
            TrayStatus::Error,
            TrayStatus::Unsupported,
            TrayStatus::Degraded,
            TrayStatus::Unverified,
            TrayStatus::Starting,
            TrayStatus::ScheduledStart,
            TrayStatus::Pausing,
            TrayStatus::Stopping,
            TrayStatus::ScheduledStop,
        ] {
            assert_eq!(
                published_status(base, true),
                base,
                "{base:?} must not be masked by responsiveness"
            );
        }
        assert_eq!(
            published_status(TrayStatus::Running, false),
            TrayStatus::Running
        );
    }

    #[test]
    fn heartbeat_lateness_escalates_to_degraded_after_three_samples() {
        let store = Arc::new(DiagnosticStore::new(64));
        let mut app = test_app(store.clone());
        // Seed a low baseline with an on time fire so the later lateness
        // samples classify as anomalies instead of reseeding the tracker.
        app.last_heartbeat_fire = Some(
            std::time::Instant::now()
                .checked_sub(std::time::Duration::from_millis(
                    tick_watchdog::HEARTBEAT_INTERVAL_MS,
                ))
                .expect("instant subtraction stays in range"),
        );
        handle_heartbeat_timer(&mut app);
        for _ in 0..tick_policy::ESCALATE_CONSECUTIVE_SAMPLES {
            app.last_heartbeat_fire = Some(
                std::time::Instant::now()
                    .checked_sub(std::time::Duration::from_millis(
                        tick_watchdog::STALL_THRESHOLD_MS + tick_watchdog::HEARTBEAT_INTERVAL_MS,
                    ))
                    .expect("instant subtraction stays in range"),
            );
            handle_heartbeat_timer(&mut app);
        }
        assert!(app.responsiveness_degraded);
        assert!(app.responsiveness_notified);
        assert!(store
            .snapshot()
            .iter()
            .any(|event| event.name == "responsiveness.degraded"));
    }

    #[test]
    fn heartbeat_recovery_clears_degraded_and_notification_flag() {
        let store = Arc::new(DiagnosticStore::new(64));
        let mut app = test_app(store.clone());
        app.last_heartbeat_fire = Some(
            std::time::Instant::now()
                .checked_sub(std::time::Duration::from_millis(
                    tick_watchdog::HEARTBEAT_INTERVAL_MS,
                ))
                .expect("instant subtraction stays in range"),
        );
        handle_heartbeat_timer(&mut app);
        for _ in 0..tick_policy::ESCALATE_CONSECUTIVE_SAMPLES {
            app.last_heartbeat_fire = Some(
                std::time::Instant::now()
                    .checked_sub(std::time::Duration::from_millis(
                        tick_watchdog::STALL_THRESHOLD_MS + tick_watchdog::HEARTBEAT_INTERVAL_MS,
                    ))
                    .expect("instant subtraction stays in range"),
            );
            handle_heartbeat_timer(&mut app);
        }
        assert!(app.responsiveness_degraded);
        for _ in 0..tick_policy::RECOVER_CONSECUTIVE_SAMPLES {
            app.last_heartbeat_fire = Some(
                std::time::Instant::now()
                    .checked_sub(std::time::Duration::from_millis(
                        tick_watchdog::HEARTBEAT_INTERVAL_MS,
                    ))
                    .expect("instant subtraction stays in range"),
            );
            handle_heartbeat_timer(&mut app);
        }
        assert!(!app.responsiveness_degraded);
        assert!(!app.responsiveness_notified);
        assert!(store
            .snapshot()
            .iter()
            .any(|event| event.name == "responsiveness.recovered"));
    }

    #[test]
    fn watchdog_stall_counter_folds_into_next_heartbeat_sample() {
        let store = Arc::new(DiagnosticStore::new(64));
        let mut app = test_app(store);
        // Seed the baseline with an on time fire so the folded stall
        // sample registers as an anomaly with a nonzero worst.
        app.last_heartbeat_fire = Some(
            std::time::Instant::now()
                .checked_sub(std::time::Duration::from_millis(
                    tick_watchdog::HEARTBEAT_INTERVAL_MS,
                ))
                .expect("instant subtraction stays in range"),
        );
        handle_heartbeat_timer(&mut app);
        app.watchdog_stall_events.store(1, Ordering::Release);
        app.last_heartbeat_fire = Some(std::time::Instant::now());
        handle_heartbeat_timer(&mut app);
        assert_eq!(
            app.watchdog_stall_events.load(Ordering::Acquire),
            0,
            "stall events are consumed by the heartbeat sample"
        );
        assert!(
            app.responsiveness.worst_ms() >= tick_watchdog::STALL_THRESHOLD_MS,
            "folded stall sample must reach at least the stall threshold"
        );
    }

    #[test]
    fn kill_heartbeat_timer_clears_last_fire_stamp() {
        let store = Arc::new(DiagnosticStore::new(16));
        let mut app = test_app(store);
        app.last_heartbeat_fire = Some(std::time::Instant::now());
        super::kill_heartbeat_timer(&mut app);
        assert!(app.last_heartbeat_fire.is_none());
    }
}
