use std::time::{Duration, Instant};
use tick_core::Status;
use tick_policy::{decide, PolicyInput, PowerState};

pub const MAX_PAUSE_DURATION: Duration = Duration::from_secs(60 * 60);
pub const MAX_TIMER_INTERVAL_MS: u32 = 60 * 60 * 1_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PauseDuration {
    Five,
    Fifteen,
    Thirty,
    Sixty,
}

impl PauseDuration {
    pub(crate) const fn minutes(self) -> u32 {
        match self {
            Self::Five => 5,
            Self::Fifteen => 15,
            Self::Thirty => 30,
            Self::Sixty => 60,
        }
    }

    pub(crate) const fn duration(self) -> Duration {
        Duration::from_secs(self.minutes() as u64 * 60)
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Five => "5 min",
            Self::Fifteen => "15 min",
            Self::Thirty => "30 min",
            Self::Sixty => "60 min",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PauseRequest {
    Started { generation: u64, deadline: Instant },
    Repeated { generation: u64, deadline: Instant },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PauseTimerEvent {
    Expired {
        generation: u64,
    },
    Early {
        generation: u64,
        remaining: Duration,
    },
    Stale,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PauseController {
    deadline: Option<Instant>,
    generation: u64,
}

impl PauseController {
    pub(crate) const fn new() -> Self {
        Self {
            deadline: None,
            generation: 0,
        }
    }

    pub(crate) const fn active(self) -> bool {
        self.deadline.is_some()
    }

    pub(crate) const fn generation(self) -> u64 {
        self.generation
    }

    pub(crate) const fn deadline(self) -> Option<Instant> {
        self.deadline
    }

    pub(crate) fn request(&mut self, duration: PauseDuration, now: Instant) -> PauseRequest {
        if let Some(deadline) = self.deadline {
            return PauseRequest::Repeated {
                generation: self.generation,
                deadline,
            };
        }
        self.generation = self.generation.saturating_add(1);
        let deadline = now + duration.duration().min(MAX_PAUSE_DURATION);
        self.deadline = Some(deadline);
        PauseRequest::Started {
            generation: self.generation,
            deadline,
        }
    }

    pub(crate) fn resume(&mut self) -> bool {
        if self.deadline.take().is_some() {
            self.generation = self.generation.saturating_add(1);
            true
        } else {
            false
        }
    }

    pub(crate) fn timer_event(&self, generation: u64, now: Instant) -> PauseTimerEvent {
        let Some(deadline) = self.deadline else {
            return PauseTimerEvent::Stale;
        };
        if generation != self.generation {
            return PauseTimerEvent::Stale;
        }
        if now >= deadline {
            PauseTimerEvent::Expired { generation }
        } else {
            PauseTimerEvent::Early {
                generation,
                remaining: deadline.duration_since(now),
            }
        }
    }
}

impl Default for PauseController {
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
    fn fixed_choices_have_exact_bounded_durations() {
        assert_eq!(
            [
                PauseDuration::Five,
                PauseDuration::Fifteen,
                PauseDuration::Thirty,
                PauseDuration::Sixty,
            ]
            .map(PauseDuration::minutes),
            [5, 15, 30, 60]
        );
        assert_eq!(PauseDuration::Sixty.duration(), MAX_PAUSE_DURATION);
        assert_eq!(PauseDuration::Five.label(), "5 min");
    }

    #[test]
    fn first_pause_sets_a_monotonic_deadline_and_generation() {
        let now = start();
        let mut pause = PauseController::new();
        let PauseRequest::Started {
            generation,
            deadline,
        } = pause.request(PauseDuration::Fifteen, now)
        else {
            panic!("first pause must start");
        };
        assert_eq!(generation, 1);
        assert_eq!(deadline.duration_since(now), Duration::from_secs(15 * 60));
        assert!(pause.active());
    }

    #[test]
    fn repeated_pause_does_not_extend_the_deadline() {
        let now = start();
        let mut pause = PauseController::new();
        let first = pause.request(PauseDuration::Five, now);
        let repeated = pause.request(PauseDuration::Sixty, now + Duration::from_secs(1));
        assert_eq!(
            repeated,
            match first {
                PauseRequest::Started {
                    generation,
                    deadline,
                } => PauseRequest::Repeated {
                    generation,
                    deadline,
                },
                PauseRequest::Repeated { .. } => unreachable!(),
            }
        );
    }

    #[test]
    fn resume_clears_pause_and_invalidates_the_old_generation() {
        let now = start();
        let mut pause = PauseController::new();
        pause.request(PauseDuration::Five, now);
        assert!(pause.resume());
        assert!(!pause.active());
        assert_eq!(
            pause.timer_event(1, now + Duration::from_secs(600)),
            PauseTimerEvent::Stale
        );
        assert!(!pause.resume());
    }

    #[test]
    fn timer_expiration_is_exact_and_early_events_are_bounded() {
        let now = start();
        let mut pause = PauseController::new();
        pause.request(PauseDuration::Five, now);
        assert!(matches!(
            pause.timer_event(1, now + Duration::from_secs(1)),
            PauseTimerEvent::Early { remaining, .. } if remaining == Duration::from_secs(299)
        ));
        assert_eq!(
            pause.timer_event(1, now + Duration::from_secs(300)),
            PauseTimerEvent::Expired { generation: 1 }
        );
    }

    #[test]
    fn stale_timer_generation_is_ignored() {
        let now = start();
        let mut pause = PauseController::new();
        pause.request(PauseDuration::Five, now);
        pause.resume();
        pause.request(PauseDuration::Thirty, now);
        assert_eq!(
            pause.timer_event(1, now + Duration::from_secs(600)),
            PauseTimerEvent::Stale
        );
        assert!(matches!(
            pause.timer_event(3, now + Duration::from_secs(1)),
            PauseTimerEvent::Early { generation: 3, .. }
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
    fn shutdown_clears_a_session_pause_without_persisting_it() {
        let now = start();
        let mut pause = PauseController::new();
        pause.request(PauseDuration::Five, now);
        assert!(pause.resume());
        assert!(!pause.active());
    }

    #[test]
    fn timer_interval_is_at_least_one_and_hard_bounded() {
        assert_eq!(timer_interval_ms(Duration::ZERO), 1);
        assert_eq!(timer_interval_ms(MAX_PAUSE_DURATION), MAX_TIMER_INTERVAL_MS);
        assert_eq!(
            timer_interval_ms(Duration::from_secs(60 * 60 + 1)),
            MAX_TIMER_INTERVAL_MS
        );
    }
}
