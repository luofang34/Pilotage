//! Snapshot/restore must be transparent to the trajectory: restoring a
//! snapshot and continuing to step must produce bit-identical telemetry to
//! an uninterrupted run over the same total ticks (ADR-0008, ADR-0012).
#![allow(clippy::expect_used, clippy::panic)]

use pilotage_adapter_api::{StepBudget, VehicleAdapter};
use pilotage_adapter_reference::ReferenceAdapter;
use pilotage_protocol::{
    ControlIntent, ControlPayload, Generation, ReferenceFrame, ScopeId, ScopedControlFrame,
    SequenceNum, SessionId, VehicleId, VelocityIntent,
};
use pilotage_timing::MonoTimestamp;

fn control_frame(vehicle: VehicleId) -> ScopedControlFrame {
    ScopedControlFrame {
        session: SessionId::new(1),
        vehicle,
        scope: ScopeId::new("vehicle.motion"),
        generation: Generation::new(1),
        sequence: SequenceNum::new(1),
        sampled_at: MonoTimestamp::from_nanos(0),
        profile_revision: 1,
        activation_revision: 0,
        payload: ControlPayload::default(),
        intent: Some(ControlIntent::Velocity(VelocityIntent {
            frame: ReferenceFrame::BodyFrd,
            vx: 0.8,
            vy: 0.0,
            vz: 0.0,
            yaw_rate: -0.4,
        })),
        actions: vec![],
    }
}

#[test]
fn restore_then_step_matches_uninterrupted_run() {
    let vehicle = VehicleId::new(7);
    const SEED: u64 = 2026;

    let mut uninterrupted = ReferenceAdapter::from_seed(vehicle, SEED);
    uninterrupted.apply_control(&control_frame(vehicle));
    for _ in 0..30u32 {
        uninterrupted.step(StepBudget { ticks: 1 });
    }
    let expected = uninterrupted.sample_telemetry();

    let mut original = ReferenceAdapter::from_seed(vehicle, SEED);
    original.apply_control(&control_frame(vehicle));
    for _ in 0..12u32 {
        original.step(StepBudget { ticks: 1 });
    }
    let snapshot = original.snapshot().expect("snapshot succeeds");

    let mut restored = ReferenceAdapter::restore(&snapshot).expect("restore succeeds");
    for _ in 0..18u32 {
        restored.step(StepBudget { ticks: 1 });
    }
    let actual = restored.sample_telemetry();

    assert_eq!(actual.samples[0].pose, expected.samples[0].pose);
    assert_eq!(actual.samples[0].speed, expected.samples[0].speed);
    assert_eq!(actual.samples[0].tick, expected.samples[0].tick);
}

#[test]
fn snapshot_round_trip_preserves_link_loss_hold_countdown() {
    use pilotage_adapter_api::LinkLossPolicy;

    let vehicle = VehicleId::new(3);
    let mut adapter = ReferenceAdapter::from_seed(vehicle, 5);
    adapter.apply_control(&control_frame(vehicle));
    adapter
        .set_link_loss_policy(
            vehicle,
            &ScopeId::new("vehicle.motion"),
            Some(LinkLossPolicy::HoldBrief { ticks: 3 }),
        )
        .expect("policy enacted");
    adapter.step(StepBudget { ticks: 1 });

    let snapshot = adapter.snapshot().expect("snapshot succeeds");
    let mut restored = ReferenceAdapter::restore(&snapshot).expect("restore succeeds");

    adapter.step(StepBudget { ticks: 5 });
    restored.step(StepBudget { ticks: 5 });

    let expected = adapter.sample_telemetry();
    let actual = restored.sample_telemetry();
    assert_eq!(actual.samples[0].speed, expected.samples[0].speed);
    assert_eq!(actual.samples[0].pose, expected.samples[0].pose);
}
