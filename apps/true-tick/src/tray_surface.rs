use tick_core::Hns;
use tick_ownership::OwnershipState;
use tick_policy::PowerState;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TrayStatus {
    Running,
    Starting,
    Stopping,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MenuItem {
    pub(crate) label: &'static str,
    pub(crate) enabled: bool,
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

impl TrayStatus {
    pub(crate) const fn icon_color(self) -> IconColor {
        match self {
            Self::Running => IconColor::Green,
            Self::Starting | Self::Stopping | Self::Pending | Self::Degraded | Self::Unverified => {
                IconColor::Yellow
            }
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
}

pub(crate) const HANDOFF_POLL_INTERVAL_MS: u32 = 250;
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
    let thousandths = value.value().saturating_add(5) / 10;
    format!("{}.{:03}", thousandths / 1_000, thousandths % 1_000)
}

fn current_label(prefix: &str, effective: Option<Hns>, external: bool) -> String {
    effective.map_or_else(
        || {
            if prefix == "Stopped" {
                format!("{prefix} (timing unknown)")
            } else {
                format!("{prefix} (unknown)")
            }
        },
        |value| {
            if external {
                format!("{prefix} (external: {} ms)", format_ms(value))
            } else {
                format!("{prefix} (current: {} ms)", format_ms(value))
            }
        },
    )
}

pub(crate) fn tooltip(status: TrayStatus, timing: TimingValues) -> String {
    let summary = match status {
        TrayStatus::Running => match (timing.requested, timing.effective) {
            (Some(requested), Some(effective)) if effective < requested => {
                format!("Running ({} ms, finer)", format_ms(effective))
            }
            (_, Some(effective)) => format!("Running ({} ms)", format_ms(effective)),
            (_, None) => "Running (unknown)".to_owned(),
        },
        TrayStatus::Stopped => current_label("Stopped", timing.effective, timing.external),
        TrayStatus::Blocked => current_label("Blocked", timing.effective, timing.external),
        TrayStatus::Unsupported => "Unsupported (timing unavailable)".to_owned(),
        TrayStatus::Error if timing.invalid_interval => "Error (invalid interval)".to_owned(),
        TrayStatus::Error => current_label("Error", timing.effective, timing.external),
        TrayStatus::Starting => timing.requested.map_or_else(
            || "Starting (unknown)".to_owned(),
            |requested| format!("Starting ({} ms)", format_ms(requested)),
        ),
        TrayStatus::Stopping if timing.handoff_pending => {
            "Stopping (waiting for handoff)".to_owned()
        }
        TrayStatus::Stopping => current_label("Stopping", timing.effective, timing.external),
        TrayStatus::Pending => "Pending (timing unknown)".to_owned(),
        TrayStatus::Degraded => current_label("Degraded", timing.effective, timing.external),
        TrayStatus::Unverified => current_label("Unverified", timing.effective, timing.external),
    };
    format!("True™ Tick: {summary}")
}

pub(crate) const STATUS_COMMAND_ID: usize = 1009;

pub(crate) const fn menu_description(command_id: usize) -> Option<&'static str> {
    match command_id {
        1001 => Some("Request the best supported timing"),
        1002 => Some("Release True Tick timing"),
        1005 | 1006 => Some("Launch True Tick when you sign in"),
        1007 | 1008 => Some("Request timing automatically on AC power"),
        STATUS_COMMAND_ID => Some("Open status and session logs"),
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
    matches!(command_id, 1001 | 1002 | 1005..=1008)
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

pub(crate) const fn menu_command_is_enabled(
    command_id: usize,
    status: TrayStatus,
    startup_enabled: bool,
    automatic: bool,
) -> bool {
    match command_id {
        1001 => !matches!(
            status,
            TrayStatus::Running | TrayStatus::Starting | TrayStatus::Stopping
        ),
        1002 => !matches!(status, TrayStatus::Stopped | TrayStatus::Stopping),
        1005 => !startup_enabled,
        1006 => startup_enabled,
        1007 => !automatic,
        1008 => automatic,
        STATUS_COMMAND_ID | 1004 => true,
        _ => false,
    }
}

pub(crate) fn menu_items(
    status: TrayStatus,
    startup_enabled: bool,
    automatic: bool,
) -> [MenuItem; 6] {
    [
        MenuItem {
            label: "Start",
            enabled: !matches!(status, TrayStatus::Running | TrayStatus::Starting),
        },
        MenuItem {
            label: "Stop",
            enabled: !matches!(status, TrayStatus::Stopped | TrayStatus::Stopping),
        },
        MenuItem {
            label: auto_start_label(startup_enabled),
            enabled: true,
        },
        MenuItem {
            label: automatic_label(automatic),
            enabled: true,
        },
        MenuItem {
            label: "Status",
            enabled: true,
        },
        MenuItem {
            label: "Quit",
            enabled: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_row_maps_to_diagnostic_command() {
        assert_eq!(STATUS_COMMAND_ID, 1009);
        assert!(menu_items(TrayStatus::Stopped, false, false)[4].enabled);
    }

    #[test]
    fn every_menu_command_has_its_short_hover_description() {
        let descriptions = [
            (1001, "Request the best supported timing"),
            (1002, "Release True Tick timing"),
            (1005, "Launch True Tick when you sign in"),
            (1006, "Launch True Tick when you sign in"),
            (1007, "Request timing automatically on AC power"),
            (1008, "Request timing automatically on AC power"),
            (STATUS_COMMAND_ID, "Open status and session logs"),
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
    fn stopped_status_reports_unknown_timing_without_a_claimed_value() {
        assert_eq!(
            tooltip(
                TrayStatus::Stopped,
                TimingValues {
                    requested: None,
                    effective: None,
                    external: false,
                    handoff_pending: false,
                    invalid_interval: false,
                },
            ),
            "True™ Tick: Stopped (timing unknown)"
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
                },
            ),
            "True™ Tick: Stopped (external: 0.497 ms)"
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
        for command in [STATUS_COMMAND_ID, 1004] {
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
    fn compact_menu_contains_manual_controls_and_toggles() {
        let items = menu_items(TrayStatus::Running, true, false);
        assert_eq!(
            items.map(|item| item.label),
            [
                "Start",
                "Stop",
                "Auto-start: On",
                "Auto-time: Off",
                "Status",
                "Quit"
            ]
        );
        assert!(!items[0].enabled);
        assert!(items[1].enabled);
        assert!(items[4].enabled);
    }

    #[test]
    fn tooltip_strings_use_verified_or_requested_timing() {
        let exact = TimingValues {
            requested: Some(Hns::new(5_000)),
            effective: Some(Hns::new(5_000)),
            external: false,
            handoff_pending: false,
            invalid_interval: false,
        };
        let finer = TimingValues {
            requested: Some(Hns::new(5_000)),
            effective: Some(Hns::new(4_966)),
            external: false,
            handoff_pending: false,
            invalid_interval: false,
        };
        assert_eq!(
            tooltip(TrayStatus::Running, exact),
            "True™ Tick: Running (0.500 ms)"
        );
        assert_eq!(
            tooltip(TrayStatus::Running, finer),
            "True™ Tick: Running (0.497 ms, finer)"
        );
        assert_eq!(
            tooltip(TrayStatus::Stopped, finer),
            "True™ Tick: Stopped (current: 0.497 ms)"
        );
        assert_eq!(
            tooltip(
                TrayStatus::Starting,
                TimingValues {
                    requested: Some(Hns::new(5_000)),
                    ..TimingValues::default()
                }
            ),
            "True™ Tick: Starting (0.500 ms)"
        );
        assert_eq!(
            tooltip(TrayStatus::Stopping, finer),
            "True™ Tick: Stopping (current: 0.497 ms)"
        );
        assert_eq!(
            tooltip(
                TrayStatus::Stopping,
                TimingValues {
                    effective: Some(Hns::new(4_966)),
                    handoff_pending: true,
                    ..TimingValues::default()
                }
            ),
            "True™ Tick: Stopping (waiting for handoff)"
        );
        assert_eq!(
            tooltip(
                TrayStatus::Error,
                TimingValues {
                    invalid_interval: true,
                    ..TimingValues::default()
                }
            ),
            "True™ Tick: Error (invalid interval)"
        );
    }

    #[test]
    fn unknown_timing_is_explicit_and_short() {
        for status in [
            TrayStatus::Running,
            TrayStatus::Stopped,
            TrayStatus::Starting,
        ] {
            let text = tooltip(status, TimingValues::default());
            assert!(text.contains("unknown"));
            assert!(text.starts_with("True™ Tick: "));
            assert!(!text.contains("HNS"));
            assert!(!text.contains('\\'));
        }
    }

    #[test]
    fn timing_format_rounds_hns_without_exposing_raw_units() {
        assert_eq!(format_ms(Hns::new(4_966)), "0.497");
        assert_eq!(format_ms(Hns::new(5_000)), "0.500");
        assert_eq!(format_ms(Hns::new(156_250)), "15.625");
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
