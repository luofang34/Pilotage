//! Golden trajectory test: a fixed seed and scripted control schedule must
//! reproduce an exact expected trajectory bit-for-bit (ADR-0008's
//! deterministic conformance anchor). Values below were computed once from
//! this crate's public dynamics model and pinned as the expected output.
#![allow(clippy::expect_used, clippy::panic)]

use pilotage_adapter_api::{StepBudget, VehicleAdapter};
use pilotage_adapter_reference::ReferenceAdapter;
use pilotage_protocol::{
    ControlPayload, Generation, LogicalAxisId, ScopeId, ScopedControlFrame, SequenceNum, SessionId,
    VehicleId,
};
use pilotage_timing::MonoTimestamp;

const SEED: u64 = 42;
const THROTTLE_AXIS: u16 = 2;
const STEERING_AXIS: u16 = 3;

fn control_frame(vehicle: VehicleId, throttle: f32, steering: f32) -> ScopedControlFrame {
    ScopedControlFrame {
        session: SessionId::new(1),
        vehicle,
        scope: ScopeId::new("vehicle.motion"),
        generation: Generation::new(1),
        sequence: SequenceNum::new(1),
        sampled_at: MonoTimestamp::from_nanos(0),
        profile_revision: 1,
        payload: ControlPayload {
            axes: vec![
                (LogicalAxisId::new(THROTTLE_AXIS), throttle),
                (LogicalAxisId::new(STEERING_AXIS), steering),
            ],
            edges: vec![],
        },
        intent: None,
        actions: vec![],
    }
}

#[test]
fn fixed_seed_and_schedule_reach_exact_checkpoints() {
    let vehicle = VehicleId::new(1);
    let mut adapter = ReferenceAdapter::from_seed(vehicle, SEED);

    adapter.apply_control(&control_frame(vehicle, 1.0, 0.0));
    for tick in 0..100u32 {
        if tick == 50 {
            let telemetry = adapter.sample_telemetry();
            let pose = telemetry.samples[0].pose.expect("pose");
            assert_eq!(pose.x, 2.334_679_171_762_922_5);
            assert_eq!(pose.y, -2.955_190_172_055_212_7);
            assert_eq!(pose.heading, 1.750_502_528_182_713_3);
            assert_eq!(telemetry.samples[0].speed, Some(1.773_499_543_450_863_6));
        }
        adapter.step(StepBudget { ticks: 1 });
    }

    let telemetry = adapter.sample_telemetry();
    let pose = telemetry.samples[0].pose.expect("pose");
    assert_eq!(pose.x, 2.113_161_393_120_975_6);
    assert_eq!(pose.y, -1.735_821_899_795_127_1);
    assert_eq!(pose.heading, 1.750_502_528_182_713_3);
    assert_eq!(telemetry.samples[0].speed, Some(3.153_836_508_074_175_7));

    adapter.apply_control(&control_frame(vehicle, 0.5, 0.3));
    for _ in 0..100u32 {
        adapter.step(StepBudget { ticks: 1 });
    }

    let telemetry = adapter.sample_telemetry();
    let pose = telemetry.samples[0].pose.expect("pose");
    assert_eq!(pose.x, 0.807_294_101_753_173);
    assert_eq!(pose.y, 1.300_095_689_237_339_2);
    assert_eq!(pose.heading, 2.200_502_546_064_115_4);
    assert_eq!(telemetry.samples[0].speed, Some(3.487_419_172_153_574_6));
}
