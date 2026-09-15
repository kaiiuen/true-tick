use crate::pause::{DurationAction, DurationChoice, ScheduledAction};
use std::time::{Duration, Instant};
use tick_core::Hns;
use tick_ownership::{OwnershipState, TimingSnapshot};
use tick_policy::PowerState;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TrayStatus {
    Running,
    Starting,
    ScheduledStart,
    Pausing,
    Stopping,
    ScheduledStop,
    Paused,
    #[allow(dead_code)]
    Pending,
    Degraded,
    Unverified,
    Stopped,
    Blocked,
    Unsupported,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IconColor {
    Green,
    Yellow,
    Red,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MenuItem {
    pub(crate) label: String,
    pub(crate) enabled: bool,
    pub(crate) command_id: Option<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PowerReconciliation {
    Acquire,
    ReleaseBlocked,
    ShowStopped,
    PreserveOwned,
    PreserveUncertain,
}

pub(crate) fn power_reconciliation(
    automatic: bool,
    power: PowerState,
    ownership: OwnershipState,
) -> PowerReconciliation {
    match power {
        PowerState::Ac if automatic && ownership == OwnershipState::Released => {
            PowerReconciliation::Acquire
        }
        PowerState::Ac if ownership == OwnershipState::Owned => PowerReconciliation::PreserveOwned,
        PowerState::Ac if ownership == OwnershipState::Uncertain => {
            PowerReconciliation::PreserveUncertain
        }
        PowerState::Ac => PowerReconciliation::ShowStopped,
        PowerState::Battery | PowerState::BatterySaver | PowerState::Unknown => {
            PowerReconciliation::ReleaseBlocked
        }
    }
}

pub(crate) const fn lifecycle_status(status: TrayStatus, handoff_active: bool) -> TrayStatus {
    if handoff_active || matches!(status, TrayStatus::Pausing) {
        TrayStatus::Stopping
    } else {
        status
    }
}

pub(crate) const fn scheduled_lifecycle_status(
    status: TrayStatus,
    handoff_active: bool,
    scheduled: Option<DurationAction>,
) -> TrayStatus {
    let status = lifecycle_status(status, handoff_active);
    if handoff_active {
        return status;
    }
    match (status, scheduled) {
        (TrayStatus::Stopped, Some(DurationAction::Start)) => TrayStatus::ScheduledStart,
        (TrayStatus::Running, Some(DurationAction::Stop)) => TrayStatus::ScheduledStop,
        _ => status,
    }
}

impl TrayStatus {
    pub(crate) const fn icon_color(self) -> IconColor {
        match self {
            Self::Running => IconColor::Green,
            Self::Starting
            | Self::ScheduledStart
            | Self::Pausing
            | Self::Stopping
            | Self::ScheduledStop
            | Self::Paused
            | Self::Pending
            | Self::Degraded
            | Self::Unverified => IconColor::Yellow,
            Self::Stopped | Self::Blocked | Self::Unsupported | Self::Error => IconColor::Red,
        }
    }
}

pub(crate) const fn auto_start_label(enabled: bool) -> &'static str {
    if enabled {
        "Auto-start: On"
    } else {
        "Auto-start: Off"
    }
}

pub(crate) const fn automatic_label(enabled: bool) -> &'static str {
    if enabled {
        "Auto-time: On"
    } else {
        "Auto-time: Off"
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct TimingValues {
    pub(crate) requested: Option<Hns>,
    pub(crate) effective: Option<Hns>,
    pub(crate) external: bool,
    pub(crate) handoff_pending: bool,
    pub(crate) invalid_interval: bool,
    pub(crate) valid: bool,
}

impl TimingValues {
    pub(crate) fn from_snapshot(
        snapshot: TimingSnapshot,
        handoff_pending: bool,
        external: bool,
        invalid_interval: bool,
        valid: bool,
    ) -> Self {
        Self {
            requested: snapshot.requested,
            effective: snapshot.effective,
            external,
            handoff_pending,
            invalid_interval,
            valid,
        }
    }
}

/// Handoff policy poll interval, not a timer-resolution value.
pub(crate) const HANDOFF_POLL_INTERVAL_MS: u32 = 250;
/// Handoff policy observation budget, not a timer-resolution value.
pub(crate) const HANDOFF_MAX_POLLS: u8 = 12;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HandoffProgress {
    Pending,
    Completed,
    TimedOut,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HandoffTracker {
    boundary: Hns,
    released_status: TrayStatus,
    polls: u8,
    max_polls: u8,
}

impl HandoffTracker {
    pub(crate) const fn new(boundary: Hns, released_status: TrayStatus) -> Self {
        Self {
            boundary,
            released_status,
            polls: 0,
            max_polls: HANDOFF_MAX_POLLS,
        }
    }

    pub(crate) const fn boundary(self) -> Hns {
        self.boundary
    }

    #[allow(dead_code)]
    pub(crate) const fn released_status(self) -> TrayStatus {
        self.released_status
    }

    pub(crate) const fn polls(self) -> u8 {
        self.polls
    }

    pub(crate) fn observe(&mut self, effective: Option<Hns>) -> HandoffProgress {
        if effective.is_some_and(|value| value >= self.boundary) {
            return HandoffProgress::Completed;
        }
        self.polls = self.polls.saturating_add(1);
        if self.polls >= self.max_polls {
            HandoffProgress::TimedOut
        } else {
            HandoffProgress::Pending
        }
    }
}

pub(crate) fn release_needs_handoff(boundary: Hns, effective: Option<Hns>) -> bool {
    matches!(effective, Some(value) if value < boundary)
}

fn format_ms(value: Hns) -> String {
    value.format_milliseconds()
}

pub(crate) fn version_header() -> String {
    format!("True™ Tick v{}", env!("CARGO_PKG_VERSION"))
}

fn effective_timing_suffix(timing: TimingValues) -> String {
    if timing.valid && !timing.invalid_interval {
        timing.effective.map_or_else(
            || "Timing unknown".to_owned(),
            |value| format!("{} ms", format_ms(value)),
        )
    } else {
        "Timing unknown".to_owned()
    }
}

fn state_summary(
    status: TrayStatus,
    timing: TimingValues,
    scheduled: Option<ScheduledAction>,
    now: Instant,
) -> String {
    let timing_suffix = effective_timing_suffix(timing);
    let action_summary = match (status, scheduled) {
        (TrayStatus::ScheduledStart, Some(action)) => Some(format!(
            "Starting in {}",
            format_remaining_duration(action.remaining(now))
        )),
        (TrayStatus::ScheduledStop, Some(action)) => Some(format!(
            "Stopping in {}",
            format_remaining_duration(action.remaining(now))
        )),
        (TrayStatus::Paused, Some(action)) if action.action == DurationAction::Pause => {
            Some(format!(
                "Paused for {}",
                format_remaining_duration(action.remaining(now))
            ))
        }
        _ => None,
    };
    let state = match action_summary {
        Some(summary) => summary,
        None => match status {
            TrayStatus::Running => "Running".to_owned(),
            TrayStatus::Starting => "Starting".to_owned(),
            TrayStatus::ScheduledStart => "Starting".to_owned(),
            TrayStatus::Pausing => "Stopping".to_owned(),
            TrayStatus::Stopping if timing.handoff_pending => "Stopping, handoff".to_owned(),
            TrayStatus::ScheduledStop => "Stopping".to_owned(),
            TrayStatus::Stopping => "Stopping".to_owned(),
            TrayStatus::Paused => "Paused".to_owned(),
            TrayStatus::Stopped => "Stopped".to_owned(),
            TrayStatus::Error => return "True™ Tick: Error".to_owned(),
            TrayStatus::Pending
            | TrayStatus::Degraded
            | TrayStatus::Blocked
            | TrayStatus::Unsupported => return "True™ Tick: Warning".to_owned(),
            TrayStatus::Unverified => return "True™ Tick: Timing unknown".to_owned(),
        },
    };
    format!("True™ Tick: {state} · {timing_suffix}")
}

#[cfg(test)]
pub(crate) fn tooltip(status: TrayStatus, timing: TimingValues) -> String {
    tooltip_at(status, timing, None, Instant::now())
}

pub(crate) fn tooltip_at(
    status: TrayStatus,
    timing: TimingValues,
    scheduled: Option<ScheduledAction>,
    now: Instant,
) -> String {
    state_summary(status, timing, scheduled, now)
}

pub(crate) const LOGS_COMMAND_ID: usize = 1009;
pub(crate) const GITHUB_COMMAND_ID: usize = 1010;
pub(crate) const STATUS_STATE_COMMAND_ID: usize = 1030;
pub(crate) const STATUS_TIMING_COMMAND_ID: usize = 1031;
pub(crate) const STATUS_RUNNING_FOR_COMMAND_ID: usize = 1032;
pub(crate) const STATUS_NEXT_ACTION_COMMAND_ID: usize = 1033;
pub(crate) const STATUS_OWNERSHIP_COMMAND_ID: usize = 1034;
pub(crate) const START_IN_1_COMMAND_ID: usize = 1011;
pub(crate) const START_IN_5_COMMAND_ID: usize = 1012;
pub(crate) const START_IN_15_COMMAND_ID: usize = 1013;
pub(crate) const START_IN_30_COMMAND_ID: usize = 1014;
pub(crate) const START_IN_60_COMMAND_ID: usize = 1015;
pub(crate) const STOP_IN_1_COMMAND_ID: usize = 1016;
pub(crate) const STOP_IN_5_COMMAND_ID: usize = 1017;
pub(crate) const STOP_IN_15_COMMAND_ID: usize = 1018;
pub(crate) const STOP_IN_30_COMMAND_ID: usize = 1019;
pub(crate) const STOP_IN_60_COMMAND_ID: usize = 1020;
pub(crate) const PAUSE_FOR_5_COMMAND_ID: usize = 1021;
pub(crate) const PAUSE_FOR_15_COMMAND_ID: usize = 1022;
pub(crate) const PAUSE_FOR_30_COMMAND_ID: usize = 1023;
pub(crate) const PAUSE_FOR_60_COMMAND_ID: usize = 1024;
pub(crate) const CANCEL_SCHEDULED_COMMAND_ID: usize = 1025;
pub(crate) const CANCEL_PAUSE_COMMAND_ID: usize = 1026;
pub(crate) const DURATION_MENU_COMMAND_ID: usize = 1101;
pub(crate) const START_IN_MENU_COMMAND_ID: usize = 1102;
pub(crate) const STOP_IN_MENU_COMMAND_ID: usize = 1103;
pub(crate) const PAUSE_FOR_MENU_COMMAND_ID: usize = 1104;
pub(crate) const STATUS_MENU_COMMAND_ID: usize = 1105;
pub(crate) const GITHUB_URL: &str = "https://github.com/kaiiuen/true-tick";

pub(crate) const fn duration_command(
    command_id: usize,
) -> Option<(DurationAction, DurationChoice)> {
    match command_id {
        START_IN_1_COMMAND_ID => Some((DurationAction::Start, DurationChoice::OneMinute)),
        START_IN_5_COMMAND_ID => Some((DurationAction::Start, DurationChoice::FiveMinutes)),
        START_IN_15_COMMAND_ID => Some((DurationAction::Start, DurationChoice::FifteenMinutes)),
        START_IN_30_COMMAND_ID => Some((DurationAction::Start, DurationChoice::ThirtyMinutes)),
        START_IN_60_COMMAND_ID => Some((DurationAction::Start, DurationChoice::OneHour)),
        STOP_IN_1_COMMAND_ID => Some((DurationAction::Stop, DurationChoice::OneMinute)),
        STOP_IN_5_COMMAND_ID => Some((DurationAction::Stop, DurationChoice::FiveMinutes)),
        STOP_IN_15_COMMAND_ID => Some((DurationAction::Stop, DurationChoice::FifteenMinutes)),
        STOP_IN_30_COMMAND_ID => Some((DurationAction::Stop, DurationChoice::ThirtyMinutes)),
        STOP_IN_60_COMMAND_ID => Some((DurationAction::Stop, DurationChoice::OneHour)),
        PAUSE_FOR_5_COMMAND_ID => Some((DurationAction::Pause, DurationChoice::FiveMinutes)),
        PAUSE_FOR_15_COMMAND_ID => Some((DurationAction::Pause, DurationChoice::FifteenMinutes)),
        PAUSE_FOR_30_COMMAND_ID => Some((DurationAction::Pause, DurationChoice::ThirtyMinutes)),
        PAUSE_FOR_60_COMMAND_ID => Some((DurationAction::Pause, DurationChoice::OneHour)),
        _ => None,
    }
}

pub(crate) fn duration_choices(action: DurationAction) -> Vec<MenuItem> {
    let choices = match action {
        DurationAction::Start | DurationAction::Stop => DurationChoice::all().to_vec(),
        DurationAction::Pause => DurationChoice::pause_choices().to_vec(),
    };
    choices
        .into_iter()
        .map(|duration| MenuItem {
            label: duration.label().to_owned(),
            enabled: true,
            command_id: None,
        })
        .collect()
}

pub(crate) fn duration_menu_items(scheduled: Option<ScheduledAction>) -> Vec<MenuItem> {
    vec![
        MenuItem {
            label: "Start in >".to_owned(),
            enabled: true,
            command_id: None,
        },
        MenuItem {
            label: "Stop in >".to_owned(),
            enabled: true,
            command_id: None,
        },
        MenuItem {
            label: "Pause for >".to_owned(),
            enabled: true,
            command_id: None,
        },
        MenuItem {
            label: "Cancel scheduled action".to_owned(),
            enabled: scheduled.is_some_and(|action| action.action != DurationAction::Pause),
            command_id: Some(CANCEL_SCHEDULED_COMMAND_ID),
        },
        MenuItem {
            label: "Resume now".to_owned(),
            enabled: scheduled.is_some_and(|action| action.action == DurationAction::Pause),
            command_id: Some(CANCEL_PAUSE_COMMAND_ID),
        },
    ]
}

pub(crate) const fn menu_description(command_id: usize) -> Option<&'static str> {
    match command_id {
        1001 => Some("Request the best supported timing"),
        1002 => Some("Release True™ Tick timing"),
        1005 | 1006 => Some("Launch True™ Tick when you sign in"),
        1007 | 1008 => Some("Request timing automatically on AC power"),
        LOGS_COMMAND_ID => Some("Open status and session logs"),
        GITHUB_COMMAND_ID => Some("Open True™ Tick on GitHub"),
        DURATION_MENU_COMMAND_ID => Some("Schedule a bounded timing action"),
        START_IN_MENU_COMMAND_ID => Some("Schedule a future guarded acquire"),
        STOP_IN_MENU_COMMAND_ID => Some("Schedule a future guarded release"),
        PAUSE_FOR_MENU_COMMAND_ID => Some("Suppress acquisition for a fixed duration"),
        CANCEL_SCHEDULED_COMMAND_ID => Some("Clear the current scheduled action"),
        CANCEL_PAUSE_COMMAND_ID => Some("Resume timing and cancel the pause"),
        STATUS_MENU_COMMAND_ID => Some("View read-only lifecycle details"),
        STATUS_STATE_COMMAND_ID => Some("Current True Tick lifecycle state"),
        STATUS_TIMING_COMMAND_ID => Some("Latest verified effective timing observation"),
        STATUS_RUNNING_FOR_COMMAND_ID => Some("Elapsed time since verified running"),
        STATUS_NEXT_ACTION_COMMAND_ID => Some("Scheduled action and remaining time"),
        STATUS_OWNERSHIP_COMMAND_ID => Some("True Tick ownership versus external timing"),
        START_IN_1_COMMAND_ID => Some("Start in 1 minute"),
        START_IN_5_COMMAND_ID => Some("Start in 5 minutes"),
        START_IN_15_COMMAND_ID => Some("Start in 15 minutes"),
        START_IN_30_COMMAND_ID => Some("Start in 30 minutes"),
        START_IN_60_COMMAND_ID => Some("Start in 1 hour"),
        STOP_IN_1_COMMAND_ID => Some("Stop in 1 minute"),
        STOP_IN_5_COMMAND_ID => Some("Stop in 5 minutes"),
        STOP_IN_15_COMMAND_ID => Some("Stop in 15 minutes"),
        STOP_IN_30_COMMAND_ID => Some("Stop in 30 minutes"),
        STOP_IN_60_COMMAND_ID => Some("Stop in 1 hour"),
        PAUSE_FOR_5_COMMAND_ID => Some("Pause for 5 minutes"),
        PAUSE_FOR_15_COMMAND_ID => Some("Pause for 15 minutes"),
        PAUSE_FOR_30_COMMAND_ID => Some("Pause for 30 minutes"),
        PAUSE_FOR_60_COMMAND_ID => Some("Pause for 1 hour"),
        1004 => Some("Stop safely and quit"),
        _ => None,
    }
}

pub(crate) const fn dpi_to_icon_canvas(dpi: u32) -> i32 {
    let dpi = if dpi == 0 { 96 } else { dpi };
    if dpi <= 107 {
        16
    } else if dpi <= 132 {
        20
    } else if dpi <= 168 {
        24
    } else if dpi <= 288 {
        32
    } else {
        64
    }
}

pub(crate) const fn icon_pixel_color(status: TrayStatus) -> u32 {
    match status.icon_color() {
        IconColor::Green => 0x0000b000,
        IconColor::Yellow => 0x00d0d000,
        IconColor::Red => 0x00d00000,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TrayClickAction {
    OpenMenu,
}

pub(crate) const fn tray_click_action(notification: usize) -> Option<TrayClickAction> {
    match notification {
        0x0202 | 0x0205 => Some(TrayClickAction::OpenMenu),
        _ => None,
    }
}

pub(crate) const fn menu_action_keeps_open(command_id: usize) -> bool {
    matches!(
        command_id,
        1001 | 1002 | 1005
            ..=1008
                | START_IN_1_COMMAND_ID
                | START_IN_5_COMMAND_ID
                | START_IN_15_COMMAND_ID
                | START_IN_30_COMMAND_ID
                | START_IN_60_COMMAND_ID
                | STOP_IN_1_COMMAND_ID
                | STOP_IN_5_COMMAND_ID
                | STOP_IN_15_COMMAND_ID
                | STOP_IN_30_COMMAND_ID
                | STOP_IN_60_COMMAND_ID
                | PAUSE_FOR_5_COMMAND_ID
                | PAUSE_FOR_15_COMMAND_ID
                | PAUSE_FOR_30_COMMAND_ID
                | PAUSE_FOR_60_COMMAND_ID
                | CANCEL_SCHEDULED_COMMAND_ID
                | CANCEL_PAUSE_COMMAND_ID
    )
}

pub(crate) const fn tray_notification_opens_menu(notification: usize, menu_active: bool) -> bool {
    !menu_active
        && matches!(
            tray_click_action(notification),
            Some(TrayClickAction::OpenMenu)
        )
}

pub(crate) const fn menu_command_dispatch_allowed(
    menu_active: bool,
    from_popup_return: bool,
) -> bool {
    from_popup_return || !menu_active
}

#[allow(dead_code)]
pub(crate) const fn menu_command_is_enabled(
    command_id: usize,
    status: TrayStatus,
    startup_enabled: bool,
    automatic: bool,
) -> bool {
    menu_command_is_enabled_with_pause(command_id, status, startup_enabled, automatic, false)
}

pub(crate) const fn menu_command_is_enabled_with_pause(
    command_id: usize,
    status: TrayStatus,
    startup_enabled: bool,
    automatic: bool,
    schedule_active: bool,
) -> bool {
    match command_id {
        1001 => !matches!(
            status,
            TrayStatus::Running | TrayStatus::Starting | TrayStatus::Stopping | TrayStatus::Paused
        ),
        1002 => !matches!(
            status,
            TrayStatus::Stopped | TrayStatus::Pausing | TrayStatus::Stopping | TrayStatus::Paused
        ),
        1005 => !startup_enabled,
        1006 => startup_enabled,
        1007 => !automatic,
        1008 => automatic,
        LOGS_COMMAND_ID | GITHUB_COMMAND_ID | 1004 => true,
        START_IN_1_COMMAND_ID
        | START_IN_5_COMMAND_ID
        | START_IN_15_COMMAND_ID
        | START_IN_30_COMMAND_ID
        | START_IN_60_COMMAND_ID
        | STOP_IN_1_COMMAND_ID
        | STOP_IN_5_COMMAND_ID
        | STOP_IN_15_COMMAND_ID
        | STOP_IN_30_COMMAND_ID
        | STOP_IN_60_COMMAND_ID
        | PAUSE_FOR_5_COMMAND_ID
        | PAUSE_FOR_15_COMMAND_ID
        | PAUSE_FOR_30_COMMAND_ID
        | PAUSE_FOR_60_COMMAND_ID => true,
        CANCEL_SCHEDULED_COMMAND_ID | CANCEL_PAUSE_COMMAND_ID => schedule_active,
        _ => false,
    }
}

fn state_label(status: TrayStatus, timing: TimingValues) -> &'static str {
    match status {
        TrayStatus::Running => "Running",
        TrayStatus::Stopped => "Stopped",
        TrayStatus::Starting | TrayStatus::ScheduledStart => "Starting",
        TrayStatus::Stopping | TrayStatus::Pausing | TrayStatus::ScheduledStop => {
            if timing.handoff_pending {
                "Stopping, handoff"
            } else {
                "Stopping"
            }
        }
        TrayStatus::Paused => "Paused",
        TrayStatus::Blocked => "Warning",
        TrayStatus::Error => "Error",
        TrayStatus::Unsupported | TrayStatus::Pending | TrayStatus::Degraded => "Warning",
        TrayStatus::Unverified => "Timing unknown",
    }
}

fn state_menu_label(
    status: TrayStatus,
    timing: TimingValues,
    scheduled: Option<ScheduledAction>,
    now: Instant,
) -> String {
    match (status, scheduled) {
        (TrayStatus::ScheduledStart, Some(action)) => format!(
            "Starting in {}",
            format_remaining_duration(action.remaining(now))
        ),
        (TrayStatus::ScheduledStop, Some(action)) => format!(
            "Stopping in {}",
            format_remaining_duration(action.remaining(now))
        ),
        (TrayStatus::Paused, Some(action)) if action.action == DurationAction::Pause => format!(
            "Paused for {}",
            format_remaining_duration(action.remaining(now))
        ),
        _ => state_label(status, timing).to_owned(),
    }
}

fn format_duration(duration: Duration) -> String {
    format_duration_seconds(duration.as_secs())
}

pub(crate) fn remaining_display_seconds(duration: Duration) -> u64 {
    duration
        .as_secs()
        .saturating_add(u64::from(duration.subsec_nanos() != 0))
}

pub(crate) fn scheduled_display_key(
    action: ScheduledAction,
    now: Instant,
) -> (DurationAction, u64, u64) {
    (
        action.action,
        action.generation,
        remaining_display_seconds(action.remaining(now)),
    )
}

pub(crate) fn format_remaining_duration(duration: Duration) -> String {
    format_duration_seconds(remaining_display_seconds(duration))
}

fn format_duration_seconds(seconds: u64) -> String {
    let hours = seconds / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

fn effective_timing_label(timing: TimingValues) -> String {
    if timing.valid && !timing.invalid_interval {
        timing.effective.map_or_else(
            || "Timing unknown".to_owned(),
            |value| value.format_detailed(),
        )
    } else {
        "Timing unknown".to_owned()
    }
}

fn running_for_label(running_for: Option<Duration>) -> String {
    running_for.map_or_else(|| "Not running".to_owned(), format_duration)
}

fn next_action_label(scheduled: Option<ScheduledAction>, now: Instant) -> String {
    scheduled.map_or_else(
        || "none".to_owned(),
        |action| {
            format!(
                "{} in {}",
                action.action.label(),
                format_remaining_duration(action.remaining(now))
            )
        },
    )
}

fn ownership_label(ownership: OwnershipState, timing: TimingValues) -> &'static str {
    match ownership {
        OwnershipState::Owned => "True™ Tick",
        OwnershipState::Uncertain => "Uncertain",
        OwnershipState::Released if timing.external => "External timing",
        OwnershipState::Released => "Released",
    }
}

pub(crate) fn status_menu_items(
    status: TrayStatus,
    timing: TimingValues,
    ownership: OwnershipState,
    scheduled: Option<ScheduledAction>,
    running_for: Option<Duration>,
    now: Instant,
) -> Vec<MenuItem> {
    vec![
        MenuItem {
            label: format!(
                "State: {}",
                state_menu_label(status, timing, scheduled, now)
            ),
            enabled: false,
            command_id: Some(STATUS_STATE_COMMAND_ID),
        },
        MenuItem {
            label: format!("Timing: {}", effective_timing_label(timing)),
            enabled: false,
            command_id: Some(STATUS_TIMING_COMMAND_ID),
        },
        MenuItem {
            label: format!("Running for: {}", running_for_label(running_for)),
            enabled: false,
            command_id: Some(STATUS_RUNNING_FOR_COMMAND_ID),
        },
        MenuItem {
            label: format!("Next action: {}", next_action_label(scheduled, now)),
            enabled: false,
            command_id: Some(STATUS_NEXT_ACTION_COMMAND_ID),
        },
        MenuItem {
            label: format!("Ownership: {}", ownership_label(ownership, timing)),
            enabled: false,
            command_id: Some(STATUS_OWNERSHIP_COMMAND_ID),
        },
    ]
}

#[allow(dead_code)]
pub(crate) fn menu_items(
    status: TrayStatus,
    startup_enabled: bool,
    automatic: bool,
    timing: TimingValues,
) -> Vec<MenuItem> {
    menu_items_with_duration(status, startup_enabled, automatic, timing, false)
}

pub(crate) fn menu_items_with_duration(
    status: TrayStatus,
    startup_enabled: bool,
    automatic: bool,
    _timing: TimingValues,
    paused: bool,
) -> Vec<MenuItem> {
    vec![
        MenuItem {
            label: version_header(),
            enabled: true,
            command_id: None,
        },
        MenuItem {
            label: "Start".to_owned(),
            enabled: !paused && !matches!(status, TrayStatus::Running | TrayStatus::Starting),
            command_id: None,
        },
        MenuItem {
            label: "Stop".to_owned(),
            enabled: !paused
                && !matches!(
                    status,
                    TrayStatus::Stopped | TrayStatus::Pausing | TrayStatus::Stopping
                ),
            command_id: None,
        },
        MenuItem {
            label: "Schedule >".to_owned(),
            enabled: true,
            command_id: None,
        },
        MenuItem {
            label: auto_start_label(startup_enabled).to_owned(),
            enabled: true,
            command_id: None,
        },
        MenuItem {
            label: automatic_label(automatic).to_owned(),
            enabled: true,
            command_id: None,
        },
        MenuItem {
            label: "Status >".to_owned(),
            enabled: true,
            command_id: None,
        },
        MenuItem {
            label: "Logs".to_owned(),
            enabled: true,
            command_id: None,
        },
        MenuItem {
            label: "Quit".to_owned(),
            enabled: true,
            command_id: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pause::{CoordinatorTimerEvent, DurationCoordinator, ScheduleRequest};

    #[test]
    fn duration_menu_exposes_logical_fixed_submenus_and_all_choices() {
        let duration = duration_menu_items(None);
        assert_eq!(
            duration
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            [
                "Start in >",
                "Stop in >",
                "Pause for >",
                "Cancel scheduled action",
                "Resume now"
            ]
        );
        assert!(!duration[3].enabled);
        assert!(!duration[4].enabled);
        assert_eq!(
            duration_choices(DurationAction::Start)
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            [
                "1 minute",
                "5 minutes",
                "15 minutes",
                "30 minutes",
                "1 hour"
            ]
        );
        assert_eq!(
            duration_choices(DurationAction::Pause)
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            ["5 minutes", "15 minutes", "30 minutes", "1 hour"]
        );
        assert_eq!(
            duration_command(START_IN_1_COMMAND_ID),
            Some((DurationAction::Start, DurationChoice::OneMinute))
        );
        assert_eq!(
            duration_command(STOP_IN_60_COMMAND_ID),
            Some((DurationAction::Stop, DurationChoice::OneHour))
        );
        assert_eq!(
            duration_command(PAUSE_FOR_30_COMMAND_ID),
            Some((DurationAction::Pause, DurationChoice::ThirtyMinutes))
        );
        assert_eq!(duration_command(9999), None);
        let now = Instant::now();
        let paused = ScheduledAction {
            action: DurationAction::Pause,
            duration: DurationChoice::FiveMinutes,
            deadline: now,
            generation: 1,
        };
        let paused_items = duration_menu_items(Some(paused));
        assert!(!paused_items[3].enabled);
        assert!(paused_items[4].enabled);
    }

    #[test]
    fn primary_menu_has_schedule_submenu_and_start_is_disabled_while_paused() {
        let items = menu_items_with_duration(
            TrayStatus::Paused,
            false,
            false,
            TimingValues::default(),
            true,
        );
        let labels = items
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>();
        assert_eq!(labels[1..4], ["Start", "Stop", "Schedule >"]);
        assert!(!items[1].enabled);
        assert!(!menu_command_is_enabled_with_pause(
            1001,
            TrayStatus::Paused,
            false,
            false,
            true
        ));
    }

    #[test]
    fn root_menu_orders_nine_entries_with_schedule_after_stop() {
        let items = menu_items_with_duration(
            TrayStatus::Stopped,
            false,
            false,
            TimingValues::default(),
            false,
        );
        assert_eq!(items.len(), 9);
        let labels = items
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            labels,
            [
                version_header().as_str(),
                "Start",
                "Stop",
                "Schedule >",
                "Auto-start: Off",
                "Auto-time: Off",
                "Status >",
                "Logs",
                "Quit"
            ]
        );
        assert!(items[3].enabled);
    }

    #[test]
    fn schedule_menu_orders_five_entries_with_pause_before_cancel() {
        let items = duration_menu_items(None);
        assert_eq!(items.len(), 5);
        let labels = items
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            labels,
            [
                "Start in >",
                "Stop in >",
                "Pause for >",
                "Cancel scheduled action",
                "Resume now"
            ]
        );
        assert!(items[0].enabled);
        assert!(items[1].enabled);
        assert!(items[2].enabled);
        assert!(!items[3].enabled);
        assert!(!items[4].enabled);
    }

    #[test]
    fn paused_lifecycle_is_yellow_and_start_is_suppressed_by_command_policy() {
        assert_eq!(TrayStatus::Pausing.icon_color(), IconColor::Yellow);
        assert_eq!(TrayStatus::Paused.icon_color(), IconColor::Yellow);
        assert_eq!(
            lifecycle_status(TrayStatus::Pausing, false),
            TrayStatus::Stopping
        );
        assert_eq!(
            lifecycle_status(TrayStatus::Pausing, true),
            TrayStatus::Stopping
        );
        assert!(!menu_command_is_enabled_with_pause(
            1001,
            TrayStatus::Paused,
            false,
            false,
            true
        ));
        assert!(!menu_command_is_enabled_with_pause(
            1002,
            TrayStatus::Paused,
            false,
            false,
            true
        ));
        assert!(menu_command_is_enabled_with_pause(
            CANCEL_SCHEDULED_COMMAND_ID,
            TrayStatus::Paused,
            false,
            false,
            true
        ));
        assert_eq!(
            tooltip(TrayStatus::Paused, TimingValues::default()),
            "True™ Tick: Paused · Timing unknown"
        );
    }

    #[test]
    fn handoff_is_authoritatively_yellow_even_when_an_unrelated_error_is_reported() {
        assert_eq!(
            lifecycle_status(TrayStatus::Error, true),
            TrayStatus::Stopping
        );
        assert_eq!(
            lifecycle_status(TrayStatus::Blocked, true),
            TrayStatus::Stopping
        );
        assert_eq!(
            lifecycle_status(TrayStatus::Error, false),
            TrayStatus::Error
        );
        assert_eq!(TrayStatus::Stopping.icon_color(), IconColor::Yellow);
    }

    #[test]
    fn one_timing_snapshot_feeds_tooltip_and_menu_status() {
        let snapshot = TimingSnapshot {
            requested: Some(Hns::new(5_000)),
            selected: Some(Hns::new(5_000)),
            effective: Some(Hns::new(4_966)),
            minimum_interval: Some(Hns::new(156_250)),
            maximum_interval: Some(Hns::new(5_000)),
            raw_status: Some(0),
        };
        let timing = TimingValues::from_snapshot(snapshot, false, false, false, true);
        assert_eq!(
            tooltip(TrayStatus::Running, timing),
            "True™ Tick: Running · 0.4966 ms"
        );
        let status = status_menu_items(
            TrayStatus::Running,
            timing,
            OwnershipState::Owned,
            None,
            Some(Duration::from_secs(252)),
            Instant::now(),
        );
        assert_eq!(status[0].label, "State: Running");
        assert_eq!(status[1].label, "Timing: 0.4966 ms (4966 HNS)");
        assert_eq!(status[2].label, "Running for: 4m 12s");
    }

    #[test]
    fn remaining_duration_rounds_positive_fractional_seconds_up() {
        assert_eq!(format_remaining_duration(Duration::from_secs(300)), "5m 0s");
        assert_eq!(
            format_remaining_duration(Duration::from_millis(299_999)),
            "5m 0s"
        );
        assert_eq!(format_duration(Duration::from_millis(299_999)), "4m 59s");
        assert_eq!(remaining_display_seconds(Duration::from_millis(4_999)), 5);
        assert_eq!(remaining_display_seconds(Duration::from_secs(5)), 5);
        assert_eq!(remaining_display_seconds(Duration::ZERO), 0);
    }

    #[test]
    fn schedule_publication_key_changes_with_generation_bucket_cancellation_and_expiration() {
        let now = Instant::now();
        let action = ScheduledAction {
            action: DurationAction::Start,
            duration: DurationChoice::FiveMinutes,
            deadline: now + Duration::from_millis(2_001),
            generation: 4,
        };
        assert_eq!(
            scheduled_display_key(action, now),
            (DurationAction::Start, 4, 3)
        );
        assert_ne!(
            scheduled_display_key(action, now + Duration::from_millis(1_100)),
            scheduled_display_key(action, now)
        );
        let replacement = ScheduledAction {
            generation: 5,
            ..action
        };
        assert_ne!(
            scheduled_display_key(replacement, now),
            scheduled_display_key(action, now)
        );
        assert_eq!(
            scheduled_display_key(
                ScheduledAction {
                    deadline: now,
                    ..action
                },
                now,
            ),
            (DurationAction::Start, 4, 0)
        );
        let cancelled: Option<(DurationAction, u64, u64)> = None;
        assert_ne!(cancelled, Some(scheduled_display_key(action, now)));
    }

    #[test]
    fn live_menu_model_updates_remaining_action_and_enabled_states() {
        let now = Instant::now();
        let scheduled = ScheduledAction {
            action: DurationAction::Pause,
            duration: DurationChoice::FiveMinutes,
            deadline: now + Duration::from_millis(299_999),
            generation: 4,
        };
        let status = status_menu_items(
            TrayStatus::Paused,
            TimingValues::default(),
            OwnershipState::Released,
            Some(scheduled),
            None,
            now,
        );
        assert_eq!(status[0].label, "State: Paused for 5m 0s");
        assert_eq!(status[3].label, "Next action: pause in 5m 0s");
        let paused_menu = menu_items_with_duration(
            TrayStatus::Paused,
            false,
            false,
            TimingValues::default(),
            true,
        );
        assert!(!paused_menu[1].enabled);
        assert!(!paused_menu[2].enabled);
        assert!(paused_menu[3].enabled);
        assert!(paused_menu[7].enabled);
    }

    #[test]
    fn status_submenu_uses_unknown_for_invalid_timing_and_exposes_the_next_action() {
        let now = Instant::now();
        let scheduled = ScheduledAction {
            action: DurationAction::Stop,
            duration: DurationChoice::FiveMinutes,
            deadline: now + Duration::from_secs(252),
            generation: 7,
        };
        let status = status_menu_items(
            TrayStatus::Blocked,
            TimingValues {
                effective: Some(Hns::new(4_966)),
                invalid_interval: true,
                ..TimingValues::default()
            },
            OwnershipState::Released,
            Some(scheduled),
            None,
            now,
        );
        assert_eq!(status[0].label, "State: Warning");
        assert_eq!(status[1].label, "Timing: Timing unknown");
        assert_eq!(status[2].label, "Running for: Not running");
        assert_eq!(status[3].label, "Next action: stop in 4m 12s");
        assert_eq!(status[4].label, "Ownership: Released");
        assert!(status.iter().all(|item| !item.enabled));
        assert_eq!(
            status
                .iter()
                .map(|item| item.command_id)
                .collect::<Vec<_>>(),
            vec![
                Some(STATUS_STATE_COMMAND_ID),
                Some(STATUS_TIMING_COMMAND_ID),
                Some(STATUS_RUNNING_FOR_COMMAND_ID),
                Some(STATUS_NEXT_ACTION_COMMAND_ID),
                Some(STATUS_OWNERSHIP_COMMAND_ID),
            ]
        );
        assert!(status.iter().all(|item| {
            item.command_id.is_some_and(|command| {
                !menu_command_is_enabled_with_pause(
                    command,
                    TrayStatus::Stopped,
                    false,
                    false,
                    false,
                )
            })
        }));
    }

    #[test]
    fn version_header_uses_the_package_version() {
        assert_eq!(
            version_header(),
            "True™ Tick v".to_owned() + env!("CARGO_PKG_VERSION")
        );
    }

    #[test]
    fn logs_command_is_separate_from_read_only_status() {
        assert_eq!(LOGS_COMMAND_ID, 1009);
        assert_eq!(GITHUB_COMMAND_ID, 1010);
        assert_eq!(GITHUB_URL, "https://github.com/kaiiuen/true-tick");
        let items = status_menu_items(
            TrayStatus::Stopped,
            TimingValues::default(),
            OwnershipState::Released,
            None,
            None,
            Instant::now(),
        );
        assert_eq!(items.len(), 5);
        assert!(items.iter().all(|item| !item.enabled));
    }

    #[test]
    fn every_menu_command_has_its_short_hover_description() {
        let descriptions = [
            (1001, "Request the best supported timing"),
            (1002, "Release True™ Tick timing"),
            (1005, "Launch True™ Tick when you sign in"),
            (1006, "Launch True™ Tick when you sign in"),
            (1007, "Request timing automatically on AC power"),
            (1008, "Request timing automatically on AC power"),
            (LOGS_COMMAND_ID, "Open status and session logs"),
            (GITHUB_COMMAND_ID, "Open True™ Tick on GitHub"),
            (DURATION_MENU_COMMAND_ID, "Schedule a bounded timing action"),
            (
                START_IN_MENU_COMMAND_ID,
                "Schedule a future guarded acquire",
            ),
            (STOP_IN_MENU_COMMAND_ID, "Schedule a future guarded release"),
            (
                PAUSE_FOR_MENU_COMMAND_ID,
                "Suppress acquisition for a fixed duration",
            ),
            (
                CANCEL_SCHEDULED_COMMAND_ID,
                "Clear the current scheduled action",
            ),
            (STATUS_MENU_COMMAND_ID, "View read-only lifecycle details"),
            (STATUS_STATE_COMMAND_ID, "Current True Tick lifecycle state"),
            (
                STATUS_TIMING_COMMAND_ID,
                "Latest verified effective timing observation",
            ),
            (
                STATUS_RUNNING_FOR_COMMAND_ID,
                "Elapsed time since verified running",
            ),
            (
                STATUS_NEXT_ACTION_COMMAND_ID,
                "Scheduled action and remaining time",
            ),
            (
                STATUS_OWNERSHIP_COMMAND_ID,
                "True Tick ownership versus external timing",
            ),
            (1004, "Stop safely and quit"),
        ];
        for (command_id, expected) in descriptions {
            assert_eq!(menu_description(command_id), Some(expected));
        }
        assert_eq!(menu_description(9999), None);
    }

    #[test]
    fn left_and_right_button_release_open_one_menu_event() {
        assert_eq!(tray_click_action(0x0202), Some(TrayClickAction::OpenMenu));
        assert_eq!(tray_click_action(0x0205), Some(TrayClickAction::OpenMenu));
        for notification in [0x0201, 0x0204, 0x0203, 0x0206, 0x0000_0000] {
            assert_eq!(tray_click_action(notification), None);
        }
    }

    #[test]
    fn dpi_maps_to_requested_canvas_sizes_and_clamps_intermediate_values() {
        for (dpi, canvas) in [(96, 16), (120, 20), (144, 24), (192, 32), (384, 64)] {
            assert_eq!(dpi_to_icon_canvas(dpi), canvas);
        }
        assert_eq!(dpi_to_icon_canvas(0), 16);
        assert_eq!(dpi_to_icon_canvas(108), 20);
        assert_eq!(dpi_to_icon_canvas(168), 24);
        assert_eq!(dpi_to_icon_canvas(175), 32);
        assert_eq!(dpi_to_icon_canvas(288), 32);
        assert_eq!(dpi_to_icon_canvas(289), 64);
    }

    #[test]
    fn stopped_status_omits_unknown_timing_from_the_hover_text() {
        assert_eq!(
            tooltip(TrayStatus::Stopped, TimingValues::default()),
            "True™ Tick: Stopped · Timing unknown"
        );
    }

    #[test]
    fn stopped_finer_observation_is_labeled_external() {
        assert_eq!(
            tooltip(
                TrayStatus::Stopped,
                TimingValues {
                    requested: Some(Hns::new(5_000)),
                    effective: Some(Hns::new(4_966)),
                    external: true,
                    handoff_pending: false,
                    invalid_interval: false,
                    valid: true,
                },
            ),
            "True™ Tick: Stopped · 0.4966 ms"
        );
    }

    #[test]
    fn auto_time_off_on_ac_without_ownership_shows_stopped() {
        assert_eq!(
            power_reconciliation(false, PowerState::Ac, OwnershipState::Released),
            PowerReconciliation::ShowStopped
        );
    }

    #[test]
    fn auto_time_off_battery_to_ac_waits_for_manual_start() {
        assert_eq!(
            power_reconciliation(false, PowerState::Battery, OwnershipState::Released),
            PowerReconciliation::ReleaseBlocked
        );
        assert_eq!(
            power_reconciliation(false, PowerState::Ac, OwnershipState::Released),
            PowerReconciliation::ShowStopped
        );
    }

    #[test]
    fn auto_time_on_battery_to_ac_acquires_when_released() {
        assert_eq!(
            power_reconciliation(true, PowerState::Battery, OwnershipState::Released),
            PowerReconciliation::ReleaseBlocked
        );
        assert_eq!(
            power_reconciliation(true, PowerState::Ac, OwnershipState::Released),
            PowerReconciliation::Acquire
        );
    }

    #[test]
    fn owned_ac_timing_is_preserved_when_auto_time_is_off() {
        assert_eq!(
            power_reconciliation(false, PowerState::Ac, OwnershipState::Owned),
            PowerReconciliation::PreserveOwned
        );
    }

    #[test]
    fn unknown_power_releases_conservatively() {
        assert_eq!(
            power_reconciliation(true, PowerState::Unknown, OwnershipState::Owned),
            PowerReconciliation::ReleaseBlocked
        );
    }

    #[test]
    fn status_colors_are_stable_for_native_icon_pixels() {
        assert_eq!(icon_pixel_color(TrayStatus::Running), 0x0000b000);
        for status in [
            TrayStatus::Starting,
            TrayStatus::Stopping,
            TrayStatus::Pending,
            TrayStatus::Degraded,
            TrayStatus::Unverified,
        ] {
            assert_eq!(icon_pixel_color(status), 0x00d0d000);
        }
        for status in [
            TrayStatus::Stopped,
            TrayStatus::Blocked,
            TrayStatus::Unsupported,
            TrayStatus::Error,
        ] {
            assert_eq!(icon_pixel_color(status), 0x00d00000);
        }
    }

    #[test]
    fn start_stop_and_toggle_actions_keep_the_native_menu_open() {
        for command in [1001, 1002, 1005, 1006, 1007, 1008] {
            assert!(menu_action_keeps_open(command));
        }
        for command in [LOGS_COMMAND_ID, GITHUB_COMMAND_ID, 1004] {
            assert!(!menu_action_keeps_open(command));
        }
    }

    #[test]
    fn notification_bursts_open_only_one_menu() {
        let mut menu_active = false;
        let mut opened = 0;
        for notification in [0x0202, 0x0205, 0x0202, 0x0205] {
            if tray_notification_opens_menu(notification, menu_active) {
                opened += 1;
                menu_active = true;
            }
        }
        assert_eq!(opened, 1);
        assert!(!tray_notification_opens_menu(0x0205, true));
        assert!(!tray_notification_opens_menu(0x0202, true));
    }

    #[test]
    fn commands_during_menu_activity_are_ignored_except_popup_return() {
        assert!(!menu_command_dispatch_allowed(true, false));
        assert!(menu_command_dispatch_allowed(true, true));
        assert!(menu_command_dispatch_allowed(false, false));
    }

    #[test]
    fn cancellation_remains_enabled_for_each_schedule_while_menu_is_open() {
        for status in [
            TrayStatus::ScheduledStart,
            TrayStatus::ScheduledStop,
            TrayStatus::Paused,
        ] {
            assert!(menu_command_is_enabled_with_pause(
                CANCEL_SCHEDULED_COMMAND_ID,
                status,
                false,
                false,
                true,
            ));
        }
        assert!(menu_action_keeps_open(CANCEL_SCHEDULED_COMMAND_ID));
        assert!(!menu_command_is_enabled_with_pause(
            CANCEL_SCHEDULED_COMMAND_ID,
            TrayStatus::Stopped,
            false,
            false,
            false,
        ));
    }

    #[test]
    fn quit_is_valid_during_menu_activity_and_closes_the_loop() {
        assert!(menu_command_dispatch_allowed(true, true));
        assert!(menu_command_is_enabled(
            1004,
            TrayStatus::Running,
            true,
            true
        ));
        assert!(!menu_action_keeps_open(1004));
    }

    #[test]
    fn command_validation_rejects_stale_and_unknown_commands() {
        assert!(!menu_command_is_enabled(
            1001,
            TrayStatus::Running,
            true,
            false
        ));
        assert!(!menu_command_is_enabled(
            1006,
            TrayStatus::Stopped,
            false,
            false
        ));
        assert!(!menu_command_is_enabled(
            1008,
            TrayStatus::Stopped,
            false,
            false
        ));
        assert!(!menu_command_is_enabled(
            9999,
            TrayStatus::Stopped,
            false,
            false
        ));
        assert!(menu_command_is_enabled(
            1001,
            TrayStatus::Stopped,
            false,
            false
        ));
        assert!(menu_command_is_enabled(
            1006,
            TrayStatus::Stopped,
            true,
            false
        ));
        assert!(menu_command_is_enabled(
            1008,
            TrayStatus::Stopped,
            false,
            true
        ));
    }

    #[test]
    fn repeated_commands_follow_the_current_state_without_duplicate_actions() {
        let mut status = TrayStatus::Stopped;
        assert!(menu_command_is_enabled(1001, status, false, false));
        status = TrayStatus::Running;
        assert!(!menu_command_is_enabled(1001, status, false, false));
        assert!(menu_command_is_enabled(1002, status, false, false));
        status = TrayStatus::Stopped;
        assert!(!menu_command_is_enabled(1002, status, false, false));

        let mut startup_enabled = false;
        assert!(menu_command_is_enabled(
            1005,
            status,
            startup_enabled,
            false
        ));
        startup_enabled = true;
        assert!(!menu_command_is_enabled(
            1005,
            status,
            startup_enabled,
            false
        ));
        assert!(menu_command_is_enabled(
            1006,
            status,
            startup_enabled,
            false
        ));

        let mut automatic = false;
        assert!(menu_command_is_enabled(
            1007,
            status,
            startup_enabled,
            automatic
        ));
        automatic = true;
        assert!(!menu_command_is_enabled(
            1007,
            status,
            startup_enabled,
            automatic
        ));
        assert!(menu_command_is_enabled(
            1008,
            status,
            startup_enabled,
            automatic
        ));
    }

    #[test]
    fn compact_menu_has_the_expected_order_and_clickability() {
        let items = menu_items(
            TrayStatus::Running,
            true,
            false,
            TimingValues {
                effective: Some(Hns::new(4_966)),
                ..TimingValues::default()
            },
        );
        assert_eq!(
            items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            [
                version_header().as_str(),
                "Start",
                "Stop",
                "Schedule >",
                "Auto-start: On",
                "Auto-time: Off",
                "Status >",
                "Logs",
                "Quit"
            ]
        );
        assert!(items[0].enabled);
        assert!(items[3].enabled);
        assert!(!items[1].enabled);
        assert!(items[6].enabled);
        assert!(items[8].enabled);
    }

    #[test]
    fn tooltip_strings_use_verified_or_requested_timing() {
        let exact = TimingValues {
            requested: Some(Hns::new(5_000)),
            effective: Some(Hns::new(5_000)),
            external: false,
            handoff_pending: false,
            invalid_interval: false,
            valid: true,
        };
        let finer = TimingValues {
            requested: Some(Hns::new(5_000)),
            effective: Some(Hns::new(4_966)),
            external: false,
            handoff_pending: false,
            invalid_interval: false,
            valid: true,
        };
        assert_eq!(
            tooltip(TrayStatus::Running, exact),
            "True™ Tick: Running · 0.5000 ms"
        );
        assert_eq!(
            tooltip(TrayStatus::Running, finer),
            "True™ Tick: Running · 0.4966 ms"
        );
        assert_eq!(
            tooltip(TrayStatus::Stopped, finer),
            "True™ Tick: Stopped · 0.4966 ms"
        );
        assert_eq!(
            tooltip(
                TrayStatus::Starting,
                TimingValues {
                    requested: Some(Hns::new(5_000)),
                    ..TimingValues::default()
                }
            ),
            "True™ Tick: Starting · Timing unknown"
        );
        assert_eq!(
            tooltip(TrayStatus::Stopping, finer),
            "True™ Tick: Stopping · 0.4966 ms"
        );
        assert_eq!(
            tooltip(
                TrayStatus::Stopping,
                TimingValues {
                    effective: Some(Hns::new(4_966)),
                    handoff_pending: true,
                    valid: true,
                    ..TimingValues::default()
                }
            ),
            "True™ Tick: Stopping, handoff · 0.4966 ms"
        );
        assert_eq!(
            tooltip(
                TrayStatus::Stopped,
                TimingValues {
                    external: true,
                    effective: Some(Hns::new(4_966)),
                    valid: true,
                    ..TimingValues::default()
                }
            ),
            "True™ Tick: Stopped · 0.4966 ms"
        );
        assert_eq!(
            tooltip(
                TrayStatus::Error,
                TimingValues {
                    invalid_interval: true,
                    ..TimingValues::default()
                }
            ),
            "True™ Tick: Error"
        );
    }

    #[test]
    fn exact_tooltip_contract_covers_running_stopped_and_transitions() {
        let now = Instant::now();
        let timing = TimingValues {
            effective: Some(Hns::new(4_966)),
            valid: true,
            ..TimingValues::default()
        };
        assert_eq!(
            tooltip_at(TrayStatus::Running, timing, None, now),
            "True™ Tick: Running · 0.4966 ms"
        );
        assert_eq!(
            tooltip_at(TrayStatus::Stopped, timing, None, now),
            "True™ Tick: Stopped · 0.4966 ms"
        );
        assert_eq!(
            tooltip_at(TrayStatus::Starting, timing, None, now),
            "True™ Tick: Starting · 0.4966 ms"
        );
        assert_eq!(
            tooltip_at(TrayStatus::Stopping, timing, None, now),
            "True™ Tick: Stopping · 0.4966 ms"
        );
        assert_eq!(
            tooltip_at(
                TrayStatus::Stopping,
                TimingValues {
                    handoff_pending: true,
                    ..timing
                },
                None,
                now,
            ),
            "True™ Tick: Stopping, handoff · 0.4966 ms"
        );
    }

    #[test]
    fn scheduled_and_paused_tooltips_use_the_live_monotonic_deadline() {
        let now = Instant::now();
        let timing = TimingValues {
            effective: Some(Hns::new(9_966)),
            valid: true,
            ..TimingValues::default()
        };
        let start = ScheduledAction {
            action: DurationAction::Start,
            duration: DurationChoice::FiveMinutes,
            deadline: now + Duration::from_millis(299_999),
            generation: 3,
        };
        let stop = ScheduledAction {
            action: DurationAction::Stop,
            ..start
        };
        let pause = ScheduledAction {
            action: DurationAction::Pause,
            ..start
        };
        assert_eq!(
            tooltip_at(TrayStatus::ScheduledStart, timing, Some(start), now),
            "True™ Tick: Starting in 5m 0s · 0.9966 ms"
        );
        assert_eq!(
            tooltip_at(TrayStatus::ScheduledStop, timing, Some(stop), now),
            "True™ Tick: Stopping in 5m 0s · 0.9966 ms"
        );
        assert_eq!(
            tooltip_at(TrayStatus::Paused, timing, Some(pause), now),
            "True™ Tick: Paused for 5m 0s · 0.9966 ms"
        );
        assert_eq!(
            tooltip_at(TrayStatus::Paused, timing, None, now),
            "True™ Tick: Paused · 0.9966 ms"
        );
    }

    #[test]
    fn schedule_replacement_and_expiration_derive_the_same_lifecycle_state() {
        let now = Instant::now();
        let mut coordinator = DurationCoordinator::new();
        let first = coordinator.schedule(DurationAction::Start, DurationChoice::FiveMinutes, now);
        assert!(matches!(first, ScheduleRequest::Started(_)));
        assert_eq!(
            scheduled_lifecycle_status(TrayStatus::Stopped, false, Some(DurationAction::Start)),
            TrayStatus::ScheduledStart
        );
        let replaced = coordinator.schedule(
            DurationAction::Pause,
            DurationChoice::FiveMinutes,
            now + Duration::from_secs(1),
        );
        assert!(matches!(replaced, ScheduleRequest::Replaced { .. }));
        assert!(coordinator.pause_active());
        assert_eq!(
            coordinator.timer_event(coordinator.generation(), now + Duration::from_secs(301)),
            CoordinatorTimerEvent::Expired(coordinator.current().unwrap())
        );
        assert_eq!(
            scheduled_lifecycle_status(TrayStatus::Paused, false, Some(DurationAction::Pause)),
            TrayStatus::Paused
        );
        assert_eq!(
            scheduled_lifecycle_status(TrayStatus::Running, false, Some(DurationAction::Stop)),
            TrayStatus::ScheduledStop
        );
    }

    #[test]
    fn unknown_and_external_timing_remain_concise_and_never_use_requested_timing() {
        let now = Instant::now();
        assert_eq!(
            tooltip_at(
                TrayStatus::Stopped,
                TimingValues {
                    requested: Some(Hns::new(5_000)),
                    ..TimingValues::default()
                },
                None,
                now,
            ),
            "True™ Tick: Stopped · Timing unknown"
        );
        assert_eq!(
            tooltip_at(TrayStatus::Unverified, TimingValues::default(), None, now),
            "True™ Tick: Timing unknown"
        );
        assert_eq!(
            tooltip_at(
                TrayStatus::Stopped,
                TimingValues {
                    requested: Some(Hns::new(5_000)),
                    effective: Some(Hns::new(4_966)),
                    external: true,
                    valid: true,
                    ..TimingValues::default()
                },
                None,
                now,
            ),
            "True™ Tick: Stopped · 0.4966 ms"
        );
        assert_eq!(
            tooltip_at(TrayStatus::Error, TimingValues::default(), None, now),
            "True™ Tick: Error"
        );
        assert_eq!(
            tooltip_at(TrayStatus::Degraded, TimingValues::default(), None, now),
            "True™ Tick: Warning"
        );
    }

    #[test]
    fn scheduled_and_transition_states_are_yellow_while_verified_running_is_green() {
        for status in [
            TrayStatus::Starting,
            TrayStatus::ScheduledStart,
            TrayStatus::Stopping,
            TrayStatus::ScheduledStop,
            TrayStatus::Paused,
            TrayStatus::Unverified,
        ] {
            assert_eq!(status.icon_color(), IconColor::Yellow);
        }
        assert_eq!(TrayStatus::Running.icon_color(), IconColor::Green);
        for status in [TrayStatus::Stopped, TrayStatus::Blocked, TrayStatus::Error] {
            assert_eq!(status.icon_color(), IconColor::Red);
        }
    }

    #[test]
    fn shortened_tooltips_keep_branding_without_full_reports() {
        for status in [
            TrayStatus::Running,
            TrayStatus::Stopped,
            TrayStatus::Starting,
            TrayStatus::Stopping,
            TrayStatus::Pending,
            TrayStatus::Error,
        ] {
            let text = tooltip(status, TimingValues::default());
            assert!(text.starts_with("True™ Tick: "));
            assert!(!text.contains("HNS"));
            assert!(!text.contains('\\'));
            assert!(text.len() < 64);
        }
        assert_eq!(
            tooltip(TrayStatus::Pending, TimingValues::default()),
            "True™ Tick: Warning"
        );
    }

    #[test]
    fn timing_format_rounds_hns_without_exposing_raw_units() {
        assert_eq!(format_ms(Hns::new(4_966)), "0.4966");
        assert_eq!(format_ms(Hns::new(5_000)), "0.5000");
        assert_eq!(format_ms(Hns::new(156_250)), "15.6250");
    }

    #[test]
    fn release_to_baseline_does_not_start_handoff() {
        assert!(!release_needs_handoff(
            Hns::new(5_000),
            Some(Hns::new(5_000))
        ));
        let mut tracker = HandoffTracker::new(Hns::new(5_000), TrayStatus::Stopped);
        assert_eq!(
            tracker.observe(Some(Hns::new(5_000))),
            HandoffProgress::Completed
        );
        assert_eq!(tracker.polls(), 0);
    }

    #[test]
    fn finer_external_client_starts_a_pending_handoff() {
        assert!(release_needs_handoff(
            Hns::new(5_000),
            Some(Hns::new(4_966))
        ));
        let mut tracker = HandoffTracker::new(Hns::new(5_000), TrayStatus::Stopped);
        assert_eq!(
            tracker.observe(Some(Hns::new(4_966))),
            HandoffProgress::Pending
        );
        assert_eq!(tracker.polls(), 1);
    }

    #[test]
    fn handoff_completes_when_effective_timing_reaches_boundary() {
        let mut tracker = HandoffTracker::new(Hns::new(5_000), TrayStatus::Stopped);
        assert_eq!(
            tracker.observe(Some(Hns::new(4_966))),
            HandoffProgress::Pending
        );
        assert_eq!(
            tracker.observe(Some(Hns::new(9_966))),
            HandoffProgress::Completed
        );
    }

    #[test]
    fn handoff_times_out_without_claiming_completion() {
        let mut tracker = HandoffTracker::new(Hns::new(5_000), TrayStatus::Stopped);
        for _ in 0..HANDOFF_MAX_POLLS.saturating_sub(1) {
            assert_eq!(
                tracker.observe(Some(Hns::new(4_966))),
                HandoffProgress::Pending
            );
        }
        assert_eq!(
            tracker.observe(Some(Hns::new(4_966))),
            HandoffProgress::TimedOut
        );
        assert_eq!(tracker.polls(), HANDOFF_MAX_POLLS);
    }

    #[test]
    fn repeated_stop_is_disabled_while_handoff_is_pending() {
        assert!(!menu_command_is_enabled(
            1002,
            TrayStatus::Stopping,
            false,
            false
        ));
        assert!(!menu_command_is_enabled(
            1001,
            TrayStatus::Stopping,
            false,
            false
        ));
    }

    #[test]
    fn transition_and_degraded_states_are_yellow() {
        for status in [
            TrayStatus::Starting,
            TrayStatus::Stopping,
            TrayStatus::Pending,
            TrayStatus::Degraded,
            TrayStatus::Unverified,
        ] {
            assert_eq!(status.icon_color(), IconColor::Yellow);
        }
    }

    #[test]
    fn status_maps_to_truthful_icon_color() {
        assert_eq!(TrayStatus::Running.icon_color(), IconColor::Green);
        for status in [
            TrayStatus::Stopped,
            TrayStatus::Blocked,
            TrayStatus::Unsupported,
            TrayStatus::Error,
        ] {
            assert_eq!(status.icon_color(), IconColor::Red);
        }
    }
}
