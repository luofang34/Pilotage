//! The control lifecycle from the sticks: an admitted observer's
//! ticks claim nothing; the arm press without the lease is the
//! cooperative ask; a regrant resumes live output only on the host's
//! recovery ack; held keys ride the pad's own runtime and lanes.

use super::*;

/// One pad tick in Standard Gamepad terms: sixteen buttons, the named
/// indices pressed.
fn pad_tick(link: &mut Link, axes: [f32; 4], pressed_indices: &[usize]) -> Vec<ClientAction> {
    let mut values = [0.0_f32; 16];
    let mut pressed = [false; 16];
    for &index in pressed_indices {
        values[index] = 1.0;
        pressed[index] = true;
    }
    link.pad_actions(&axes, &values, &pressed)
}

fn datagram_count(actions: &[ClientAction]) -> usize {
    actions
        .iter()
        .filter(|action| matches!(action, ClientAction::SendDatagram(_)))
        .count()
}

/// The reliable-stream payloads inside the actions, by wire variant name.
fn bootstrap_payloads(actions: &[ClientAction]) -> Vec<&'static str> {
    actions
        .iter()
        .filter_map(|action| {
            let ClientAction::SendBootstrap(bytes) = action else {
                return None;
            };
            let (envelope, _) =
                pilotage_protocol::decode_envelope_length_delimited(bytes).expect("decodes");
            Some(match envelope.payload {
                Some(wire::envelope::Payload::LeaseRequest(_)) => "lease-request",
                Some(wire::envelope::Payload::LeaseRelease(_)) => "lease-release",
                Some(wire::envelope::Payload::ControlActionCommand(_)) => "action",
                _ => "other",
            })
        })
        .collect()
}

/// An admitted observer's ticks must lease nothing and send nothing:
/// watching a vehicle is not a claim on any of its scopes, and the
/// runtime's own reacquisition plan must never leave this shell.
#[test]
fn an_admitted_observer_tick_asks_for_nothing() {
    let mut link = admitted_link();
    link.control.select_device("gamepad");
    for _ in 0..3 {
        let actions = pad_tick(&mut link, [0.3, -0.8, 0.0, 0.0], &[]);
        assert_eq!(datagram_count(&actions), 0, "no frame without a lease");
        assert!(
            bootstrap_payloads(&actions).is_empty(),
            "no lease traffic from an observer's deflected stick"
        );
    }
}

/// The arm press without the lease IS the ask — once per answer.
#[test]
fn an_arm_press_without_the_lease_becomes_one_cooperative_ask() {
    let mut link = admitted_link();
    link.control.select_device("gamepad");
    // The swap lands on the first tick and re-seeds edge baselines on
    // the second: a press can fire as an edge only after both.
    let _ = pad_tick(&mut link, [0.0; 4], &[]);
    let _ = pad_tick(&mut link, [0.0; 4], &[]);
    let ask = pad_tick(&mut link, [0.0; 4], &[9]);
    assert_eq!(
        bootstrap_payloads(&ask),
        vec!["lease-request"],
        "the press becomes the motion-lease ask"
    );
    let released = pad_tick(&mut link, [0.0; 4], &[]);
    assert!(bootstrap_payloads(&released).is_empty());
    let second = pad_tick(&mut link, [0.0; 4], &[9]);
    assert!(
        bootstrap_payloads(&second).is_empty(),
        "a second press waits for the host's answer"
    );
    // The answer arrives (a denial): the next press may ask again.
    link.emit(ModuleEvent::Lease(lease_response(
        "vehicle.motion",
        false,
        5,
    )));
    let _ = pad_tick(&mut link, [0.0; 4], &[]);
    let third = pad_tick(&mut link, [0.0; 4], &[9]);
    assert_eq!(
        bootstrap_payloads(&third),
        vec!["lease-request"],
        "an answered ask re-arms the press"
    );
}

