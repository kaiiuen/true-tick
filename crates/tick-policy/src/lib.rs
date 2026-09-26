//! Pure policy decisions for the True™ Tick skeleton.
//!
//! This crate aggregates logical reasons only. It does not observe the system
//! and does not call a platform adapter.

use tick_core::Status;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PowerState {
    Ac,
    Battery,
    BatterySaver,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyReason {
    NoEligibleProfile,
    EligibleProfile,
    BatteryRestricted,
    BatterySaverRestricted,
    PowerUnknown,
    GloballyDisabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PolicyInput {
    pub enabled: bool,
    pub eligible_profile: bool,
    pub power: PowerState,
    pub battery_lockout_enabled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PolicyDecision {
    pub status: Status,
    pub reason: PolicyReason,
}

pub fn decide(input: PolicyInput) -> PolicyDecision {
    if !input.enabled {
        return PolicyDecision {
            status: Status::Blocked,
            reason: PolicyReason::GloballyDisabled,
        };
    }

    if !input.eligible_profile {
        return PolicyDecision {
            status: Status::Released,
            reason: PolicyReason::NoEligibleProfile,
        };
    }

    match input.power {
        PowerState::Ac => PolicyDecision {
            status: Status::Requested,
            reason: PolicyReason::EligibleProfile,
        },
        PowerState::Battery => {
            if input.battery_lockout_enabled {
                PolicyDecision {
                    status: Status::Blocked,
                    reason: PolicyReason::BatteryRestricted,
                }
            } else {
                PolicyDecision {
                    status: Status::Requested,
                    reason: PolicyReason::EligibleProfile,
                }
            }
        }
        PowerState::BatterySaver => {
            if input.battery_lockout_enabled {
                PolicyDecision {
                    status: Status::Blocked,
                    reason: PolicyReason::BatterySaverRestricted,
                }
            } else {
                PolicyDecision {
                    status: Status::Requested,
                    reason: PolicyReason::EligibleProfile,
                }
            }
        }
        PowerState::Unknown => PolicyDecision {
            status: Status::Blocked,
            reason: PolicyReason::PowerUnknown,
        },
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReasonSet(Vec<PolicyReason>);

impl ReasonSet {
    pub fn from_reasons(reasons: impl IntoIterator<Item = PolicyReason>) -> Self {
        let mut unique = Vec::new();
        for reason in reasons {
            if !unique.contains(&reason) {
                unique.push(reason);
            }
        }
        Self(unique)
    }

    pub fn as_slice(&self) -> &[PolicyReason] {
        &self.0
    }
}

/// Number of unhandled anomalies that forces a quiescent tier.
pub const MAX_ANOMALIES_BEFORE_QUIESCENT: u32 = 3;

/// Number of kernel rejected requests that forces a floored timing tier.
pub const KERNEL_REJECTIONS_BEFORE_FLOOR: u32 = 3;

/// Conservative timing floor used when the kernel rejects high resolution requests.
pub const FALLBACK_FLOOR_HNS: u64 = 10 * 1_000;

/// Operating capability tier, ordered from most to least capable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperatingTier {
    Nominal,
    SurfaceDegraded,
    MetrologyDegraded { floored_hns: u64 },
    Quiescent,
}

/// Transition requested by a single evaluation step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TierTransition {
    Stay,
    EnterSurfaceDegraded,
    EnterMetrologyDegraded { floored_hns: u64 },
    EnterQuiescent,
    Recover,
}

/// Inputs used to evaluate the next tier transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TierContext {
    pub tray_surface_available: bool,
    pub kernel_rejected_requests: u32,
    pub unhandled_anomalies: u32,
    pub current_floor_hns: u64,
    pub nominally_recoverable: bool,
}

/// Pure evaluation of the next tier transition.
pub fn evaluate_tier(current: OperatingTier, ctx: &TierContext) -> TierTransition {
    if ctx.unhandled_anomalies >= MAX_ANOMALIES_BEFORE_QUIESCENT {
        return TierTransition::EnterQuiescent;
    }

    if !ctx.tray_surface_available && current == OperatingTier::Nominal {
        return TierTransition::EnterSurfaceDegraded;
    }

    if ctx.kernel_rejected_requests >= KERNEL_REJECTIONS_BEFORE_FLOOR
        && !matches!(
            current,
            OperatingTier::MetrologyDegraded { .. } | OperatingTier::Quiescent
        )
    {
        return TierTransition::EnterMetrologyDegraded {
            floored_hns: FALLBACK_FLOOR_HNS,
        };
    }

    if matches!(
        current,
        OperatingTier::SurfaceDegraded | OperatingTier::MetrologyDegraded { .. }
    ) && ctx.nominally_recoverable
        && ctx.unhandled_anomalies == 0
    {
        return TierTransition::Recover;
    }

    TierTransition::Stay
}

/// Whether a tier permits high resolution timing requests.
pub fn tier_allows_high_resolution(tier: OperatingTier) -> bool {
    matches!(
        tier,
        OperatingTier::Nominal | OperatingTier::SurfaceDegraded
    )
}

/// Responsiveness classification derived from observed latencies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResponsivenessState {
    Normal,
    Elevated,
    Degraded,
}

/// Weight shift applied to the latency baseline estimator.
pub const LATENCY_BASELINE_SHIFT: u32 = 3;

/// Percent of the baseline above which a sample counts as anomalous.
pub const ANOMALY_FACTOR_PERCENT: u64 = 300;

/// Absolute floor in milliseconds for the anomaly threshold.
pub const ANOMALY_ABSOLUTE_FLOOR_MS: u64 = 250;

/// Consecutive anomalous samples required to reach the degraded state.
pub const ESCALATE_CONSECUTIVE_SAMPLES: u32 = 3;

/// Consecutive clean samples required to recover to normal.
pub const RECOVER_CONSECUTIVE_SAMPLES: u32 = 5;

/// Percent of the episode worst that escalates to degraded immediately.
pub const WORSENING_FACTOR_PERCENT: u64 = 200;

/// Pure integer latency classifier with hysteresis and no allocation.
#[derive(Clone, Debug)]
pub struct ResponsivenessTracker {
    baseline_ms: u64,
    worst_ms: u64,
    anomalous_streak: u32,
    clean_streak: u32,
    seeded: bool,
    state: ResponsivenessState,
}

impl ResponsivenessTracker {
    pub fn new() -> Self {
        Self {
            baseline_ms: 0,
            worst_ms: 0,
            anomalous_streak: 0,
            clean_streak: 0,
            seeded: false,
            state: ResponsivenessState::Normal,
        }
    }

    pub fn observe(&mut self, latency_ms: u64) -> ResponsivenessState {
        if !self.seeded {
            self.baseline_ms = latency_ms;
            self.seeded = true;
        }

        let scaled_baseline = self.baseline_ms.saturating_mul(ANOMALY_FACTOR_PERCENT) / 100;
        let threshold = scaled_baseline.max(ANOMALY_ABSOLUTE_FLOOR_MS);

        if latency_ms > threshold {
            let worsening = self.worst_ms != 0
                && latency_ms >= self.worst_ms.saturating_mul(WORSENING_FACTOR_PERCENT) / 100;
            self.worst_ms = self.worst_ms.max(latency_ms);
            self.anomalous_streak = self.anomalous_streak.saturating_add(1);
            self.clean_streak = 0;
            self.state = if worsening || self.anomalous_streak >= ESCALATE_CONSECUTIVE_SAMPLES {
                ResponsivenessState::Degraded
            } else {
                ResponsivenessState::Elevated
            };
        } else {
            let weight = (1u64 << LATENCY_BASELINE_SHIFT) - 1;
            self.baseline_ms = self
                .baseline_ms
                .saturating_mul(weight)
                .saturating_add(latency_ms)
                >> LATENCY_BASELINE_SHIFT;
            self.anomalous_streak = 0;
            self.clean_streak = self.clean_streak.saturating_add(1);
            if self.clean_streak >= RECOVER_CONSECUTIVE_SAMPLES {
                self.state = ResponsivenessState::Normal;
                self.worst_ms = 0;
            }
        }

        self.state
    }

    pub fn state(&self) -> ResponsivenessState {
        self.state
    }

    pub fn baseline_ms(&self) -> u64 {
        self.baseline_ms
    }

    pub fn worst_ms(&self) -> u64 {
        self.worst_ms
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }
}

impl Default for ResponsivenessTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restrictive_power_never_requests() {
        for power in [PowerState::Battery, PowerState::BatterySaver] {
            let input = PolicyInput {
                enabled: true,
                eligible_profile: true,
                power,
                battery_lockout_enabled: true,
            };
            assert_eq!(decide(input).status, Status::Blocked);
        }
    }

    #[test]
    fn ac_allows_an_eligible_request() {
        let input = PolicyInput {
            enabled: true,
            eligible_profile: true,
            power: PowerState::Ac,
            battery_lockout_enabled: true,
        };
        assert_eq!(decide(input).status, Status::Requested);
    }

    #[test]
    fn battery_is_released_by_default() {
        let input = PolicyInput {
            enabled: true,
            eligible_profile: true,
            power: PowerState::Battery,
            battery_lockout_enabled: true,
        };
        assert_eq!(decide(input).status, Status::Blocked);
        assert_eq!(decide(input).reason, PolicyReason::BatteryRestricted);
    }

    #[test]
    fn dc_does_not_block_when_lockout_is_disabled() {
        let input = PolicyInput {
            enabled: true,
            eligible_profile: true,
            power: PowerState::Battery,
            battery_lockout_enabled: false,
        };
        let decision = decide(input);
        assert_eq!(decision.status, Status::Requested);
        assert_eq!(decision.reason, PolicyReason::EligibleProfile);
    }

    #[test]
    fn battery_saver_does_not_block_when_lockout_is_disabled() {
        let input = PolicyInput {
            enabled: true,
            eligible_profile: true,
            power: PowerState::BatterySaver,
            battery_lockout_enabled: false,
        };
        let decision = decide(input);
        assert_eq!(decision.status, Status::Requested);
        assert_eq!(decision.reason, PolicyReason::EligibleProfile);
    }

    #[test]
    fn dc_blocks_when_lockout_is_enabled() {
        let input = PolicyInput {
            enabled: true,
            eligible_profile: true,
            power: PowerState::Battery,
            battery_lockout_enabled: true,
        };
        let decision = decide(input);
        assert_eq!(decision.status, Status::Blocked);
        assert_eq!(decision.reason, PolicyReason::BatteryRestricted);
    }

    #[test]
    fn battery_saver_blocks_when_lockout_is_enabled() {
        let input = PolicyInput {
            enabled: true,
            eligible_profile: true,
            power: PowerState::BatterySaver,
            battery_lockout_enabled: true,
        };
        let decision = decide(input);
        assert_eq!(decision.status, Status::Blocked);
        assert_eq!(decision.reason, PolicyReason::BatterySaverRestricted);
    }

    #[test]
    fn unknown_power_is_conservatively_blocked() {
        for battery_lockout_enabled in [false, true] {
            let input = PolicyInput {
                enabled: true,
                eligible_profile: true,
                power: PowerState::Unknown,
                battery_lockout_enabled,
            };
            assert_eq!(decide(input).status, Status::Blocked);
        }
    }

    #[test]
    fn duplicate_reasons_are_aggregated_once() {
        let reasons =
            ReasonSet::from_reasons([PolicyReason::EligibleProfile, PolicyReason::EligibleProfile]);
        assert_eq!(reasons.as_slice(), &[PolicyReason::EligibleProfile]);
    }

    fn healthy_ctx() -> TierContext {
        TierContext {
            tray_surface_available: true,
            kernel_rejected_requests: 0,
            unhandled_anomalies: 0,
            current_floor_hns: 0,
            nominally_recoverable: false,
        }
    }

    #[test]
    fn nominal_stays_nominal_when_all_capabilities_present() {
        let ctx = healthy_ctx();
        assert_eq!(
            evaluate_tier(OperatingTier::Nominal, &ctx),
            TierTransition::Stay
        );
    }

    #[test]
    fn missing_tray_surface_enters_surface_degraded() {
        let ctx = TierContext {
            tray_surface_available: false,
            ..healthy_ctx()
        };
        assert_eq!(
            evaluate_tier(OperatingTier::Nominal, &ctx),
            TierTransition::EnterSurfaceDegraded
        );
    }

    #[test]
    fn repeated_kernel_rejections_enter_metrology_degraded_with_floor() {
        let ctx = TierContext {
            kernel_rejected_requests: KERNEL_REJECTIONS_BEFORE_FLOOR,
            ..healthy_ctx()
        };
        assert_eq!(
            evaluate_tier(OperatingTier::Nominal, &ctx),
            TierTransition::EnterMetrologyDegraded {
                floored_hns: FALLBACK_FLOOR_HNS
            }
        );
        assert_eq!(
            evaluate_tier(OperatingTier::SurfaceDegraded, &ctx),
            TierTransition::EnterMetrologyDegraded {
                floored_hns: FALLBACK_FLOOR_HNS
            }
        );
    }

    #[test]
    fn anomaly_threshold_enters_quiescent_and_overrides_other_transitions() {
        let ctx = TierContext {
            tray_surface_available: false,
            kernel_rejected_requests: KERNEL_REJECTIONS_BEFORE_FLOOR,
            unhandled_anomalies: MAX_ANOMALIES_BEFORE_QUIESCENT,
            nominally_recoverable: true,
            ..healthy_ctx()
        };
        assert_eq!(
            evaluate_tier(OperatingTier::Nominal, &ctx),
            TierTransition::EnterQuiescent
        );
        assert_eq!(
            evaluate_tier(
                OperatingTier::MetrologyDegraded {
                    floored_hns: FALLBACK_FLOOR_HNS
                },
                &ctx
            ),
            TierTransition::EnterQuiescent
        );
    }

    #[test]
    fn recover_returns_to_nominal_when_healthy() {
        let ctx = TierContext {
            nominally_recoverable: true,
            ..healthy_ctx()
        };
        assert_eq!(
            evaluate_tier(OperatingTier::SurfaceDegraded, &ctx),
            TierTransition::Recover
        );
        assert_eq!(
            evaluate_tier(
                OperatingTier::MetrologyDegraded {
                    floored_hns: FALLBACK_FLOOR_HNS
                },
                &ctx
            ),
            TierTransition::Recover
        );
        assert_eq!(
            evaluate_tier(OperatingTier::Nominal, &ctx),
            TierTransition::Stay
        );
    }

    #[test]
    fn quiescent_never_recovers_without_anomaly_clearing() {
        let ctx = TierContext {
            unhandled_anomalies: 1,
            nominally_recoverable: true,
            ..healthy_ctx()
        };
        assert_eq!(
            evaluate_tier(OperatingTier::Quiescent, &ctx),
            TierTransition::Stay
        );
        assert_eq!(
            evaluate_tier(OperatingTier::SurfaceDegraded, &ctx),
            TierTransition::Stay
        );
    }

    #[test]
    fn tier_allows_high_resolution_matches_tier_capability() {
        assert!(tier_allows_high_resolution(OperatingTier::Nominal));
        assert!(tier_allows_high_resolution(OperatingTier::SurfaceDegraded));
        assert!(!tier_allows_high_resolution(
            OperatingTier::MetrologyDegraded {
                floored_hns: FALLBACK_FLOOR_HNS
            }
        ));
        assert!(!tier_allows_high_resolution(OperatingTier::Quiescent));
    }

    #[test]
    fn new_tracker_is_normal_with_zero_baseline() {
        let tracker = ResponsivenessTracker::new();
        assert_eq!(tracker.state(), ResponsivenessState::Normal);
        assert_eq!(tracker.baseline_ms(), 0);
        assert_eq!(tracker.worst_ms(), 0);
        let default_tracker = ResponsivenessTracker::default();
        assert_eq!(default_tracker.state(), ResponsivenessState::Normal);
    }

    #[test]
    fn steady_samples_stay_normal() {
        let mut tracker = ResponsivenessTracker::new();
        for _ in 0..20 {
            assert_eq!(tracker.observe(100), ResponsivenessState::Normal);
        }
        assert_eq!(tracker.baseline_ms(), 100);
        assert_eq!(tracker.worst_ms(), 0);
    }

    #[test]
    fn single_spike_is_elevated_not_degraded() {
        let mut tracker = ResponsivenessTracker::new();
        tracker.observe(100);
        let state = tracker.observe(1_000);
        assert_eq!(state, ResponsivenessState::Elevated);
        assert_eq!(tracker.worst_ms(), 1_000);
    }

    #[test]
    fn three_consecutive_spikes_are_degraded() {
        let mut tracker = ResponsivenessTracker::new();
        tracker.observe(100);
        assert_eq!(tracker.observe(1_000), ResponsivenessState::Elevated);
        assert_eq!(tracker.observe(1_100), ResponsivenessState::Elevated);
        assert_eq!(tracker.observe(1_200), ResponsivenessState::Degraded);
    }

    #[test]
    fn doubling_an_anomaly_escalates_immediately() {
        let mut tracker = ResponsivenessTracker::new();
        tracker.observe(100);
        assert_eq!(tracker.observe(1_000), ResponsivenessState::Elevated);
        assert_eq!(tracker.observe(2_000), ResponsivenessState::Degraded);
        assert_eq!(tracker.worst_ms(), 2_000);
    }

    #[test]
    fn recovery_requires_full_count_then_clears_worst() {
        let mut tracker = ResponsivenessTracker::new();
        tracker.observe(100);
        tracker.observe(1_000);
        tracker.observe(2_000);
        assert_eq!(tracker.state(), ResponsivenessState::Degraded);
        for _ in 0..RECOVER_CONSECUTIVE_SAMPLES - 1 {
            assert_eq!(tracker.observe(100), ResponsivenessState::Degraded);
        }
        assert_eq!(tracker.worst_ms(), 2_000);
        assert_eq!(tracker.observe(100), ResponsivenessState::Normal);
        assert_eq!(tracker.worst_ms(), 0);
    }

    #[test]
    fn spike_does_not_move_baseline_but_normal_samples_do() {
        let mut tracker = ResponsivenessTracker::new();
        tracker.observe(100);
        tracker.observe(100);
        tracker.observe(100);
        let before = tracker.baseline_ms();
        tracker.observe(4_000);
        assert_eq!(tracker.baseline_ms(), before);
        tracker.observe(200);
        assert!(tracker.baseline_ms() > before);
    }

    #[test]
    fn max_u64_sample_does_not_panic() {
        let mut tracker = ResponsivenessTracker::new();
        tracker.observe(u64::MAX);
        tracker.observe(u64::MAX);
        tracker.observe(100);
        tracker.observe(u64::MAX);
        tracker.reset();
        assert_eq!(tracker.state(), ResponsivenessState::Normal);
        assert_eq!(tracker.baseline_ms(), 0);
        assert_eq!(tracker.worst_ms(), 0);
    }

    #[test]
    fn zero_baseline_never_divides_by_zero() {
        let mut tracker = ResponsivenessTracker::new();
        assert_eq!(tracker.observe(0), ResponsivenessState::Normal);
        assert_eq!(tracker.observe(0), ResponsivenessState::Normal);
        assert_eq!(tracker.observe(300), ResponsivenessState::Elevated);
        tracker.reset();
        assert_eq!(tracker.observe(0), ResponsivenessState::Normal);
    }
}
