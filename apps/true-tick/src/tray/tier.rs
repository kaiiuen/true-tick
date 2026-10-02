use std::ffi::c_void;
use std::sync::atomic::Ordering;

use tick_core::DesiredIntent;
use tick_diagnostics::{DiagnosticOutcome, DiagnosticSource};
use tick_observation_windows::ObservationSource;
use tick_ownership::{
    OwnershipState, SettleProbeOutcome, SettleProbeStep, Verification, SETTLE_PROBE_ATTEMPTS,
    SETTLE_PROBE_SPACING_MS,
};
use tick_platform_windows::TimerObservation;
use tick_policy::{
    decide, evaluate_tier, tier_allows_high_resolution, OperatingTier, PolicyInput, TierContext,
    TierTransition,
};

use crate::pause::{acquisition_is_allowed, DurationAction};
use crate::tray::commands::{
    HANDOFF_TIMER_ID, SURFACE_RECOVERY_ATTEMPTS, SURFACE_RECOVERY_SPACING_MS,
    SURFACE_RECOVERY_TIMER_ID,
};
use crate::tray::controller::{self, App};
use crate::tray::icon::{NotifyIconData, Shell_NotifyIconW, NIM_ADD};
use crate::tray::native::{GetLastError, KillTimer, NativeResult, SetTimer};
use crate::tray::schedule::{arm_duration_timer, cancel_duration_timer};
use crate::tray_surface::{
    release_needs_handoff, HandoffProgress, HandoffTracker, TrayStatus, HANDOFF_POLL_INTERVAL_MS,
};

pub(crate) fn reconcile(app: &mut App) {
    app.record("policy.recalculate", "trigger=reconcile");
    refresh_timing_observation(app);
    apply_policy(app);
}

pub(crate) fn apply_policy(app: &mut App) {
    let power = app.observation.power().state;
    let decision = decide(PolicyInput {
        enabled: true,
        eligible_profile: true,
        power,
        battery_lockout_enabled: app.config.battery_lockout,
    });
    app.record(
        "policy.evaluation",
        format!(
            "power={power:?} status={:?} reason={:?}",
            decision.status, decision.reason
        ),
    );
    let intent = if app.pause.pause_active() {
        app.record(
            "policy.pause_suppressed",
            format!("requested_status={:?} reason=pause_active", decision.status),
        );
        DesiredIntent::Release
    } else if let Some(scheduled) = app.pause.current() {
        match scheduled.action {
            DurationAction::Start | DurationAction::Stop
                if decision.status != tick_core::Status::Requested =>
            {
                app.record(
                    "policy.duration.suppressed",
                    format!(
                        "action={} reason={:?} result=release_safe timing_snapshot={}",
                        scheduled.action.label(),
                        decision.reason,
                        app.timing_snapshot_details()
                    ),
                );
                return release_for_policy(app, decision.reason);
            }
            DurationAction::Start | DurationAction::Stop => {
                app.record(
                    "policy.duration.deferred",
                    format!(
                        "action={} reason=scheduled_action deadline_remaining_ms={}",
                        scheduled.action.label(),
                        scheduled.remaining(std::time::Instant::now()).as_millis()
                    ),
                );
                show_ownership_status(app, TrayStatus::Stopped);
                return;
            }
            DurationAction::Pause => DesiredIntent::Release,
        }
    } else if acquisition_is_allowed(
        false,
        app.config.automatic,
        power,
        app.config.battery_lockout,
    ) {
        DesiredIntent::Acquire
    } else {
        DesiredIntent::Release
    };
    if app.handoff.is_some() {
        app.record(
            "policy.recalculate.deferred",
            format!("reason=handoff_pending queued_intent={intent:?}"),
        );
    }
    queue_intent(app, intent, format!("policy reason={:?}", decision.reason));
}

pub(crate) fn queue_intent(app: &mut App, intent: DesiredIntent, source: impl AsRef<str>) {
    app.desired_intent.request(intent);
    app.record(
        "lifecycle.desired_intent",
        format!("intent={intent:?} source={}", source.as_ref()),
    );
    if app.handoff.is_some() {
        app.tray_status = if app.pause.pause_active() {
            TrayStatus::Pausing
        } else {
            TrayStatus::Stopping
        };
        app.publish();
        return;
    }
    process_desired_intent(app);
}

