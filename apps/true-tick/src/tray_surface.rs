use tick_core::Hns;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TrayStatus {
    Running,
    Starting,
    Stopping,
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
    pub(crate) invalid_interval: bool,
}

fn format_ms(value: Hns) -> String {
    let thousandths = value.value().saturating_add(5) / 10;
    format!("{}.{:03}", thousandths / 1_000, thousandths % 1_000)
}

fn current_label(prefix: &str, effective: Option<Hns>) -> String {
    effective.map_or_else(
        || format!("{prefix} (unknown)"),
        |value| format!("{prefix} (current: {} ms)", format_ms(value)),
    )
}

pub(crate) fn tooltip(status: TrayStatus, timing: TimingValues) -> String {
    match status {
        TrayStatus::Running => match (timing.requested, timing.effective) {
            (Some(requested), Some(effective)) if effective < requested => {
                format!("Running ({} ms, finer)", format_ms(effective))
            }
            (_, Some(effective)) => format!("Running ({} ms)", format_ms(effective)),
            (_, None) => "Running (unknown)".to_owned(),
        },
        TrayStatus::Stopped => current_label("Stopped", timing.effective),
        TrayStatus::Blocked => current_label("Blocked", timing.effective),
        TrayStatus::Unsupported => "Unsupported (timing unavailable)".to_owned(),
        TrayStatus::Error if timing.invalid_interval => "Error (invalid interval)".to_owned(),
        TrayStatus::Error => current_label("Error", timing.effective),
        TrayStatus::Starting => timing.requested.map_or_else(
            || "Starting (unknown)".to_owned(),
            |requested| format!("Starting ({} ms)", format_ms(requested)),
        ),
        TrayStatus::Stopping => current_label("Stopping", timing.effective),
        TrayStatus::Pending => "Pending (timing unknown)".to_owned(),
        TrayStatus::Degraded => current_label("Degraded", timing.effective),
        TrayStatus::Unverified => current_label("Unverified", timing.effective),
    }
}

pub(crate) const STATUS_COMMAND_ID: usize = 1009;

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
    matches!(command_id, 1005..=1008)
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
    fn only_toggle_actions_keep_the_native_menu_open() {
        for command in [1005, 1006, 1007, 1008] {
            assert!(menu_action_keeps_open(command));
        }
        for command in [1001, 1002, STATUS_COMMAND_ID, 1004] {
            assert!(!menu_action_keeps_open(command));
        }
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
            requested: Some(Hns::new(10_000)),
            effective: Some(Hns::new(10_000)),
            invalid_interval: false,
        };
        let finer = TimingValues {
            requested: Some(Hns::new(10_000)),
            effective: Some(Hns::new(4_966)),
            invalid_interval: false,
        };
        assert_eq!(tooltip(TrayStatus::Running, exact), "Running (1.000 ms)");
        assert_eq!(
            tooltip(TrayStatus::Running, finer),
            "Running (0.497 ms, finer)"
        );
        assert_eq!(
            tooltip(TrayStatus::Stopped, finer),
            "Stopped (current: 0.497 ms)"
        );
        assert_eq!(
            tooltip(
                TrayStatus::Starting,
                TimingValues {
                    requested: Some(Hns::new(10_000)),
                    ..TimingValues::default()
                }
            ),
            "Starting (1.000 ms)"
        );
        assert_eq!(
            tooltip(TrayStatus::Stopping, finer),
            "Stopping (current: 0.497 ms)"
        );
        assert_eq!(
            tooltip(
                TrayStatus::Error,
                TimingValues {
                    invalid_interval: true,
                    ..TimingValues::default()
                }
            ),
            "Error (invalid interval)"
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
            assert!(!text.contains("True Tick"));
            assert!(!text.contains("HNS"));
            assert!(!text.contains('\\'));
        }
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
