use std::time::Duration;

use pilotage_adapter_api::{Disposition, VehicleAdapter};
use pilotage_protocol::{ControlIntent, ReferenceFrame, ScopeId, SequenceNum, VelocityIntent};

use super::{
    harness::{Harness, frame},
    virtual_stick::{ack, commands, ready, report},
};
use crate::{adapter::FLIGHT_SCOPE, flight::VirtualStickMode};

mod confirmation;
mod diagonal;
mod landing;
mod override_control;
mod remote_control;

fn input(h: &mut Harness, sequence: u32, time: u64, values: [f32; 4]) -> Disposition {
    let [vx, vy, vz, yaw_rate] = values;
    let mut value = frame(0.0);
    value.scope = ScopeId::new(FLIGHT_SCOPE);
    value.sequence = SequenceNum::new(sequence);
    value.intent = Some(ControlIntent::Velocity(VelocityIntent {
        frame: ReferenceFrame::BodyFrd,
        vx,
        vy,
        vz,
        yaw_rate,
    }));
    h.adapter
        .apply_at(&value, Duration::from_millis(time))
        .disposition
}

fn flying(h: &mut Harness, sequence: u16) {
    flying_mode(h, sequence, 6);
}

fn flying_mode(h: &mut Harness, sequence: u16, mode: u8) {
    let mut bytes = super::harness::captured_hex_frames(include_str!(
        "../airframe/fixtures/mini3-reported-attitude.hex"
    ))
    .remove(0);
    bytes[6..8].copy_from_slice(&sequence.to_le_bytes());
    bytes[41] = mode;
    bytes[43] = 0x0c;
    super::harness::seal_duml(&mut bytes);
    super::flight::feed(h, &bytes);
}

fn enabled() -> Harness {
    let mut h = ready();
    h.adapter.experimental_virtual_stick = true;
    flying(&mut h, 2);
    h.step(11);
    assert_eq!(input(&mut h, 1, 11, [0.0; 4]), Disposition::Accepted);
    h.step(12);
    ack(&mut h, VirtualStickMode::Enable, 2, 0);
    report(&mut h, 11, true, true, false);
    h.step(13);
    h.step(14);
    h
}

#[test]
fn held_four_axes_pass_through_adapter_writer_at_twenty_hz() {
    let mut h = enabled();
    for tick in 0..1000_u32 {
        let now = 20 + u64::from(tick) * 10;
        if tick % 10 == 0 {
            flying(&mut h, 3 + (tick / 10) as u16);
            super::flight::remote(&mut h, 3 + (tick / 10) as u16, [1024; 4]);
        }
        if tick % 100 == 0 {
            report(&mut h, 12 + (tick / 100) as u16, true, true, false);
        }
        assert_eq!(
            input(&mut h, tick + 2, now, [0.4, -0.3, 0.2, 0.1]),
            Disposition::Accepted
        );
        h.step(now);
    }
    let writes: Vec<_> = commands(&h)
        .into_iter()
        .filter(|f| f.command_id() == 0x8e)
        .collect();
    assert_eq!(writes.len(), 200);
    for command in &writes[1..] {
        assert_eq!(command.receiver().0, 0xc3);
        assert_eq!(command.payload()[0], 0x4b);
        let values: Vec<f32> = command.payload()[1..]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|v| f32::from_le_bytes(*v))
            .collect();
        assert_eq!(values, [0.4, -0.3, -0.2, 0.1_f32.to_degrees()]);
    }
}

#[test]
fn missing_input_sends_one_zero_then_disables_without_restarting() {
    let mut h = enabled();
    assert_eq!(
        input(&mut h, 2, 20, [0.4, 0.0, 0.0, 0.0]),
        Disposition::Accepted
    );
    h.step(70);
    h.step(120);
    h.step(130);
    let sent = commands(&h);
    assert_eq!(sent.last().unwrap().command_id(), 0x80);
    assert_eq!(sent.last().unwrap().payload(), [2]);
    let zero = &sent[sent.len() - 2];
    assert_eq!(zero.command_id(), 0x8e);
    assert!(
        zero.payload()[1..]
            .as_chunks::<4>()
            .0
            .iter()
            .all(|v| f32::from_le_bytes(*v) == 0.0)
    );
    assert!(matches!(
        input(&mut h, 3, 131, [0.0; 4]),
        Disposition::Rejected(_)
    ));
}

#[test]
fn remote_mode_exit_fences_held_input_and_gimbal_invalidation_does_not_stop_motion() {
    let mut h = enabled();
    h.adapter.invalidate_scope(
        pilotage_protocol::VehicleId::new(1),
        &ScopeId::new("vehicle.gimbal"),
        Duration::from_millis(14),
    );
    assert_eq!(
        input(&mut h, 2, 20, [0.2, 0.0, 0.0, 0.0]),
        Disposition::Accepted
    );
    report(&mut h, 12, false, true, false);
    h.step(21);
    let count = commands(&h)
        .iter()
        .filter(|f| f.command_id() == 0x8e)
        .count();
    assert!(matches!(
        input(&mut h, 3, 30, [0.2, 0.0, 0.0, 0.0]),
        Disposition::Rejected(_)
    ));
    h.step(40);
    assert_eq!(
        commands(&h)
            .iter()
            .filter(|f| f.command_id() == 0x8e)
            .count(),
        count
    );
}