pub(crate) fn process_desired_intent(app: &mut App) {
    if app.handoff.is_some() {
        app.tray_status = if app.pause.pause_active() {
            TrayStatus::Pausing
        } else {
            TrayStatus::Stopping
        };
        app.publish();
        return;
    }
    let Some(intent) = app.desired_intent.take() else {
        return;
    };
    match intent {
        DesiredIntent::Acquire => {
            if app.pause.pause_active() {
                app.record(
                    "lifecycle.acquire.suppressed",
                    "reason=pause_active result=suppressed",
                );
                if app.controller.ownership() == OwnershipState::Released {
                    app.tray_status = TrayStatus::Paused;
                    app.publish();
                    arm_duration_timer(app);
                }
                return;
            }
            let power = app.observation.power().state;
            let decision = decide(PolicyInput {
                enabled: true,
                eligible_profile: true,
                power,
                battery_lockout_enabled: app.config.battery_lockout,
            });
            if decision.status == tick_core::Status::Requested {
                acquire_timer(app);
            } else {
                release_for_policy(app, decision.reason);
            }
        }
        DesiredIntent::Release => {
            if app.tray_status == TrayStatus::Starting {
                app.desired_intent.request(DesiredIntent::Release);
                app.record(
                    "lifecycle.release_queued",
                    "reason=acquisition_verification_pending",
                );
            } else if matches!(
                app.controller.ownership(),
                OwnershipState::Owned | OwnershipState::Uncertain
            ) {
                let source = "desired intent";
                match guarded_release(
                    app,
                    source,
                    if app.pause.pause_active() {
                        TrayStatus::Paused
                    } else {
                        TrayStatus::Stopped
                    },
                ) {
                    Ok(_) => {}
                    Err(message) => app.record(
                        "ownership.release.caller_failed",
                        format!("source={source} reason={message}"),
                    ),
                }
            } else {
                app.tray_status =
                    if app.pause.pause_active() && app.tray_status != TrayStatus::Unverified {
                        TrayStatus::Paused
                    } else {
                        TrayStatus::Stopped
                    };
                app.publish();
                if app.pause.pause_active() {
                    arm_duration_timer(app);
                }
            }
        }
    }
}

pub(crate) fn acquire_timer(app: &mut App) {
    if app.controller.ownership() != OwnershipState::Released {
        show_ownership_status(app, TrayStatus::Stopped);
        return;
    }
    if !tier_allows_high_resolution(app.operating_tier) {
        app.record(
            "tier.acquisition_blocked",
            format!("tier={:?} result=suppressed", app.operating_tier),
        );
        app.tray_status = if app.pause.pause_active() {
            TrayStatus::Paused
        } else {
            TrayStatus::Stopped
        };
        app.publish();
        return;
    }
    app.tray_status = TrayStatus::Starting;
    app.publish();
    app.record("ownership.acquire.request", "source=desired_intent");
    let start_result = app.controller.start();
    app.sync_timing_snapshot();
    app.timing_snapshot_valid = matches!(
        start_result,
        Ok(Verification::Verified | Verification::FinerThanRequested)
    );
    app.invalid_interval = matches!(
        start_result,
        Err(tick_platform_windows::TimerError::InvalidInterval)
    );
    if matches!(
        start_result,
        Err(tick_platform_windows::TimerError::RequestFailed { .. })
    ) {
        app.kernel_rejected_requests = app.kernel_rejected_requests.saturating_add(1);
    }
    let status = match start_result {
        Ok(Verification::Verified | Verification::FinerThanRequested) => {
            app.record("verification.result", format!("result={start_result:?}"));
            app.record("ownership.changed", "state=owned");
            TrayStatus::Running
        }
        Ok(verification) => {
            app.record("verification.result", format!("result={verification:?}"));
            TrayStatus::Unverified
        }
        Err(error) => {
            app.record_with_outcome(
                "ownership.acquire.error",
                format!("error={error:?}"),
                DiagnosticOutcome::Failed,
            );
            match error {
                tick_platform_windows::TimerError::Unsupported => TrayStatus::Unsupported,
                _ => TrayStatus::Error,
            }
        }
    };
    if start_result.is_ok() {
        // A successful acquisition supersedes any earlier policy block.
        app.last_block_reason = None;
    }
    if status == TrayStatus::Running {
        app.running_since = Some(std::time::Instant::now());
    } else {
        app.running_since = None;
    }
    app.tray_status = status;
    app.publish();
    if app.desired_intent.pending().is_some() {
        process_desired_intent(app);
    }
}

