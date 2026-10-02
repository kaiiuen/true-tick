use std::ffi::c_void;

use crate::tray::controller::App;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QuitDecision {
    ExitNormally,
    RequireSafetyDialog { reason: QuitSafetyReason },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QuitDialogDecision {
    StopAndQuit,
    Cancel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct QuitWarningResult {
    pub(crate) decision: QuitDialogDecision,
    pub(crate) message_box_result: Option<i32>,
    pub(crate) dialog_shown: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QuitSafetyReason {
    OwnedActive,
    OwnershipUncertain,
    TimingNotSettled,
}

pub(crate) fn quit_decision(app: &App) -> QuitDecision {
    quit_decision_for_handoff(
        app.lifecycle_status(),
        app.controller.ownership(),
        app.handoff.is_some(),
    )
}

pub(crate) fn quit_decision_for_handoff(
    status: crate::tray_surface::TrayStatus,
    ownership: tick_ownership::OwnershipState,
    handoff_active: bool,
) -> QuitDecision {
    if handoff_active && ownership == tick_ownership::OwnershipState::Released {
        return QuitDecision::ExitNormally;
    }
    if handoff_active {
        return QuitDecision::RequireSafetyDialog {
            reason: QuitSafetyReason::TimingNotSettled,
        };
    }
    quit_decision_for(status, ownership)
}

pub(crate) fn quit_decision_for(
    status: crate::tray_surface::TrayStatus,
    ownership: tick_ownership::OwnershipState,
) -> QuitDecision {
    use crate::tray_surface::TrayStatus;
    use tick_ownership::OwnershipState;

    match ownership {
        OwnershipState::Owned => {
            return QuitDecision::RequireSafetyDialog {
                reason: QuitSafetyReason::OwnedActive,
            };
        }
        OwnershipState::Uncertain => {
            return QuitDecision::RequireSafetyDialog {
                reason: QuitSafetyReason::OwnershipUncertain,
            };
        }
        OwnershipState::Released => {}
    }
    if matches!(
        status,
        TrayStatus::Running
            | TrayStatus::Starting
            | TrayStatus::Stopping
            | TrayStatus::Pending
            | TrayStatus::Degraded
            | TrayStatus::Unverified
    ) {
        return QuitDecision::RequireSafetyDialog {
            reason: QuitSafetyReason::TimingNotSettled,
        };
    }
    QuitDecision::ExitNormally
}

pub(crate) fn quit_warning_text(reason: QuitSafetyReason) -> &'static str {
    match reason {
        QuitSafetyReason::OwnershipUncertain => {
            "True™ Tick could not verify that timing is fully released. Keep the app open and retry cleanup?"
        }
        QuitSafetyReason::OwnedActive | QuitSafetyReason::TimingNotSettled => {
            "True™ Tick is currently controlling timer resolution. Stop timing and quit?"
        }
    }
}

pub(crate) unsafe fn show_quit_warning(
    hwnd: *mut c_void,
    reason: QuitSafetyReason,
) -> QuitWarningResult {
    let title = super::wide("True™ Tick");
    let text = super::wide(quit_warning_text(reason));
    let result = super::MessageBoxW(
        hwnd,
        text.as_ptr(),
        title.as_ptr(),
        super::MB_YESNO | super::MB_ICONWARNING | super::MB_DEFBUTTON2,
    );
    quit_warning_result(Some(result))
}

pub(crate) fn quit_warning_result(message_box_result: Option<i32>) -> QuitWarningResult {
    let result = message_box_result.unwrap_or(0);
    QuitWarningResult {
        decision: message_box_decision(result),
        message_box_result,
        dialog_shown: result != 0,
    }
}

pub(crate) fn message_box_decision(result: i32) -> QuitDialogDecision {
    if result == super::IDYES {
        QuitDialogDecision::StopAndQuit
    } else {
        QuitDialogDecision::Cancel
    }
}
