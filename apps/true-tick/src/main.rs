#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

mod config;
mod portable;
mod shutdown;
mod tray_surface;

#[cfg(windows)]
mod tray;

#[cfg(windows)]
fn main() {
    tray::run();
}

#[cfg(not(windows))]
fn main() {
    println!("True Tick is a Windows tray application");
}