/// Pulls newly observed anomaly producer counts into `unhandled_anomalies`.
/// Each producer contributes at most one increment per distinct occurrence.
pub(crate) fn drain_anomaly_producers(app: &mut App) {
    let pending = app.pending_anomalies.swap(0, Ordering::Acquire);
    if pending > 0 {
        let reasons = {
            let mut guard = app
                .pending_anomaly_reasons
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            std::mem::take(&mut *guard)
        };
        app.unhandled_anomalies = app.unhandled_anomalies.saturating_add(pending);
        for reason in reasons {
            app.record(
                "tier.anomaly.observed",
                format!("reason={reason} total={}", app.unhandled_anomalies),
            );
        }
    }
    let episodes = app
        .watchdog
        .as_ref()
        .map(tick_watchdog::WatchdogHandle::stall_episodes);
    if let Some(episodes) = episodes {
        if episodes > app.last_watchdog_episodes {
            let new_episodes = episodes - app.last_watchdog_episodes;
            app.last_watchdog_episodes = episodes;
            app.unhandled_anomalies = app.unhandled_anomalies.saturating_add(new_episodes);
            app.record(
                "tier.anomaly.observed",
                format!(
                    "reason=watchdog_stall_episode episodes={episodes} total={}",
                    app.unhandled_anomalies
                ),
            );
        }
    }
    let storage_failures = app
        .diagnostics
        .snapshot()
        .iter()
        .filter(|event| is_storage_failure_event(event))
        .map(|event| event.sequence)
        .max()
        .unwrap_or(0);
    if storage_failures > app.consumed_log_write_failures {
        let delta = app
            .diagnostics
            .snapshot()
            .iter()
            .filter(|event| {
                is_storage_failure_event(event)
                    && event.sequence > app.consumed_log_write_failures
                    && event.sequence <= storage_failures
            })
            .count() as u32;
        app.consumed_log_write_failures = storage_failures;
        app.unhandled_anomalies = app.unhandled_anomalies.saturating_add(delta);
        app.record(
            "tier.anomaly.observed",
            format!(
                "reason=log_write_failure failures={storage_failures} total={}",
                app.unhandled_anomalies
            ),
        );
    }
    let observed_panics = controller::caught_dispatch_panics();
    if observed_panics > app.consumed_dispatch_panics {
        let delta = observed_panics - app.consumed_dispatch_panics;
        app.consumed_dispatch_panics = observed_panics;
        app.unhandled_anomalies = app.unhandled_anomalies.saturating_add(delta);
        app.record(
            "tier.anomaly.observed",
            format!(
                "reason=caught_panic panics={observed_panics} total={}",
                app.unhandled_anomalies
            ),
        );
    }
}

/// Records a producer-side anomaly so the next tier evaluation sees it.
/// Safe to call from any thread that holds the shared app counters.
/// Today the producers that need it are exercised through tests. Runtime
/// producers fold directly through the dedicated counters drained in
/// `drain_anomaly_producers`.
#[allow(dead_code)]
pub(crate) fn report_anomaly(app: &App, reason: &'static str) {
    app.pending_anomalies.fetch_add(1, Ordering::AcqRel);
    let mut guard = app
        .pending_anomaly_reasons
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.push(reason);
}

/// True for diagnostic events that prove a log write or retention failure.
/// The insufficient free space path records `storage.write_suppressed` with
/// a `Suppressed` outcome and purge or append faults land on `storage.*`
/// failure events, so scanning the store sees every persisted storage fault
/// from the anomaly ladder side.
fn is_storage_failure_event(event: &tick_diagnostics::DiagnosticEvent) -> bool {
    event.name.starts_with("storage.")
        && matches!(
            event.outcome,
            DiagnosticOutcome::Failed | DiagnosticOutcome::Suppressed
        )
}

