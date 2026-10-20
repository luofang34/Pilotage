use pilotage_adapter_api::{
    CommandOutcome, FlightProcedureKind, FlightProcedurePhase, FlightProcedureReason,
    FlightProcedureRequest, FlightProcedureSample, SourceIncarnation, TelemetrySample,
};
use pilotage_protocol::{Generation, ScopeId, SequenceNum, SessionId, VehicleId, wire};
use pilotage_timing::{MonoTimestamp, SimTick};
use prost::Message;

fn sample() -> TelemetrySample {
    TelemetrySample {
        vehicle: VehicleId::new(31),
        tick: SimTick::new(22),
        flight_procedure: Some(FlightProcedureSample {
            request: FlightProcedureRequest {
                session: SessionId::new(51),
                scope: ScopeId::new("vehicle.motion"),
                generation: Generation::new(71),
                sequence: SequenceNum::new(0),
                action_id: 91,
            },
            adapter_incarnation: SourceIncarnation::new([19; 16]),
            revision: 3,
            changed_at: MonoTimestamp::from_nanos(1234),
            kind: FlightProcedureKind::Takeoff,
            phase: FlightProcedurePhase::TimedOut,
            reason: Some(FlightProcedureReason::DeadlineExpired),
            fc_outcome: Some(CommandOutcome::Accepted),
        }),
        payload: None,
        pose: None,
        speed: None,
        avionics: None,
        sim_truth: None,
        fc_state: None,
        gimbal: None,
        battery: None,
    }
}

#[test]
fn flight_progress_survives_telemetry_encoding_without_implying_measurements() {
    let wire = super::super::sample_to_wire(sample(), MonoTimestamp::from_nanos(9000));
    let decoded = wire::TelemetrySample::decode(wire.encode_to_vec().as_slice())
        .expect("telemetry roundtrip");
    assert_eq!(decoded.vehicle.expect("vehicle").value, 31);
    let procedure = decoded.flight_procedure.expect("procedure");
    let request = procedure.request.expect("request identity");
    assert_eq!(request.session.expect("session").value, 51);
    assert_eq!(request.scope.expect("scope").value, "vehicle.motion");
    assert_eq!(request.generation.expect("generation").value, 71);
    assert_eq!(request.sequence.expect("sequence").value, 0);
    assert_eq!(request.action_id, 91);
    assert_eq!(procedure.adapter_incarnation, [19; 16]);
    assert_eq!(procedure.revision, 3);
    assert_eq!(procedure.changed_at.expect("transition time").nanos, 1234);
    assert_eq!(procedure.kind, wire::FlightProcedureKind::Takeoff as i32);
    assert_eq!(procedure.phase, wire::FlightProcedurePhase::TimedOut as i32);
    assert_eq!(
        procedure.reason,
        wire::FlightProcedureReason::DeadlineExpired as i32
    );
    assert_eq!(procedure.fc_outcome, wire::CommandOutcome::Accepted as i32);
    assert!(decoded.fc_state.is_none());
    assert!(decoded.avionics.is_none());
    assert!(decoded.payload.is_none());
    assert!(decoded.pose.is_none());
}

#[test]
fn publication_does_not_renew_procedure_revision_or_transition_time() {
    let early = super::super::sample_to_wire(sample(), MonoTimestamp::from_nanos(9000));
    let later = super::super::sample_to_wire(sample(), MonoTimestamp::from_nanos(19000));
    assert_ne!(early.observed_at, later.observed_at);
    assert_eq!(early.flight_procedure, later.flight_procedure);
}

#[test]
fn an_absent_procedure_does_not_invent_a_default_status() {
    let mut sample = sample();
    sample.flight_procedure = None;
    let wire = super::super::sample_to_wire(sample, MonoTimestamp::from_nanos(9000));
    let decoded = wire::TelemetrySample::decode(wire.encode_to_vec().as_slice())
        .expect("telemetry roundtrip");
    assert!(decoded.flight_procedure.is_none());
}

#[test]
fn written_without_an_fc_answer_does_not_report_acceptance_or_completion() {
    let mut sample = sample();
    let progress = sample.flight_procedure.as_mut().expect("procedure");
    progress.phase = FlightProcedurePhase::Written;
    progress.reason = None;
    progress.fc_outcome = None;
    let wire = super::super::sample_to_wire(sample, MonoTimestamp::from_nanos(9000));
    let progress = wire.flight_procedure.expect("procedure");
    assert_eq!(progress.phase, wire::FlightProcedurePhase::Written as i32);
    assert_eq!(
        progress.reason,
        wire::FlightProcedureReason::Unspecified as i32
    );
    assert_eq!(
        progress.fc_outcome,
        wire::CommandOutcome::Unspecified as i32
    );
}
