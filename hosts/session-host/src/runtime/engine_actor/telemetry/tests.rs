#![allow(clippy::expect_used, clippy::panic)]

use pilotage_adapter_api::{
    AvionicsAttitudeSample, AvionicsKinematicsSample, AvionicsSample, FcStateSample,
    MeasurementClock, MeasurementStamp, Pose2d, SimTruthSample, SourceIncarnation, SourceIntegrity,
    SourceRole, TelemetrySample,
};
use pilotage_protocol::{VehicleId, wire};
use pilotage_timing::{MonoTimestamp, SimTick};

use super::{avionics_to_wire, sample_to_wire};

#[test]
fn publication_time_and_source_acquisition_stamps_stay_distinct() {
    let attitude = MeasurementStamp {
        role: SourceRole::OperationalEstimate,
        integrity: SourceIntegrity::ChecksummedOnly,
        source_id: 7,
        source_incarnation: SourceIncarnation::new([0xA5; 16]),
        source_epoch: 3,
        sequence: u32::MAX,
        acquired_at_ns: 1_234_567,
        clock: MeasurementClock::VehicleBoot,
    };
    let with = |sequence, acquired_at_ns| MeasurementStamp {
        sequence,
        acquired_at_ns,
        ..attitude
    };
    let kinematics = with(19, 1_200_000);
    let estimator_status = with(20, 1_234_567);
    let avionics = AvionicsSample {
        attitude: Some(AvionicsAttitudeSample {
            quat_wxyz: [1.0, 0.0, 0.0, 0.0],
            rates_rps: [0.0; 3],
            stamp: attitude,
        }),
        kinematics: Some(AvionicsKinematicsSample {
            pos_ned_m: [0.0; 3],
            vel_ned_mps: [0.0; 3],
            stamp: kinematics,
        }),
        estimator_status_stamp: Some(estimator_status),
        valid_flags: 0b1111,
        quality: 0,
    };
    let sample = TelemetrySample {
        vehicle: VehicleId::new(9),
        tick: SimTick::new(42),
        pose: Some(Pose2d {
            x: 1.0,
            y: 2.0,
            heading: 0.5,
        }),
        speed: Some(3.0),
        avionics: Some(avionics),
        sim_truth: None,
        fc_state: None,
        gimbal: None,
    };

    let wire_sample = sample_to_wire(sample, MonoTimestamp::from_nanos(9_000_000));
    assert_eq!(wire_sample.vehicle.expect("vehicle").value, 9);
    assert_eq!(
        wire_sample.observed_at.expect("publication").nanos,
        9_000_000
    );
    let avionics = wire_sample.avionics.expect("avionics");
    assert_eq!(avionics.valid_flags, 0b1111);
    assert_eq!(avionics.quality, 0);
    let attitude_wire = avionics.attitude_stamp.expect("attitude stamp");
    assert_eq!(attitude_wire.source_id, attitude.source_id);
    assert_eq!(
        attitude_wire.source_incarnation,
        attitude.source_incarnation.into_bytes()
    );
    assert_eq!(attitude_wire.source_epoch, attitude.source_epoch);
    assert_eq!(attitude_wire.sequence, attitude.sequence);
    assert_eq!(attitude_wire.acquired_at_ns, attitude.acquired_at_ns);
    assert_eq!(
        attitude_wire.clock,
        wire::MeasurementClock::VehicleBoot as i32
    );
    let kinematics_wire = avionics.kinematics_stamp.expect("kinematics stamp");
    assert_eq!(kinematics_wire.sequence, kinematics.sequence);
    assert_eq!(kinematics_wire.acquired_at_ns, kinematics.acquired_at_ns);
    let status_wire = avionics
        .estimator_status_stamp
        .expect("estimator status stamp");
    assert_eq!(status_wire.sequence, estimator_status.sequence);
    assert_eq!(status_wire.acquired_at_ns, estimator_status.acquired_at_ns);
}

