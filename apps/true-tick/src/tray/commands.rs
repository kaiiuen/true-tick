use std::ffi::c_void;
use std::path::Path;

use tick_diagnostics::{truncate_utf8, DiagnosticOutcome};
use tick_startup_windows::{StartupRegistration, WindowsUserStartup};

use crate::config;
use crate::tray::controller::App;

pub const WM_APP: u32 = 0x8000;
pub const WM_TRAY: u32 = WM_APP + 1;
pub(crate) const WM_APP_IPC: u32 = WM_APP + 3;

pub(crate) const WM_CREATE: u32 = 0x0001;
pub(crate) const WM_COMMAND: u32 = 0x0111;
pub(crate) const WM_DESTROY: u32 = 0x0002;
pub const WM_POWERBROADCAST: u32 = 0x0218;
pub(crate) const WM_TIMER: u32 = 0x0113;
pub const WM_MENUSELECT: u32 = 0x011F;
pub const PBT_APMPOWERSTATUSCHANGE: usize = 0x000A;
pub const PBT_APMSUSPEND: usize = 0x0004;
pub const PBT_APMRESUMESUSPEND: usize = 0x0007;
pub const PBT_APMRESUMEAUTOMATIC: usize = 0x0012;

pub(crate) const HANDOFF_TIMER_ID: usize = 0x5449;
pub(crate) const DURATION_TIMER_ID_BASE: usize = 0x6000;
pub(crate) const DURATION_TIMER_ID_MASK: usize = 0x07FF;
pub(crate) const SURFACE_RECOVERY_TIMER_ID: usize = 0x6A00;
pub(crate) const POPUP_REFRESH_TIMER_ID: usize = 0x7000;
pub(crate) const POPUP_REFRESH_INTERVAL_MS: u32 = 500;
pub(crate) const SCHEDULE_DISPLAY_TIMER_ID: usize = 0x7100;
pub(crate) const SCHEDULE_DISPLAY_INTERVAL_MS: u32 = 1_000;
pub(crate) const POWER_DEBOUNCE_TIMER_ID: usize = 0x7200;
pub(crate) const POWER_DEBOUNCE_INTERVAL_MS: u64 = 2_000;
pub(crate) const HEARTBEAT_TIMER_ID: usize = 0x7300;

pub const SURFACE_RECOVERY_ATTEMPTS: u32 = 3;
pub const SURFACE_RECOVERY_SPACING_MS: u64 = 2000;

pub(crate) const ID_START: usize = 1001;
pub(crate) const ID_STOP: usize = 1002;
pub(crate) const ID_QUIT: usize = 1004;
pub(crate) const ID_STARTUP_ON: usize = 1005;
pub(crate) const ID_STARTUP_OFF: usize = 1006;
pub(crate) const ID_AUTOMATIC_ON: usize = 1007;
pub(crate) const ID_AUTOMATIC_OFF: usize = 1008;
pub(crate) const ID_AUTO_RESUME_ON: usize = 1009;
pub(crate) const ID_AUTO_RESUME_OFF: usize = 1010;
pub(crate) const ID_BATTERY_LOCKOUT_ON: usize = crate::tray_surface::BATTERY_LOCKOUT_ON_COMMAND_ID;
pub(crate) const ID_BATTERY_LOCKOUT_OFF: usize =
    crate::tray_surface::BATTERY_LOCKOUT_OFF_COMMAND_ID;
pub const ID_SETTINGS_RESET: usize = 1061;
pub(crate) const ID_RESET: usize = 1060;

pub(crate) const EN_CHANGE: usize = 0x0300;
pub(crate) const BN_CLICKED: usize = 0;
pub(crate) const WS_VSCROLL: u32 = 0x00200000;
pub(crate) const WM_USER: u32 = 0x0400;
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
pub const MAX_STARTUP_STATUS_BYTES: usize = 512;
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
pub const MB_SETFOREGROUND: u32 = 0x0001_0000;
pub(crate) const MB_YESNO: u32 = 0x0000_0004;
pub const MB_DEFBUTTON2: u32 = 0x0000_0100;
pub(crate) const IDYES: i32 = 6;

