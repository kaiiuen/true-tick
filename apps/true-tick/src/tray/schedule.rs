use tick_core::DesiredIntent;
use tick_diagnostics::{DiagnosticOutcome, DiagnosticSource};
use tick_observation_windows::ObservationSource;
use tick_ownership::{OwnershipState, Verification};
use tick_policy::{decide, PolicyInput};

use crate::pause::{
    timer_interval_ms, CoordinatorTimerEvent, DurationAction, DurationPreset, ScheduleRequest,
};
use crate::tray::commands::{
    DURATION_TIMER_ID_BASE, DURATION_TIMER_ID_MASK, SCHEDULE_DISPLAY_INTERVAL_MS,
    SCHEDULE_DISPLAY_TIMER_ID,
};
use crate::tray::controller::App;
use crate::tray::native::{GetLastError, KillTimer, SetTimer};
use crate::tray_surface::TrayStatus;

pub(crate) fn manual_start(app: &mut App) {
    app.record("tray.command", "command=start");
    app.record("lifecycle.start_request", "source=manual");
    super::queue_intent(app, DesiredIntent::Acquire, "manual");
}

pub(crate) fn manual_stop(app: &mut App) {
    app.pending_resume_on_ac = false;
    app.record("tray.command", "command=stop");
    app.record("lifecycle.stop_request", "source=manual");
    super::queue_intent(app, DesiredIntent::Release, "manual");
}

pub(crate) fn schedule_duration_action(
    app: &mut App,
    action: DurationAction,
    duration: DurationPreset,
) {
    let now = std::time::Instant::now();
    let replacing_pause = app.pause.pause_active();
    if let Some(previous) = app.pause.current() {
        app.record(
            "duration.schedule.replacement",
            format!(
                "previous_action={} previous_duration_minutes={} previous_remaining_ms={} result=replacing",
                previous.action.label(),
                previous.duration.minutes(),
                previous.remaining(now).as_millis()
            ),
        );
        app.record(
            "duration.schedule.replacement.timing",
            app.timing_snapshot_details(),
        );
    }
    cancel_duration_timer(app);
    let request = app.pause.schedule(action, duration, now);
    let current = match request {
        ScheduleRequest::Started(current) => current,
        ScheduleRequest::Replaced { current, .. } => current,
    };
    app.scheduled_operation = app.operation;
    app.record(
        "duration.schedule",
        format!(
            "action={} duration_seconds={} generation={} deadline_monotonic_ms={} remaining_ms={}",
            action.label(),
            duration.seconds(),
            current.generation,
            duration.duration().as_millis(),
            current.remaining(now).as_millis()
        ),
    );
    app.record("duration.schedule.timing", app.timing_snapshot_details());
    app.record(
        "duration.schedule.policy",
        "policy_result=deferred safety_recheck=deadline",
    );
    if !arm_duration_timer(app) {
        let _ = app.pause.cancel();
        app.scheduled_operation = None;
        app.record_with_outcome(
            "duration.schedule.result",
            "outcome=failed reason=coordinator_timer_unavailable",
            DiagnosticOutcome::Failed,
        );
        app.tray_status = TrayStatus::Error;
        app.publish();
        return;
    }
    begin_schedule_display_timer(app);
    app.record(
        "duration.schedule.result",
        format!(
            "outcome=scheduled action={} duration_seconds={} generation={} remaining_ms={}",
            action.label(),
            duration.seconds(),
            current.generation,
            current.remaining(std::time::Instant::now()).as_millis()
        ),
    );
    if replacing_pause && action != DurationAction::Pause {
        super::show_ownership_status(app, TrayStatus::Stopped);
    }
    if action == DurationAction::Pause {
        app.tray_status = TrayStatus::Pausing;
        app.publish();
        super::queue_intent(app, DesiredIntent::Release, "duration pause");
    } else {
        app.publish();
    }
    if app.menu_active {
        unsafe {
            // SAFETY: runs on the UI thread with the app exclusively borrowed for the menu rebuild
            super::refresh_popup_menu(app)
        };
    }
}

