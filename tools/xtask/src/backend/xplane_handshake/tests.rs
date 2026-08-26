//! What the handshake says, and what it refuses to say.

#![allow(clippy::expect_used, clippy::panic)]

use std::os::unix::fs::PermissionsExt as _;

use pilotage_trial::Digest;
use pilotage_xplane_trial::VerifiedXPlaneIdentity;

use super::{SimulatorModel, VERIFIER_ID, model_contract_digest, write_handshake};
use std::io::Write as _;

fn verified() -> VerifiedXPlaneIdentity {
    let mut identity = VerifiedXPlaneIdentity {
        protocol_version: 2,
        xplane_version: 12_431,
        sdk_version: 430,
        host_application_id: 1,
        trial_plugin_digest: Digest::from_bytes([1; 32]),
        bridge_plugin_digest: Digest::from_bytes([2; 32]),
        bridge_config_digest: Digest::from_bytes([3; 32]),
        aircraft_digest: Digest::from_bytes([4; 32]),
        simulator_model_digest: Digest::from_bytes([5; 32]),
        trial_source_build_id: "1c52f8e7".to_owned(),
        binding_digest: Digest::from_bytes([0; 32]),
    };
    identity.refresh_binding_digest();
    identity
}

#[test]
fn the_document_names_the_verifier_aviate_checks_for() {
    // Aviate refuses a handshake issued by anyone else, so this string is a
    // contract with another repository rather than a label.
    let dir = tempdir();
    let path = write_handshake(
        &verified(),
        &model(verified().aircraft_digest.to_string()),
        &dir,
    )
    .expect("write");
    let text = std::fs::read_to_string(&path).expect("read back");
    assert!(
        text.contains(VERIFIER_ID),
        "the document does not name its verifier: {text}"
    );
    assert!(text.contains("xplane12-laminar-alia250"));
    assert!(text.contains("lane_order"));
    assert!(text.contains("sample_rate_hz = 100"));
}

#[test]
fn the_document_is_readable_only_by_this_account() {
    // The flight controller refuses a handshake any other account could have
    // read or substituted, and it is right to: the document is what says the
    // run flew the simulator it claims.
    let dir = tempdir();
    let path = write_handshake(
        &verified(),
        &model(verified().aircraft_digest.to_string()),
        &dir,
    )
    .expect("write");
    let mode = std::fs::metadata(&path).expect("stat").permissions().mode();
    assert_eq!(mode & 0o077, 0, "group or other can reach it: {mode:o}");
}

#[test]
fn a_second_launch_does_not_inherit_the_first_ones_document() {
    // A stale file would be claimed instead of the fresh one, binding the run
    // to a simulator that has since stopped.
    let dir = tempdir();
    let first = write_handshake(
        &verified(),
        &model(verified().aircraft_digest.to_string()),
        &dir,
    )
    .expect("first");
    std::fs::write(&first, "stale").expect("overwrite with a stale document");
    let second = write_handshake(
        &verified(),
        &model(verified().aircraft_digest.to_string()),
        &dir,
    )
    .expect("second");
    assert_eq!(first, second);
    let text = std::fs::read_to_string(&second).expect("read back");
    assert!(!text.contains("stale"), "the stale document survived");
}

#[test]
fn the_model_contract_follows_every_field_that_decides_the_vehicle() {
    // Binding every run to one declared model however the preset changed would
    // make the field decoration. The two fields that name the aircraft are
    // already bound elsewhere, so what earns this digest its place is the
    // rest: a `lane_order` or a rate edited between runs is a different
    // vehicle, and nothing else in the document would say so.
    let config = Digest::from_bytes([8; 32]);
    let base = model_contract_digest(&model("aa".repeat(32)), config);

    let changed: [(&str, SimulatorModel); 7] = [
        (
            "another simulator",
            SimulatorModel {
                simulator_id: "xplane-11".to_owned(),
                ..model("aa".repeat(32))
            },
        ),
        (
            "another aircraft id",
            SimulatorModel {
                aircraft_id: "xplane12-laminar-cessna172".to_owned(),
                ..model("aa".repeat(32))
            },
        ),
        (
            "another aircraft file",
            SimulatorModel {
                aircraft_file_digest: "bb".repeat(32),
                ..model("aa".repeat(32))
            },
        ),
        (
            "another bridge protocol",
            SimulatorModel {
                bridge_protocol: "mavlink-hil-tcp-v2".to_owned(),
                ..model("aa".repeat(32))
            },
        ),
        (
            "another motor count",
            SimulatorModel {
                motor_count: 6,
                lane_order: [0, 2, 1, 3],
                ..model("aa".repeat(32))
            },
        ),
        (
            "another sample rate",
            SimulatorModel {
                sample_rate_hz: 92,
                ..model("aa".repeat(32))
            },
        ),
        (
            "another lane order",
            SimulatorModel {
                lane_order: [0, 1, 2, 3],
                ..model("aa".repeat(32))
            },
        ),
    ];
    for (name, edited) in changed {
        assert_ne!(
            base,
            model_contract_digest(&edited, config),
            "{name} produced the same contract"
        );
    }

    assert_ne!(
        base,
        model_contract_digest(&model("aa".repeat(32)), Digest::from_bytes([9; 32])),
        "another bridge configuration produced the same contract"
    );
    assert_eq!(
        base,
        model_contract_digest(&model("aa".repeat(32)), config),
        "the same model produced a different contract"
    );
    // Zero is refused by the verifier, so a derivation that could produce it
    // would fail the run rather than bind it.
    assert!(!base.is_zero());
}

