//! The shaped operator modes: installing one, asking for one by name, and
//! the boundary each arrives at.

use std::time::Duration;

use pilotage_adapter_api::{Disposition, VehicleAdapter};
use pilotage_control_feel::{FeelDigest, FeelMode, FlightFeelProfile, ValidatedFlightFeelProfile};
use pilotage_protocol::LogicalAxisId;

use super::super::super::PITCH_AXIS;
use super::super::fixtures::flight_frame;
use super::{
    active_digest, adapter_with_fc, airborne_adapter_with_fc, neutral_frame, receive_frame,
};

#[test]
fn every_shaped_operator_mode_installs_after_its_neutral_dwell() {
    // The shaped modes are the answer to a command law that steps, and a
    // profile the adapter will not take is no answer at all. Each one is
    // staged on a flying vehicle and has to reach the active artifact.
    //
    // The law this replaces had no dwell, so activation was instant: one frame
    // at centre and the law changed. A shaped mode holds a quiet band for a
    // stated time before it calls an input neutral, which is what keeps a
    // resting hand from commanding — and the same rule governs the boundary
    // the new law arrives at, so a stick crossing centre in passing does not
    // change the law underneath it.
    //
    // The dwell accumulates across frames rather than from one long gap: a
    // control loop reports continuously, and a single sample after a silence
    // says nothing about where the stick was during it.
    for mode in [FeelMode::Precision, FeelMode::Balanced, FeelMode::Agile] {
        let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
        fc.set_read_timeout(Some(Duration::from_secs(1)))
            .expect("read timeout");
        let mut adapter = adapter_with_fc(&fc);
        let before = active_digest(&adapter);

        // A shaped profile keeps the artifact bindings and the demand envelope
        // of the law it replaces, so it is installable on the vehicle the
        // adapter was started with rather than describing a different one.
        let profile = FlightFeelProfile::shaped(mode);
        let dwell = Duration::from_millis(u64::from(profile.horizontal.neutral.dwell_ms));
        assert!(dwell > Duration::ZERO, "{mode:?} must state a dwell");
        let shaped = ValidatedFlightFeelProfile::new(profile)
            .unwrap_or_else(|error| panic!("{mode:?} must be a valid profile: {error}"));
        let expected = *FeelDigest::calculate(&shaped).expect("digest").as_bytes();
        adapter
            .stage_control_feel(shaped)
            .unwrap_or_else(|error| panic!("{mode:?} must be stageable: {error}"));

        // A deflected stick does not activate it.
        let deflected = flight_frame(vec![(LogicalAxisId::new(PITCH_AXIS), 0.5)], vec![]);
        assert_eq!(
            adapter.apply_control(&deflected).disposition,
            Disposition::Accepted
        );
        assert_eq!(
            active_digest(&adapter),
            before,
            "{mode:?} activated deflected"
        );

        // Held at centre, it arrives — and not before the dwell it states. The
        // clock is advanced rather than waited on, so the boundary is driven
        // deterministically at a rate a control loop actually reports at.
        let step = Duration::from_millis(20);
        let mut held = Duration::ZERO;
        let mut activated_after = None;
        for _ in 0..40 {
            assert_eq!(
                adapter.apply_control(&neutral_frame()).disposition,
                Disposition::Accepted
            );
            if active_digest(&adapter) == expected {
                activated_after = Some(held);
                break;
            }
            adapter.uplink_mut().expect("uplink").advance_clock(step);
            held += step;
        }

        let elapsed = activated_after
            .unwrap_or_else(|| panic!("{mode:?} never activated while held at centre"));
        assert!(
            elapsed >= dwell,
            "{mode:?} activated after {elapsed:?}, before its {dwell:?} dwell"
        );
    }
}

#[test]
fn an_operator_asks_for_a_mode_by_name_and_gets_the_qualified_one() {
    // The choice an operator makes is between three named laws, not between
    // arbitrary profiles. A caller that could supply any profile could supply
    // one nobody qualified; a mode a vehicle advertises is one somebody stood
    // behind.
    for mode in [FeelMode::Precision, FeelMode::Balanced, FeelMode::Agile] {
        let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
        fc.set_read_timeout(Some(Duration::from_secs(1)))
            .expect("read timeout");
        let mut adapter = airborne_adapter_with_fc(&fc);

        let staged = adapter
            .request_feel_mode(mode)
            .unwrap_or_else(|error| panic!("{mode:?} must be requestable: {error}"));
        let named = ValidatedFlightFeelProfile::new(FlightFeelProfile::shaped(mode))
            .expect("the named mode is a valid profile");
        let expected = FeelDigest::calculate(&named).expect("digest the named mode");
        assert_eq!(
            staged.as_bytes(),
            expected.as_bytes(),
            "{mode:?} staged a different law than the one it names"
        );

        // It arrives at the same neutral boundary a rollback uses, so asking
        // for a mode does not change the law under a deflected stick.
        let deflected = flight_frame(vec![(LogicalAxisId::new(PITCH_AXIS), 0.5)], vec![]);
        assert_eq!(
            adapter.apply_control(&deflected).disposition,
            Disposition::Accepted
        );
        receive_frame(&fc, "deflected response frame");
        assert_ne!(
            active_digest(&adapter),
            *expected.as_bytes(),
            "{mode:?} activated under a deflected stick"
        );
    }
}

#[test]
fn a_physical_vehicle_refuses_a_mode_request() {
    // These laws are shaped for a simulator and qualified there. A physical
    // gateway takes its command law from the aircraft it is bound to, and a
    // request to change it from the ground is refused rather than negotiated.
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    let mut adapter = airborne_adapter_with_fc(&fc).with_profile(crate::AviateProfile::Physical);
    assert!(adapter.request_feel_mode(FeelMode::Agile).is_err());
}