pub(crate) fn schedule_preset_action(app: &mut App, action: DurationAction, preset_index: usize) {
    let Some(preset) = app.presets_manager.presets().get(preset_index).copied() else {
        app.record(
            "duration.schedule.preset",
            format!(
                "result=missing action={} index={preset_index}",
                action.label()
            ),
        );
        return;
    };
    schedule_duration_action(app, action, preset);
}

pub(crate) fn cancel_scheduled_action(app: &mut App) {
    let now = std::time::Instant::now();
    let Some(previous) = app.pause.current() else {
        app.record(
            "duration.cancel",
            "outcome=noop reason=no_scheduled_action remaining_ms=0",
        );
        if app.menu_active {
            unsafe {
                // SAFETY: runs on the UI thread with the app exclusively borrowed for the menu rebuild
                super::refresh_popup_menu(app)
            };
        }
        return;
    };
    app.record(
        "duration.cancel.request",
        format!(
            "action={} duration_minutes={} generation={} remaining_ms={}",
            previous.action.label(),
            previous.duration.minutes(),
            previous.generation,
            previous.remaining(now).as_millis()
        ),
    );
    app.record("duration.cancel.timing", app.timing_snapshot_details());
    cancel_duration_timer(app);
    kill_schedule_display_timer(app);
    let cancelled = app.pause.cancel().expect("scheduled action was checked");
    app.scheduled_operation = None;
    app.record_with_outcome(
        "duration.cancel",
        format!(
            "outcome=cancelled action={} duration_minutes={} generation={} remaining_ms=0",
            cancelled.action.label(),
            cancelled.duration.minutes(),
            cancelled.generation
        ),
        DiagnosticOutcome::Cancelled,
    );
    refresh_power_for_duration(app, "cancel");
    super::apply_power_reconciliation(app);
    if app.menu_active {
        unsafe {
            // SAFETY: runs on the UI thread with the app exclusively borrowed for the menu rebuild
            super::refresh_popup_menu(app)
        };
    }
}

pub(crate) fn duration_timer_id(generation: u64) -> usize {
    DURATION_TIMER_ID_BASE + (generation as usize & DURATION_TIMER_ID_MASK)
}

pub(crate) fn cancel_duration_timer(app: &mut App) {
    let Some(timer_id) = app.duration_timer_id.take() else {
        app.duration_timer_generation = None;
        return;
    };
    app.duration_timer_generation = None;
    if let Some(hwnd) = app.tray_icon.as_ref().map(|icon| icon.h_wnd) {
        if unsafe {
            // SAFETY: hwnd is the live tray window and the timer id was created by this module
            KillTimer(hwnd, timer_id)
        } == 0
        {
            app.record_with_outcome(
                "native.KillTimer.duration.error",
                format!("timer_id={timer_id} raw_status={}", unsafe {
                    // SAFETY: called immediately on the same thread after a failed Win32 call so the thread-local last-error value belongs to this call
                    GetLastError()
                }),
                DiagnosticOutcome::Failed,
            );
        }
    }
}

pub(crate) fn begin_schedule_display_timer(app: &mut App) {
    if app.schedule_display_timer_active {
        return;
    }
    let Some(hwnd) = app.tray_icon.as_ref().map(|icon| icon.h_wnd) else {
        app.record(
            "duration.display_timer",
            "outcome=unavailable reason=tray_window_missing",
        );
        return;
    };
    let result = unsafe {
        // SAFETY: hwnd is the live tray window and the callback is null so WM_TIMER posts to our queue
        SetTimer(
            hwnd,
            SCHEDULE_DISPLAY_TIMER_ID,
            SCHEDULE_DISPLAY_INTERVAL_MS,
            std::ptr::null_mut(),
        )
    };
    if result == 0 {
        app.record_with_outcome(
            "native.SetTimer.duration_display.error",
            format!("raw_status={}", unsafe {
                // SAFETY: called immediately on the same thread after a failed Win32 call so the thread-local last-error value belongs to this call
                GetLastError()
            }),
            DiagnosticOutcome::Failed,
        );
    } else {
        app.schedule_display_timer_active = true;
    }
}

