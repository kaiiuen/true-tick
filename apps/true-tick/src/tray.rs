use crate::config;
use std::ffi::c_void;
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tick_diagnostics::{format_event, DiagnosticStore, DEFAULT_MAX_EVENTS};
use tick_observation_windows::{ObservationSource, WindowsObservation};
use tick_ownership::{OwnershipState, TimerController, Verification};
use tick_platform_windows::{TimerObservation, WindowsTimerPlatform};
use tick_policy::{decide, PolicyInput, PowerState};
use tick_startup_windows::{
    startup_operation, StartupOperation, StartupRegistration, WindowsUserStartup,
};

use crate::tray_surface::{
    dpi_to_icon_canvas, icon_pixel_color, menu_action_keeps_open, menu_items, tooltip,
    tray_click_action, TimingValues, TrayClickAction, TrayStatus, STATUS_COMMAND_ID,
};

const WM_APP: u32 = 0x8000;
const WM_TRAY: u32 = WM_APP + 1;
const WM_CREATE: u32 = 0x0001;
const WM_COMMAND: u32 = 0x0111;
const WM_DESTROY: u32 = 0x0002;
const WM_POWERBROADCAST: u32 = 0x0218;
const PBT_APMPOWERSTATUSCHANGE: usize = 0x000A;
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
const DIAGNOSTIC_WINDOW_TITLE: &str = "True Tick Status and Diagnostics";
const DIAGNOSTIC_WINDOW_PARENT: *mut c_void = std::ptr::null_mut();
const WS_CHILD: u32 = 0x40000000;
const WS_VSCROLL: u32 = 0x00200000;
const ES_MULTILINE: u32 = 0x0004;
const ES_READONLY: u32 = 0x0800;
const ES_AUTOVSCROLL: u32 = 0x0040;
const ES_AUTOHSCROLL: u32 = 0x0080;
const TPM_RIGHTBUTTON: u32 = 0x0002;
const TPM_NONOTIFY: u32 = 0x0080;
const TPM_RETURNCMD: u32 = 0x0100;
const MF_STRING: u32 = 0x0000;
const MF_SEPARATOR: u32 = 0x0800;
const MF_GRAYED: u32 = 0x0001;

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
const TASKDIALOG_BUTTON_CANCEL: i32 = 2001;
const TASKDIALOG_BUTTON_STOP_AND_QUIT: i32 = 2002;
const TASKDIALOG_ICON_WARNING: *const u16 = (-1isize) as *const u16;
const TDF_ALLOW_DIALOG_CANCELLATION: u32 = 0x0008;

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
    task_dialog_hresult: i32,
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

