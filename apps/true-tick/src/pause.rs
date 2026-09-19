use std::time::{Duration, Instant};
use tick_core::Status;
use tick_policy::{decide, PolicyInput, PowerState};

pub const MAX_DURATION: Duration = Duration::from_secs(MAX_PRESET_SECONDS as u64);
pub const MAX_TIMER_INTERVAL_MS: u32 = 60 * 60 * 1_000;

pub const MIN_PRESET_SECONDS: u32 = 10;
pub const MAX_PRESET_SECONDS: u32 = 86_400;
pub const MAX_PRESETS: usize = 12;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DurationChoice {
    OneMinute,
    FiveMinutes,
    FifteenMinutes,
    ThirtyMinutes,
    OneHour,
}

impl DurationChoice {
    pub(crate) const fn minutes(self) -> u32 {
        match self {
            Self::OneMinute => 1,
            Self::FiveMinutes => 5,
            Self::FifteenMinutes => 15,
            Self::ThirtyMinutes => 30,
            Self::OneHour => 60,
        }
    }

    #[allow(dead_code)]
    pub(crate) const fn duration(self) -> Duration {
        Duration::from_secs(self.minutes() as u64 * 60)
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::OneMinute => "1 minute",
            Self::FiveMinutes => "5 minutes",
            Self::FifteenMinutes => "15 minutes",
            Self::ThirtyMinutes => "30 minutes",
            Self::OneHour => "1 hour",
        }
    }

    pub(crate) const fn all() -> [Self; 5] {
        [
            Self::OneMinute,
            Self::FiveMinutes,
            Self::FifteenMinutes,
            Self::ThirtyMinutes,
            Self::OneHour,
        ]
    }

    pub(crate) const fn pause_choices() -> [Self; 4] {
        [
            Self::FiveMinutes,
            Self::FifteenMinutes,
            Self::ThirtyMinutes,
            Self::OneHour,
        ]
    }

    pub(crate) fn to_preset(self) -> DurationPreset {
        DurationPreset::new(self.minutes() * 60).unwrap()
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct DurationPreset {
    seconds: u32,
}

impl DurationPreset {
    pub fn new(seconds: u32) -> Result<Self, &'static str> {
        if seconds < MIN_PRESET_SECONDS {
            return Err("preset must be at least 10 seconds");
        }
        if seconds > MAX_PRESET_SECONDS {
            return Err("preset must be at most 24 hours");
        }
        Ok(Self { seconds })
    }

    pub const fn seconds(self) -> u32 {
        self.seconds
    }

    pub const fn minutes(self) -> u32 {
        self.seconds / 60
    }

    pub const fn duration(self) -> Duration {
        Duration::from_secs(self.seconds as u64)
    }

    pub fn format_label(&self) -> String {
        let seconds = self.seconds;
        if seconds < 60 {
            return unit_label(seconds, "second");
        }
        if seconds.is_multiple_of(3600) {
            return unit_label(seconds / 3600, "hour");
        }
        if seconds < 3600 && seconds.is_multiple_of(60) {
            return unit_label(seconds / 60, "minute");
        }
        let hours = seconds / 3600;
        let minutes = (seconds % 3600) / 60;
        let remainder = seconds % 60;
        let mut parts = Vec::new();
        if hours > 0 {
            parts.push(format!("{}h", hours));
        }
        if minutes > 0 {
            parts.push(format!("{}m", minutes));
        }
        if remainder > 0 {
            parts.push(format!("{}s", remainder));
        }
        parts.join(" ")
    }

    pub fn parse(text: &str) -> Result<Self, &'static str> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err("duration text is empty");
        }
        let lower = trimmed.to_ascii_lowercase();
        if lower
            .chars()
            .all(|character| character.is_ascii_digit() || character == '.')
        {
            let minutes: f64 = lower.parse().map_err(|_| "invalid duration number")?;
            return Self::from_seconds_f64(minutes * 60.0);
        }
        let mut chars = lower.chars().peekable();
        let mut total_seconds = 0u64;
        let mut consumed_segment = false;
        loop {
            while matches!(chars.peek(), Some(character) if character.is_whitespace()) {
                chars.next();
            }
            if chars.peek().is_none() {
                break;
            }
            let mut number_text = String::new();
            let mut seen_digit = false;
            let mut seen_dot = false;
            while let Some(character) = chars.peek().copied() {
                if character.is_ascii_digit() {
                    seen_digit = true;
                    number_text.push(character);
                    chars.next();
                } else if character == '.' && !seen_dot {
                    seen_dot = true;
                    number_text.push(character);
                    chars.next();
                } else {
                    break;
                }
            }
            if !seen_digit {
                return Err("expected a number in duration text");
            }
            let mut unit = String::new();
            while let Some(character) = chars.peek().copied() {
                if character.is_ascii_alphabetic() {
                    unit.push(character);
                    chars.next();
                } else {
                    break;
                }
            }
            let multiplier = match unit.as_str() {
                "s" | "sec" | "secs" | "second" | "seconds" => 1u64,
                "m" | "min" | "mins" | "minute" | "minutes" => 60,
                "h" | "hr" | "hrs" | "hour" | "hours" => 3600,
                "" => return Err("expected a unit after the number"),
                _ => return Err("unknown duration unit"),
            };
            let segment_seconds = if seen_dot {
                let fraction: f64 = number_text
                    .parse()
                    .map_err(|_| "invalid duration number")?;
                let scaled = fraction * multiplier as f64;
                if !(scaled.is_finite() && scaled >= 0.0 && scaled <= u64::MAX as f64) {
                    return Err("invalid duration number");
                }
                scaled.round() as u64
            } else {
                let value: u64 = number_text
                    .parse()
                    .map_err(|_| "invalid duration number")?;
                match value.checked_mul(multiplier) {
                    Some(product) => product,
                    None => return Err("invalid duration number"),
                }
            };
            total_seconds = match total_seconds.checked_add(segment_seconds) {
                Some(total) => total,
                None => return Err("invalid duration number"),
            };
            if total_seconds > MAX_PRESET_SECONDS as u64 {
                return Err("preset must be between 10 seconds and 24 hours");
            }
            consumed_segment = true;
        }
        if !consumed_segment {
            return Err("duration text is empty");
        }
        if !(MIN_PRESET_SECONDS as u64..=MAX_PRESET_SECONDS as u64).contains(&total_seconds) {
            return Err("preset must be between 10 seconds and 24 hours");
        }
        Self::new(total_seconds as u32)
    }

    fn from_seconds_f64(total_seconds: f64) -> Result<Self, &'static str> {
        if !(total_seconds.is_finite()
            && total_seconds >= MIN_PRESET_SECONDS as f64
            && total_seconds <= MAX_PRESET_SECONDS as f64)
        {
            return Err("preset must be between 10 seconds and 24 hours");
        }
        Self::new(total_seconds.round() as u32)
    }
}

