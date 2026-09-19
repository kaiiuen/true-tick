#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

mod config;
mod logging;
mod pause;
mod portable;
mod session;
mod shutdown;
mod tray_surface;

#[cfg(windows)]
mod win32;

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
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        // Attempt emergency reset of timer resolution before invoking default hook
        let mut current = 0u32;
        unsafe {
            // Unset resolution: interval parameter is ignored by NtSetTimerResolution when set_resolution is 0
            let _ = NtSetTimerResolution(0, 0, &mut current);
        }
        default_hook(panic_info);
    }));
}

#[cfg(windows)]
fn main() {
    install_panic_hook();
    tray::run();
}

#[cfg(not(windows))]
fn main() {
    println!("True™ Tick is a Windows tray application");
}