pub(crate) fn refresh_operating_tier(app: &mut App) {
    drain_anomaly_producers(app);
    let current_floor_hns = match app.operating_tier {
        OperatingTier::MetrologyDegraded { floored_hns } => floored_hns,
        _ => 0,
    };
    let nominally_recoverable = app.tray_icon.is_some()
        && app.kernel_rejected_requests == 0
        && app.unhandled_anomalies == 0;
    let context = TierContext {
        tray_surface_available: app.tray_icon.is_some(),
        kernel_rejected_requests: app.kernel_rejected_requests,
        unhandled_anomalies: app.unhandled_anomalies,
        current_floor_hns,
        nominally_recoverable,
    };
    let transition = evaluate_tier(app.operating_tier, &context);
    let leaving_surface = app.operating_tier == OperatingTier::SurfaceDegraded
        && !matches!(
            transition,
            TierTransition::Stay | TierTransition::EnterSurfaceDegraded
        );
    if leaving_surface {
        // SAFETY: `tray_hwnd` stays valid while the app runs and killing a
        // stale timer id is a no-op when the timer was never armed.
        unsafe { KillTimer(app.tray_hwnd, SURFACE_RECOVERY_TIMER_ID) };
    }
    match transition {
        TierTransition::Stay => {}
        TierTransition::EnterSurfaceDegraded => {
            app.operating_tier = OperatingTier::SurfaceDegraded;
            app.surface_recovery_attempts = 0;
            app.surface_recovery_exhausted = false;
            app.surface_recovery_due = Some(
                std::time::Instant::now()
                    + std::time::Duration::from_millis(SURFACE_RECOVERY_SPACING_MS),
            );
            // SAFETY: `tray_hwnd` is the live tray window while the app is
            // running. The callback is null so `WM_TIMER` posts to the queue.
            let timer_set = unsafe {
                SetTimer(
                    app.tray_hwnd,
                    SURFACE_RECOVERY_TIMER_ID,
                    SURFACE_RECOVERY_SPACING_MS as u32,
                    std::ptr::null_mut(),
                )
            };
            app.record(
                "tier.surface_recovery.armed",
                format!("timer_valid={}", timer_set != 0),
            );
        }
        TierTransition::EnterMetrologyDegraded { floored_hns } => {
            app.operating_tier = OperatingTier::MetrologyDegraded { floored_hns };
            app.controller
                .clamp_requested_interval_floor(tick_core::Hns::new(floored_hns));
        }
        TierTransition::EnterQuiescent => {
            app.operating_tier = OperatingTier::Quiescent;
        }
        TierTransition::Recover => {
            app.operating_tier = OperatingTier::Nominal;
            app.kernel_rejected_requests = 0;
            app.unhandled_anomalies = 0;
            app.last_watchdog_episodes = app
                .watchdog
                .as_ref()
                .map_or(0, |handle| handle.stall_episodes());
            app.consumed_log_write_failures = app
                .diagnostics
                .snapshot()
                .iter()
                .filter(|event| is_storage_failure_event(event))
                .map(|event| event.sequence)
                .max()
                .unwrap_or(0);
            app.surface_recovery_attempts = 0;
            app.surface_recovery_exhausted = false;
            app.surface_recovery_due = None;
            app.consumed_dispatch_panics = controller::caught_dispatch_panics();
        }
    }
    if !matches!(transition, TierTransition::Stay) {
        app.record(
            "tier.transition",
            format!("transition={transition:?} tier={:?}", app.operating_tier),
        );
        if matches!(transition, TierTransition::EnterQuiescent)
            && app.controller.ownership() == OwnershipState::Owned
        {
            app.record(
                "tier.quiescent.restorative_release",
                "action=guarded_release reason=quiescent_entered",
            );
            let source = "quiescent tier entry";
            match guarded_release(app, source, TrayStatus::Stopped) {
                Ok(_) => {}
                Err(message) => app.record(
                    "ownership.release.caller_failed",
                    format!("source={source} reason={message}"),
                ),
            }
        }
    }
}

pub(crate) fn service_surface_recovery(app: &mut App, hwnd: *mut c_void) {
    if app.operating_tier != OperatingTier::SurfaceDegraded {
        return;
    }
    if app.tray_icon.is_some() {
        return;
    }
    if app.surface_recovery_exhausted {
        return;
    }
    let Some(due) = app.surface_recovery_due else {
        app.surface_recovery_due = Some(
            std::time::Instant::now()
                + std::time::Duration::from_millis(SURFACE_RECOVERY_SPACING_MS),
        );
        return;
    };
    if std::time::Instant::now() < due {
        return;
    }
    let recovered = unsafe { attempt_surface_restore(hwnd, app) };
    advance_surface_recovery(app, recovered);
}

