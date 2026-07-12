#![allow(clippy::expect_used, clippy::panic)]

use pilotage_adapter_api::{
    AvionicsAttitudeSample, AvionicsKinematicsSample, AvionicsSample, MeasurementClock,
    MeasurementStamp, Pose2d, SourceIncarnation, TelemetrySample,
};
use pilotage_protocol::{VehicleId, wire};
use pilotage_timing::{MonoTimestamp, SimTick};

use super::sample_to_wire;

#[test]
fn publication_time_and_source_acquisition_stamps_stay_distinct() {
    let attitude = MeasurementStamp {
        source_id: 7,
        source_incarnation: SourceIncarnation::new([0xA5; 16]),
        source_epoch: 3,
        sequence: u32::MAX,
        acquired_at_ns: 1_234_567,
        clock: MeasurementClock::VehicleBoot,
    };
    let kinematics = MeasurementStamp {
        sequence: 19,
        acquired_at_ns: 1_200_000,
        ..attitude
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
        avionics: Some(AvionicsSample {
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
            valid_flags: 0b1111,
            quality: 0,
            arm_state: 2,
        }),
    };

    let wire_sample = sample_to_wire(sample, MonoTimestamp::from_nanos(9_000_000));
    assert_eq!(wire_sample.vehicle.expect("vehicle").value, 9);
    assert_eq!(
        wire_sample.observed_at.expect("publication").nanos,
        9_000_000
    );
    let avionics = wire_sample.avionics.expect("avionics");
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
}

fn simulation_stamp(sequence: u32) -> MeasurementStamp {
    MeasurementStamp {
        source_id: 1,
        source_incarnation: SourceIncarnation::new([0x5A; 16]),
        source_epoch: 1,
        sequence,
        acquired_at_ns: 42,
        clock: MeasurementClock::Simulation,
    }
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
            valid_flags: 0b1100,
            quality: 0,
            arm_state: 0,
        }),
    };

    let wire_sample = sample_to_wire(sample, MonoTimestamp::from_nanos(1));
    assert!(wire_sample.pose.is_none());
    assert!(wire_sample.velocity.is_none());
    let avionics = wire_sample.avionics.expect("avionics");
    assert!(avionics.attitude_stamp.is_none());
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
            valid_flags: 0b0011,
            quality: 0,
            arm_state: 2,
        }),
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
