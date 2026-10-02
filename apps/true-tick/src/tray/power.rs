use tick_observation_windows::ObservationSource;
use tick_ownership::OwnershipState;
use tick_policy::PowerState;

use crate::tray::commands::POWER_DEBOUNCE_TIMER_ID;
use crate::tray::controller::App;
use crate::tray::native::KillTimer;
use crate::tray_surface::{
    power_reconciliation, uncertain_recovery_attempts_acquire, PowerReconciliation, TrayStatus,
};

pub(crate) fn kill_power_debounce_timer(app: &mut App) {
    if !app.power_debounce_active {
        return;
    }
    app.power_debounce_active = false;
    app.power_debounce_target_state = None;
    if let Some(hwnd) = app.tray_icon.as_ref().map(|icon| icon.h_wnd) {
        unsafe {
            // SAFETY: hwnd is the live tray window and the timer id was created by this module
            let _ = KillTimer(hwnd, POWER_DEBOUNCE_TIMER_ID);
        }
    }
    app.record("power.debounce.suppressed", "reason=shutdown_teardown");
}

pub(crate) fn release_for_power_change(app: &mut App) {
    app.record(
        "power.transition",
        format!("state={:?}", app.observation.power().state),
    );
    super::refresh_timing_observation(app);
    apply_power_reconciliation(app);
}

pub(crate) const fn resume_on_ac_applies(
    auto_resume: bool,
    pending: bool,
    power: PowerState,
    ownership: OwnershipState,
) -> bool {
    auto_resume
        && pending
        && matches!(power, PowerState::Ac)
        && matches!(ownership, OwnershipState::Released)
}

pub(crate) fn apply_power_reconciliation(app: &mut App) {
    if app.pause.pause_active() {
        app.record(
            "policy.power_reconciliation.suppressed",
            "reason=pause_active result=release_only",
        );
        super::queue_intent(
            app,
            tick_core::DesiredIntent::Release,
            "pause power reconciliation",
        );
        return;
    }
    let power = app.observation.power().state;
    let automatic = app.config.automatic;
    let ownership = app.controller.ownership();
    if resume_on_ac_applies(
        app.config.auto_resume_on_ac,
        app.pending_resume_on_ac,
        power,
        ownership,
    ) {
        app.pending_resume_on_ac = false;
        app.record("policy.power_resume_applied", "action=acquire");
        super::queue_intent(
            app,
            tick_core::DesiredIntent::Acquire,
            "power auto resume on ac",
        );
        return;
    }
    let action = power_reconciliation(automatic, power, ownership, app.config.battery_lockout);
    app.record(
        "policy.power_reconciliation",
        format!(
            "power={power:?} automatic={automatic} ownership={:?} action={action:?}",
            app.controller.ownership()
        ),
    );
    match action {
        PowerReconciliation::Acquire => super::apply_policy(app),
        PowerReconciliation::ReleaseBlocked => super::release_for_policy(
            app,
            match power {
                PowerState::Battery => tick_policy::PolicyReason::BatteryRestricted,
                PowerState::BatterySaver => tick_policy::PolicyReason::BatterySaverRestricted,
                PowerState::Unknown => tick_policy::PolicyReason::PowerUnknown,
                PowerState::Ac => unreachable!("AC power never selects ReleaseBlocked"),
            },
        ),
        PowerReconciliation::ShowStopped => super::show_ownership_status(app, TrayStatus::Stopped),
        PowerReconciliation::PreserveOwned => {
            super::show_ownership_status(app, TrayStatus::Stopped)
        }
        PowerReconciliation::PreserveUncertain => {
            if uncertain_recovery_attempts_acquire(automatic, power) {
                let settle = super::guarded_release(
                    app,
                    "power reconciliation settles uncertain ownership",
                    TrayStatus::Stopped,
                );
                let settled = app.controller.ownership() == OwnershipState::Released;
                app.record(
                    "policy.power_reconciliation.uncertain_settle",
                    format!(
                        "result={settle:?} ownership={:?}",
                        app.controller.ownership()
                    ),
                );
                if settle.is_ok() && settled {
                    super::apply_policy(app);
                } else {
                    app.record(
                        "policy.power_reconciliation.uncertain_unsettled",
                        format!(
                            "result={settle:?} ownership={:?} fallback=stopped",
                            app.controller.ownership()
                        ),
                    );
                    super::show_ownership_status(app, TrayStatus::Stopped);
                }
            } else {
                super::show_ownership_status(app, TrayStatus::Stopped);
            }
        }
    }
}
