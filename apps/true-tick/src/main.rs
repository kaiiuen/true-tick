#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

mod config;
mod emergency;
mod environment;
mod logging;
mod pause;
mod portable;
mod session;
mod shutdown;
mod tray_surface;

#[cfg(windows)]
mod tray;

#[cfg(windows)]
mod ui;

#[cfg(windows)]
#[link(name = "ntdll")]
extern "system" {
    fn NtSetTimerResolution(
        desired_resolution: u32,
        set_resolution: u8,
        current_resolution: *mut u32,
    ) -> i32;
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn SetConsoleCtrlHandler(
        handler_routine: Option<unsafe extern "system" fn(u32) -> i32>,
        add: i32,
    ) -> i32;
}

/// Runs on a separate thread created by the OS, so it must never
/// dereference any pointer into `App`. Shutdown class events perform
/// emergency cleanup and return 1 so the process controls its own exit.
/// Console interruption events return 0 so default processing applies.
#[cfg(windows)]
unsafe extern "system" fn console_ctrl_handler(event: u32) -> i32 {
    match emergency::console_control_response(event) {
        1 => {
            emergency::emergency_cleanup();
            1
        }
        _ => 0,
    }
}

#[cfg(windows)]
fn install_console_ctrl_handler() {
    // Safety: the handler only touches process-wide statics and the
    // emergency context installed during startup.
    let _ = unsafe { SetConsoleCtrlHandler(Some(console_ctrl_handler), 1) };
}

#[cfg(windows)]
fn write_panic_snapshot(detail: &str) {
    let Ok(executable) = std::env::current_exe() else {
        return;
    };
    let data_directory = crate::logging::resolve_state_directory(&executable);
    let environment = crate::environment::collect_environment_snapshot();
    let snapshot = tick_diagnostics::recorder::build_snapshot(
        tick_diagnostics::recorder::FailureVector::Panic,
        detail,
        environment,
        &[],
    );
    let _ = tick_diagnostics::recorder::write_snapshot(&snapshot, &data_directory);
}

#[cfg(windows)]
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        // Attempt emergency reset of timer resolution before invoking default hook
        let mut current = 0u32;
        unsafe {
            // Unset resolution: interval parameter is ignored by NtSetTimerResolution when set_resolution is 0
            let _ = NtSetTimerResolution(0, 0, &mut current);
        }
        let payload = panic_info
            .payload()
            .downcast_ref::<&str>()
            .map(|value| (*value).to_owned())
            .or_else(|| panic_info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_default();
        let detail = format!(
            "panic payload={} location={}",
            payload,
            panic_info
                .location()
                .map(|location| format!(
                    "{}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                ))
                .unwrap_or_else(|| "unknown".to_owned())
        );
        write_panic_snapshot(&detail);
        default_hook(panic_info);
    }));
}

#[cfg(windows)]
fn main() {
    install_panic_hook();
    install_console_ctrl_handler();
    tray::run();
}

#[cfg(not(windows))]
fn main() {
    println!("True™ Tick is a Windows tray application");
}
