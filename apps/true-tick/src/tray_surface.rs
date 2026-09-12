use tick_core::Hns;
use tick_ownership::{OwnershipState, TimingSnapshot};
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MenuItem {
    pub(crate) label: String,
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

pub(crate) const fn lifecycle_status(status: TrayStatus, handoff_active: bool) -> TrayStatus {
    if handoff_active {
        TrayStatus::Stopping
    } else {
        status
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

impl TimingValues {
    pub(crate) fn from_snapshot(
        snapshot: TimingSnapshot,
        handoff_pending: bool,
        external: bool,
        invalid_interval: bool,
    ) -> Self {
        Self {
            requested: snapshot.requested,
            effective: snapshot.effective,
            external,
            handoff_pending,
            invalid_interval,
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

pub(crate) fn strip_status_branding(value: &str) -> &str {
    value.strip_prefix("True™ Tick: ").unwrap_or(value)
}

pub(crate) fn tooltip(status: TrayStatus, timing: TimingValues) -> String {
    let summary = match status {
        TrayStatus::Running => timing.effective.map_or_else(
            || "Running".to_owned(),
            |value| format!("Running {} ms", format_ms(value)),
        ),
        TrayStatus::Stopped if timing.external => "Stopped, external timing".to_owned(),
        TrayStatus::Stopped => timing.effective.map_or_else(
            || "Stopped".to_owned(),
            |value| format!("Stopped {} ms", format_ms(value)),
        ),
        TrayStatus::Starting => timing.requested.map_or_else(
            || "Starting, verifying".to_owned(),
            |requested| format!("Starting {} ms, verifying", format_ms(requested)),
        ),
        TrayStatus::Stopping if timing.handoff_pending => "Stopping, handoff pending".to_owned(),
        TrayStatus::Stopping => "Stopping".to_owned(),
        TrayStatus::Error => "Error".to_owned(),
        TrayStatus::Pending
        | TrayStatus::Degraded
        | TrayStatus::Unverified
        | TrayStatus::Blocked
        | TrayStatus::Unsupported => "Warning".to_owned(),
    };
    format!("True™ Tick: {summary}")
}

pub(crate) fn status_label(status: TrayStatus, timing: TimingValues) -> String {
    let tooltip_text = tooltip(status, timing);
    let concise = strip_status_branding(&tooltip_text);
    let concise = match status {
        TrayStatus::Running => timing.effective.map_or_else(
            || concise.to_owned(),
            |value| format!("Running ({} ms)", format_ms(value)),
        ),
        TrayStatus::Stopped if timing.external => "Stopped (external timing)".to_owned(),
        TrayStatus::Stopped => timing.effective.map_or_else(
            || concise.to_owned(),
            |value| format!("Stopped ({} ms)", format_ms(value)),
        ),
        TrayStatus::Starting => timing.requested.map_or_else(
            || concise.to_owned(),
            |value| format!("Starting ({} ms, verifying)", format_ms(value)),
        ),
        TrayStatus::Stopping if timing.handoff_pending => "Stopping (handoff pending)".to_owned(),
        TrayStatus::Stopping if timing.external => "Stopped (external timing)".to_owned(),
        TrayStatus::Error if timing.invalid_interval => "Error (invalid interval)".to_owned(),
        TrayStatus::Error => "Error (operation failed)".to_owned(),
        _ => concise.to_owned(),
    };
    format!("Status: {concise}")
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
    timing: TimingValues,
) -> [MenuItem; 8] {
    [
        MenuItem {
            label: version_header(),
            enabled: false,
        },
        MenuItem {
            label: "Start".to_owned(),
            enabled: !matches!(status, TrayStatus::Running | TrayStatus::Starting),
        },
        MenuItem {
            label: "Stop".to_owned(),
            enabled: !matches!(status, TrayStatus::Stopped | TrayStatus::Stopping),
        },
        MenuItem {
            label: auto_start_label(startup_enabled).to_owned(),
            enabled: true,
        },
        MenuItem {
            label: automatic_label(automatic).to_owned(),
            enabled: true,
        },
        MenuItem {
            label: status_label(status, timing),
            enabled: false,
        },
        MenuItem {
            label: "Status".to_owned(),
            enabled: true,
        },
        MenuItem {
            label: "Quit".to_owned(),
            enabled: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let timing = TimingValues::from_snapshot(snapshot, false, false, false);
        assert_eq!(
            tooltip(TrayStatus::Running, timing),
            "True™ Tick: Running 0.497 ms"
        );
        assert_eq!(
            menu_items(TrayStatus::Running, false, false, timing)[5].label,
            "Status: Running (0.497 ms)"
        );
    }

    #[test]
    fn version_header_uses_the_package_version() {
        assert_eq!(
            version_header(),
            "True™ Tick v".to_owned() + env!("CARGO_PKG_VERSION")
        );
    }

    #[test]
    fn status_label_strips_branding_and_keeps_the_row_short() {
        assert_eq!(
            strip_status_branding("True™ Tick: Running 0.497 ms"),
            "Running 0.497 ms"
        );
        assert_eq!(
            status_label(
                TrayStatus::Stopping,
                TimingValues {
                    external: true,
                    handoff_pending: true,
                    ..TimingValues::default()
                }
            ),
            "Status: Stopping (handoff pending)"
        );
        assert_eq!(
            status_label(
                TrayStatus::Error,
                TimingValues {
                    invalid_interval: true,
                    ..TimingValues::default()
                }
            ),
            "Status: Error (invalid interval)"
        );
    }

    #[test]
    fn status_command_remains_clickable_below_the_disabled_summary() {
        assert_eq!(STATUS_COMMAND_ID, 1009);
        let items = menu_items(TrayStatus::Stopped, false, false, TimingValues::default());
        assert!(!items[0].enabled);
        assert!(!items[5].enabled);
        assert!(items[6].enabled);
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
    fn stopped_status_omits_unknown_timing_from_the_hover_text() {
        assert_eq!(
            tooltip(TrayStatus::Stopped, TimingValues::default()),
            "True™ Tick: Stopped"
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
            "True™ Tick: Stopped, external timing"
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
                "Auto-start: On",
                "Auto-time: Off",
                "Status: Running (0.497 ms)",
                "Status",
                "Quit"
            ]
        );
        assert!(!items[0].enabled);
        assert!(!items[5].enabled);
        assert!(!items[1].enabled);
        assert!(items[2].enabled);
        assert!(items[6].enabled);
        assert!(items[7].enabled);
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
            "True™ Tick: Running 0.500 ms"
        );
        assert_eq!(
            tooltip(TrayStatus::Running, finer),
            "True™ Tick: Running 0.497 ms"
        );
        assert_eq!(
            tooltip(TrayStatus::Stopped, finer),
            "True™ Tick: Stopped 0.497 ms"
        );
        assert_eq!(
            tooltip(
                TrayStatus::Starting,
                TimingValues {
                    requested: Some(Hns::new(5_000)),
                    ..TimingValues::default()
                }
            ),
            "True™ Tick: Starting 0.500 ms, verifying"
        );
        assert_eq!(tooltip(TrayStatus::Stopping, finer), "True™ Tick: Stopping");
        assert_eq!(
            tooltip(
                TrayStatus::Stopping,
                TimingValues {
                    effective: Some(Hns::new(4_966)),
                    handoff_pending: true,
                    ..TimingValues::default()
                }
            ),
            "True™ Tick: Stopping, handoff pending"
        );
        assert_eq!(
            tooltip(
                TrayStatus::Stopped,
                TimingValues {
                    external: true,
                    effective: Some(Hns::new(4_966)),
                    ..TimingValues::default()
                }
            ),
            "True™ Tick: Stopped, external timing"
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