pub(crate) fn kill_schedule_display_timer(app: &mut App) {
    if !app.schedule_display_timer_active {
        return;
    }
    app.schedule_display_timer_active = false;
    if let Some(hwnd) = app.tray_icon.as_ref().map(|icon| icon.h_wnd) {
        unsafe {
            // SAFETY: hwnd is the live tray window and the timer id was created by this module
            let _ = KillTimer(hwnd, SCHEDULE_DISPLAY_TIMER_ID);
        }
    }
}

pub(crate) fn handle_schedule_display_timer(app: &mut App) {
    if app.pause.active() {
        app.publish();
    } else {
        kill_schedule_display_timer(app);
    }
}

pub(crate) fn arm_duration_timer(app: &mut App) -> bool {
    let Some(deadline) = app.pause.deadline() else {
        return false;
    };
    let Some(hwnd) = app.tray_icon.as_ref().map(|icon| icon.h_wnd) else {
        app.record(
            "duration.timer",
            "outcome=unavailable reason=tray_window_missing",
        );
        return false;
    };
    let generation = app.pause.generation();
    let timer_id = duration_timer_id(generation);
    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
    let result = unsafe {
        // SAFETY: hwnd is the live tray window and the callback is null so WM_TIMER posts to our queue
        SetTimer(
            hwnd,
            timer_id,
            timer_interval_ms(remaining),
            std::ptr::null_mut(),
        )
    };
    if result == 0 {
        app.record_with_outcome(
            "native.SetTimer.duration.error",
            format!("generation={generation} raw_status={}", unsafe {
                // SAFETY: called immediately on the same thread after a failed Win32 call so the thread-local last-error value belongs to this call
                GetLastError()
            }),
            DiagnosticOutcome::Failed,
        );
        let _ = app.pause.cancel();
        kill_schedule_display_timer(app);
        app.duration_timer_id = None;
        app.duration_timer_generation = None;
        app.tray_status = TrayStatus::Error;
        app.publish();
        return false;
    }
    app.duration_timer_id = Some(timer_id);
    app.duration_timer_generation = Some(generation);
    app.record(
        "duration.timer.armed",
        format!(
            "generation={generation} remaining_ms={} interval_ms={}",
            remaining.as_millis(),
            timer_interval_ms(remaining)
        ),
    );
    true
}