struct App {
    controller: TimerController<WindowsTimerPlatform>,
    observation: WindowsObservation,
    config: config::Config,
    tray_status: TrayStatus,
    config_path: PathBuf,
    executable: PathBuf,
    startup_status: String,
    tray_icon: Option<NotifyIconData>,
    timing_observation: Option<TimerObservation>,
    invalid_interval: bool,
    diagnostics: Arc<DiagnosticStore>,
    diagnostic_window: Option<*mut c_void>,
    shutdown_cleanup_done: bool,
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
                "legacy_request_hns=10000 result=automatic_selection",
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
        let app = Box::new(App {
            controller: TimerController::new(
                WindowsTimerPlatform::with_diagnostics(diagnostics.clone()),
                loaded.request_interval,
            ),
            observation,
            config: loaded,
            tray_status: if startup_status.starts_with("Red") {
                TrayStatus::Error
            } else if power_observation_error.is_some() {
                TrayStatus::Degraded
            } else {
                TrayStatus::Pending
            },
            config_path,
            executable,
            startup_status,
            tray_icon: None,
            timing_observation: None,
            invalid_interval: false,
            diagnostics,
            diagnostic_window: None,
            shutdown_cleanup_done: false,
        });
        let app_ptr = Box::into_raw(app);
        let app = &mut *app_ptr;
        let class_name = wide("TrueTickTrayClass");
        let wnd_class = WndClass {
            style: 0,
            wnd_proc: Some(window_proc),
            cls_extra: 0,
            wnd_extra: 0,
            instance: GetModuleHandleW(std::ptr::null()),
            icon: LoadIconW(std::ptr::null_mut(), IDI_APPLICATION as *const u16),
            cursor: std::ptr::null_mut(),
            background: std::ptr::null_mut(),
            menu_name: std::ptr::null(),
            class_name: class_name.as_ptr(),
        };
        RegisterClassW(&wnd_class);
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
        RegisterClassW(&diagnostic_class);
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
        let mut icon = NotifyIconData::new(hwnd, app.tray_status, app.timing_values());
        Shell_NotifyIconW(NIM_ADD, &mut icon);
        app.tray_icon = Some(icon);
        if app.config.automatic {
            app.record("policy.startup_automatic", "enabled=true");
            reconcile(app);
        }
        app.publish();
        let mut message = Message::default();
        while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        let _ = app.cleanup_normal_shutdown();
        if let Some(mut icon) = app.tray_icon.take() {
            Shell_NotifyIconW(NIM_DELETE, &mut icon);
        }
        app.record("lifecycle.shutdown", "application_shutdown");
        drop(Box::from_raw(app_ptr));
    }
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
    if decision.status == tick_core::Status::Requested {
        app.tray_status = TrayStatus::Starting;
        app.publish();
        app.record("ownership.acquire.request", "source=policy");
        let start_result = app.controller.start();
        app.sync_timing_observation();
        app.invalid_interval = matches!(
            start_result,
            Err(tick_platform_windows::TimerError::InvalidInterval)
        );
        app.tray_status = match start_result {
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
        app.publish();
        return;
    }

    release_for_policy(app, decision.reason);
}

fn release_for_policy(app: &mut App, reason: tick_policy::PolicyReason) {
    let released_status = match reason {
        tick_policy::PolicyReason::BatteryRestricted
        | tick_policy::PolicyReason::BatterySaverRestricted
        | tick_policy::PolicyReason::PowerUnknown
        | tick_policy::PolicyReason::GloballyDisabled => TrayStatus::Blocked,
        tick_policy::PolicyReason::NoEligibleProfile
        | tick_policy::PolicyReason::EligibleProfile => TrayStatus::Stopped,
    };
    let _ = guarded_release(app, format!("policy reason={reason:?}"), released_status);
}

