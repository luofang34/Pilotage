//! What one direct record has to agree with before it can be scored.

use flight_tune::{ControlChannel, ControlFamily, Digest};

use crate::direct_transport::{
    DIRECT_COMMAND_RECORD_SCHEMA_VERSION, DirectCommandPurpose, DirectCommandRecord,
    DirectCommandTimes, DirectSenderIdentity, DirectSetpoint,
};
use crate::runtime::phase::direct::ledger::{DIRECT_INTENT_SCHEMA_VERSION, DirectIntentRecord};
use crate::runtime::phase::direct::readback::{PublicationContext, validate_publication};

const RUN_INTENT: [u8; 32] = [9; 32];
const TRANSPORT: [u8; 32] = [8; 32];
const ENVELOPE: [u8; 32] = [7; 32];

fn setpoint(pitch_rad: f64) -> DirectSetpoint {
    DirectSetpoint {
        roll_rad: 0.01,
        pitch_rad,
        yaw_rad: 1.2,
        collective_force: 0.72,
    }
}

fn intent() -> DirectIntentRecord {
    DirectIntentRecord {
        schema_version: DIRECT_INTENT_SCHEMA_VERSION,
        sequence: 0,
        purpose: DirectCommandPurpose::Step,
        run_intent_digest: Digest::from_bytes(RUN_INTENT),
        transport_identity_digest: Digest::from_bytes(TRANSPORT),
        envelope_digest: Digest::from_bytes(ENVELOPE),
        requested: setpoint(0.1),
    }
}

fn record() -> DirectCommandRecord {
    DirectCommandRecord {
        schema_version: DIRECT_COMMAND_RECORD_SCHEMA_VERSION,
        purpose: DirectCommandPurpose::Step,
        family: ControlFamily::DirectAttitudeThrust,
        channel: ControlChannel::Pitch,
        normalized: 0.4,
        envelope_digest: Digest::from_bytes(ENVELOPE),
        baseline: setpoint(-0.02),
        requested: setpoint(0.1),
        transmitted: setpoint(0.1),
        effective: setpoint(0.1),
        sender: DirectSenderIdentity {
            endpoint: "127.0.0.1:20000".to_owned(),
            system_id: 1,
            component_id: 1,
            sequence: 1,
            time_boot_ms: 10,
            frame_digest: Digest::from_bytes([0xab; 32]),
        },
        effective_sample_sequence: 3,
        times: DirectCommandTimes {
            requested_at_ns: 100,
            transmitted_at_ns: 200,
            effective_at_ns: 300,
            estimate_at_ns: 290,
            simulator_truth_at_ns: 300,
        },
        run_intent_digest: Digest::from_bytes(RUN_INTENT),
        transport_identity_digest: Digest::from_bytes(TRANSPORT),
    }
}

fn context() -> PublicationContext {
    PublicationContext {
        run_intent_digest: Digest::from_bytes(RUN_INTENT),
        transport_identity_digest: Digest::from_bytes(TRANSPORT),
        tolerance: 1e-9,
    }
}

fn refusal(record: &DirectCommandRecord) -> String {
    match validate_publication(record, &intent(), &context()) {
        Ok(()) => panic!("the record was accepted"),
        Err(error) => error.to_string(),
    }
}

#[test]
fn a_complete_record_that_closes_its_intent_is_published() {
    validate_publication(&record(), &intent(), &context()).expect("a complete record publishes");
}

#[test]
fn a_record_that_does_not_close_its_prepared_intent_is_refused() {
    let mut changed = record();
    changed.requested = setpoint(0.2);
    assert!(
        refusal(&changed).contains("does not close its durable prepared intent"),
        "{}",
        refusal(&changed)
    );

    let mut released = record();
    released.purpose = DirectCommandPurpose::Release;
    assert!(
        refusal(&released).contains("does not close"),
        "{}",
        refusal(&released)
    );
}

#[test]
fn a_record_that_names_another_run_intent_is_refused() {
    let mut changed = record();
    changed.run_intent_digest = Digest::from_bytes([1; 32]);
    // The intent comparison catches it first, and the context comparison
    // catches it whichever way the record was altered.
    assert!(!refusal(&changed).is_empty());

    let context = PublicationContext {
        run_intent_digest: Digest::from_bytes([2; 32]),
        ..context()
    };
    let detail = match validate_publication(&record(), &intent(), &context) {
        Ok(()) => panic!("another run intent was accepted"),
        Err(error) => error.to_string(),
    };
    assert!(
        detail.contains("another run intent or transport"),
        "{detail}"
    );
}

#[test]
fn a_record_whose_times_are_not_causal_is_refused() {
    let mut early = record();
    early.times.effective_at_ns = 150;
    assert!(
        refusal(&early).contains("causally ordered"),
        "{}",
        refusal(&early)
    );

    let mut late = record();
    late.times.requested_at_ns = 250;
    assert!(
        refusal(&late).contains("causally ordered"),
        "{}",
        refusal(&late)
    );
}

#[test]
fn a_record_outside_the_declared_tolerance_is_refused() {
    let mut transmitted = record();
    transmitted.transmitted = setpoint(0.2);
    assert!(
        refusal(&transmitted).contains("transmitted setpoint left"),
        "{}",
        refusal(&transmitted)
    );

    let mut effective = record();
    effective.effective = setpoint(0.2);
    assert!(
        refusal(&effective).contains("effective setpoint left"),
        "{}",
        refusal(&effective)
    );
}

#[test]
fn a_record_carrying_a_value_that_is_not_a_number_is_refused() {
    let mut record = record();
    record.normalized = f64::NAN;
    assert!(
        refusal(&record).contains("not a number"),
        "{}",
        refusal(&record)
    );
}
