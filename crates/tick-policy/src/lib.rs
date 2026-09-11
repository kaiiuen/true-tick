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
        PowerState::Battery => PolicyDecision {
            status: Status::Blocked,
            reason: PolicyReason::BatteryRestricted,
        },
        PowerState::BatterySaver => PolicyDecision {
            status: Status::Blocked,
            reason: PolicyReason::BatterySaverRestricted,
        },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restrictive_power_never_requests() {
        let input = PolicyInput {
            enabled: true,
            eligible_profile: true,
            power: PowerState::BatterySaver,
        };
        assert_eq!(decide(input).status, Status::Blocked);
    }

    #[test]
    fn unknown_power_is_conservatively_blocked() {
        let input = PolicyInput {
            enabled: true,
            eligible_profile: true,
            power: PowerState::Unknown,
        };
        assert_eq!(decide(input).status, Status::Blocked);
    }

    #[test]
    fn duplicate_reasons_are_aggregated_once() {
        let reasons =
            ReasonSet::from_reasons([PolicyReason::EligibleProfile, PolicyReason::EligibleProfile]);
        assert_eq!(reasons.as_slice(), &[PolicyReason::EligibleProfile]);
    }
}