#[test]
fn a_feel_request_on_a_control_frame_stages_the_named_law() {
    // The operator's path end to end at the vehicle: a typed action arrives on
    // a control frame, is accepted, and the named law is staged — waiting for
    // a neutral boundary rather than changing under the stick that carried it.
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(1)))
        .expect("read timeout");
    let mut adapter = airborne_adapter_with_fc(&fc);
    let before = active_digest(&adapter);

    // The request rides a frame whose stick is deflected, which is the case
    // that matters: an operator changes mode while flying.
    let mut frame = flight_frame(vec![(LogicalAxisId::new(PITCH_AXIS), 0.5)], vec![]);
    frame.actions = vec![pilotage_protocol::ControlAction::FeelModeRequest {
        target: pilotage_protocol::FeelTarget::Agile,
    }];
    let outcome = adapter.apply_control(&frame);
    assert_eq!(outcome.disposition, Disposition::Accepted);
    receive_frame(&fc, "feel request response frame");

    let answered = outcome
        .action_results
        .iter()
        .find(|result| {
            matches!(
                result.action,
                pilotage_protocol::ControlAction::FeelModeRequest { .. }
            )
        })
        .expect("the request was answered");
    assert!(answered.accepted, "refused: {}", answered.detail);
    assert_eq!(
        active_digest(&adapter),
        before,
        "the law changed under the stick"
    );

    // Held at centre, the requested law arrives.
    let named = ValidatedFlightFeelProfile::new(FlightFeelProfile::shaped(FeelMode::Agile))
        .expect("the named mode is a valid profile");
    let expected = *FeelDigest::calculate(&named)
        .expect("digest the named mode")
        .as_bytes();
    let step = Duration::from_millis(20);
    let mut arrived = false;
    for _ in 0..40 {
        adapter
            .uplink_mut()
            .expect("uplink")
            .seed_hold_for_test([1.0, 2.0, 3.0]);
        assert_eq!(
            adapter.apply_control(&neutral_frame()).disposition,
            Disposition::Accepted
        );
        receive_frame(&fc, "neutral response frame");
        if active_digest(&adapter) == expected {
            arrived = true;
            break;
        }
        adapter.uplink_mut().expect("uplink").advance_clock(step);
    }
    assert!(arrived, "the requested law never arrived");
}

#[test]
fn the_vehicle_advertises_the_modes_it_can_serve() {
    // A client offers only what the vehicle advertises, so a control never
    // asks for a law the vehicle has no qualified profile for.
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    let adapter = airborne_adapter_with_fc(&fc);
    let advertised = adapter
        .advertised_capabilities()
        .vehicles
        .into_iter()
        .flat_map(|vehicle| vehicle.scopes)
        .flat_map(|scope| scope.actions)
        .find(|action| action.action == pilotage_protocol::ActionKind::FeelModeRequest)
        .expect("the vehicle advertises a feel-mode request");
    assert_eq!(
        advertised.feel_targets,
        vec![
            pilotage_protocol::FeelTarget::Precision,
            pilotage_protocol::FeelTarget::Balanced,
            pilotage_protocol::FeelTarget::Agile,
        ]
    );
    assert!(
        advertised.mode_targets.is_empty(),
        "a feel request carries no flight-mode target"
    );
}

/// The shipped profile artifacts, so a launcher can name one on disk.
const SHAPED_PROFILES: [(FeelMode, &str, &str); 3] = [
    (
        FeelMode::Precision,
        "precision",
        include_str!("../../../../profiles/alia250-shaped-precision-v1.json"),
    ),
    (
        FeelMode::Balanced,
        "balanced",
        include_str!("../../../../profiles/alia250-shaped-balanced-v1.json"),
    ),
    (
        FeelMode::Agile,
        "agile",
        include_str!("../../../../profiles/alia250-shaped-agile-v1.json"),
    ),
];

#[test]
fn each_shipped_profile_is_the_law_the_code_shapes() {
    // The host installs a law from a file named in the environment. A file
    // that had drifted from the code would fly a law nobody reviewed, and
    // nothing at runtime would notice: it parses, it validates, and it is
    // simply not the law the mode is documented to be.
    for (mode, name, json) in SHAPED_PROFILES {
        let shipped = ValidatedFlightFeelProfile::from_json_str(json)
            .unwrap_or_else(|error| panic!("{name} must parse and validate: {error}"));
        let shaped = ValidatedFlightFeelProfile::new(FlightFeelProfile::shaped(mode))
            .expect("the code's shaped mode is valid");
        assert_eq!(
            FeelDigest::calculate(&shipped)
                .expect("digest the shipped profile")
                .as_bytes(),
            FeelDigest::calculate(&shaped)
                .expect("digest the shaped mode")
                .as_bytes(),
            "{name} on disk is not the law the code shapes"
        );
        assert_eq!(shipped.profile().mode, mode);
    }
}

#[test]
fn a_shipped_profile_installs_on_the_vehicle() {
    // A file the adapter refuses is a file a launcher cannot use, however
    // well it parses.
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(1)))
        .expect("read timeout");
    let mut adapter = airborne_adapter_with_fc(&fc);
    for (_, name, json) in SHAPED_PROFILES {
        let shipped =
            ValidatedFlightFeelProfile::from_json_str(json).expect("the shipped profile parses");
        adapter
            .stage_control_feel(shipped)
            .unwrap_or_else(|error| panic!("{name} must be installable: {error}"));
    }
}
