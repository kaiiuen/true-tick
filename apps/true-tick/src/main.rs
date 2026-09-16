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
fn main() {
    tray::run();
}

#[cfg(not(windows))]
fn main() {
    println!("True™ Tick is a Windows tray application");
}
