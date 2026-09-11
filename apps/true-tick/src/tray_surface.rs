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
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Running => "Running",
            Self::Starting => "Starting...",
            Self::Stopping => "Stopping...",
            Self::Pending => "Pending",
            Self::Degraded => "Degraded",
            Self::Unverified => "Unverified",
            Self::Stopped => "Stopped",
            Self::Blocked => "Blocked",
            Self::Unsupported => "Unsupported",
            Self::Error => "Error",
        }
    }

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
        "Automatic timing activation: On"
    } else {
        "Automatic timing activation: Off"
    }
}

pub(crate) fn tooltip(status: TrayStatus) -> &'static str {
    match status {
        TrayStatus::Running => "Running (current timing)",
        TrayStatus::Stopped | TrayStatus::Blocked | TrayStatus::Unsupported | TrayStatus::Error => {
            "Stopped (current timing)"
        }
        TrayStatus::Starting => "Starting...",
        TrayStatus::Stopping => "Stopping...",
        TrayStatus::Pending => "Pending...",
        TrayStatus::Degraded => "Degraded",
        TrayStatus::Unverified => "Unverified",
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
            enabled: false,
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
    fn compact_menu_contains_manual_controls_and_toggles() {
        let items = menu_items(TrayStatus::Running, true, false);
        assert_eq!(
            items.map(|item| item.label),
            [
                "Start",
                "Stop",
                "Auto-start: On",
                "Automatic timing activation: Off",
                "Status",
                "Quit"
            ]
        );
        assert!(!items[0].enabled);
        assert!(items[1].enabled);
        assert!(!items[4].enabled);
    }

    #[test]
    fn tooltip_strings_are_short_runtime_labels() {
        assert_eq!(tooltip(TrayStatus::Running), "Running (current timing)");
        assert_eq!(tooltip(TrayStatus::Stopped), "Stopped (current timing)");
        assert_eq!(tooltip(TrayStatus::Starting), "Starting...");
        assert_eq!(tooltip(TrayStatus::Stopping), "Stopping...");
        for status in [
            TrayStatus::Running,
            TrayStatus::Stopped,
            TrayStatus::Starting,
        ] {
            let text = tooltip(status);
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
