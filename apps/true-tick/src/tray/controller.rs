use super::*;

use crate::config;
use crate::pause::{DurationCoordinator, PresetsManager};
use crate::shutdown::{
    message_loop_exit, shutdown_disposition, MessageLoopExit, ShutdownDisposition, ShutdownGate,
};
use crate::tray_surface::{
    menu_command_dispatch_allowed, scheduled_display_key, tooltip_at, tray_notification_opens_menu,
    HandoffTracker, TimingValues, TrayStatus,
};
use crate::ui::presets_window;
use std::ffi::c_void;
use std::mem::size_of;
use std::path::PathBuf;
use std::sync::Arc;
use tick_core::DesiredIntentQueue;
use tick_diagnostics::{
    DiagnosticOutcome, DiagnosticPhase, DiagnosticRecord, DiagnosticSource, DiagnosticStore,
    EventCategory, NativeOutcome, OperationContext, RowSelection, DEFAULT_MAX_EVENTS,
};
use tick_observation_windows::{ObservationSource, WindowsObservation};
use tick_ownership::{OwnershipState, TimerController, TimingSnapshot};
use tick_platform_windows::WindowsTimerPlatform;
use tick_startup_windows::{
    startup_operation, StartupOperation, StartupRegistration, WindowsUserStartup,
};

const ICC_LISTVIEW_CLASSES: u32 = 0x0000_0001;
// The standard bar-class group includes the native tooltip control.
const ICC_BAR_CLASSES: u32 = 0x0000_0004;
const REQUIRED_COMMON_CONTROL_CLASSES: u32 = ICC_LISTVIEW_CLASSES | ICC_BAR_CLASSES;

const ERROR_CLASS_ALREADY_EXISTS: u32 = 1410;
const ERROR_ALREADY_EXISTS: u32 = 183;
const SINGLE_INSTANCE_MUTEX_NAME: &str = "Local\\TrueTickSingleInstance";
const TASKBAR_CREATED_MESSAGE_NAME: &str = "TaskbarCreated";
const TRAY_WINDOW_CLASS_NAME: &str = "TrueTickTrayClass";
const ANOTHER_INSTANCE_MESSAGE_NAME: &str = "TrueTickAnotherInstance";
const ANOTHER_INSTANCE_INFO_TITLE: &str = "True Tick";
const ANOTHER_INSTANCE_INFO_TEXT: &str = "True Tick is already running.";
const MB_ICONINFORMATION: u32 = 0x0000_0040;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AnotherInstanceDecision {
    Notify,
    Record,
    Ignore,
}

fn another_instance_decision(message: u32, registered_message: u32) -> AnotherInstanceDecision {
    if registered_message != 0 && message == registered_message {
        AnotherInstanceDecision::Record
    } else {
        AnotherInstanceDecision::Ignore
    }
}

