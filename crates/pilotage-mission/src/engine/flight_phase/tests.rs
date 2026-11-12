#![allow(clippy::expect_used, clippy::panic)]

use super::{
    APPROACH_RNP_NM, CifpProcedureKind, ENROUTE_RNP_NM, LegRole, M_PER_NM, PhaseMachine,
    ProcedureFixRole, TERMINAL_RNP_NM, default_rnp_nm, effective_rnp_nm,
};
use crate::FlightPhase;

#[test]
fn untagged_plan_remains_enroute_at_two_nm() {
    let roles = [None, None, None];
    for idx in 0..roles.len() {
        let phase = PhaseMachine::evaluate(&roles, idx);
        assert_eq!(phase, FlightPhase::Enroute);
        assert_eq!(default_rnp_nm(phase), ENROUTE_RNP_NM);
    }
}

#[test]
fn departure_to_untagged_airway_to_arrival_to_approach_to_missed_transitions() {
    let roles = [
        Some(LegRole::Departure),
        Some(LegRole::Departure),
        None,
        Some(LegRole::Arrival),
        None,
        Some(LegRole::Approach),
        None,
        Some(LegRole::MissedApproach),
        Some(LegRole::Arrival),
    ];
    let expected = [
        (FlightPhase::Terminal, TERMINAL_RNP_NM),
        (FlightPhase::Terminal, TERMINAL_RNP_NM),
        (FlightPhase::Enroute, ENROUTE_RNP_NM),
        (FlightPhase::Terminal, TERMINAL_RNP_NM),
        (FlightPhase::Terminal, TERMINAL_RNP_NM),
        (FlightPhase::Approach, APPROACH_RNP_NM),
        (FlightPhase::Approach, APPROACH_RNP_NM),
        (FlightPhase::Missed, TERMINAL_RNP_NM),
        (FlightPhase::Terminal, TERMINAL_RNP_NM),
    ];

    for (idx, &(want_phase, want_rnp)) in expected.iter().enumerate() {
        let phase = PhaseMachine::evaluate(&roles, idx);
        assert_eq!(phase, want_phase, "leg {idx}");
        assert!(
            (default_rnp_nm(phase) - want_rnp).abs() < 1e-12,
            "leg {idx}"
        );
    }
}

#[test]
fn cifp_fix_roles_classify_initial_intermediate_final_and_missed_segments() {
    assert_eq!(
        LegRole::from_cifp(CifpProcedureKind::Sid, None, false),
        LegRole::Departure
    );
    assert_eq!(
        LegRole::from_cifp(CifpProcedureKind::Star, None, false),
        LegRole::Arrival
    );
    assert_eq!(
        LegRole::from_cifp(
            CifpProcedureKind::Approach,
            Some(ProcedureFixRole::InitialApproachFix),
            false,
        ),
        LegRole::Arrival
    );
    assert_eq!(
        LegRole::from_cifp(
            CifpProcedureKind::Approach,
            Some(ProcedureFixRole::IntermediateFix),
            false,
        ),
        LegRole::Arrival
    );
    assert_eq!(
        LegRole::from_cifp(
            CifpProcedureKind::Approach,
            Some(ProcedureFixRole::FinalApproachFix),
            false,
        ),
        LegRole::Arrival
    );
    assert_eq!(
        LegRole::from_cifp(
            CifpProcedureKind::Approach,
            Some(ProcedureFixRole::StepDownFix),
            false,
        ),
        LegRole::Approach
    );
    assert_eq!(
        LegRole::from_cifp(
            CifpProcedureKind::Approach,
            Some(ProcedureFixRole::MissedApproachPoint),
            false,
        ),
        LegRole::Approach
    );
    assert_eq!(
        LegRole::from_cifp(
            CifpProcedureKind::Approach,
            Some(ProcedureFixRole::Runway),
            false,
        ),
        LegRole::Approach
    );
    assert_eq!(
        LegRole::from_cifp(
            CifpProcedureKind::Approach,
            Some(ProcedureFixRole::MissedApproachHoldingFix),
            false,
        ),
        LegRole::MissedApproach
    );
    assert_eq!(
        LegRole::from_cifp(CifpProcedureKind::Approach, None, true),
        LegRole::MissedApproach
    );
}

#[test]
fn procedure_rnp_tightens_but_never_loosens_phase_default() {
    assert!((effective_rnp_nm(FlightPhase::Approach, Some(0.10)) - 0.10).abs() < 1e-12);
    assert!((effective_rnp_nm(FlightPhase::Approach, Some(1.0)) - APPROACH_RNP_NM).abs() < 1e-12);
    assert!((effective_rnp_nm(FlightPhase::Terminal, Some(0.5)) - 0.5).abs() < 1e-12);
    assert!((effective_rnp_nm(FlightPhase::Approach, None) * M_PER_NM - 555.6).abs() < 1e-9);
}