#[test]
fn fields_that_run_together_cannot_be_traded_between_each_other() {
    // Concatenating strings is injective only while at most one can vary in
    // length. Four do here, so moving a character from one to the next must
    // still change the contract.
    let config = Digest::from_bytes([8; 32]);
    let mut left = model("aa".repeat(32));
    left.simulator_id = "xplane".to_owned();
    left.aircraft_id = "-12-alia".to_owned();
    let mut right = model("aa".repeat(32));
    right.simulator_id = "xplane-12".to_owned();
    right.aircraft_id = "-alia".to_owned();

    assert_ne!(
        model_contract_digest(&left, config),
        model_contract_digest(&right, config),
        "a character moved across a field boundary went unnoticed"
    );
}

/// The Alia's own model, as its preset declares it.
fn model(aircraft_file_digest: String) -> SimulatorModel {
    SimulatorModel {
        simulator_id: "xplane-12".to_owned(),
        aircraft_id: "xplane12-laminar-alia250".to_owned(),
        aircraft_file_digest,
        bridge_protocol: "mavlink-hil-tcp-v1".to_owned(),
        motor_count: 4,
        sample_rate_hz: 100,
        lane_order: [0, 2, 1, 3],
    }
}

#[test]
fn an_aircraft_the_model_was_not_written_for_is_refused() {
    // The reader verified which aircraft is loaded; the model states which one
    // it describes. If they differ the run is not the run the model describes,
    // and writing the document anyway would say it was.
    let dir = tempdir();
    let wrong = model(Digest::from_bytes([9; 32]).to_string());
    let refused = write_handshake(&verified(), &wrong, &dir);
    assert!(refused.is_err(), "a mismatched aircraft was accepted");
}

fn tempdir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pilotage-handshake-test-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).expect("scratch directory");
    dir
}

#[test]
fn a_model_that_cannot_describe_a_vehicle_is_refused() {
    // These fields configure motor mixing on the far side of the handshake, so
    // a preset stating nothing useful has to fail the launch rather than be
    // copied into a document the flight controller trusts.
    let base = model("aa".repeat(32));
    let cases: Vec<(&str, SimulatorModel)> = vec![
        (
            "no simulator",
            SimulatorModel {
                simulator_id: String::new(),
                ..model("aa".repeat(32))
            },
        ),
        (
            "no aircraft",
            SimulatorModel {
                aircraft_id: String::new(),
                ..model("aa".repeat(32))
            },
        ),
        (
            "no protocol",
            SimulatorModel {
                bridge_protocol: String::new(),
                ..model("aa".repeat(32))
            },
        ),
        (
            "no motors",
            SimulatorModel {
                motor_count: 0,
                ..model("aa".repeat(32))
            },
        ),
        (
            "no rate",
            SimulatorModel {
                sample_rate_hz: 0,
                ..model("aa".repeat(32))
            },
        ),
        (
            "repeated lane",
            SimulatorModel {
                lane_order: [0, 0, 1, 3],
                ..model("aa".repeat(32))
            },
        ),
        (
            "lane out of range",
            SimulatorModel {
                lane_order: [0, 1, 2, 7],
                ..model("aa".repeat(32))
            },
        ),
        (
            "motors disagree with lanes",
            SimulatorModel {
                motor_count: 6,
                ..model("aa".repeat(32))
            },
        ),
    ];
    for (name, broken) in cases {
        assert!(broken.validate().is_err(), "{name} was accepted");
    }
    assert!(base.validate().is_ok(), "a sound model was refused");
}

#[test]
fn a_document_left_by_anyone_else_is_never_written_through() {
    // What this pins is that the previous file's CONTENT cannot survive into
    // the one the flight controller claims.
    //
    // It does not pin the exclusive create. `write_handshake` removes before
    // it creates, so `create_new` only decides what happens to a racer who
    // wins the gap between the two, and that is not reachable from a unit
    // test. Its failure mode is an aborted launch rather than a forged
    // document — `O_CREAT|O_EXCL` refuses rather than following a planted
    // symlink — and the directory is owner-writable only, so only a
    // same-user process can reach it at all.
    let dir = tempdir();
    let path = dir.join("runtime-handshake.toml");
    let mut planted = std::fs::File::create(&path).expect("plant a file");
    planted
        .write_all(b"someone else's document")
        .expect("write");
    drop(planted);

    let written = write_handshake(
        &verified(),
        &model(verified().aircraft_digest.to_string()),
        &dir,
    )
    .expect("write");
    let text = std::fs::read_to_string(&written).expect("read back");
    assert!(
        !text.contains("someone else"),
        "the planted content survived"
    );
    assert!(
        text.contains(VERIFIER_ID),
        "the document is not the one written"
    );
}
