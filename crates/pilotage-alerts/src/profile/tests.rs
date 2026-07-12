#![allow(clippy::expect_used, clippy::panic)]

use super::*;
use crate::condition::{AlertCondition, DisplayFault, MiscompareFault};

fn caution_id() -> AlertId {
    AlertCondition::Altitude(crate::condition::AltFault::ReferenceLost).id()
}

#[test]
fn simulator_profile_is_valid_and_wired() {
    let profile = AlertProfile::simulator();
    let turn = AlertCondition::TurnSlip(DynFault::TurnRateInvalid).id();
    let db = AlertCondition::System(SystemNote::DatabaseStale).id();
    assert!(profile.is_inhibited(turn, FlightPhase::Takeoff));
    assert!(!profile.is_inhibited(turn, FlightPhase::Cruise));
    assert!(profile.is_inhibited(db, FlightPhase::Approach));
    assert_eq!(profile.escalation_ms(AlertClass::Warning), 5_000);
    assert_eq!(profile.escalation_ms(AlertClass::Caution), 10_000);
    assert_eq!(profile.escalation_ms(AlertClass::Advisory), 20_000);
    assert_eq!(profile.escalation_ms(AlertClass::Status), u64::MAX);
}

#[test]
fn zero_escalation_is_rejected() {
    assert_eq!(
        AlertProfile::new(0, 10, 10, &[]),
        Err(ProfileError::NonPositiveEscalation {
            class: AlertClass::Warning
        })
    );
    assert_eq!(
        AlertProfile::new(10, 0, 10, &[]),
        Err(ProfileError::NonPositiveEscalation {
            class: AlertClass::Caution
        })
    );
    assert_eq!(
        AlertProfile::new(10, 10, 0, &[]),
        Err(ProfileError::NonPositiveEscalation {
            class: AlertClass::Advisory
        })
    );
}

#[test]
fn too_many_inhibits_is_rejected() {
    let rule = InhibitRule {
        id: caution_id(),
        phase: FlightPhase::Cruise,
    };
    let rules = [rule; MAX_INHIBIT_RULES + 1];
    assert_eq!(
        AlertProfile::new(1, 1, 1, &rules),
        Err(ProfileError::TooManyInhibits {
            count: MAX_INHIBIT_RULES + 1
        })
    );
}

#[test]
fn exactly_the_budget_is_accepted() {
    let rule = InhibitRule {
        id: caution_id(),
        phase: FlightPhase::Cruise,
    };
    let rules = [rule; MAX_INHIBIT_RULES];
    assert!(AlertProfile::new(1, 1, 1, &rules).is_ok());
}

#[test]
fn inhibiting_a_warning_is_rejected() {
    let warning = AlertCondition::Display(DisplayFault::RendererStalled).id();
    let attitude = AlertCondition::Miscompare(MiscompareFault::Attitude).id();
    let rules = [InhibitRule {
        id: warning,
        phase: FlightPhase::Cruise,
    }];
    assert_eq!(
        AlertProfile::new(1, 1, 1, &rules),
        Err(ProfileError::UninhibitableAlert { id: warning })
    );
    let rules = [InhibitRule {
        id: attitude,
        phase: FlightPhase::Cruise,
    }];
    assert_eq!(
        AlertProfile::new(1, 1, 1, &rules),
        Err(ProfileError::UninhibitableAlert { id: attitude })
    );
}

#[test]
fn inhibiting_an_unknown_identity_is_rejected() {
    let unknown = AlertId(0x0099);
    let rules = [InhibitRule {
        id: unknown,
        phase: FlightPhase::Cruise,
    }];
    assert_eq!(
        AlertProfile::new(1, 1, 1, &rules),
        Err(ProfileError::UnknownInhibitAlert { id: unknown })
    );
}