fn guarded_release(
    app: &mut App,
    source: impl AsRef<str>,
    released_status: TrayStatus,
) -> Result<bool, String> {
    app.record(
        "ownership.release.request",
        format!("source={}", source.as_ref()),
    );
    app.tray_status = TrayStatus::Stopping;
    app.publish();
    let stop_result = app.controller.stop();
    app.sync_timing_observation();
    match stop_result {
        Ok(released) => {
            app.record(
                "ownership.changed",
                format!("state=released changed={released}"),
            );
            app.tray_status = released_status;
            app.publish();
            Ok(released)
        }
        Err(error) => {
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
    reconcile(app);
}

fn manual_stop(app: &mut App) {
    app.record("tray.command", "command=stop");
    app.record("lifecycle.stop_request", "source=manual");
    let _ = guarded_release(app, "manual", TrayStatus::Stopped);
}

fn release_for_power_change(app: &mut App) {
    app.record(
        "power.transition",
        format!("state={:?}", app.observation.power().state),
    );
    refresh_timing_observation(app);
    if app.observation.power().state != PowerState::Ac {
        release_for_policy(
            app,
            match app.observation.power().state {
                PowerState::Battery => tick_policy::PolicyReason::BatteryRestricted,
                PowerState::BatterySaver => tick_policy::PolicyReason::BatterySaverRestricted,
                PowerState::Unknown => tick_policy::PolicyReason::PowerUnknown,
                PowerState::Ac => unreachable!(),
            },
        );
    }
}

impl App {
    fn sync_timing_observation(&mut self) {
        self.timing_observation = self.controller.observation();
    }

    fn timing_values(&self) -> TimingValues {
        TimingValues {
            requested: self
                .timing_observation
                .map(|observation| observation.requested)
                .or_else(|| {
                    (self.config.request_interval != config::AUTOMATIC_REQUEST_INTERVAL)
                        .then_some(self.config.request_interval)
                }),
            effective: self
                .timing_observation
                .map(|observation| observation.reported_current),
            invalid_interval: self.invalid_interval,
        }
    }

    fn cleanup_normal_shutdown(&mut self) -> Result<(), String> {
        if self.shutdown_cleanup_done {
            return Ok(());
        }
        self.record("shutdown.cleanup", "attempt=guarded");
        match guarded_release(self, "shutdown", TrayStatus::Stopped) {
            Ok(released) => {
                self.shutdown_cleanup_done = true;
                self.record(
                    "shutdown.cleanup.result",
                    format!("result=verified released={released}"),
                );
                Ok(())
            }
            Err(error) => {
                self.record(
                    "shutdown.cleanup.result",
                    format!("result=unverified error={error}"),
                );
                Err(error)
            }
        }
    }

    fn record(&mut self, name: &str, details: impl AsRef<str>) {
        self.diagnostics.record(name, details);
        if let Some(window) = self.diagnostic_window {
            unsafe { refresh_diagnostic_window(window, self) };
        }
    }

    fn publish(&mut self) {
        let timing = self.timing_values();
        let status_text = tooltip(self.tray_status, timing);
        self.record(
            "tray.status.changed",
            format!("status={:?} tooltip={status_text}", self.tray_status),
        );
        if let Some(icon) = self.tray_icon.as_mut() {
            update_icon(icon, self.tray_status, timing);
        }
    }
}

fn refresh_timing_observation(app: &mut App) {
    match app.controller.query() {
        Ok(observation) => {
            app.sync_timing_observation();
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
        }
        Err(error) => app.record("timer.query.error", format!("error={error:?}")),
    }
}

fn update_icon(icon: &mut NotifyIconData, status: TrayStatus, timing: TimingValues) {
    let replacement = unsafe { status_icon(status, dpi_for_window(icon.h_wnd)) };
    if replacement.is_null() {
        return;
    }
    let old_icon = icon.h_icon;
    icon.h_icon = replacement;
    icon.sz_tip = [0; 128];
    for (target, source) in icon
        .sz_tip
        .iter_mut()
        .zip(tooltip(status, timing).encode_utf16())
    {
        *target = source;
    }
    unsafe {
        Shell_NotifyIconW(NIM_MODIFY, icon);
        destroy_icon(old_icon);
    }
}

unsafe fn status_icon(status: TrayStatus, dpi: u32) -> *mut c_void {
    let color = icon_pixel_color(status);
    let canvas = dpi_to_icon_canvas(dpi);
    let pixel_count = (canvas * canvas) as usize;
    let pixels = vec![color; pixel_count];
    let mask = vec![0u8; pixel_count / 8];
    let bitmap = CreateBitmap(canvas, canvas, 1, 32, pixels.as_ptr() as *const c_void);
    let mask_bitmap = CreateBitmap(canvas, canvas, 1, 1, mask.as_ptr() as *const c_void);
    let info = IconInfo {
        f_icon: 1,
        x_hotspot: 0,
        y_hotspot: 0,
        h_bm_mask: mask_bitmap,
        h_bm_color: bitmap,
    };
    let icon = CreateIconIndirect(&info);
    DeleteObject(bitmap);
    DeleteObject(mask_bitmap);
    icon
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
    if !app.is_null() {
        let app = &mut *app;
        match message {
            WM_TRAY
                if matches!(
                    tray_click_action(l_param as usize),
                    Some(TrayClickAction::OpenMenu)
                ) =>
            {
                show_menu(hwnd, app)
            }
            WM_COMMAND => {
                handle_menu_command(hwnd, app, w_param & 0xffff);
            }
            WM_POWERBROADCAST if w_param == PBT_APMPOWERSTATUSCHANGE => {
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
                if app.config.automatic {
                    reconcile(app);
                } else {
                    release_for_power_change(app);
                }
                app.publish();
            }
            WM_DESTROY => PostQuitMessage(0),
            _ => {}
        }
    }
    DefWindowProcW(hwnd, message, w_param, l_param)
}

unsafe fn show_menu(hwnd: *mut c_void, app: &mut App) {
    let mut anchor = Point { x: 0, y: 0 };
    GetCursorPos(&mut anchor);
    loop {
        let menu = CreatePopupMenu();
        let items = menu_items(
            app.tray_status,
            app.config.startup_enabled,
            app.config.automatic,
        );
        let start_flags = if items[0].enabled {
            MF_STRING
        } else {
            MF_STRING | MF_GRAYED
        };
        let stop_flags = if items[1].enabled {
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
        AppendMenuW(menu, start_flags, ID_START, wide(items[0].label).as_ptr());
        AppendMenuW(menu, stop_flags, ID_STOP, wide(items[1].label).as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(menu, MF_STRING, startup_id, wide(items[2].label).as_ptr());
        AppendMenuW(menu, MF_STRING, automatic_id, wide(items[3].label).as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(
            menu,
            MF_STRING,
            STATUS_COMMAND_ID,
            wide(&format!(
                "Status: {}",
                tooltip(app.tray_status, app.timing_values())
            ))
            .as_ptr(),
        );
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(menu, MF_STRING, ID_QUIT, wide(items[5].label).as_ptr());
        SetForegroundWindow(hwnd);
        let command = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_NONOTIFY | TPM_RETURNCMD,
            anchor.x,
            anchor.y,
            0,
            hwnd,
            std::ptr::null(),
        );
        DestroyMenu(menu);
        let Some(command) = returned_menu_command(command) else {
            break;
        };
        app.record(
            "tray.command.dispatch",
            format!("source=TPM_RETURNCMD id={command}"),
        );
        if !handle_menu_command(hwnd, app, command) {
            break;
        }
    }
}

unsafe fn handle_menu_command(hwnd: *mut c_void, app: &mut App, command: usize) -> bool {
    app.record("tray.command.id", format!("id={command}"));
    match command {
        ID_START => manual_start(app),
        ID_STOP => manual_stop(app),
        STATUS_COMMAND_ID => {
            app.record("tray.command", "command=status");
            open_diagnostic_window(app);
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
                    app.record("quit.cleanup.result", "result=not_needed");
                    app.record("quit.exit.allowed", "result=allowed reason=already_stopped");
                    PostQuitMessage(0);
                    return false;
                }
                QuitDecision::RequireSafetyDialog { reason } => {
                    app.record("quit.warning.shown", format!("reason={reason:?}"));
                    let warning = show_quit_warning(hwnd);
                    let task_dialog_hresult = warning.task_dialog_hresult;
                    app.record(
                        "quit.dialog.result",
                        format!(
                            "task_dialog_hresult={task_dialog_hresult} task_dialog_hresult_hex=0x{:08X} message_box_result={:?} final_decision={:?} dialog_shown={}",
                            task_dialog_hresult as u32,
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
                                        "Tick could not verify a safe stop. The app remains open.\n\n{error}"
                                    );
                                    MessageBoxW(
                                        hwnd,
                                        wide(&message).as_ptr(),
                                        wide("True Tick quit warning").as_ptr(),
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
    menu_action_keeps_open(command)
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
        app.record("diagnostic.window.result", "result=create_failed");
    } else {
        SetWindowTextW(window, title.as_ptr());
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
    let timing_details = match app.timing_observation {
        Some(observation) => format!(
            "Timing observation: {} requested_hns={} effective_hns={} raw_status={} effective_relation={}\r\n",
            tooltip(app.tray_status, app.timing_values()),
            observation.requested.value(),
            observation.reported_current.value(),
            observation.raw_status,
            observation.effective_relation()
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
    let text = wide(&text);
    SetWindowTextW(edit, text.as_ptr());
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
        if !app_ptr.is_null() {
            refresh_diagnostic_window(hwnd, &*(app_ptr as *mut App));
        }
        return if edit.is_null() { 1 } else { 0 };
    }
    if !app.is_null() {
        if message == WM_SIZE {
            let width = (l_param as u32 & 0xffff) as i32;
            let height = ((l_param as u32 >> 16) & 0xffff) as i32;
            let edit = GetWindow(hwnd, GW_CHILD);
            MoveWindow(edit, 0, 0, width, height, 1);
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
        app.tray_status = TrayStatus::Error;
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
                    app.startup_status =
                        "startup registration unavailable and config persistence failed".into();
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
                    app.startup_status = format!(
                        "auto-start enabled in config, current-user registration unavailable: {error}"
                    );
                }
                app.tray_status = TrayStatus::Error;
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
        app.startup_status = "startup registration error".into();
        app.tray_status = TrayStatus::Error;
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
        app.startup_status = if rollback.is_ok() {
            "startup config persistence failed, registry change rolled back".into()
        } else {
            "startup config persistence failed, repair required".into()
        };
        app.tray_status = if rollback.is_ok() {
            TrayStatus::Error
        } else {
            TrayStatus::Unverified
        };
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
    app.startup_status = if let Some(target) = target {
        format!(
            "boot startup registered for current-user {}",
            target.description()
        )
    } else {
        "boot startup registration disabled by config".into()
    };
    app.publish();
}

fn quit_decision(app: &App) -> QuitDecision {
    quit_decision_for(app.tray_status, app.controller.ownership())
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
    let title = wide("True Tick quit warning");
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
    let task_dialog_hresult = TaskDialogIndirect(
        &config,
        &mut selected,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
    );
    QuitWarningResult {
        decision: if task_dialog_hresult < 0 {
            QuitDialogDecision::Cancel
        } else {
            task_dialog_decision(selected)
        },
        task_dialog_hresult,
        message_box_result: None,
        dialog_shown: task_dialog_hresult >= 0,
    }
}

fn task_dialog_decision(button_id: i32) -> QuitDialogDecision {
    if button_id == TASKDIALOG_BUTTON_STOP_AND_QUIT {
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
    fn new(hwnd: *mut c_void, status: TrayStatus, timing: TimingValues) -> Self {
        let mut value = Self {
            cb_size: size_of::<Self>() as u32,
            h_wnd: hwnd,
            u_id: 1,
            u_flags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            u_callback_message: WM_TRAY,
            h_icon: unsafe { status_icon(status, dpi_for_window(hwnd)) },
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
        value
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
    fn MessageBoxW(hwnd: *mut c_void, text: *const u16, title: *const u16, flags: u32) -> i32;
    fn CreatePopupMenu() -> *mut c_void;
    fn AppendMenuW(menu: *mut c_void, flags: u32, id: usize, text: *const u16) -> i32;
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn failed_task_dialog_is_not_shown_and_fails_closed() {
        let result = QuitWarningResult {
            decision: QuitDialogDecision::Cancel,
            task_dialog_hresult: -2,
            message_box_result: None,
            dialog_shown: false,
        };
        assert_eq!(result.decision, QuitDialogDecision::Cancel);
        assert!(result.task_dialog_hresult < 0);
        assert!(!result.dialog_shown);
    }

    #[test]
    fn diagnostic_window_contract_is_normal_and_taskbar_visible() {
        assert_eq!(DIAGNOSTIC_WINDOW_TITLE, "True Tick Status and Diagnostics");
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
