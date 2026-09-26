//! Process-wide emergency cleanup state shared between the tray controller,
//! the console control handler, and the session-end window messages.
//!
//! The console control handler runs on a separate thread created by the OS,
//! so it must never dereference any pointer into `App`. The only shared
//! state is this module's static tracked interval plus the context installed
//! during startup.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};
use tick_diagnostics::DiagnosticStore;

#[link(name = "ntdll")]
extern "system" {
    fn NtSetTimerResolution(
        desired_resolution: u32,
        set_resolution: u8,
        current_resolution: *mut u32,
    ) -> i32;
}

pub(crate) const CTRL_C_EVENT: u32 = 0;
pub(crate) const CTRL_BREAK_EVENT: u32 = 1;
pub(crate) const CTRL_CLOSE_EVENT: u32 = 2;
pub(crate) const CTRL_LOGOFF_EVENT: u32 = 5;
pub(crate) const CTRL_SHUTDOWN_EVENT: u32 = 6;
pub(crate) const WM_QUERYENDSESSION: u32 = 0x0011;
pub(crate) const WM_ENDSESSION: u32 = 0x0016;

const CLEANUP_PENDING: u8 = 0;
const CLEANUP_IN_PROGRESS: u8 = 1;
const CLEANUP_DONE: u8 = 2;

/// Tracked timer resolution request in 100ns units. The controller writes the
/// tracked HNS on every publish while ownership is owned and writes zero
/// otherwise, so the emergency path never needs `App` state.
static TRACKED_INTERVAL_HNS: AtomicU64 = AtomicU64::new(0);

/// Idempotence gate for the emergency cleanup path.
static CLEANUP_STATE: AtomicU8 = AtomicU8::new(CLEANUP_PENDING);

/// Serializes the file writes inside emergency cleanup when both the console
/// handler thread and the window thread could reach the path concurrently.
static CLEANUP_LOCK: Mutex<()> = Mutex::new(());

struct EmergencyContext {
    diagnostics: std::sync::Arc<DiagnosticStore>,
    last_persisted_event_sequence: std::sync::Arc<AtomicU64>,
    log_directory: PathBuf,
    state_directory: PathBuf,
}

/// Safety: the context is installed exactly once during startup before the
/// message loop and console handler run, and it is never cleared afterwards.
/// Raw pointer reads from `App` are forbidden, so only `Arc` clones and owned
/// paths are stored here.
static EMERGENCY_CONTEXT: OnceLock<EmergencyContext> = OnceLock::new();

/// Publishes the currently tracked timer resolution interval for the
/// emergency path. Callers pass zero when ownership is not held.
pub(crate) fn publish_tracked_interval(interval_hns: u64) {
    TRACKED_INTERVAL_HNS.store(interval_hns, Ordering::Release);
}

#[cfg(test)]
pub(crate) fn tracked_interval_hns() -> u64 {
    TRACKED_INTERVAL_HNS.load(Ordering::Acquire)
}

/// Installs the diagnostics and state locations the emergency path needs.
/// Must run once during startup before the message loop begins.
pub(crate) fn install_emergency_context(
    diagnostics: std::sync::Arc<DiagnosticStore>,
    last_persisted_event_sequence: std::sync::Arc<AtomicU64>,
    log_directory: PathBuf,
    state_directory: PathBuf,
) {
    let _ = EMERGENCY_CONTEXT.set(EmergencyContext {
        diagnostics,
        last_persisted_event_sequence,
        log_directory,
        state_directory,
    });
}

/// Restores the timer resolution when a tracked interval is known to be held.
/// Returns true when a restorative call was issued.
fn restore_timer_resolution() -> bool {
    let interval = TRACKED_INTERVAL_HNS.swap(0, Ordering::AcqRel);
    if interval == 0 {
        return false;
    }
    let mut current = 0u32;
    // Safety: `current` is a valid out pointer and the call is process-wide
    // with no other invariants required.
    let _ = unsafe { NtSetTimerResolution(interval as u32, 0, &mut current) };
    true
}