pub(crate) fn advance_surface_recovery(app: &mut App, recovered: bool) {
    app.surface_recovery_attempts = app.surface_recovery_attempts.saturating_add(1);
    let attempt = app.surface_recovery_attempts;
    if recovered {
        app.record(
            "tier.surface_recovered",
            format!("attempt={attempt} budget={SURFACE_RECOVERY_ATTEMPTS}"),
        );
        app.surface_recovery_due = None;
        app.surface_recovery_attempts = 0;
        if !app.tray_hwnd.is_null() {
            // SAFETY: recovery finished, so the wake timer is no longer needed.
            unsafe { KillTimer(app.tray_hwnd, SURFACE_RECOVERY_TIMER_ID) };
        }
        app.publish();
        return;
    }
    app.record_with_outcome(
        "tier.surface_recovery.attempt",
        format!("attempt={attempt} budget={SURFACE_RECOVERY_ATTEMPTS} result=failed"),
        DiagnosticOutcome::Failed,
    );
    if app.surface_recovery_attempts >= SURFACE_RECOVERY_ATTEMPTS {
        app.surface_recovery_exhausted = true;
        app.surface_recovery_due = None;
        if !app.tray_hwnd.is_null() {
            // SAFETY: the budget is spent, so no further wake ticks are needed.
            unsafe { KillTimer(app.tray_hwnd, SURFACE_RECOVERY_TIMER_ID) };
        }
        app.record(
            "tier.surface_recovery_exhausted",
            format!(
                "attempts={} result=staying_surface_degraded",
                app.surface_recovery_attempts
            ),
        );
    } else {
        app.surface_recovery_due = Some(
            std::time::Instant::now()
                + std::time::Duration::from_millis(SURFACE_RECOVERY_SPACING_MS),
        );
    }
    app.publish();
}

unsafe fn attempt_surface_restore(hwnd: *mut c_void, app: &mut App) -> bool {
    let mut icon = match NotifyIconData::new(
        hwnd,
        app.lifecycle_status(),
        app.timing_values(),
        app.pause.current(),
        app.last_block_reason,
    ) {
        Ok(icon) => icon,
        Err(raw_error) => {
            app.record_with_outcome(
                "native.tray_icon.create.error",
                format!("raw_status={raw_error} context=surface_recovery"),
                DiagnosticOutcome::Failed,
            );
            return false;
        }
    };
    let add_result = Shell_NotifyIconW(NIM_ADD, &mut icon);
    let add_error = if add_result == 0 { GetLastError() } else { 0 };
    if matches!(
        crate::tray::native::native_bool_result(add_result, add_error),
        NativeResult::Failed { .. }
    ) {
        app.record_with_outcome(
            "native.Shell_NotifyIconW.add.error",
            format!("raw_status={add_error} context=surface_recovery"),
            DiagnosticOutcome::Failed,
        );
        return false;
    }
    app.tray_icon = Some(icon);
    app.last_publication = None;
    true
}

pub(crate) fn release_for_policy(app: &mut App, reason: tick_policy::PolicyReason) {
    if app.handoff.is_some() {
        queue_intent(
            app,
            DesiredIntent::Release,
            format!("policy reason={reason:?}"),
        );
        return;
    }
    let is_power_restriction = matches!(
        reason,
        tick_policy::PolicyReason::BatteryRestricted
            | tick_policy::PolicyReason::BatterySaverRestricted
            | tick_policy::PolicyReason::PowerUnknown
    );
    let ownership = app.controller.ownership();
    if is_power_restriction
        && matches!(ownership, OwnershipState::Owned | OwnershipState::Uncertain)
    {
        app.pending_resume_on_ac = true;
        app.record("policy.power_resume_pending", format!("reason={reason:?}"));
    }
    let released_status = match reason {
        tick_policy::PolicyReason::BatteryRestricted
        | tick_policy::PolicyReason::BatterySaverRestricted
        | tick_policy::PolicyReason::PowerUnknown
        | tick_policy::PolicyReason::GloballyDisabled => TrayStatus::Blocked,
        tick_policy::PolicyReason::NoEligibleProfile
        | tick_policy::PolicyReason::EligibleProfile => TrayStatus::Stopped,
    };
    if released_status == TrayStatus::Blocked {
        app.last_block_reason = Some(reason);
    } else {
        app.last_block_reason = None;
    }
    if matches!(
        app.controller.ownership(),
        OwnershipState::Owned | OwnershipState::Uncertain
    ) {
        let source = format!("policy reason={reason:?}");
        match guarded_release(app, &source, released_status) {
            Ok(_) => {}
            Err(message) => app.record(
                "ownership.release.caller_failed",
                format!("source={source} reason={message}"),
            ),
        }
    } else {
        app.tray_status = released_status;
        app.publish();
    }
}

