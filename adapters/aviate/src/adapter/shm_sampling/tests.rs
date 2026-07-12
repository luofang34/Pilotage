#![allow(clippy::expect_used, clippy::panic)]

use pilotage_adapter_api::SourceIncarnation;
use pilotage_protocol::VehicleId;

use crate::shm::GzStateSample;

use super::batch_from_sample;

#[test]
fn simulator_sample_has_explicit_ground_truth_authorization() {
    let batch = batch_from_sample(
        VehicleId::new(1),
        0,
        GzStateSample {
            quat_wxyz: [1.0, 0.0, 0.0, 0.0],
            rates_rps: [0.0; 3],
            pos_ned_m: [0.0; 3],
            vel_ned_mps: [0.0; 3],
            time_us: 42,
            seq: 7,
        },
        0,
        3,
        SourceIncarnation::new([0x5a; 16]),
    );

    let avionics = batch.samples[0].avionics.expect("avionics");
    let status = avionics.estimator_status_stamp.expect("status stamp");
    assert_eq!(status, avionics.attitude.expect("attitude").stamp);
    assert_eq!(status, avionics.kinematics.expect("kinematics").stamp);
    assert_eq!(avionics.valid_flags, 0x0f);
    assert_eq!(avionics.quality, 0);
}