/// Writes the clean session marker so the next launch classifies this
/// session as a clean exit rather than an unclean shutdown.
fn write_clean_session_marker(state_directory: &std::path::Path) {
    let _ = crate::session::write_session_bytes_atomic(
        state_directory,
        crate::session::SESSION_STATE_CLEAN.as_bytes(),
    );
}

/// Emergency cleanup shared by the console control handler and the
/// WM_ENDSESSION path. Idempotent through CLEANUP_STATE and the interval
/// swap so a second caller never issues a duplicate restorative call or a
/// duplicate marker write.
///
/// Steps: restore the tracked timer resolution when held, synchronously
/// flush pending diagnostics to disk, and write the clean session marker.
/// Never touches `App` state, so it is safe on the OS-created handler
/// thread.
pub(crate) fn emergency_cleanup() {
    emergency_cleanup_with(&CLEANUP_STATE);
}

fn emergency_cleanup_with(state: &AtomicU8) {
    match state.compare_exchange(
        CLEANUP_PENDING,
        CLEANUP_IN_PROGRESS,
        Ordering::AcqRel,
        Ordering::Acquire,
    ) {
        Ok(_) => {}
        Err(_) => return,
    }
    let _guard = CLEANUP_LOCK.lock();
    restore_timer_resolution();
    if let Some(context) = EMERGENCY_CONTEXT.get() {
        crate::logging::flush_diagnostic_events_to_disk_sync(
            &context.diagnostics,
            &context.last_persisted_event_sequence,
            &context.log_directory,
        );
        write_clean_session_marker(&context.state_directory);
    }
    state.store(CLEANUP_DONE, Ordering::Release);
}

/// Session-end cleanup used by the window procedure. Runs the emergency
/// path only when the session is actually ending.
pub(crate) fn session_end_cleanup(w_param: usize) {
    if session_is_ending(w_param) {
        emergency_cleanup();
    }
}

#[cfg(test)]
fn session_end_cleanup_with(state: &AtomicU8, w_param: usize) {
    if session_is_ending(w_param) {
        emergency_cleanup_with(state);
    }
}

/// Console control event classification. Shutdown class events map to 1 so
/// the handler runs emergency cleanup and the process controls its own
/// exit. Console interruption events map to 0 so default processing
/// applies.
pub(crate) fn console_control_response(event: u32) -> i32 {
    match event {
        CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT => 1,
        CTRL_C_EVENT | CTRL_BREAK_EVENT => 0,
        _ => 0,
    }
}

/// WM_ENDSESSION decision: the session is ending only when wParam is
/// nonzero. A zero wParam means the session end was cancelled.
pub(crate) const fn session_is_ending(w_param: usize) -> bool {
    w_param != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_handler_returns_one_for_shutdown_events_and_zero_for_control_c_events() {
        for event in [CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT] {
            assert_eq!(console_control_response(event), 1);
        }
        for event in [CTRL_C_EVENT, CTRL_BREAK_EVENT] {
            assert_eq!(console_control_response(event), 0);
        }
    }

    #[test]
    fn emergency_cleanup_is_idempotent() {
        let state = AtomicU8::new(CLEANUP_PENDING);
        publish_tracked_interval(10_000);
        emergency_cleanup_with(&state);
        assert_eq!(tracked_interval_hns(), 0);
        emergency_cleanup_with(&state);
        assert_eq!(tracked_interval_hns(), 0);
        assert_eq!(state.load(Ordering::Acquire), CLEANUP_DONE);
    }

    #[test]
    fn session_end_cleanup_runs_when_wparam_is_nonzero() {
        let state = AtomicU8::new(CLEANUP_PENDING);
        session_end_cleanup_with(&state, 0);
        assert_eq!(state.load(Ordering::Acquire), CLEANUP_PENDING);
        session_end_cleanup_with(&state, 1);
        assert_eq!(state.load(Ordering::Acquire), CLEANUP_DONE);
    }
}
