use super::*;
use pilotage_client_session::{MotionDemand, velocity_intent};
use pilotage_protocol::wire;

#[test]
fn app_diagonals_keep_direction_and_both_left_stick_axes_through_the_device_writer() {
    let mut h = enabled();
    let capabilities = h.adapter.capabilities();
    let offer = capabilities.vehicles[0]
        .scopes
        .iter()
        .find(|scope| scope.scope.as_str() == FLIGHT_SCOPE)
        .unwrap();
    let velocity = offer
        .intents
        .iter()
        .find(|intent| intent.family == pilotage_protocol::IntentFamily::Velocity)
        .unwrap();
    assert!(velocity.frames.contains(&ReferenceFrame::BodyFrd));
    let advertised = wire::IntentCapability {
        family: wire::IntentFamily::Velocity as i32,
        frames: vec![wire::ReferenceFrame::BodyFrd as i32],
        max_linear: velocity.max_linear,
        max_vertical: velocity.max_vertical,
        max_angular: velocity.max_angular,
        ..Default::default()
    };
    for (index, (pitch, roll)) in [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)]
        .into_iter()
        .enumerate()
    {
        let demand = MotionDemand {
            pitch,
            roll,
            throttle: 1.0,
            yaw: -1.0,
        };
        let intent = velocity_intent(demand, Some(&advertised)).unwrap();
        let Some(wire::control_intent::Family::Velocity(value)) = intent.family else {
            panic!("velocity intent");
        };
        let now = 20 + index as u64 * 50;
        assert_eq!(
            input(
                &mut h,
                index as u32 + 2,
                now,
                [value.vx, value.vy, value.vz, value.yaw_rate]
            ),
            Disposition::Accepted
        );
        h.step(now + 50);
        let sent = commands(&h);
        let packet = sent
            .iter()
            .rev()
            .find(|frame| frame.command_id() == 0x8e)
            .unwrap();
        let values: Vec<f32> = packet.payload()[1..]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|bytes| f32::from_le_bytes(*bytes))
            .collect();
        let horizontal = crate::flight::HORIZONTAL_LIMIT_MPS;
        assert!((values[0] - pitch * horizontal * core::f32::consts::FRAC_1_SQRT_2).abs() < 1e-4);
        assert!((values[1] - roll * horizontal * core::f32::consts::FRAC_1_SQRT_2).abs() < 1e-4);
        assert!(values[0].hypot(values[1]) <= horizontal);
        assert_eq!(values[2], crate::flight::VERTICAL_LIMIT_MPS);
        assert!((values[3] + 100.0).abs() < 1e-3);
    }
}

#[test]
fn radial_limiting_does_not_admit_out_of_contract_or_non_finite_axes() {
    for values in [
        [15.01, 0.0, 0.0, 0.0],
        [0.0, -15.01, 0.0, 0.0],
        [f32::NAN, 0.0, 0.0, 0.0],
        [0.0, f32::INFINITY, 0.0, 0.0],
        [15.0, 15.0, 4.01, 0.0],
        [15.0, 15.0, 0.0, 1.75],
    ] {
        let mut h = enabled();
        assert!(matches!(
            input(&mut h, 2, 20, values),
            Disposition::Rejected(_)
        ));
    }
}