#[test]
fn motion_capability_requires_its_own_opt_in_and_initial_neutral() {
    let mut h = ready();
    assert!(matches!(
        input(&mut h, 1, 11, [0.0; 4]),
        Disposition::Rejected(_)
    ));
    h.adapter.experimental_virtual_stick = true;
    assert!(h.adapter.capabilities().vehicles[0].scopes.iter().any(|s| {
        s.intents
            .iter()
            .any(|i| i.family == pilotage_protocol::IntentFamily::Velocity)
    }));
    assert!(matches!(
        input(&mut h, 1, 11, [0.1, 0.0, 0.0, 0.0]),
        Disposition::Rejected(_)
    ));
    assert!(commands(&h).is_empty());
}

#[test]
fn partial_velocity_cannot_be_followed_by_a_neutral_or_mode_packet() {
    let mut h = enabled();
    assert_eq!(
        input(&mut h, 2, 20, [0.2, 0.0, 0.0, 0.0]),
        Disposition::Accepted
    );
    h.set_write_limit(3).unwrap();
    h.step(70);
    let bytes = h.written().unwrap().len();
    h.adapter.invalidate_scope(
        pilotage_protocol::VehicleId::new(1),
        &ScopeId::new(FLIGHT_SCOPE),
        Duration::from_millis(71),
    );
    h.step(71);
    h.set_write_limit(4096).unwrap();
    h.step(72);
    assert!(h.adapter.status_cached().fault.is_some());
    assert_eq!(h.written().unwrap().len(), bytes);
}

#[test]
fn new_lease_requires_confirmed_release_and_new_neutral_input() {
    let mut h = enabled();
    h.adapter.invalidate_scope(
        pilotage_protocol::VehicleId::new(1),
        &ScopeId::new(FLIGHT_SCOPE),
        Duration::from_millis(14),
    );
    h.step(14);
    h.step(15);
    let mut request = frame(0.0);
    request.scope = ScopeId::new(FLIGHT_SCOPE);
    request.generation = pilotage_protocol::Generation::new(2);
    request.intent = Some(ControlIntent::Velocity(VelocityIntent {
        frame: ReferenceFrame::BodyFrd,
        vx: 0.0,
        vy: 0.0,
        vz: 0.0,
        yaw_rate: 0.0,
    }));
    assert!(matches!(
        h.adapter
            .apply_at(&request, Duration::from_millis(16))
            .disposition,
        Disposition::Rejected(_)
    ));
    ack(&mut h, VirtualStickMode::Disable, 3, 0);
    report(&mut h, 12, false, true, false);
    h.step(17);
    assert_eq!(
        h.adapter
            .apply_at(&request, Duration::from_millis(18))
            .disposition,
        Disposition::Accepted
    );
    h.step(19);
    assert_eq!(commands(&h).last().unwrap().payload(), [1]);
}

#[test]
fn native_flight_mode_preempts_motion_without_zero_velocity_override() {
    let mut h = enabled();
    let mut bytes = super::harness::captured_hex_frames(include_str!(
        "../airframe/fixtures/mini3-reported-attitude.hex"
    ))
    .remove(0);
    bytes[6..8].copy_from_slice(&3_u16.to_le_bytes());
    bytes[41] = 12;
    bytes[43] = 0x0c;
    super::harness::seal_duml(&mut bytes);
    super::flight::feed(&mut h, &bytes);
    h.step(20);
    let sent = commands(&h);
    assert_eq!(sent.iter().filter(|f| f.command_id() == 0x8e).count(), 1);
    assert_eq!(sent.last().unwrap().payload(), [2]);
}

#[test]
fn ground_neutral_does_not_enable_mode_or_compete_with_native_takeoff() {
    let mut h = ready();
    h.adapter.experimental_virtual_stick = true;
    assert_eq!(input(&mut h, 1, 11, [0.0; 4]), Disposition::Accepted);
    h.step(12);
    assert!(commands(&h).is_empty());
    assert!(matches!(
        input(&mut h, 2, 13, [0.0, 0.0, -0.2, 0.0]),
        Disposition::Rejected(_)
    ));
    h.step(14);
    assert!(commands(&h).is_empty());
}

#[test]
fn initial_link_loss_clear_allows_the_first_neutral_lease() {
    let mut h = ready();
    h.adapter.experimental_virtual_stick = true;
    h.adapter
        .motion_link_loss(
            Some(pilotage_adapter_api::LinkLossPolicy::Neutralize),
            Duration::from_millis(11),
        )
        .unwrap();
    h.adapter
        .motion_link_loss(None, Duration::from_millis(12))
        .unwrap();
    flying(&mut h, 2);
    h.step(13);
    assert_eq!(input(&mut h, 1, 14, [0.0; 4]), Disposition::Accepted);
    h.step(15);
    ack(&mut h, VirtualStickMode::Enable, 2, 0);
    report(&mut h, 11, true, true, false);
    h.step(16);
    h.step(17);
    assert!(commands(&h).iter().any(|f| f.command_id() == 0x8e));
}
