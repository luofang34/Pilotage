use super::*;
use turret_core::{
    Angle, CommandIdentity, CommandProgress, EndReason, Observation, ObservationSource, Tick,
};

#[test]
fn receipt_does_not_become_sample_time_or_reference_frame() {
    let value = Observation {
        value: Angle::ZERO,
        source: ObservationSource::DeviceMeasured,
        frame: None,
        sampled_at: None,
        received_at: Tick(55),
        epoch: 7,
    };
    let wire = payload_angle_observation_to_wire(&value, wire::MeasurementStamp::default());
    assert_eq!(wire.angle_rad, Some(0.0));
    assert_eq!(wire.rate_rad_s, None);
    assert_eq!(wire.sampled_at, None);
    assert_eq!(wire.received_at.map(|time| time.nanos), Some(55_000));
    assert_eq!(wire.frame, wire::PayloadReferenceFrame::Unknown as i32);
    assert_eq!(wire.connection_epoch, 7);
}

#[test]
fn cancelled_command_retains_sent_and_acknowledged_evidence() {
    let value = CommandProgress {
        identity: CommandIdentity {
            epoch: 9,
            generation: 4,
            id: 3,
            sequence: 5,
        },
        accepted_at: Tick(10),
        prepared_at: Some(Tick(20)),
        sent_at: Some(Tick(30)),
        acknowledged_at: Some(Tick(40)),
        acknowledgement_timed_out: false,
        observed_complete_at: None,
        ended: Some(EndReason::Revoked),
    };
    let wire = payload_progress_to_wire(&value, None, Tick(50));
    assert_eq!(wire.stage, wire::PayloadCommandStage::Cancelled as i32);
    assert_eq!(wire.sent_at.map(|time| time.nanos), Some(30_000));
    assert_eq!(wire.acknowledged_at.map(|time| time.nanos), Some(40_000));
    assert_eq!(wire.observed_complete_at, None);
    assert_eq!(wire.generation, 4);
}
