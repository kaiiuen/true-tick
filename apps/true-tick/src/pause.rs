use std::time::{Duration, Instant};
use tick_core::Status;
use tick_policy::{decide, PolicyInput, PowerState};

pub const MAX_DURATION: Duration = Duration::from_secs(60 * 60);
pub const MAX_TIMER_INTERVAL_MS: u32 = 60 * 60 * 1_000;

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
    pub(crate) duration: DurationChoice,
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
        duration: DurationChoice,
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
        assert_eq!(DurationChoice::OneHour.duration(), MAX_DURATION);
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
                coordinator.schedule(action, DurationChoice::FifteenMinutes, now)
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
                    coordinator.schedule(action, duration, now)
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
        let ScheduleRequest::Started(first) =
            coordinator.schedule(DurationAction::Start, DurationChoice::FiveMinutes, now)
        else {
            panic!("first schedule must start");
        };
        let ScheduleRequest::Replaced { previous, current } = coordinator.schedule(
            DurationAction::Stop,
            DurationChoice::OneHour,
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
        coordinator.schedule(DurationAction::Pause, DurationChoice::FiveMinutes, now);
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
        coordinator.schedule(DurationAction::Start, DurationChoice::FiveMinutes, now);
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
    fn stale_replacement_generation_cannot_fire_the_new_action() {
        let now = start();
        let mut coordinator = DurationCoordinator::new();
        coordinator.schedule(DurationAction::Pause, DurationChoice::FiveMinutes, now);
        coordinator.schedule(
            DurationAction::Start,
            DurationChoice::ThirtyMinutes,
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
            timer_interval_ms(Duration::from_secs(60 * 60 + 1)),
            MAX_TIMER_INTERVAL_MS
        );
    }
}
