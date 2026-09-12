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
    format_event, truncate_utf8, DiagnosticOutcome, DiagnosticPhase, DiagnosticRecord,
    DiagnosticSource, DiagnosticStore, NativeOutcome, OperationContext, DEFAULT_MAX_EVENTS,
};
use tick_observation_windows::{ObservationSource, WindowsObservation};
use tick_ownership::{OwnershipState, TimerController, TimingSnapshot, Verification};
use tick_platform_windows::{TimerObservation, WindowsTimerPlatform};
use tick_policy::{decide, PolicyInput, PowerState};
use tick_startup_windows::{
    startup_operation, StartupOperation, StartupRegistration, WindowsUserStartup,
};

use crate::shutdown::{
    message_loop_exit, shutdown_disposition, MessageLoopExit, ShutdownDisposition, ShutdownGate,
};
use crate::tray_surface::{
    dpi_to_icon_canvas, duration_choices, duration_command, duration_menu_items, icon_pixel_color,
    menu_action_keeps_open, menu_command_dispatch_allowed, menu_command_is_enabled_with_pause,
    menu_description, menu_items_with_duration, power_reconciliation, release_needs_handoff,
    status_menu_items, tooltip, tray_notification_opens_menu, HandoffProgress, HandoffTracker,
    PowerReconciliation, TimingValues, TrayStatus, CANCEL_SCHEDULED_COMMAND_ID, GITHUB_COMMAND_ID,
    GITHUB_URL, HANDOFF_POLL_INTERVAL_MS, LOGS_COMMAND_ID, PAUSE_FOR_15_COMMAND_ID,
    PAUSE_FOR_30_COMMAND_ID, PAUSE_FOR_5_COMMAND_ID, PAUSE_FOR_60_COMMAND_ID,
    START_IN_15_COMMAND_ID, START_IN_1_COMMAND_ID, START_IN_30_COMMAND_ID, START_IN_5_COMMAND_ID,
    START_IN_60_COMMAND_ID, STOP_IN_15_COMMAND_ID, STOP_IN_1_COMMAND_ID, STOP_IN_30_COMMAND_ID,
    STOP_IN_5_COMMAND_ID, STOP_IN_60_COMMAND_ID,
};

const WM_APP: u32 = 0x8000;
const WM_TRAY: u32 = WM_APP + 1;
const WM_CREATE: u32 = 0x0001;
const WM_COMMAND: u32 = 0x0111;
const WM_DESTROY: u32 = 0x0002;
const WM_POWERBROADCAST: u32 = 0x0218;
const WM_TIMER: u32 = 0x0113;
const WM_MENUSELECT: u32 = 0x011F;
const PBT_APMPOWERSTATUSCHANGE: usize = 0x000A;
const HANDOFF_TIMER_ID: usize = 0x5449;
const DURATION_TIMER_ID_BASE: usize = 0x6000;
const POPUP_REFRESH_TIMER_ID: usize = 0x7000;
/// Popup-only UI cadence for live countdown and status refresh.
const POPUP_REFRESH_INTERVAL_MS: u32 = 500;
const ID_START: usize = 1001;
const ID_STOP: usize = 1002;
const ID_QUIT: usize = 1004;
const ID_STARTUP_ON: usize = 1005;
const ID_STARTUP_OFF: usize = 1006;
const ID_AUTOMATIC_ON: usize = 1007;
const ID_AUTOMATIC_OFF: usize = 1008;