pub(crate) fn guarded_release(
    app: &mut App,
    source: impl AsRef<str>,
    released_status: TrayStatus,
) -> Result<bool, String> {
    if app.handoff.is_some() {
        app.record(
            "ownership.release.repeated",
            format!("source={} result=handoff_already_pending", source.as_ref()),
        );
        return Err("handoff already pending".to_owned());
    }
    app.record(
        "ownership.release.request",
        format!("source={}", source.as_ref()),
    );
    app.external_timing = false;
    app.running_since = None;
    app.tray_status = if app.pause.pause_active() {
        TrayStatus::Pausing
    } else {
        TrayStatus::Stopping
    };
    app.publish();
    let stop_result = app.controller.stop();
    app.sync_timing_snapshot();
    app.timing_snapshot_valid = stop_result.is_ok();
    match stop_result {
        Ok(released) => {
            app.record(
                "ownership.changed",
                format!("state=released changed={released}"),
            );
            let effective = app.timing_snapshot.effective;
            let boundary = app.timing_snapshot.requested;
            if released && boundary.is_some_and(|value| release_needs_handoff(value, effective)) {
                let Some(boundary) = boundary else {
                    app.record_with_outcome(
                        "ownership.release.error",
                        "reason=missing_release_boundary",
                        DiagnosticOutcome::Failed,
                    );
                    return Err("missing release boundary".to_owned());
                };
                if begin_handoff(
                    app,
                    boundary,
                    if app.pause.pause_active() {
                        TrayStatus::Paused
                    } else {
                        TrayStatus::Stopped
                    },
                ) {
                    return Ok(released);
                }
            }
            app.record(
                "ownership.result",
                format!("state=released effective_system={effective:?} handoff=not_required"),
            );
            app.controller.clear_release_boundary();
            app.sync_timing_snapshot();
            app.timing_snapshot_valid = true;
            app.tray_status = released_status;
            app.publish();
            if app.pause.pause_active() {
                arm_duration_timer(app);
            }
            Ok(released)
        }
        Err(error) => {
            app.timing_snapshot_valid = false;
            app.running_since = None;
            app.record_with_outcome(
                "ownership.release.error",
                format!("error={error:?}"),
                DiagnosticOutcome::Failed,
            );
            app.tray_status = TrayStatus::Unverified;
            app.publish();
            Err(format!("{error:?}"))
        }
    }
}

pub(crate) fn show_ownership_status(app: &mut App, released_status: TrayStatus) {
    if app.handoff.is_some() {
        app.tray_status = if app.pause.pause_active() {
            TrayStatus::Pausing
        } else {
            TrayStatus::Stopping
        };
        app.publish();
        return;
    }
    app.tray_status = match app.controller.ownership() {
        OwnershipState::Released if app.pause.pause_active() => TrayStatus::Paused,
        OwnershipState::Released => released_status,
        OwnershipState::Owned => TrayStatus::Running,
        OwnershipState::Uncertain => TrayStatus::Unverified,
    };
    app.publish();
}

pub(crate) fn refresh_timing_observation(app: &mut App) -> Option<TimerObservation> {
    match app.controller.query() {
        Ok(observation) => {
            app.sync_timing_snapshot();
            app.timing_snapshot_valid = true;
            app.record_with_outcome(
                "timer.query.observation",
                format!(
                    "requested_hns={} effective_hns={} raw_status={} effective_relation={}",
                    observation.requested.value(),
                    observation.reported_current.value(),
                    observation.raw_status,
                    observation.effective_relation()
                ),
                DiagnosticOutcome::Completed,
            );
            Some(observation)
        }
        Err(error) => {
            app.sync_timing_snapshot();
            app.timing_snapshot_valid = false;
            app.record_with_outcome(
                "timer.query.error",
                format!("error={error:?} effective=unknown"),
                DiagnosticOutcome::Failed,
            );
            None
        }
    }
}