fn unit_label(value: u32, unit: &str) -> String {
    if value == 1 {
        format!("{} {}", value, unit)
    } else {
        format!("{} {}s", value, unit)
    }
}

pub struct PresetsManager {
    presets: Vec<DurationPreset>,
}

impl PresetsManager {
    pub fn new() -> Self {
        Self {
            presets: Self::default_presets(),
        }
    }

    pub fn default_presets() -> Vec<DurationPreset> {
        [60, 300, 900, 1800, 3600]
            .into_iter()
            .filter_map(|seconds| DurationPreset::new(seconds).ok())
            .collect()
    }

    pub fn presets(&self) -> &[DurationPreset] {
        &self.presets
    }

    pub fn add(&mut self, preset: DurationPreset) -> Result<(), &'static str> {
        if self.presets.contains(&preset) {
            return Err("preset already exists");
        }
        if self.presets.len() >= MAX_PRESETS {
            return Err("maximum of 12 presets reached");
        }
        self.presets.push(preset);
        self.presets.sort();
        Ok(())
    }

    pub fn remove(&mut self, index: usize) -> Result<(), &'static str> {
        if index >= self.presets.len() {
            return Err("preset index out of range");
        }
        if self.presets.len() <= 1 {
            return Err("at least one preset must remain");
        }
        self.presets.remove(index);
        Ok(())
    }

    pub fn reset_defaults(&mut self) {
        self.presets = Self::default_presets();
    }

    pub fn to_seconds_list(&self) -> Vec<u32> {
        self.presets.iter().map(|preset| preset.seconds()).collect()
    }

    pub fn from_seconds_list(list: &[u32]) -> Self {
        let mut presets: Vec<DurationPreset> = list
            .iter()
            .filter_map(|seconds| DurationPreset::new(*seconds).ok())
            .collect();
        presets.sort();
        presets.dedup();
        presets.truncate(MAX_PRESETS);
        if presets.is_empty() {
            presets = Self::default_presets();
        }
        Self { presets }
    }
}