pub(crate) fn execute_scheduled_action(app: &mut App, action: crate::pause::ScheduledAction) {
    if let Some(parent) = app.scheduled_operation.take() {
        app.begin_child_operation(parent, DiagnosticSource::Timer);
    } else {
        app.begin_operation(DiagnosticSource::Timer);
    }
    app.record(
        "duration.deadline",
        format!(
            "action={} duration_minutes={} generation={} remaining_ms=0",
            action.action.label(),
            action.duration.minutes(),
            action.generation
        ),
    );
    app.record("duration.deadline.timing", app.timing_snapshot_details());
    match action.action {
        DurationAction::Start => {
            refresh_power_for_duration(app, "start_deadline");
            let power = app.observation.power().state;
            let decision = decide(PolicyInput {
                enabled: true,
                eligible_profile: true,
                power,
                battery_lockout_enabled: app.config.battery_lockout,
            });
            app.record(
                "duration.start.policy",
                format!(
                    "power={power:?} result={:?} reason={:?} selected_duration_minutes={} deadline_remaining_ms=0",
                    decision.status,
                    decision.reason,
                    action.duration.minutes()
                ),
            );
            app.record(
                "duration.start.policy.timing",
                app.timing_snapshot_details(),
            );
            if decision.status == tick_core::Status::Requested {
                if app.controller.ownership() == OwnershipState::Released {
                    app.record(
                        "duration.start.result",
                        "outcome=acquire_intent_queued policy_result=allowed",
                    );
                    super::queue_intent(app, DesiredIntent::Acquire, "duration start deadline");
                } else {
                    app.record(
                        "duration.start.result",
                        format!(
                            "outcome=noop reason=ownership_{:?} policy_result=allowed",
                            app.controller.ownership()
                        ),
                    );
                    super::show_ownership_status(app, TrayStatus::Stopped);
                }
            } else {
                app.record(
                    "duration.start.suppressed",
                    format!(
                        "outcome=suppressed reason={:?} policy_result=blocked",
                        decision.reason
                    ),
                );
                app.record(
                    "duration.start.suppressed.timing",
                    app.timing_snapshot_details(),
                );
                super::release_for_policy(app, decision.reason);
            }
        }
        DurationAction::Stop => {
            if app.controller.ownership() == OwnershipState::Released {
                app.record(
                    "duration.stop.result",
                    "outcome=noop reason=already_released policy_result=not_needed remaining_ms=0",
                );
                super::show_ownership_status(app, TrayStatus::Stopped);
            } else {
                let result =
                    super::guarded_release(app, "duration stop deadline", TrayStatus::Stopped);
                app.record(
                    "duration.stop.result",
                    format!("outcome={:?} remaining_ms=0", result),
                );
            }
            app.record("duration.stop.timing", app.timing_snapshot_details());
        }
        DurationAction::Pause => {
            app.record(
                "duration.pause.expired",
                format!(
                    "outcome=resume_and_reconcile duration_minutes={} remaining_ms=0",
                    action.duration.minutes()
                ),
            );
            refresh_power_for_duration(app, "pause_expiry");
            super::reconcile(app);
        }
    }
    if matches!(action.action, DurationAction::Start | DurationAction::Stop) {
        let timing_verified = match action.action {
            DurationAction::Start => {
                app.controller.ownership() == OwnershipState::Owned
                    && matches!(
                        app.controller.verification(),
                        Verification::Verified | Verification::FinerThanRequested
                    )
            }
            DurationAction::Stop => {
                app.controller.ownership() == OwnershipState::Released && app.handoff.is_none()
            }
            DurationAction::Pause => false,
        };
        let action_outcome = if timing_verified {
            DiagnosticOutcome::Completed
        } else {
            DiagnosticOutcome::Unverified
        };
        let action_context = app.operation.map_or_else(
            || app.diagnostics.begin_operation(DiagnosticSource::Timer),
            |root| {
                app.diagnostics
                    .child_operation(root, DiagnosticSource::Timer)
            },
        );
        app.diagnostics.record_verification(
            action_context,
            DiagnosticSource::Timer,
            action_outcome,
            "schedule.action.verify",
            format!(
                "action={} generation={} timing_verified={timing_verified}",
                action.action.label(),
                action.generation
            ),
        );
    }
    if app.handoff_operation.is_none() {
        app.finish_operation(DiagnosticOutcome::Completed);
    }
}

pub(crate) fn handle_duration_timer(app: &mut App, timer_id: usize) {
    if app.duration_timer_id != Some(timer_id) {
        app.record(
            "duration.timer",
            format!("outcome=ignored reason=stale_timer timer_id={timer_id}"),
        );
        return;
    }
    let generation = app.duration_timer_generation.unwrap_or_default();
    match app.pause.timer_event(generation, std::time::Instant::now()) {
        CoordinatorTimerEvent::Early {
            remaining, action, ..
        } => {
            app.record(
                "duration.timer",
                format!(
                    "outcome=early action={} generation={generation} remaining_ms={}",
                    action.label(),
                    remaining.as_millis()
                ),
            );
            arm_duration_timer(app);
        }
        CoordinatorTimerEvent::Expired(action) => {
            cancel_duration_timer(app);
            kill_schedule_display_timer(app);
            let _ = app.pause.cancel();
            execute_scheduled_action(app, action);
        }
        CoordinatorTimerEvent::Stale => {
            app.record(
                "duration.timer",
                format!("outcome=ignored reason=stale_generation generation={generation}"),
            );
        }
    }
}

pub(crate) fn refresh_power_for_duration(app: &mut App, source: &str) {
    let previous = app.observation.power().state;
    app.record(
        "native.GetSystemPowerStatus.call",
        format!("source={source} fields=sanitized"),
    );
    let result = app.observation.refresh_power();
    let power_snapshot = app.observation.power();
    app.record(
        "power.observation.raw",
        format!(
            "power_state={:?} battery_saver={:?}",
            power_snapshot.state, power_snapshot.battery_saver
        ),
    );
    app.record(
        "duration.power_observation",
        format!(
            "source={source} previous={previous:?} result={result:?} current={:?} policy=authoritative",
            app.observation.power().state
        ),
    );
}