pub(crate) const WS_POPUP: u32 = 0x8000_0000;
pub(crate) const WS_EX_TOPMOST: u32 = 0x0000_0008;
pub(crate) const TTS_ALWAYSTIP: u32 = 0x0001;
pub(crate) const TTS_NOPREFIX: u32 = 0x0002;
pub(crate) const TTF_IDISHWND: u32 = 0x0001;
pub(crate) const TTF_TRACK: u32 = 0x0020;
pub(crate) const TTF_ABSOLUTE: u32 = 0x0080;
pub(crate) const TTM_TRACKACTIVATE: u32 = WM_USER + 17;
pub(crate) const TTM_TRACKPOSITION: u32 = WM_USER + 18;
pub(crate) const TTM_ADDTOOLW: u32 = WM_USER + 50;
pub(crate) const TTM_UPDATETIPTEXTW: u32 = WM_USER + 57;

pub(crate) const ABM_GETTASKBARPOS: usize = 0x0000_0005;
#[allow(dead_code)]
pub(crate) const ABE_LEFT: u32 = 0;
#[allow(dead_code)]
pub(crate) const ABE_TOP: u32 = 1;
pub(crate) const ABE_RIGHT: u32 = 2;
pub(crate) const ABE_BOTTOM: u32 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StartupRollback {
    SkipDestructiveInverse,
    RestoreRegistration,
}

pub(crate) const fn startup_rollback(enabled: bool) -> StartupRollback {
    if enabled {
        StartupRollback::SkipDestructiveInverse
    } else {
        StartupRollback::RestoreRegistration
    }
}

pub(crate) fn bounded_startup_status(value: impl AsRef<str>) -> String {
    truncate_utf8(value.as_ref(), MAX_STARTUP_STATUS_BYTES)
}

pub(crate) enum StartupTarget {
    PortableLauncher(std::path::PathBuf),
    DevelopmentExecutable(std::path::PathBuf),
}

impl StartupTarget {
    pub(crate) fn description(&self) -> &'static str {
        match self {
            Self::PortableLauncher(_) => "portable Launcher.exe entry point",
            Self::DevelopmentExecutable(_) => "debug true-tick.exe development fallback",
        }
    }
}

pub(crate) fn startup_target(executable: &Path) -> Result<StartupTarget, String> {
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

pub(crate) fn register_startup_target(
    target: &StartupTarget,
) -> Result<(), tick_startup_windows::StartupError> {
    let mut startup = WindowsUserStartup;
    match target {
        StartupTarget::PortableLauncher(path) => startup.register(path),
        StartupTarget::DevelopmentExecutable(path) => startup.register_development(path),
    }
}

pub(crate) fn set_automatic(app: &mut App, enabled: bool) {
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
        super::reconcile(app);
    } else {
        super::manual_stop(app);
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
    app.config = next;
    app.record(
        "toggle.result",
        format!(
            "setting=auto_resume value={} result=applied",
            app.config.auto_resume_on_ac
        ),
    );
    super::apply_power_reconciliation(app);
}

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
        super::apply_power_reconciliation(app);
    } else {
        app.last_block_reason = None;
        super::apply_power_reconciliation(app);
    }
}

pub(crate) fn set_startup(app: &mut App, enabled: bool) {
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

pub(crate) unsafe fn open_github_page(hwnd: *mut c_void, app: &mut App) {
    let operation = super::wide("open");
    let url = super::wide(crate::tray_surface::GITHUB_URL);
    let result = super::ShellExecuteW(
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
            format!(
                "result={} raw_status={}",
                result as isize,
                super::GetLastError()
            ),
            DiagnosticOutcome::Failed,
        );
    } else {
        app.record(
            "native.ShellExecuteW.github.result",
            format!("result=success url={}", crate::tray_surface::GITHUB_URL),
        );
    }
}