pub(crate) fn probe_startup_kernel_settle(app: &mut App) {
    let mut attempts_used = 0u32;
    let outcome = loop {
        match app
            .controller
            .attempt_startup_kernel_settle_probe(attempts_used)
        {
            SettleProbeStep::NotApplicable => return,
            SettleProbeStep::Retry { budget_remaining } => {
                attempts_used += 1;
                app.record(
                    "recovery.settle_probe",
                    format!(
                        "result=retry attempt={} budget_remaining={} spacing_ms={}",
                        attempts_used, budget_remaining, SETTLE_PROBE_SPACING_MS
                    ),
                );
                std::thread::sleep(std::time::Duration::from_millis(SETTLE_PROBE_SPACING_MS));
            }
            SettleProbeStep::Settled(outcome) => break outcome,
        }
    };
    app.sync_timing_snapshot();
    app.timing_snapshot_valid = true;
    match outcome {
        SettleProbeOutcome::Resolved { freed_hns } => {
            app.record(
                "kernel.self_heal.settle_probe_restored",
                format!(
                    "ownership=released attempts={} freed_hns={} detail={}",
                    attempts_used + 1,
                    freed_hns,
                    app.timing_snapshot_details()
                ),
            );
        }
        SettleProbeOutcome::ExternalClientConfirmed => {
            app.external_timing = true;
            app.record(
                "kernel.self_heal.external_timing_confirmed",
                format!(
                    "ownership=released external_timing=true attempts={} detail={}",
                    attempts_used + 1,
                    app.timing_snapshot_details()
                ),
            );
        }
        SettleProbeOutcome::Exhausted => {
            app.record(
                "kernel.self_heal.settle_probe_exhausted",
                format!(
                    "ownership=released attempts={} budget=0 detail={}",
                    SETTLE_PROBE_ATTEMPTS,
                    app.timing_snapshot_details()
                ),
            );
        }
    }
}

pub(crate) fn begin_handoff(
    app: &mut App,
    boundary: tick_core::Hns,
    released_status: TrayStatus,
) -> bool {
    let tracker = HandoffTracker::new(boundary, released_status);
    let handoff_operation = app.begin_operation(DiagnosticSource::Handoff);
    app.handoff_operation = Some(handoff_operation);
    cancel_duration_timer(app);
    let Some(hwnd) = app.tray_icon.as_ref().map(|icon| icon.h_wnd) else {
        app.record(
            "handoff.timeout",
            "reason=watcher_unavailable ownership=released effective_system=finer",
        );
        app.external_timing = true;
        app.controller.clear_release_boundary();
        app.sync_timing_snapshot();
        app.tray_status = if released_status == TrayStatus::Paused {
            TrayStatus::Unverified
        } else {
            TrayStatus::Stopped
        };
        app.publish();
        process_desired_intent(app);
        if app.pause.pause_active() {
            arm_duration_timer(app);
        }
        app.finish_operation(DiagnosticOutcome::TimedOut);
        app.handoff_operation = None;
        return false;
    };
    let timer = unsafe {
        // SAFETY: hwnd is the live tray window and the callback is null so WM_TIMER posts to our queue
        SetTimer(
            hwnd,
            HANDOFF_TIMER_ID,
            HANDOFF_POLL_INTERVAL_MS,
            std::ptr::null_mut(),
        )
    };
    if timer == 0 {
        app.record(
            "handoff.timeout",
            format!(
                "reason=watcher_start_failed ownership=released boundary_hns={} effective_system=finer raw_status={}",
                boundary.value(),
                unsafe {
                    // SAFETY: called immediately on the same thread after a failed Win32 call so the thread-local last-error value belongs to this call
                    GetLastError()
                }
            ),
        );
        app.record(
            "ownership.result",
            "state=released effective_system=finer due_to=external_or_unknown_client",
        );
        app.external_timing = true;
        app.controller.clear_release_boundary();
        app.sync_timing_snapshot();
        app.tray_status = if released_status == TrayStatus::Paused {
            TrayStatus::Unverified
        } else {
            TrayStatus::Stopped
        };
        app.publish();
        process_desired_intent(app);
        if app.pause.pause_active() {
            arm_duration_timer(app);
        }
        app.finish_operation(DiagnosticOutcome::TimedOut);
        app.handoff_operation = None;
        return false;
    }
    app.handoff = Some(tracker);
    app.tray_status = if released_status == TrayStatus::Paused {
        TrayStatus::Pausing
    } else {
        TrayStatus::Stopping
    };
    app.record(
        "handoff.started",
        format!(
            "boundary_hns={} interval_ms={} max_polls={} ownership=released effective_system=finer",
            boundary.value(),
            HANDOFF_POLL_INTERVAL_MS,
            crate::tray_surface::HANDOFF_MAX_POLLS
        ),
    );
    app.publish();
    true
}