const WM_SIZE: u32 = 0x0005;
const WM_CLOSE: u32 = 0x0010;
const WM_NCDESTROY: u32 = 0x0082;
const WM_SETFOCUS: u32 = 0x0007;
const WM_GETMINMAXINFO: u32 = 0x0024;
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
const DIAGNOSTIC_MIN_WIDTH: i32 = 420;
const DIAGNOSTIC_MIN_HEIGHT: i32 = 260;
const DIAGNOSTIC_WINDOW_TITLE: &str = "True™ Tick Status and Diagnostics";
const MAX_STARTUP_STATUS_BYTES: usize = 512;
const MAX_DIAGNOSTIC_TEXT_BYTES: usize = 64 * 1024;
const DIAGNOSTIC_WINDOW_PARENT: *mut c_void = std::ptr::null_mut();
const WS_CHILD: u32 = 0x40000000;
const WS_VSCROLL: u32 = 0x00200000;
const ES_MULTILINE: u32 = 0x0004;
const ES_READONLY: u32 = 0x0800;
const ES_AUTOVSCROLL: u32 = 0x0040;
const ES_AUTOHSCROLL: u32 = 0x0080;
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
const GW_CHILD: u32 = 5;
const IDI_APPLICATION: usize = 32512;
const MB_ICONWARNING: u32 = 0x0000_0030;
const MB_YESNO: u32 = 0x0000_0004;
const MB_DEFBUTTON2: u32 = 0x0000_0100;
const IDYES: i32 = 6;
const TASKDIALOG_BUTTON_CANCEL: i32 = 2001;
const TASKDIALOG_BUTTON_STOP_AND_QUIT: i32 = 2002;
const TASKDIALOG_ICON_WARNING: *const u16 = (-1isize) as *const u16;
const TDF_ALLOW_DIALOG_CANCELLATION: u32 = 0x0008;
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
    task_dialog_hresult: Option<i32>,
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
struct TaskDialogButton {
    button_id: i32,
    button_text: *const u16,
}

#[repr(C)]
struct TaskDialogConfig {
    size: u32,
    parent: *mut c_void,
    instance: *mut c_void,
    flags: u32,
    common_buttons: u32,
    window_title: *const u16,
    main_icon: *const u16,
    main_instruction: *const u16,
    content: *const u16,
    button_count: u32,
    buttons: *const TaskDialogButton,
    default_button: i32,
    radio_button_count: u32,
    radio_buttons: *const c_void,
    default_radio_button: i32,
    verification_text: *const u16,
    expanded_information: *const u16,
    expanded_control_text: *const u16,
    collapsed_control_text: *const u16,
    footer: *const u16,
    callback: *const c_void,
    callback_data: isize,
    width: u32,
}

#[repr(C)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
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
    menu_active: bool,
    popup_menus: Option<PopupMenuHandles>,
    popup_refresh_timer_active: bool,
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
            menu_active: false,
            popup_menus: None,
            popup_refresh_timer_active: false,
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
            background: std::ptr::null_mut(),
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
        let mut icon = match NotifyIconData::new(hwnd, app.lifecycle_status(), app.timing_values())
        {
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
    }
}