impl Default for PresetsManager {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DurationAction {
    Start,
    Stop,
    Pause,
}

impl DurationAction {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Pause => "pause",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ScheduledAction {
    pub(crate) action: DurationAction,
    pub(crate) duration: DurationPreset,
    pub(crate) deadline: Instant,
    pub(crate) generation: u64,
}

impl ScheduledAction {
    pub(crate) fn remaining(self, now: Instant) -> Duration {
        self.deadline.saturating_duration_since(now)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScheduleRequest {
    Started(ScheduledAction),
    Replaced {
        previous: ScheduledAction,
        current: ScheduledAction,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoordinatorTimerEvent {
    Expired(ScheduledAction),
    Early {
        action: DurationAction,
        generation: u64,
        remaining: Duration,
    },
    Stale,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DurationCoordinator {
    scheduled: Option<ScheduledAction>,
    generation: u64,
}

impl DurationCoordinator {
    pub(crate) const fn new() -> Self {
        Self {
            scheduled: None,
            generation: 0,
        }
    }

    pub(crate) const fn active(self) -> bool {
        self.scheduled.is_some()
    }

    pub(crate) const fn pause_active(self) -> bool {
        matches!(self.scheduled, Some(action) if matches!(action.action, DurationAction::Pause))
    }

    pub(crate) const fn generation(self) -> u64 {
        self.generation
    }

    pub(crate) const fn current(self) -> Option<ScheduledAction> {
        self.scheduled
    }

    pub(crate) const fn deadline(self) -> Option<Instant> {
        match self.scheduled {
            Some(action) => Some(action.deadline),
            None => None,
        }
    }

    pub(crate) fn schedule(
        &mut self,
        action: DurationAction,
        duration: DurationPreset,
        now: Instant,
    ) -> ScheduleRequest {
        self.generation = self.generation.saturating_add(1);
        let current = ScheduledAction {
            action,
            duration,
            deadline: now + duration.duration().min(MAX_DURATION),
            generation: self.generation,
        };
        match self.scheduled.replace(current) {
            Some(previous) => ScheduleRequest::Replaced { previous, current },
            None => ScheduleRequest::Started(current),
        }
    }

    pub(crate) fn cancel(&mut self) -> Option<ScheduledAction> {
        let previous = self.scheduled.take();
        if previous.is_some() {
            self.generation = self.generation.saturating_add(1);
        }
        previous
    }

    pub(crate) fn timer_event(&self, generation: u64, now: Instant) -> CoordinatorTimerEvent {
        let Some(action) = self.scheduled else {
            return CoordinatorTimerEvent::Stale;
        };
        if generation != self.generation || generation != action.generation {
            return CoordinatorTimerEvent::Stale;
        }
        let remaining = action.remaining(now);
        if remaining.is_zero() {
            CoordinatorTimerEvent::Expired(action)
        } else {
            CoordinatorTimerEvent::Early {
                action: action.action,
                generation,
                remaining,
            }
        }
    }
}

impl Default for DurationCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn acquisition_is_allowed(paused: bool, automatic: bool, power: PowerState) -> bool {
    !paused
        && automatic
        && decide(PolicyInput {
            enabled: true,
            eligible_profile: true,
            power,
        })
        .status
            == Status::Requested
}

pub(crate) fn timer_interval_ms(remaining: Duration) -> u32 {
    let milliseconds = remaining
        .as_millis()
        .max(1)
        .min(MAX_TIMER_INTERVAL_MS as u128);
    milliseconds as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start() -> Instant {
        Instant::now()
    }

    #[test]
    fn fixed_choices_cover_all_start_stop_and_pause_durations() {
        assert_eq!(
            DurationChoice::all().map(DurationChoice::minutes),
            [1, 5, 15, 30, 60]
        );
        assert_eq!(
            DurationChoice::pause_choices().map(DurationChoice::minutes),
            [5, 15, 30, 60]
        );
        assert_eq!(DurationChoice::OneHour.to_preset().minutes(), 60);
        assert_eq!(DurationChoice::OneMinute.label(), "1 minute");
    }

    #[test]
    fn each_action_sets_an_exact_monotonic_deadline_and_generation() {
        let now = start();
        for action in [
            DurationAction::Start,
            DurationAction::Stop,
            DurationAction::Pause,
        ] {
            let mut coordinator = DurationCoordinator::new();
            let ScheduleRequest::Started(scheduled) =
                coordinator.schedule(action, DurationChoice::FifteenMinutes.to_preset(), now)
            else {
                panic!("first schedule must start");
            };
            assert_eq!(scheduled.action, action);
            assert_eq!(scheduled.generation, 1);
            assert_eq!(scheduled.remaining(now), Duration::from_secs(15 * 60));
        }
    }

    #[test]
    fn all_fixed_duration_choices_have_exact_deadlines_for_start_and_stop() {
        let now = start();
        for action in [DurationAction::Start, DurationAction::Stop] {
            for duration in DurationChoice::all() {
                let mut coordinator = DurationCoordinator::new();
                let ScheduleRequest::Started(scheduled) =
                    coordinator.schedule(action, duration.to_preset(), now)
                else {
                    panic!("first schedule must start");
                };
                assert_eq!(scheduled.action, action);
                assert_eq!(scheduled.remaining(now), duration.duration());
            }
        }
    }

    #[test]
    fn replacement_is_latest_wins_and_only_one_action_remains() {
        let now = start();
        let mut coordinator = DurationCoordinator::new();
        let ScheduleRequest::Started(first) = coordinator.schedule(
            DurationAction::Start,
            DurationChoice::FiveMinutes.to_preset(),
            now,
        ) else {
            panic!("first schedule must start");
        };
        let ScheduleRequest::Replaced { previous, current } = coordinator.schedule(
            DurationAction::Stop,
            DurationChoice::OneHour.to_preset(),
            now + Duration::from_secs(1),
        ) else {
            panic!("second schedule must replace");
        };
        assert_eq!(previous, first);
        assert_eq!(current.action, DurationAction::Stop);
        assert_eq!(coordinator.current(), Some(current));
        assert!(!coordinator.pause_active());
    }

    #[test]
    fn pause_is_active_immediately_and_pause_choice_is_bounded() {
        let now = start();
        let mut coordinator = DurationCoordinator::new();
        coordinator.schedule(
            DurationAction::Pause,
            DurationChoice::FiveMinutes.to_preset(),
            now,
        );
        assert!(coordinator.active());
        assert!(coordinator.pause_active());
        assert_eq!(
            coordinator.timer_event(1, now + Duration::from_secs(300)),
            CoordinatorTimerEvent::Expired(coordinator.current().unwrap())
        );
    }

    #[test]
    fn cancellation_invalidates_the_old_generation() {
        let now = start();
        let mut coordinator = DurationCoordinator::new();
        coordinator.schedule(
            DurationAction::Start,
            DurationChoice::FiveMinutes.to_preset(),
            now,
        );
        let old_generation = coordinator.generation();
        assert!(coordinator.cancel().is_some());
        assert!(!coordinator.active());
        assert_eq!(
            coordinator.timer_event(old_generation, now + Duration::from_secs(600)),
            CoordinatorTimerEvent::Stale
        );
        assert!(coordinator.cancel().is_none());
    }

    #[test]
    fn cancellation_clears_start_stop_and_pause_after_replacement() {
        let now = start();
        for action in [
            DurationAction::Start,
            DurationAction::Stop,
            DurationAction::Pause,
        ] {
            let mut coordinator = DurationCoordinator::new();
            coordinator.schedule(action, DurationChoice::FiveMinutes.to_preset(), now);
            let replaced = coordinator.schedule(
                action,
                DurationChoice::FifteenMinutes.to_preset(),
                now + Duration::from_secs(1),
            );
            let current = match replaced {
                ScheduleRequest::Replaced { current, .. } => current,
                ScheduleRequest::Started(_) => panic!("replacement must retain one action"),
            };
            assert!(coordinator.cancel().is_some());
            assert!(!coordinator.active());
            assert_eq!(coordinator.current(), None);
            assert_eq!(
                coordinator.timer_event(current.generation, current.deadline),
                CoordinatorTimerEvent::Stale
            );
        }
    }

    #[test]
    fn cancel_without_an_action_is_idempotent_and_does_not_change_generation() {
        let mut coordinator = DurationCoordinator::new();
        assert_eq!(coordinator.generation(), 0);
        assert!(coordinator.cancel().is_none());
        assert_eq!(coordinator.generation(), 0);
    }

    #[test]
    fn stale_replacement_generation_cannot_fire_the_new_action() {
        let now = start();
        let mut coordinator = DurationCoordinator::new();
        coordinator.schedule(
            DurationAction::Pause,
            DurationChoice::FiveMinutes.to_preset(),
            now,
        );
        coordinator.schedule(
            DurationAction::Start,
            DurationChoice::ThirtyMinutes.to_preset(),
            now + Duration::from_secs(1),
        );
        assert_eq!(
            coordinator.timer_event(1, now + Duration::from_secs(600)),
            CoordinatorTimerEvent::Stale
        );
        assert!(matches!(
            coordinator.timer_event(2, now + Duration::from_secs(2)),
            CoordinatorTimerEvent::Early {
                action: DurationAction::Start,
                generation: 2,
                ..
            }
        ));
    }

    #[test]
    fn pause_suppresses_acquisition_for_all_power_states() {
        for power in [
            PowerState::Ac,
            PowerState::Battery,
            PowerState::BatterySaver,
            PowerState::Unknown,
        ] {
            assert!(!acquisition_is_allowed(true, true, power));
        }
        assert!(acquisition_is_allowed(false, true, PowerState::Ac));
        assert!(!acquisition_is_allowed(false, false, PowerState::Ac));
        assert!(!acquisition_is_allowed(false, true, PowerState::Battery));
    }

    #[test]
    fn timer_interval_is_at_least_one_and_hard_bounded() {
        assert_eq!(timer_interval_ms(Duration::ZERO), 1);
        assert_eq!(timer_interval_ms(MAX_DURATION), MAX_TIMER_INTERVAL_MS);
        assert_eq!(
            timer_interval_ms(Duration::from_secs(86_401)),
            MAX_TIMER_INTERVAL_MS
        );
    }

    #[test]
    fn default_presets_match_factory_intervals() {
        let presets = PresetsManager::default_presets();
        let seconds: Vec<u32> = presets.iter().map(|preset| preset.seconds()).collect();
        assert_eq!(seconds, vec![60, 300, 900, 1800, 3600]);
        let manager = PresetsManager::new();
        assert_eq!(manager.to_seconds_list(), vec![60, 300, 900, 1800, 3600]);
    }

    #[test]
    fn preset_bounds_are_enforced() {
        assert!(DurationPreset::new(9).is_err());
        assert_eq!(DurationPreset::new(10).unwrap().seconds(), 10);
        assert_eq!(DurationPreset::new(86_400).unwrap().seconds(), 86_400);
        assert!(DurationPreset::new(86_401).is_err());
    }

    #[test]
    fn preset_labels_format_seconds_minutes_hours_and_mixed() {
        assert_eq!(
            DurationPreset::new(30).unwrap().format_label(),
            "30 seconds"
        );
        assert_eq!(DurationPreset::new(60).unwrap().format_label(), "1 minute");
        assert_eq!(
            DurationPreset::new(300).unwrap().format_label(),
            "5 minutes"
        );
        assert_eq!(
            DurationPreset::new(2700).unwrap().format_label(),
            "45 minutes"
        );
        assert_eq!(DurationPreset::new(3600).unwrap().format_label(), "1 hour");
        assert_eq!(DurationPreset::new(5400).unwrap().format_label(), "1h 30m");
    }

    #[test]
    fn preset_parse_handles_natural_strings() {
        assert_eq!(DurationPreset::parse("10").unwrap().seconds(), 600);
        assert_eq!(DurationPreset::parse("30s").unwrap().seconds(), 30);
        assert_eq!(DurationPreset::parse("5m").unwrap().seconds(), 300);
        assert_eq!(DurationPreset::parse("45min").unwrap().seconds(), 2700);
        assert_eq!(DurationPreset::parse("2h").unwrap().seconds(), 7200);
        assert_eq!(DurationPreset::parse("1.5h").unwrap().seconds(), 5400);
        assert_eq!(DurationPreset::parse("1h30m").unwrap().seconds(), 5400);
        assert_eq!(DurationPreset::parse("1h 30m").unwrap().seconds(), 5400);
    }

    #[test]
    fn preset_parse_rejects_invalid_and_out_of_range_text() {
        assert!(DurationPreset::parse("").is_err());
        assert!(DurationPreset::parse("   ").is_err());
        assert!(DurationPreset::parse("abc").is_err());
        assert!(DurationPreset::parse("5x").is_err());
        assert!(DurationPreset::parse("0").is_err());
        assert!(DurationPreset::parse("0.1m").is_err());
        assert!(DurationPreset::parse("25h").is_err());
        assert!(DurationPreset::parse("9s").is_err());
        assert!(DurationPreset::parse("m").is_err());
    }

    #[test]
    fn test_parse_preset_string_rejects_astronomical_hours_overflow() {
        assert!(DurationPreset::parse("99999999999999h").is_err());
        assert!(DurationPreset::parse("4294967295h").is_err());
        assert!(DurationPreset::parse("99999999999999999999h").is_err());
    }

    #[test]
    fn test_parse_preset_string_rejects_multiplication_overflow() {
        assert!(DurationPreset::parse("5000000h").is_err());
    }

    #[test]
    fn manager_add_deduplicates_sorts_and_enforces_cap() {
        let mut manager = PresetsManager::new();
        assert!(manager.add(DurationPreset::new(120).unwrap()).is_ok());
        assert_eq!(
            manager.to_seconds_list(),
            vec![60, 120, 300, 900, 1800, 3600]
        );
        assert!(manager.add(DurationPreset::new(120).unwrap()).is_err());
        for seconds in [20, 30, 40, 50, 70, 80] {
            let result = manager.add(DurationPreset::new(seconds).unwrap());
            assert!(result.is_ok());
        }
        assert_eq!(manager.presets().len(), MAX_PRESETS);
        assert!(manager.add(DurationPreset::new(100).unwrap()).is_err());
        let list = manager.to_seconds_list();
        let mut sorted = list.clone();
        sorted.sort();
        assert_eq!(list, sorted);
    }

    #[test]
    fn manager_remove_rejects_invalid_index_and_last_item() {
        let mut manager = PresetsManager::new();
        assert!(manager.remove(999).is_err());
        while manager.presets().len() > 1 {
            assert!(manager.remove(0).is_ok());
        }
        assert!(manager.remove(0).is_err());
        assert_eq!(manager.presets().len(), 1);
    }

    #[test]
    fn manager_reset_defaults_restores_factory_list() {
        let mut manager = PresetsManager::new();
        while manager.presets().len() > 1 {
            let _ = manager.remove(0);
        }
        manager.add(DurationPreset::new(120).unwrap()).unwrap();
        manager.reset_defaults();
        assert_eq!(manager.to_seconds_list(), vec![60, 300, 900, 1800, 3600]);
    }

    #[test]
    fn manager_roundtrip_through_seconds_list_filters_invalid_values() {
        let manager = PresetsManager::from_seconds_list(&[3600, 60, 300, 5, 200_000, 60]);
        assert_eq!(manager.to_seconds_list(), vec![60, 300, 3600]);
        let restored = PresetsManager::from_seconds_list(&manager.to_seconds_list());
        assert_eq!(restored.to_seconds_list(), vec![60, 300, 3600]);
        let empty = PresetsManager::from_seconds_list(&[]);
        assert_eq!(empty.to_seconds_list(), vec![60, 300, 900, 1800, 3600]);
    }
}