pub(crate) fn finish_handoff_timer(app: &mut App) {
    if let Some(hwnd) = app.tray_icon.as_ref().map(|icon| icon.h_wnd) {
        let result = unsafe {
            // SAFETY: hwnd is the live tray window and the timer id was created by this module
            KillTimer(hwnd, HANDOFF_TIMER_ID)
        };
        if result == 0 {
            app.record_with_outcome(
                "native.KillTimer.handoff.error",
                format!("raw_status={}", unsafe {
                    // SAFETY: called immediately on the same thread after a failed Win32 call so the thread-local last-error value belongs to this call
                    GetLastError()
                }),
                DiagnosticOutcome::Failed,
            );
        }
    }
}

pub(crate) fn handle_handoff_timer(app: &mut App) {
    if let Some(context) = app.handoff_operation {
        app.operation = Some(context);
        app.operation_source = DiagnosticSource::Handoff;
    }
    let Some(tracker) = app.handoff.take() else {
        return;
    };
    app.handoff = Some(tracker);
    let boundary = tracker.boundary();
    let observation = match app.controller.observe_current(boundary) {
        Ok(observation) => {
            app.sync_timing_snapshot();
            app.timing_snapshot_valid = true;
            app.record(
                "timer.handoff.observation",
                format!(
                    "requested_hns={} effective_hns={} raw_status={} effective_relation={}",
                    observation.requested.value(),
                    observation.reported_current.value(),
                    observation.raw_status,
                    observation.effective_relation()
                ),
            );
            Some(observation)
        }
        Err(error) => {
            app.sync_timing_snapshot();
            app.timing_snapshot_valid = false;
            app.record_with_outcome(
                "timer.handoff.query.error",
                format!("error={error:?} effective=unknown"),
                DiagnosticOutcome::Failed,
            );
            None
        }
    };
    let effective = observation.map(|value| value.reported_current);
    let Some(mut tracker) = app.handoff.take() else {
        return;
    };
    let progress = tracker.observe(effective);
    app.record(
        "handoff.observation",
        format!(
            "boundary_hns={} effective_hns={} poll={} result={progress:?}",
            tracker.boundary().value(),
            effective.map_or_else(|| "unknown".to_owned(), |value| value.value().to_string()),
            tracker.polls()
        ),
    );
    match progress {
        HandoffProgress::Pending => {
            app.handoff = Some(tracker);
            app.tray_status = if tracker.released_status() == TrayStatus::Paused {
                TrayStatus::Pausing
            } else {
                TrayStatus::Stopping
            };
            app.publish();
        }
        HandoffProgress::Completed => {
            finish_handoff_timer(app);
            app.record(
                "handoff.completed",
                format!(
                    "boundary_hns={} effective_hns={} ownership=released",
                    tracker.boundary().value(),
                    effective
                        .map_or_else(|| "unknown".to_owned(), |value| value.value().to_string())
                ),
            );
            app.record(
                "ownership.result",
                "state=released effective_system=non_finer handoff=completed",
            );
            app.controller.clear_release_boundary();
            app.sync_timing_snapshot();
            app.tray_status = tracker.released_status();
            app.external_timing = false;
            app.publish();
            process_desired_intent(app);
            if app.pause.pause_active() {
                arm_duration_timer(app);
            }
            app.finish_operation(DiagnosticOutcome::Completed);
            app.handoff_operation = None;
        }
        HandoffProgress::TimedOut => {
            finish_handoff_timer(app);
            app.record(
                "handoff.timeout",
                format!(
                    "boundary_hns={} effective_hns={} ownership=released reason=external_or_unknown_client_remains_finer",
                    tracker.boundary().value(),
                    effective.map_or_else(|| "unknown".to_owned(), |value| value.value().to_string())
                ),
            );
            app.record(
                "ownership.result",
                "state=released effective_system=finer due_to=external_or_unknown_client",
            );
            app.controller.clear_release_boundary();
            app.sync_timing_snapshot();
            app.tray_status = if tracker.released_status() == TrayStatus::Paused {
                TrayStatus::Unverified
            } else {
                TrayStatus::Stopped
            };
            app.external_timing = true;
            app.publish();
            process_desired_intent(app);
            if app.pause.pause_active() {
                arm_duration_timer(app);
            }
            app.finish_operation(DiagnosticOutcome::TimedOut);
            app.handoff_operation = None;
        }
    }
}
