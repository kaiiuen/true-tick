#![cfg_attr(windows, windows_subsystem = "windows")]

mod config;
mod portable;

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
