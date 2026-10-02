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
#[link(name = "kernel32")]
extern "system" {
    fn SetConsoleCtrlHandler(
        handler_routine: Option<unsafe extern "system" fn(u32) -> i32>,
        add: i32,
    ) -> i32;
}

/// Runs on a separate thread created by the OS, so it must never
/// dereference any pointer into `App`. CTRL_CLOSE_EVENT performs emergency
/// cleanup and returns 1 so the process controls its own exit during the
/// close grace period. Logoff and shutdown events return 0 so the handler
/// returns promptly and the session end is not stalled by file I/O on the
/// handler thread. Console interruption events return 0 so default
/// processing applies.
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

/// Writes the panic anomaly snapshot. Runs inside the panic hook, so the
/// executable path is resolved lazily and every fallible step is guarded.
/// The snapshot path itself may allocate and touch the filesystem, which is
/// acceptable here because the recorder writes to an already known state
/// directory and all I/O errors are discarded by the caller.
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
        // Restore the tracked timer resolution before invoking the default
        // hook. The call uses the tracked interval, so it never issues an
        // invalid zero resolution request and it is safe on a panicking
        // thread because it only swaps an atomic and calls stateless FFI.
        let _ = emergency::restore_timer_resolution();
        let payload = panic_info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| {
                panic_info
                    .payload()
                    .downcast_ref::<String>()
                    .map(String::as_str)
            })
            .unwrap_or("");
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
        // The snapshot writer allocates and performs file I/O, so it is
        // wrapped in catch_unwind. A re-panic inside the hook would abort
        // the process and lose the default hook output, so any nested
        // panic is caught here and the default hook still runs.
        let detail_ref = detail.as_str();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            write_panic_snapshot(detail_ref);
        }));
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
