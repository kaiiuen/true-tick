//! Environment snapshot collection for the anomaly recorder.
//!
//! Fills `EnvironmentSnapshot` fields with values that are cheap to collect.
//! Fields that cannot be collected on the current platform stay at zero.

use std::sync::OnceLock;
use std::time::Instant;

use tick_diagnostics::recorder::EnvironmentSnapshot;

/// Monotonic process epoch used for `uptime_ms`.
static PROCESS_EPOCH: OnceLock<Instant> = OnceLock::new();

/// Milliseconds elapsed since the process epoch was first observed.
fn process_uptime_ms() -> u64 {
    let epoch = PROCESS_EPOCH.get_or_init(Instant::now);
    u64::try_from(epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Maps `std::env::consts::ARCH` to a small integer for the snapshot format.
fn cpu_arch_code() -> u32 {
    match std::env::consts::ARCH {
        "x86" => 0x014c,
        "x86_64" => 0x8664,
        "arm" => 0x01c0,
        "aarch64" => 0xaa64,
        _ => 0,
    }
}

#[cfg(windows)]
#[repr(C)]
struct OsVersionInfoW {
    size: u32,
    major: u32,
    minor: u32,
    build: u32,
    platform_id: u32,
    service_pack: [u16; 128],
}

#[cfg(windows)]
#[link(name = "ntdll")]
extern "system" {
    fn RtlGetVersion(info: *mut OsVersionInfoW) -> i32;
}

/// Reads the kernel reported OS version. Returns zeros on failure.
#[cfg(windows)]
fn os_version() -> (u32, u32, u32) {
    let mut info = OsVersionInfoW {
        size: std::mem::size_of::<OsVersionInfoW>() as u32,
        major: 0,
        minor: 0,
        build: 0,
        platform_id: 0,
        service_pack: [0; 128],
    };
    let status = unsafe { RtlGetVersion(&mut info) };
    if status != 0 {
        return (0, 0, 0);
    }
    (info.major, info.minor, info.build)
}

#[cfg(not(windows))]
fn os_version() -> (u32, u32, u32) {
    (0, 0, 0)
}

#[cfg(windows)]
#[repr(C)]
struct SystemPowerStatus {
    ac_line_status: u8,
    battery_flag: u8,
    battery_life_percent: u8,
    system_status_flag: u8,
    battery_life_time: u32,
    battery_full_life_time: u32,
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn GetSystemPowerStatus(status: *mut SystemPowerStatus) -> i32;
}

/// Reads the AC line status and battery percent. Returns zeros on failure or
/// when the fields are reported as unknown by the platform.
#[cfg(windows)]
fn power_fields() -> (u32, u32) {
    let mut status = SystemPowerStatus {
        ac_line_status: 255,
        battery_flag: 255,
        battery_life_percent: 255,
        system_status_flag: 0,
        battery_life_time: u32::MAX,
        battery_full_life_time: u32::MAX,
    };
    let ok = unsafe { GetSystemPowerStatus(&mut status) } != 0;
    if !ok {
        return (0, 0);
    }
    let ac_line = match status.ac_line_status {
        0 | 1 => status.ac_line_status as u32,
        _ => 0,
    };
    let battery = match status.battery_life_percent {
        0..=100 => status.battery_life_percent as u32,
        _ => 0,
    };
    (ac_line, battery)
}

#[cfg(not(windows))]
fn power_fields() -> (u32, u32) {
    (0, 0)
}

/// Collects an `EnvironmentSnapshot` with values that are cheap to read.
///
/// `os_ubr` requires a registry read and is left at zero. Power fields fall
/// back to zero when the platform query fails.
pub fn collect_environment_snapshot() -> EnvironmentSnapshot {
    let (os_major, os_minor, os_build) = os_version();
    let (ac_line_status, battery_percent) = power_fields();
    EnvironmentSnapshot {
        os_major,
        os_minor,
        os_build,
        os_ubr: 0,
        cpu_arch: cpu_arch_code(),
        processor_count: std::thread::available_parallelism()
            .map(|count| count.get() as u32)
            .unwrap_or(0),
        ac_line_status,
        battery_percent,
        uptime_ms: process_uptime_ms(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_snapshot_reports_processor_count_and_arch() {
        let snapshot = collect_environment_snapshot();
        assert!(snapshot.processor_count >= 1);
        assert!(snapshot.cpu_arch != 0 || std::env::consts::ARCH.is_empty());
    }

    #[test]
    fn uptime_is_monotonic() {
        let first = process_uptime_ms();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let second = process_uptime_ms();
        assert!(second >= first);
    }
}
