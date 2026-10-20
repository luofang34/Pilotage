use pilotage_adapter_api::{
    AvionicsAttitudeSample, AvionicsKinematicsSample, AvionicsSample, MeasurementClock,
    MeasurementStamp, SourceIncarnation, SourceIntegrity, SourceRole, TelemetrySample,
};
use pilotage_protocol::{VehicleId, wire};
use pilotage_timing::{MonoTimestamp, SimTick};
use prost::Message;

use super::super::{avionics_to_wire, sample_to_wire};

fn stamp() -> MeasurementStamp {
    MeasurementStamp {
        role: SourceRole::OperationalEstimate,
        integrity: SourceIntegrity::ChecksummedOnly,
        source_id: 17,
        source_incarnation: SourceIncarnation::new([4; 16]),
        source_epoch: 3,
        sequence: 9,
        acquired_at_ns: 31,
        clock: MeasurementClock::HostMonotonic,
    }
}

fn sample(attitude: AvionicsAttitudeSample) -> AvionicsSample {
    AvionicsSample {
        attitude: Some(attitude),
        kinematics: None,
        baro: None,
        geodetic: None,
        magnetic_variation: None,
        estimator_status_stamp: Some(stamp()),
        valid_flags: 0b0011,
        quality: 0,
    }
}

fn reported() -> AvionicsAttitudeSample {
    AvionicsAttitudeSample::ReportedEuler {
        pitch_rad: -0.2,
        roll_rad: 0.1,
        yaw_rad: 1.2,
        stamp: stamp(),
    }
}

#[derive(Clone, PartialEq, Message)]
struct CanonicalOnlyAvionics {
    #[prost(message, optional, tag = "17")]
    attitude_stamp: Option<wire::MeasurementStamp>,
}

#[test]
fn reported_roundtrip_has_no_canonical_measurement_or_position() {
    let mut avionics = sample(reported());
    avionics.estimator_status_stamp = None;
    let sample = sample_to_wire(
        TelemetrySample {
            vehicle: VehicleId::new(1),
            tick: SimTick::new(0),
            pose: None,
            speed: None,
            avionics: Some(avionics),
            payload: None,
            sim_truth: None,
            fc_state: None,
            gimbal: None,
            battery: None,
        },
        MonoTimestamp::from_nanos(70),
    );
    let decoded = wire::TelemetrySample::decode(sample.encode_to_vec().as_slice())
        .expect("telemetry decodes");
    assert!(decoded.pose.is_none());
    assert!(decoded.velocity.is_none());
    let avionics = decoded.avionics.expect("avionics");
    assert!(avionics.attitude_stamp.is_none());
    assert!(avionics.estimator_status_stamp.is_none());
    assert!(avionics.kinematics_stamp.is_none());
    assert!(avionics.geodetic.is_none());
    assert_eq!(avionics.valid_flags, 0);
    assert_eq!(avionics.quality, 2);
    let old = CanonicalOnlyAvionics::decode(avionics.encode_to_vec().as_slice())
        .expect("a canonical-only reader skips the new field");
    assert!(old.attitude_stamp.is_none());
    let report = avionics.reported_euler.expect("reported Euler");
    assert_eq!(
        [report.pitch_rad, report.roll_rad, report.yaw_rad],
        [-0.2, 0.1, 1.2]
    );
    let stamp = report.stamp.expect("report stamp");
    assert_eq!(stamp.acquired_at_ns, 31);
    assert_eq!(stamp.sequence, 9);
    assert_eq!(stamp.source_incarnation, [4; 16]);
}

#[test]
fn reported_cannot_inherit_attitude_or_rates_authorization() {
    let state = avionics_to_wire(sample(reported()));
    assert_eq!(state.valid_flags, 0);
    assert_eq!(state.quality, 2);
    assert!(state.attitude_stamp.is_none());
    assert!(state.rate_p_rad_s.is_nan());
    assert!(state.rate_q_rad_s.is_nan());
    assert!(state.rate_r_rad_s.is_nan());
}

#[test]
fn reported_preserves_independently_authorized_kinematics() {
    let mut sample = sample(reported());
    sample.valid_flags = 0b1111;
    sample.quality = 1;
    sample.kinematics = Some(AvionicsKinematicsSample {
        pos_ned_m: [2.0, 3.0, -4.0],
        vel_ned_mps: [5.0, 6.0, -7.0],
        stamp: stamp(),
    });
    let state = avionics_to_wire(sample);
    assert_eq!(state.valid_flags, 0b1100);
    assert_eq!(state.quality, 1);
    assert!(state.attitude_stamp.is_none());
    assert!(state.reported_euler.is_some());
    assert!(state.kinematics_stamp.is_some());
    assert_eq!(
        [state.pos_n_m, state.pos_e_m, state.pos_d_m],
        [2.0, 3.0, -4.0]
    );
    assert_eq!(
        [state.vel_n_mps, state.vel_e_mps, state.vel_d_mps],
        [5.0, 6.0, -7.0]
    );
}

#[test]
fn canonical_unknown_rates_stay_unknown_and_unauthorized() {
    let state = avionics_to_wire(sample(AvionicsAttitudeSample::Canonical {
        quat_wxyz: [1.0, 0.0, 0.0, 0.0],
        rates_rps: None,
        stamp: stamp(),
    }));
    assert_eq!(state.quat_w, 1.0);
    assert!(state.attitude_stamp.is_some());
    assert!(state.reported_euler.is_none());
    assert!(state.rate_p_rad_s.is_nan());
    assert!(state.rate_q_rad_s.is_nan());
    assert!(state.rate_r_rad_s.is_nan());
    assert_eq!(state.valid_flags, 1);
    assert_eq!(state.quality, 0);
}

#[test]
fn nonfinite_rates_do_not_become_authorized() {
    let state = avionics_to_wire(sample(AvionicsAttitudeSample::Canonical {
        quat_wxyz: [1.0, 0.0, 0.0, 0.0],
        rates_rps: Some([0.0, f32::NAN, 0.0]),
        stamp: stamp(),
    }));
    assert_eq!(state.valid_flags, 1);
    assert!(state.rate_p_rad_s.is_nan());
    assert!(state.rate_q_rad_s.is_nan());
    assert!(state.rate_r_rad_s.is_nan());
}

#[test]
fn a_nonfinite_report_has_no_measurement_stamp() {
    let state = avionics_to_wire(sample(AvionicsAttitudeSample::ReportedEuler {
        pitch_rad: f32::NAN,
        roll_rad: 0.1,
        yaw_rad: 1.2,
        stamp: stamp(),
    }));
    assert!(state.reported_euler.is_none());
    assert!(state.attitude_stamp.is_none());
    assert_eq!(state.valid_flags, 0);
}
