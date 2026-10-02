use std::sync::atomic::Ordering;

use crate::tray::commands::HEARTBEAT_TIMER_ID;
use crate::tray::controller::App;
use crate::tray::icon::show_balloon;
use crate::tray::native::KillTimer;
use crate::tray_surface::TrayStatus;

pub(crate) fn kill_heartbeat_timer(app: &mut App) {
    if let Some(hwnd) = app.tray_icon.as_ref().map(|icon| icon.h_wnd) {
        unsafe {
            // SAFETY: hwnd is the live tray window and the timer id was created by this module
            let _ = KillTimer(hwnd, HEARTBEAT_TIMER_ID);
        }
    }
    app.last_heartbeat_fire = None;
}

/// Recurring UI thread timer that proves message pump liveness so the
/// watchdog measures a wedged UI thread rather than an idle one.
///
/// Each fire also feeds the responsiveness tracker with the lateness of
/// this WM_TIMER delivery so a sustained slow pump surfaces as a
/// degraded tray status instead of a silent stall.
pub(crate) fn handle_heartbeat_timer(app: &mut App) {
    app.heartbeat.kick();
    let now = std::time::Instant::now();
    let elapsed_ms = app
        .last_heartbeat_fire
        .map(|prev| {
            u64::try_from(now.saturating_duration_since(prev).as_millis()).unwrap_or(u64::MAX)
        })
        .unwrap_or(0);
    let lateness_ms = elapsed_ms.saturating_sub(tick_watchdog::HEARTBEAT_INTERVAL_MS);
    let sample = if app.watchdog_stall_events.swap(0, Ordering::AcqRel) > 0 {
        lateness_ms.max(tick_watchdog::STALL_THRESHOLD_MS)
    } else {
        lateness_ms
    };
    app.last_heartbeat_fire = Some(now);
    app.responsiveness.observe(sample);
    let degraded_now = app.responsiveness.state() == tick_policy::ResponsivenessState::Degraded;
    if degraded_now && !app.responsiveness_degraded {
        app.responsiveness_degraded = true;
        app.record(
            "responsiveness.degraded",
            format!(
                "baseline_ms={} worst_ms={}",
                app.responsiveness.baseline_ms(),
                app.responsiveness.worst_ms()
            ),
        );
        if !app.responsiveness_notified {
            app.responsiveness_notified = true;
            if let Some(icon) = app.tray_icon.as_mut() {
                let _ = show_balloon(
                    icon,
                    "True Tick",
                    "System responsiveness is slow. Timing was not changed.",
                    true,
                );
            }
        }
        app.publish();
    } else if !degraded_now && app.responsiveness_degraded {
        app.responsiveness_degraded = false;
        app.responsiveness_notified = false;
        app.record(
            "responsiveness.recovered",
            format!("baseline_ms={}", app.responsiveness.baseline_ms()),
        );
        app.publish();
    }
}

/// Maps the lifecycle status to the published status so a slow but
/// healthy pump shows degraded without masking a real block or error.
pub(crate) fn published_status(base: TrayStatus, responsiveness_degraded: bool) -> TrayStatus {
    if responsiveness_degraded
        && matches!(
            base,
            TrayStatus::Running | TrayStatus::Stopped | TrayStatus::Paused
        )
    {
        TrayStatus::Degraded
    } else {
        base
    }
}