fn simulation_stamp(sequence: u32) -> MeasurementStamp {
    MeasurementStamp {
        role: SourceRole::OperationalEstimate,
        integrity: SourceIntegrity::ChecksummedOnly,
        source_id: 1,
        source_incarnation: SourceIncarnation::new([0x5A; 16]),
        source_epoch: 1,
        sequence,
        acquired_at_ns: 42,
        clock: MeasurementClock::Simulation,
    }
}

#[test]
fn estimator_authorization_is_normalized_at_wire_boundary() {
    let avionics = avionics_to_wire(AvionicsSample {
        attitude: None,
        kinematics: None,
        estimator_status_stamp: Some(simulation_stamp(1)),
        valid_flags: u32::MAX,
        quality: u32::MAX,
    });

    assert_eq!(avionics.valid_flags, 0x0f);
    assert_eq!(avionics.quality, 2);
}

#[test]
fn kinematics_only_omits_planar_projection_while_group_flows() {
    let sample = TelemetrySample {
        vehicle: VehicleId::new(1),
        tick: SimTick::new(0),
        pose: None,
        speed: None,
        avionics: Some(AvionicsSample {
            attitude: None,
            kinematics: Some(AvionicsKinematicsSample {
                pos_ned_m: [12.0, 3.0, -40.0],
                vel_ned_mps: [5.0, 2.0, -1.0],
                stamp: simulation_stamp(8),
            }),
            estimator_status_stamp: None,
            valid_flags: 0b1100,
            quality: 0,
        }),
        sim_truth: None,
        fc_state: None,
        gimbal: None,
    };

    let wire_sample = sample_to_wire(sample, MonoTimestamp::from_nanos(1));
    assert!(wire_sample.pose.is_none());
    assert!(wire_sample.velocity.is_none());
    let avionics = wire_sample.avionics.expect("avionics");
    assert!(avionics.attitude_stamp.is_none());
    assert!(avionics.estimator_status_stamp.is_none());
    assert_eq!(avionics.valid_flags, 0);
    assert_eq!(avionics.quality, 2);
    assert_eq!(avionics.pos_n_m, 12.0);
    assert_eq!(avionics.vel_n_mps, 5.0);
    assert_eq!(
        avionics.kinematics_stamp.expect("kinematics").clock,
        wire::MeasurementClock::Simulation as i32
    );
}

#[test]
fn attitude_only_omits_planar_projection_while_group_flows() {
    let sample = TelemetrySample {
        vehicle: VehicleId::new(1),
        tick: SimTick::new(0),
        pose: None,
        speed: None,
        avionics: Some(AvionicsSample {
            attitude: Some(AvionicsAttitudeSample {
                quat_wxyz: [0.7, 0.0, 0.0, 0.7],
                rates_rps: [0.1, 0.2, 0.3],
                stamp: simulation_stamp(9),
            }),
            kinematics: None,
            estimator_status_stamp: None,
            valid_flags: 0b0011,
            quality: 0,
        }),
        sim_truth: None,
        fc_state: None,
        gimbal: None,
    };

    let wire_sample = sample_to_wire(sample, MonoTimestamp::from_nanos(1));
    assert!(wire_sample.pose.is_none());
    assert!(wire_sample.velocity.is_none());
    let avionics = wire_sample.avionics.expect("avionics");
    assert_eq!(avionics.quat_w, 0.7);
    assert_eq!(avionics.rate_r_rad_s, 0.3);
    assert_eq!(avionics.attitude_stamp.expect("attitude").sequence, 9);
    assert!(avionics.kinematics_stamp.is_none());
}

