use crate::control::{
    ButtonEdge, ControlPayload, LogicalAxisId, LogicalButtonId, ScopedControlFrame,
};
use crate::ids::{Generation, ScopeId, SequenceNum, SessionId, VehicleId};
use crate::wire;
use pilotage_timing::MonoTimestamp;
use proptest::prelude::*;

fn arb_axis_value() -> impl Strategy<Value = f32> {
    // Finite f32 in [-1.0, 1.0], matching the domain's canonical axis
    // convention; NaN/inf are excluded because they are not valid
    // control-frame axis samples.
    (-1.0f32..=1.0f32).prop_filter("must be finite", |v| v.is_finite())
}

fn arb_button_edge() -> impl Strategy<Value = ButtonEdge> {
    prop_oneof![Just(ButtonEdge::Pressed), Just(ButtonEdge::Released)]
}

fn arb_payload() -> impl Strategy<Value = ControlPayload> {
    (
        prop::collection::vec((any::<u16>(), arb_axis_value()), 0..8),
        prop::collection::vec((any::<u16>(), arb_button_edge()), 0..8),
    )
        .prop_map(|(axes, edges)| ControlPayload {
            axes: axes
                .into_iter()
                .map(|(id, value)| (LogicalAxisId::new(id), value))
                .collect(),
            edges: edges
                .into_iter()
                .map(|(id, edge)| (LogicalButtonId::new(id), edge))
                .collect(),
        })
}

fn arb_control_frame() -> impl Strategy<Value = ScopedControlFrame> {
    (
        any::<u64>(),
        any::<u64>(),
        any::<u64>(),
        any::<u32>(),
        any::<u64>(),
        any::<u32>(),
        arb_payload(),
    )
        .prop_map(
            |(session, vehicle, generation, sequence, sampled_at, profile_revision, payload)| {
                ScopedControlFrame {
                    timing: None,
                    admitted_timing: None,
                    action_ids: vec![],
                    session: SessionId::new(session),
                    vehicle: VehicleId::new(vehicle),
                    scope: ScopeId::new("vehicle.motion"),
                    generation: Generation::new(generation),
                    sequence: SequenceNum::new(sequence),
                    sampled_at: MonoTimestamp::from_nanos(sampled_at),
                    profile_revision,
                    activation_revision: 0,
                    payload,
                    intent: None,
                    actions: vec![],
                }
            },
        )
}

proptest! {
    #[test]
    fn control_frame_roundtrips_through_wire(frame in arb_control_frame()) {
        let wire_frame: wire::ControlFrame = (&frame).into();
        let back = ScopedControlFrame::try_from(wire_frame);
        prop_assert_eq!(back, Ok(frame));
    }
}