fn another_instance_notify_decision(hwnd: *mut c_void) -> AnotherInstanceDecision {
    if hwnd.is_null() {
        AnotherInstanceDecision::Ignore
    } else {
        AnotherInstanceDecision::Notify
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

fn class_registration_result(atom: u16, raw_error: u32) -> NativeResult {
    if atom != 0 || raw_error == ERROR_CLASS_ALREADY_EXISTS {
        NativeResult::Succeeded
    } else {
        NativeResult::Failed { raw_error }
    }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PublicationKey {
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
    pub(crate) tray_status: TrayStatus,
    pub(crate) config_path: PathBuf,
    pub(crate) executable: PathBuf,
    pub(crate) startup_status: String,
    pub(crate) operating_slot_label: String,
    pub(crate) portable_root_label: String,
    pub(crate) taskbar_created_message: u32,
    pub(crate) another_instance_message: u32,
    pub(crate) tray_icon: Option<NotifyIconData>,
    pub(crate) timing_snapshot: TimingSnapshot,
    pub(crate) timing_snapshot_valid: bool,
    pub(crate) invalid_interval: bool,
    pub(crate) external_timing: bool,
    pub(crate) desired_intent: DesiredIntentQueue,
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
    pub(crate) menu_active: bool,
    pub(crate) popup_menus: Option<PopupMenuHandles>,
    pub(crate) popup_refresh_timer_active: bool,
    pub(crate) schedule_display_timer_active: bool,
    pub(crate) handoff: Option<HandoffTracker>,
    pub(crate) menu_help: Option<*mut c_void>,
    pub(crate) menu_help_text: Vec<u16>,
    pub(crate) pause: DurationCoordinator,
    pub(crate) presets_manager: PresetsManager,
    pub(crate) presets_window: Option<*mut c_void>,
    pub(crate) presets_listbox: Option<*mut c_void>,
    pub(crate) presets_input: Option<*mut c_void>,
    pub(crate) duration_timer_id: Option<usize>,
    pub(crate) duration_timer_generation: Option<u64>,
    pub(crate) scheduled_operation: Option<OperationContext>,
    pub(crate) running_since: Option<std::time::Instant>,
    pub(crate) shutdown_gate: ShutdownGate,
    pub(crate) operation: Option<OperationContext>,
    pub(crate) operation_source: DiagnosticSource,
    pub(crate) handoff_operation: Option<OperationContext>,
    pub(crate) last_publication: Option<PublicationKey>,
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
                let another_instance_name = wide(ANOTHER_INSTANCE_MESSAGE_NAME);
                let another_instance_message =
                    RegisterWindowMessageW(another_instance_name.as_ptr());
                let another_instance_error = if another_instance_message == 0 {
                    GetLastError()
                } else {
                    0
                };
                diagnostics.record(
                    "native.RegisterWindowMessageW.AnotherInstance",
                    format!(
                        "message_id={another_instance_message} raw_status={another_instance_error}"
                    ),
                );
                let tray_class_name = wide(TRAY_WINDOW_CLASS_NAME);
                let running_window = FindWindowW(tray_class_name.as_ptr(), std::ptr::null());
                let find_error = if running_window.is_null() {
                    GetLastError()
                } else {
                    0
                };
                diagnostics.record(
                    "native.FindWindowW.tray",
                    format!(
                        "found={} raw_status={find_error}",
                        !running_window.is_null()
                    ),
                );
                if matches!(
                    another_instance_notify_decision(running_window),
                    AnotherInstanceDecision::Notify
                ) && another_instance_message != 0
                {
                    let posted = PostMessageW(running_window, another_instance_message, 0, 0);
                    let post_error = if posted == 0 { GetLastError() } else { 0 };
                    diagnostics.record(
                        "native.PostMessageW.another_instance",
                        format!("result={} raw_status={post_error}", posted != 0),
                    );
                }
                let info_title = wide(ANOTHER_INSTANCE_INFO_TITLE);
                let info_text = wide(ANOTHER_INSTANCE_INFO_TEXT);
                MessageBoxW(
                    std::ptr::null_mut(),
                    info_text.as_ptr(),
                    info_title.as_ptr(),
                    MB_OK | MB_ICONINFORMATION | MB_SETFOREGROUND,
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
        let previous_marker = crate::session::read_session_marker(&log_directory);
        match crate::session::classify_previous_session(previous_marker.as_deref()) {
            crate::session::PreviousSession::UncleanShutdown => {
                diagnostics.record(
                    "lifecycle.unclean_shutdown",
                    format!(
                        "previous_marker={}",
                        previous_marker.as_deref().map(str::trim).unwrap_or("")
                    ),
                );
                show_unclean_shutdown_warning();
            }
            crate::session::PreviousSession::CleanExit => {
                diagnostics.record("lifecycle.clean_start", "previous_marker=clean");
            }
            crate::session::PreviousSession::FirstRun => {
                diagnostics.record("lifecycle.clean_start", "previous_marker=absent");
            }
        }
        if let Err(error) = crate::session::write_session_marker(
            &log_directory,
            crate::session::SESSION_STATE_RUNNING,
        ) {
            diagnostics.record(
                "lifecycle.session_marker.error",
                format!("result=running_write_failed error={error}"),
            );
        }
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
        let operating_slot_label = if is_slot_layout {
            executable
                .parent()
                .and_then(|directory| directory.file_name())
                .and_then(|name| name.to_str())
                .unwrap_or("Unknown")
                .to_owned()
        } else if cfg!(debug_assertions)
            && tick_startup_windows::is_development_executable(&executable)
        {
            "Development".to_owned()
        } else {
            "Standalone".to_owned()
        };
        let portable_root_label = portable_root_str.clone();
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
        let another_instance_name = wide(ANOTHER_INSTANCE_MESSAGE_NAME);
        let another_instance_message = RegisterWindowMessageW(another_instance_name.as_ptr());
        let another_instance_error = if another_instance_message == 0 {
            GetLastError()
        } else {
            0
        };
        diagnostics.record(
            "native.RegisterWindowMessageW.AnotherInstance",
            format!("message_id={another_instance_message} raw_status={another_instance_error}"),
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
            operating_slot_label,
            portable_root_label,
            taskbar_created_message,
            another_instance_message,
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
        let class_name = wide(TRAY_WINDOW_CLASS_NAME);
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
        mark_session_clean(app);
        crate::logging::flush_diagnostic_events_to_disk(
            &app.diagnostics,
            &mut app.last_persisted_event_sequence,
            &app.log_directory,
        );
        drop(Box::from_raw(app_ptr));
    }
}

fn mark_session_clean(app: &App) {
    if let Err(error) = crate::session::write_session_marker(
        &app.log_directory,
        crate::session::SESSION_STATE_CLEAN,
    ) {
        app.diagnostics.record(
            "lifecycle.session_marker.error",
            format!("result=clean_write_failed error={error}"),
        );
    }
}

unsafe fn abort_startup(app_ptr: *mut App, message: &str) {
    let app = Box::from_raw(app_ptr);
    app.diagnostics.record("lifecycle.startup.abort", message);
    mark_session_clean(&app);
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
    mark_session_clean(&app);
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

impl App {
    pub(crate) fn sync_timing_snapshot(&mut self) {
        self.timing_snapshot = self.controller.snapshot();
    }

    pub(crate) fn timing_snapshot_details(&self) -> String {
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

    pub(crate) fn begin_child_operation(
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

    pub(crate) fn cleanup_normal_shutdown(&mut self) -> Result<(), String> {
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

    pub(crate) fn publish(&mut self) {
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
            msg if matches!(
                another_instance_decision(msg, app.another_instance_message),
                AnotherInstanceDecision::Record
            ) =>
            {
                app.record("lifecycle.another_instance_attempt", "source=second_launch");
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

#[link(name = "user32")]
extern "system" {
    fn RegisterWindowMessageW(string: *const u16) -> u32;
    fn RegisterClassW(class: *const WndClass) -> u16;
    fn GetMessageW(message: *mut Message, hwnd: *mut c_void, min: u32, max: u32) -> i32;
    fn TranslateMessage(message: *const Message) -> i32;
    fn DispatchMessageW(message: *const Message) -> isize;
    fn FindWindowW(class: *const u16, title: *const u16) -> *mut c_void;
    fn PostMessageW(hwnd: *mut c_void, message: u32, w: usize, l: isize) -> i32;
}

#[link(name = "comctl32")]
extern "system" {
    fn InitCommonControlsEx(init: *const InitCommonControlsEx) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleFileNameW(module: *mut c_void, filename: *mut u16, size: u32) -> u32;
    fn CreateMutexW(
        security_attributes: *mut c_void,
        initial_owner: i32,
        name: *const u16,
    ) -> *mut c_void;
    fn CloseHandle(handle: *mut c_void) -> i32;
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
    fn common_controls_initialization_uses_list_view_and_tooltip_classes() {
        let init = common_controls_initialization_contract();
        assert_eq!(init.size as usize, size_of::<InitCommonControlsEx>());
        assert_eq!(init.classes, ICC_LISTVIEW_CLASSES | ICC_BAR_CLASSES);
        assert_ne!(init.classes & ICC_LISTVIEW_CLASSES, 0);
        assert_ne!(init.classes & ICC_BAR_CLASSES, 0);
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
    fn another_instance_decision_matches_registered_message_only_when_valid() {
        assert_eq!(
            another_instance_decision(0xC000, 0xC000),
            AnotherInstanceDecision::Record
        );
        assert_eq!(
            another_instance_decision(0xC000, 0xC001),
            AnotherInstanceDecision::Ignore
        );
        assert_eq!(
            another_instance_decision(0, 0),
            AnotherInstanceDecision::Ignore
        );
        assert_eq!(
            another_instance_decision(0x0001, 0),
            AnotherInstanceDecision::Ignore
        );
    }

    #[test]
    fn another_instance_notify_decision_maps_found_window_and_null() {
        assert_eq!(
            another_instance_notify_decision(std::ptr::null_mut()),
            AnotherInstanceDecision::Ignore
        );
        let found = std::ptr::dangling_mut::<c_void>();
        assert_eq!(
            another_instance_notify_decision(found),
            AnotherInstanceDecision::Notify
        );
    }

    #[test]
    fn another_instance_registered_message_name_is_exact() {
        assert_eq!(ANOTHER_INSTANCE_MESSAGE_NAME, "TrueTickAnotherInstance");
        let wide_name = wide(ANOTHER_INSTANCE_MESSAGE_NAME);
        assert_eq!(wide_name.last(), Some(&0));
        assert_eq!(wide_name.len(), "TrueTickAnotherInstance".len() + 1);
    }
}