/// Denial fences the mirror; the regrant must walk neutral activation
/// and resume live only on the host's cleared ack — the browser's own
/// recovery contract, which this shell once dropped on the floor.
#[test]
fn a_regrant_after_denial_recovers_through_the_cleared_ack() {
    let mut link = admitted_link();
    link.control.select_device("gamepad");
    let _ = pad_tick(&mut link, [0.0; 4], &[]);
    // Denied: the mirror fences at the host's generation.
    link.emit(ModuleEvent::Lease(lease_response(
        "vehicle.motion",
        false,
        5,
    )));
    // The holder let go; our ask lands on a fresh generation.
    grant(&mut link, "vehicle.motion", 6);
    // Deflected before recovery: gated, nothing live leaves.
    let deflected = pad_tick(&mut link, [0.0, -0.8, 0.0, 0.0], &[]);
    assert_eq!(
        datagram_count(&deflected),
        0,
        "no live frame before the cleared ack"
    );
    // Neutral controls stream the activation the host needs.
    let neutral = pad_tick(&mut link, [0.0; 4], &[]);
    assert!(
        datagram_count(&neutral) > 0,
        "neutral activation is retransmitted while recovering"
    );
    // The ack lands on the granted generation: recovery is complete.
    link.emit(ModuleEvent::LinkLossCleared(wire::LinkLossCleared {
        vehicle: Some(wire::VehicleId { value: 1 }),
        scope: Some(wire::ScopeId {
            value: "vehicle.motion".into(),
        }),
        generation: Some(wire::Generation { value: 6 }),
    }));
    let _ = pad_tick(&mut link, [0.0; 4], &[]);
    let live = pad_tick(&mut link, [0.0, -0.8, 0.0, 0.0], &[]);
    assert!(
        datagram_count(&live) > 0,
        "live output resumes after the cleared ack"
    );
    assert_eq!(datagram_scope(&live), "vehicle.motion");
}

/// The first velocity intent inside the actions' datagrams.
fn motion_velocity(actions: &[ClientAction]) -> wire::VelocityIntent {
    let intent = actions
        .iter()
        .find_map(|action| {
            let ClientAction::SendDatagram(bytes) = action else {
                return None;
            };
            let envelope = wire::Envelope::decode(bytes.as_slice()).expect("frame decodes");
            let Some(wire::envelope::Payload::ControlFrame(frame)) = envelope.payload else {
                return None;
            };
            frame.intent.and_then(|intent| intent.family)
        })
        .expect("a typed intent leaves");
    let wire::control_intent::Family::Velocity(velocity) = intent else {
        panic!("the motion frame commands velocity");
    };
    velocity
}

/// Held keys ride the same runtime, curves, and lanes as the pad: the
/// keyboard is a device layer, not a second control path.
#[test]
fn held_keys_fly_the_motion_lane_through_the_shared_runtime() {
    let mut link = admitted_link_with_two_lanes();
    let _ = link.key_actions();
    let _ = link.key_actions();
    link.control.key_event("w", true);
    let live = link.key_actions();
    assert_eq!(datagram_scope(&live), "vehicle.motion");
    let velocity = motion_velocity(&live);
    assert!(
        velocity.vz.abs() > 0.0,
        "a held climb key must command vertical velocity, got {velocity:?}"
    );
    link.control.clear_keys();
    let neutral = link.key_actions();
    let velocity = motion_velocity(&neutral);
    assert_eq!(
        velocity.vz, 0.0,
        "cleared keys must fall back to a neutral stream"
    );
}

/// Enter is the keyboard's arm control; without the lease the press is
/// the same cooperative ask the pad's arm button makes.
#[test]
fn an_enter_press_without_the_lease_asks_for_control() {
    let mut link = admitted_link();
    let _ = link.key_actions();
    let _ = link.key_actions();
    link.control.key_event("Enter", true);
    let ask = link.key_actions();
    assert_eq!(
        bootstrap_payloads(&ask),
        vec!["lease-request"],
        "the keyboard's arm press becomes the motion-lease ask"
    );
}

/// With the lever on SAFE and the vehicle confirmed disarmed, the
/// disarm press means "stand down": both held lanes go back.
#[test]
fn a_settled_safe_disarm_press_stands_down_control() {
    let mut link = admitted_link_with_two_lanes();
    link.telegraph.on_fc_arm_state(1);
    link.control.select_device("gamepad");
    let _ = pad_tick(&mut link, [0.0; 4], &[]);
    let _ = pad_tick(&mut link, [0.0; 4], &[]);
    let standdown = pad_tick(&mut link, [0.0; 4], &[8]);
    assert_eq!(
        bootstrap_payloads(&standdown),
        vec!["lease-release", "lease-release"],
        "the settled-safe press releases the motion and gimbal lanes"
    );
}
