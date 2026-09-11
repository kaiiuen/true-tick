#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TrayStatus {
    Active,
    Warning,
    Stopped,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IconColor {
    Green,
    Yellow,
    Red,
}

impl TrayStatus {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Active => "Active",
            Self::Warning => "Warning",
            Self::Stopped => "Stopped",
        }
    }

    pub(crate) const fn icon_color(self) -> IconColor {
        match self {
            Self::Active => IconColor::Green,
            Self::Warning => IconColor::Yellow,
            Self::Stopped => IconColor::Red,
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

pub(crate) fn tooltip(status: TrayStatus) -> String {
    format!("True Tick: {}", status.label())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_labels_are_short_and_fixed() {
        let labels = [
            auto_start_label(false),
            auto_start_label(true),
            automatic_label(false),
            automatic_label(true),
            TrayStatus::Active.label(),
            TrayStatus::Warning.label(),
            TrayStatus::Stopped.label(),
        ];

        for label in labels {
            assert!(label.len() <= 32, "label is too long: {label}");
            assert!(!label.contains("config"));
            assert!(!label.contains("HNS"));
            assert!(!label.contains("error"));
            assert!(!label.contains('\n'));
        }
    }

    #[test]
    fn tooltip_is_compact_and_status_only() {
        for status in [TrayStatus::Active, TrayStatus::Warning, TrayStatus::Stopped] {
            let text = tooltip(status);
            assert_eq!(text, format!("True Tick: {}", status.label()));
            assert!(text.len() <= 24);
        }
    }

    #[test]
    fn status_maps_to_truthful_icon_color() {
        assert_eq!(TrayStatus::Active.icon_color(), IconColor::Green);
        assert_eq!(TrayStatus::Warning.icon_color(), IconColor::Yellow);
        assert_eq!(TrayStatus::Stopped.icon_color(), IconColor::Red);
    }
}