#[test]
fn truth_and_fc_state_survive_the_wire_under_their_own_identities() {
    let truth_stamp = MeasurementStamp {
        role: SourceRole::SimulationTruth,
        integrity: SourceIntegrity::Unprotected,
        source_id: 1,
        source_incarnation: SourceIncarnation::new([0x11; 16]),
        source_epoch: 2,
        sequence: 40,
        acquired_at_ns: 1_000_000,
        clock: MeasurementClock::Simulation,
    };
    let fc_stamp = MeasurementStamp {
        role: SourceRole::FcState,
        integrity: SourceIntegrity::ChecksummedOnly,
        source_id: 1,
        source_incarnation: SourceIncarnation::new([0x22; 16]),
        source_epoch: 1,
        sequence: 3,
        acquired_at_ns: 77,
        clock: MeasurementClock::HostMonotonic,
    };
    let sample = TelemetrySample {
        vehicle: VehicleId::new(1),
        tick: SimTick::new(1_000_000),
        pose: None,
        speed: None,
        // No estimate exists: nothing may synthesize one from truth.
        avionics: None,
        sim_truth: Some(SimTruthSample {
            quat_wxyz: [1.0, 0.0, 0.0, 0.0],
            pos_ned_m: [2.0, 1.0, -3.0],
            vel_ned_mps: [0.0, 0.5, 1.0],
            valid_flags: 0b1101,
            stamp: truth_stamp,
        }),
        fc_state: Some(FcStateSample {
            arm_state: 2,
            last_command: None,
            stamp: fc_stamp,
        }),
        gimbal: None,
    };

    let wire_sample = sample_to_wire(sample, MonoTimestamp::from_nanos(5));
    assert!(
        wire_sample.avionics.is_none(),
        "truth must not be projected into the estimator lane"
    );
    let truth = wire_sample.sim_truth.expect("sim truth");
    assert_eq!(truth.pos_n_m, 2.0);
    assert_eq!(truth.vel_d_mps, 1.0);
    assert_eq!(truth.valid_flags, 0b1101);
    let truth_wire_stamp = truth.stamp.expect("truth stamp");
    assert_eq!(
        truth_wire_stamp.role,
        wire::SourceRole::SimulationTruth as i32
    );
    assert_eq!(
        truth_wire_stamp.integrity,
        wire::SourceIntegrity::Unprotected as i32
    );
    assert_eq!(
        truth_wire_stamp.clock,
        wire::MeasurementClock::Simulation as i32
    );
    let fc = wire_sample.fc_state.expect("fc state");
    assert_eq!(fc.arm_state, 2);
    let fc_wire_stamp = fc.stamp.expect("fc stamp");
    assert_eq!(fc_wire_stamp.role, wire::SourceRole::FcState as i32);
    assert_eq!(
        fc_wire_stamp.clock,
        wire::MeasurementClock::HostMonotonic as i32
    );
    assert_ne!(
        truth_wire_stamp.source_incarnation, fc_wire_stamp.source_incarnation,
        "roles carry independent identities"
    );
}

#[test]
fn the_fc_command_verdict_crosses_the_wire() {
    let stamp = MeasurementStamp {
        role: SourceRole::FcState,
        integrity: SourceIntegrity::ChecksummedOnly,
        source_id: 1,
        source_incarnation: SourceIncarnation::new([0x22; 16]),
        source_epoch: 1,
        sequence: 3,
        acquired_at_ns: 77,
        clock: MeasurementClock::HostMonotonic,
    };
    // A refused arm: kind 1 with the raw MAV_RESULT.
    let refused = super::fc_state_to_wire(FcStateSample {
        arm_state: 1,
        last_command: Some(pilotage_adapter_api::FcCommandAck {
            arm: true,
            result: 4,
        }),
        stamp,
    });
    assert_eq!(
        refused.last_command_kind, 1,
        "an arm verdict maps to kind 1"
    );
    assert_eq!(refused.last_command_result, 4, "the raw MAV_RESULT crosses");
    // No verdict observed: the lane stays at kind 0.
    let none = super::fc_state_to_wire(FcStateSample {
        arm_state: 2,
        last_command: None,
        stamp,
    });
    assert_eq!(none.last_command_kind, 0);
    assert_eq!(none.last_command_result, 0);
}
