use super::{FlightProcedureKind, FlightProcedurePhase, FlightProcedureReason};

#[test]
fn every_procedure_value_roundtrips_without_losing_its_meaning() {
    for kind in [FlightProcedureKind::Takeoff, FlightProcedureKind::Land] {
        assert_eq!(
            FlightProcedureKind::from_wire(kind.to_wire() as i32),
            Some(kind)
        );
    }
    for phase in [
        FlightProcedurePhase::Queued,
        FlightProcedurePhase::Written,
        FlightProcedurePhase::InProgress,
        FlightProcedurePhase::Completed,
        FlightProcedurePhase::Failed,
        FlightProcedurePhase::TimedOut,
        FlightProcedurePhase::Cancelled,
        FlightProcedurePhase::OutcomeUnknown,
    ] {
        assert_eq!(
            FlightProcedurePhase::from_wire(phase.to_wire() as i32),
            Some(phase)
        );
    }
    for reason in [
        FlightProcedureReason::CommandRefused,
        FlightProcedureReason::WriteFailed,
        FlightProcedureReason::DeadlineExpired,
        FlightProcedureReason::MeasurementUnavailable,
        FlightProcedureReason::SourceChanged,
        FlightProcedureReason::AuthorityLost,
        FlightProcedureReason::VehicleDisarmed,
        FlightProcedureReason::ModeChanged,
        FlightProcedureReason::Interrupted,
    ] {
        assert_eq!(
            FlightProcedureReason::from_wire(reason.to_wire() as i32),
            Some(reason)
        );
    }
}

#[test]
fn absent_or_unknown_progress_never_implies_completion() {
    for value in [0, -1, i32::MAX] {
        assert_eq!(FlightProcedureKind::from_wire(value), None);
        assert_eq!(FlightProcedurePhase::from_wire(value), None);
        assert_eq!(FlightProcedureReason::from_wire(value), None);
    }
    for phase in [
        FlightProcedurePhase::Queued,
        FlightProcedurePhase::Written,
        FlightProcedurePhase::InProgress,
    ] {
        assert!(!phase.is_terminal());
    }
    for phase in [
        FlightProcedurePhase::Completed,
        FlightProcedurePhase::Failed,
        FlightProcedurePhase::TimedOut,
        FlightProcedurePhase::Cancelled,
        FlightProcedurePhase::OutcomeUnknown,
    ] {
        assert!(phase.is_terminal());
    }
}