fn cancel_scheduled_action(app: &mut App) {
    let now = std::time::Instant::now();
    let Some(previous) = app.pause.current() else {
        app.record(
            "duration.cancel",
            "outcome=noop reason=no_scheduled_action remaining_ms=0",
        );
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
    if cancelled.action == DurationAction::Pause {
        refresh_power_for_duration(app, "cancel");
        reconcile(app);
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
    let ntstatus = if name.contains("Nt") || name.contains("timer") {
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
        crate::tray_surface::lifecycle_status(self.tray_status, self.handoff.is_some())
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
        self.scheduled_operation = None;
        self.begin_operation(DiagnosticSource::Shutdown);
        self.record(
            "shutdown.cleanup",
            format!(
                "attempt=guarded number={}",
                self.shutdown_gate.attempts() + 1
            ),
        );
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
        if let Some(window) = self.diagnostic_window {
            unsafe { refresh_diagnostic_window(window, self) };
        }
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
        if let Some(window) = self.diagnostic_window {
            unsafe { refresh_diagnostic_window(window, self) };
        }
    }

    fn publish(&mut self) {
        let status = self.lifecycle_status();
        if status != TrayStatus::Running || self.controller.ownership() != OwnershipState::Owned {
            self.running_since = None;
        }
        self.tray_status = status;
        let timing = self.timing_values();
        let key = PublicationKey {
            status,
            ownership: self.controller.ownership(),
            effective: timing.effective,
            requested: timing.requested,
            handoff: timing.handoff_pending,
        };
        if self.last_publication == Some(key) {
            return;
        }
        self.last_publication = Some(key);
        let status_text = tooltip(status, timing);
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
            if let Err(raw_error) = update_icon(icon, status, timing) {
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
) -> Result<(), u32> {
    let replacement = unsafe { status_icon(status, dpi_for_window(icon.h_wnd)) }?;
    let old_icon = icon.h_icon;
    let old_tip = icon.sz_tip;
    icon.h_icon = replacement;
    icon.sz_tip = [0; 128];
    for (target, source) in icon
        .sz_tip
        .iter_mut()
        .zip(tooltip(status, timing).encode_utf16())
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
            && append_duration_submenu(app, menu)
            && append_menu_checked(app, menu, MF_SEPARATOR, 0, std::ptr::null())
            && append_menu_checked(
                app,
                menu,
                MF_STRING,
                startup_id,
                wide(&items[4].label).as_ptr(),
            )
            && append_menu_checked(
                app,
                menu,
                MF_STRING,
                automatic_id,
                wide(&items[5].label).as_ptr(),
            )
            && append_menu_checked(app, menu, MF_SEPARATOR, 0, std::ptr::null())
            && append_status_submenu(app, menu)
            && append_menu_checked(app, menu, MF_SEPARATOR, 0, std::ptr::null())
            && append_menu_checked(
                app,
                menu,
                MF_STRING,
                ID_QUIT,
                wide(&items[7].label).as_ptr(),
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
        app.pause.pause_active(),
    );
    let stop_enabled = menu_command_is_enabled_with_pause(
        ID_STOP,
        app.lifecycle_status(),
        app.config.startup_enabled,
        app.config.automatic,
        app.pause.pause_active(),
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
        6,
        MF_BYPOSITION | MF_STRING,
        startup_id,
        wide(crate::tray_surface::auto_start_label(
            app.config.startup_enabled,
        ))
        .as_ptr(),
    );
    let _ = ModifyMenuW(
        handles.root,
        7,
        MF_BYPOSITION | MF_STRING,
        automatic_id,
        wide(crate::tray_surface::automatic_label(app.config.automatic)).as_ptr(),
    );
    if let Some(duration) = handles.duration {
        let cancel_enabled = app.pause.current().is_some();
        let _ = ModifyMenuW(
            duration,
            3,
            MF_BYPOSITION | MF_STRING | if cancel_enabled { 0 } else { MF_GRAYED },
            CANCEL_SCHEDULED_COMMAND_ID,
            wide("Cancel scheduled action").as_ptr(),
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
                0,
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
        ok &= append_menu_checked(
            app,
            submenu,
            MF_STRING,
            *command,
            wide(&item.label).as_ptr(),
        );
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
    ok &= append_duration_choice_submenu(app, submenu, DurationAction::Pause);
    let cancel = duration_menu_items(app.pause.current())[3].clone();
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
    if !ok {
        DestroyMenu(submenu);
        return false;
    }
    if !append_menu_checked(
        app,
        menu,
        MF_STRING | MF_POPUP,
        submenu as usize,
        wide("Duration >").as_ptr(),
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
        let command = if item.enabled && item.label == "Logs" {
            LOGS_COMMAND_ID
        } else {
            0
        };
        ok &= append_menu_checked(app, submenu, flags, command, wide(&item.label).as_ptr());
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
            "Duration >" => Some("Schedule a bounded timing action"),
            "Start in >" => Some("Schedule a future guarded acquire"),
            "Stop in >" => Some("Schedule a future guarded release"),
            "Pause for >" => Some("Suppress acquisition for a fixed duration"),
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
        app.record(
            "tray.command.rejected",
            format!("id={command} reason=disabled_or_unknown"),
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
        CANCEL_SCHEDULED_COMMAND_ID => cancel_scheduled_action(app),
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
                    let warning = show_quit_warning(hwnd);
                    app.record(
                        "quit.dialog.result",
                        format!(
                            "dialog_path=message_box task_dialog=secondary_not_invoked task_dialog_hresult={:?} task_dialog_hresult_hex=not_invoked message_box_result={:?} final_decision={:?} dialog_shown={}",
                            warning.task_dialog_hresult,
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
            refresh_diagnostic_window(window, app);
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
        820,
        560,
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
        ShowWindow(window, SW_SHOWNORMAL);
        UpdateWindow(window);
        SetForegroundWindow(window);
        app.diagnostic_window = Some(window);
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
        refresh_diagnostic_window(window, app);
    }
}

unsafe fn refresh_diagnostic_window(window: *mut c_void, app: &App) {
    let edit = GetWindow(window, GW_CHILD);
    if edit.is_null() {
        return;
    }
    let timing_details = match app.timing_snapshot.effective {
        Some(effective) => format!(
            "Timing observation: {} requested_hns={} selected_hns={} effective_hns={} minimum_hns={} maximum_hns={} raw_status={} effective_relation={}\r\n",
            tooltip(app.lifecycle_status(), app.timing_values()),
            app.timing_snapshot.requested.map_or_else(|| "unknown".to_owned(), |value| value.value().to_string()),
            app.timing_snapshot.selected.map_or_else(|| "unknown".to_owned(), |value| value.value().to_string()),
            effective.value(),
            app.timing_snapshot.minimum_interval.map_or_else(|| "unknown".to_owned(), |value| value.value().to_string()),
            app.timing_snapshot.maximum_interval.map_or_else(|| "unknown".to_owned(), |value| value.value().to_string()),
            app.timing_snapshot.raw_status.map_or_else(|| "unknown".to_owned(), |value| value.to_string()),
            app.timing_snapshot.effective_relation().unwrap_or("unknown")
        ),
        None => "Timing observation: unknown\r\n".to_owned(),
    };
    let mut text = format!(
        "Current status: {}\r\n{}Power observation: {:?}\r\nStartup: {}\r\nSession log is local to this process. Maximum events: {}\r\n\r\n",
        tooltip(app.tray_status, app.timing_values()),
        timing_details,
        app.observation.power().state,
        app.startup_status,
        app.diagnostics.maximum_events()
    );
    for event in app.diagnostics.snapshot() {
        text.push_str(&format_event(&event));
        text.push_str("\r\n");
    }
    let text = truncate_utf8(&text, MAX_DIAGNOSTIC_TEXT_BYTES);
    let text = wide(&text);
    if SetWindowTextW(edit, text.as_ptr()) == 0 {
        app.diagnostics.record(
            "native.SetWindowTextW.edit.error",
            format!("raw_status={}", GetLastError()),
        );
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
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, app_ptr as isize);
        let edit_class = wide("EDIT");
        let edit = CreateWindowExW(
            0,
            edit_class.as_ptr(),
            std::ptr::null(),
            WS_CHILD
                | WS_VISIBLE
                | WS_VSCROLL
                | WS_CLIPCHILDREN
                | ES_MULTILINE
                | ES_READONLY
                | WS_BORDER
                | ES_AUTOVSCROLL
                | ES_AUTOHSCROLL,
            0,
            0,
            800,
            500,
            hwnd,
            std::ptr::null_mut(),
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null_mut(),
        );
        if edit.is_null() {
            if !app_ptr.is_null() {
                (*(app_ptr as *mut App)).diagnostics.record(
                    "native.CreateWindowExW.diagnostic_edit.error",
                    format!("raw_status={}", GetLastError()),
                );
            }
            return 1;
        }
        if !app_ptr.is_null() {
            refresh_diagnostic_window(hwnd, &*(app_ptr as *mut App));
        }
        return 0;
    }
    if !app.is_null() {
        if message == WM_SIZE {
            let width = (l_param as u32 & 0xffff) as i32;
            let height = ((l_param as u32 >> 16) & 0xffff) as i32;
            let edit = GetWindow(hwnd, GW_CHILD);
            if edit.is_null() {
                (*app).diagnostics.record(
                    "native.GetWindow.edit.error",
                    format!("raw_status={}", GetLastError()),
                );
            } else if MoveWindow(edit, 0, 0, width, height, 1) == 0 {
                (*app).diagnostics.record(
                    "native.MoveWindow.error",
                    format!("raw_status={}", GetLastError()),
                );
            }
        } else if message == WM_GETMINMAXINFO {
            let limits = l_param as *mut MinMaxInfo;
            if !limits.is_null() {
                (*limits).minimum_track_size.x = DIAGNOSTIC_MIN_WIDTH;
                (*limits).minimum_track_size.y = DIAGNOSTIC_MIN_HEIGHT;
            }
            return 0;
        } else if message == WM_CLOSE {
            DestroyWindow(hwnd);
            return 0;
        } else if message == WM_SETFOCUS {
            if !app.is_null() {
                refresh_diagnostic_window(hwnd, &*app);
            }
        } else if message == WM_NCDESTROY {
            (*app).diagnostic_window = None;
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
    if app.handoff.is_some() {
        return QuitDecision::RequireSafetyDialog {
            reason: QuitSafetyReason::TimingNotSettled,
        };
    }
    quit_decision_for(app.lifecycle_status(), app.controller.ownership())
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

unsafe fn show_quit_warning(hwnd: *mut c_void) -> QuitWarningResult {
    let title = wide("True™ Tick quit warning");
    let text = wide(
        "True™ Tick is still active or its ownership is uncertain.\n\nYes = Stop and Quit\nNo = Cancel\n\nOnly Yes will stop timing and quit after verification.",
    );
    let result = MessageBoxW(
        hwnd,
        text.as_ptr(),
        title.as_ptr(),
        MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
    );
    quit_warning_result(None, 0, Some(result))
}

#[allow(dead_code)]
unsafe fn show_quit_warning_task_dialog_secondary(hwnd: *mut c_void) -> (i32, i32) {
    let cancel = wide("Cancel");
    let stop_and_quit = wide("Stop and Quit");
    let buttons = [
        TaskDialogButton {
            button_id: TASKDIALOG_BUTTON_CANCEL,
            button_text: cancel.as_ptr(),
        },
        TaskDialogButton {
            button_id: TASKDIALOG_BUTTON_STOP_AND_QUIT,
            button_text: stop_and_quit.as_ptr(),
        },
    ];
    let title = wide("True™ Tick quit warning");
    let instruction = wide("Stop timing before quitting?");
    let content = wide(
        "Timing is active or ownership is uncertain. Quit only after a safe stop is verified.",
    );
    let config = TaskDialogConfig {
        size: size_of::<TaskDialogConfig>() as u32,
        parent: hwnd,
        instance: std::ptr::null_mut(),
        flags: TDF_ALLOW_DIALOG_CANCELLATION,
        common_buttons: 0,
        window_title: title.as_ptr(),
        main_icon: TASKDIALOG_ICON_WARNING,
        main_instruction: instruction.as_ptr(),
        content: content.as_ptr(),
        button_count: buttons.len() as u32,
        buttons: buttons.as_ptr(),
        default_button: TASKDIALOG_BUTTON_CANCEL,
        radio_button_count: 0,
        radio_buttons: std::ptr::null(),
        default_radio_button: 0,
        verification_text: std::ptr::null(),
        expanded_information: std::ptr::null(),
        expanded_control_text: std::ptr::null(),
        collapsed_control_text: std::ptr::null(),
        footer: std::ptr::null(),
        callback: std::ptr::null(),
        callback_data: 0,
        width: 0,
    };
    let mut selected = 0;
    let hresult = TaskDialogIndirect(
        &config,
        &mut selected,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
    );
    (hresult, selected)
}

fn quit_warning_result(
    task_dialog_hresult: Option<i32>,
    task_dialog_button: i32,
    message_box_result: Option<i32>,
) -> QuitWarningResult {
    match task_dialog_hresult {
        Some(hresult) if hresult < 0 => {
            let result = message_box_result.unwrap_or(0);
            QuitWarningResult {
                decision: message_box_decision(result),
                task_dialog_hresult: Some(hresult),
                message_box_result,
                dialog_shown: result != 0,
            }
        }
        Some(hresult) => QuitWarningResult {
            decision: task_dialog_decision(task_dialog_button),
            task_dialog_hresult: Some(hresult),
            message_box_result: None,
            dialog_shown: true,
        },
        None => {
            let result = message_box_result.unwrap_or(0);
            QuitWarningResult {
                decision: message_box_decision(result),
                task_dialog_hresult: None,
                message_box_result,
                dialog_shown: result != 0,
            }
        }
    }
}

fn task_dialog_decision(button_id: i32) -> QuitDialogDecision {
    if button_id == TASKDIALOG_BUTTON_STOP_AND_QUIT {
        QuitDialogDecision::StopAndQuit
    } else {
        QuitDialogDecision::Cancel
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
    fn new(hwnd: *mut c_void, status: TrayStatus, timing: TimingValues) -> Result<Self, u32> {
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
            .zip(tooltip(status, timing).encode_utf16())
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
    fn GetWindow(window: *mut c_void, command: u32) -> *mut c_void;
    fn SetWindowTextW(window: *mut c_void, text: *const u16) -> i32;
    fn SendMessageW(hwnd: *mut c_void, message: u32, w: usize, l: isize) -> isize;
    fn MoveWindow(
        window: *mut c_void,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        repaint: i32,
    ) -> i32;
    fn GetCursorPos(point: *mut Point) -> i32;
    fn LoadIconW(instance: *mut c_void, name: *const u16) -> *mut c_void;
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    fn GetDpiForWindow(window: *mut c_void) -> u32;
    fn CreateIconIndirect(info: *const IconInfo) -> *mut c_void;
    fn DestroyIcon(icon: *mut c_void) -> i32;
}

#[link(name = "comctl32")]
extern "system" {
    fn TaskDialogIndirect(
        config: *const TaskDialogConfig,
        button: *mut i32,
        radio_button: *mut i32,
        verification_flag: *mut i32,
    ) -> i32;
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn task_dialog_uses_non_colliding_ids_and_only_stop_id_quits() {
        assert_eq!(TASKDIALOG_BUTTON_CANCEL, 2001);
        assert_eq!(TASKDIALOG_BUTTON_STOP_AND_QUIT, 2002);
        assert_ne!(TASKDIALOG_BUTTON_CANCEL, 2);
        assert_ne!(TASKDIALOG_BUTTON_STOP_AND_QUIT, 2);
        assert_eq!(
            task_dialog_decision(TASKDIALOG_BUTTON_STOP_AND_QUIT),
            QuitDialogDecision::StopAndQuit
        );
        for button_id in [TASKDIALOG_BUTTON_CANCEL, 2, 0, -1, 9999] {
            assert_eq!(task_dialog_decision(button_id), QuitDialogDecision::Cancel);
        }
    }

    #[test]
    fn failed_task_dialog_uses_the_message_box_fallback_decision() {
        let result = quit_warning_result(Some(-0x7ff8_ffff), 0, Some(IDYES));
        assert_eq!(result.decision, QuitDialogDecision::StopAndQuit);
        assert_eq!(result.message_box_result, Some(IDYES));
        assert!(result.dialog_shown);
        assert_eq!(result.task_dialog_hresult, Some(-0x7ff8_ffff));
    }

    #[test]
    fn primary_message_box_path_does_not_claim_task_dialog_was_shown() {
        let result = quit_warning_result(None, 0, Some(IDYES));
        assert_eq!(result.decision, QuitDialogDecision::StopAndQuit);
        assert_eq!(result.task_dialog_hresult, None);
        assert_eq!(result.message_box_result, Some(IDYES));
        assert!(result.dialog_shown);
    }

    #[test]
    fn message_box_maps_only_yes_to_stop_and_quit() {
        assert_eq!(message_box_decision(IDYES), QuitDialogDecision::StopAndQuit);
        for result in [7, 0, 1, -1, 9999] {
            assert_eq!(message_box_decision(result), QuitDialogDecision::Cancel);
        }
        let failed = quit_warning_result(Some(-1), 0, Some(0));
        assert_eq!(failed.decision, QuitDialogDecision::Cancel);
        assert!(!failed.dialog_shown);
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
        assert_eq!(DIAGNOSTIC_MIN_WIDTH, 420);
        assert_eq!(DIAGNOSTIC_MIN_HEIGHT, 260);
    }

    #[test]
    fn quit_decision_allows_only_settled_released_state_to_exit() {
        assert_eq!(
            quit_decision_for(TrayStatus::Stopped, OwnershipState::Released),
            QuitDecision::ExitNormally
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
}
