//! The exact step: one target, in one sample, with every other axis frozen.

use flight_tune::{ControlChannel, ControlFamily, StimulusMapping};

use super::super::{
    DirectCommandPurpose, DirectCommandSender, DirectEnactment, DirectSetpoint,
    DirectTransportError,
};
use super::sender::RecordingSender;
use super::{
    HOVER_TRIM, attitude_envelope, authorize, baseline_request, collective_envelope, digest,
    frozen, step_request,
};

fn enacted(outcome: DirectEnactment) -> super::super::DirectCommandRecord {
    match outcome {
        DirectEnactment::Enacted(record) => *record,
        other => panic!("expected an enacted command, got {other:?}"),
    }
}

#[test]
fn a_neutral_stimulus_targets_the_frozen_baseline_exactly() {
    let (transport, _sender) = frozen();
    let baseline = transport.baseline().expect("frozen baseline").setpoint();

    let prepared = transport
        .prepare_step(&step_request(ControlChannel::Roll, 0.0))
        .expect("prepared step");

    assert_eq!(
        prepared.requested(),
        baseline,
        "a zero normalized value on a zero-neutral envelope IS the baseline"
    );
}

#[test]
fn each_channel_moves_only_its_own_axis_from_the_frozen_baseline() {
    let (transport, _sender) = frozen();
    let baseline = transport.baseline().expect("frozen baseline").setpoint();

    for (channel, index, span) in [
        (ControlChannel::Roll, 0_usize, 0.25),
        (ControlChannel::Pitch, 1, 0.25),
        (ControlChannel::Yaw, 2, 0.25),
        (ControlChannel::Vertical, 3, 0.1),
    ] {
        let prepared = transport
            .prepare_step(&step_request(channel, 0.5))
            .expect("prepared step");
        let target = prepared.requested().axes();
        let expected = baseline.axes();
        for axis in 0..4 {
            if axis == index {
                assert_eq!(
                    target[axis],
                    expected[axis] + 0.5 * span,
                    "the commanded axis takes the exact offset"
                );
            } else {
                assert_eq!(
                    target[axis], expected[axis],
                    "an unrelated axis keeps its frozen baseline value"
                );
            }
        }
    }
}

#[test]
fn a_vertical_step_measures_from_the_identified_hover_trim() {
    let (transport, _sender) = frozen();

    let prepared = transport
        .prepare_step(&step_request(ControlChannel::Vertical, -1.0))
        .expect("prepared step");

    assert_eq!(
        prepared.requested().collective_force,
        HOVER_TRIM - 0.1,
        "the vertical offset applies to the hover trim baseline"
    );
}

#[test]
fn the_operator_velocity_family_cannot_reach_the_direct_transport() {
    let (transport, _sender) = frozen();
    let mut stimulus = step_request(ControlChannel::Roll, 0.5);
    stimulus.family = ControlFamily::OperatorVelocity;

    let step = transport.prepare_step(&stimulus);
    let release = transport.prepare_release(&stimulus);

    for result in [step, release] {
        match result {
            Err(DirectTransportError::UnsupportedFamily { family }) => {
                assert_eq!(family, "operator_velocity");
            }
            other => panic!("expected a family refusal, got {other:?}"),
        }
    }
}

#[test]
fn an_inexact_mapping_is_refused() {
    let (transport, _sender) = frozen();
    let mut stimulus = step_request(ControlChannel::Roll, 0.5);
    stimulus.mapping = StimulusMapping::CandidateBoundCurve;

    let result = transport.prepare_step(&stimulus);

    assert!(matches!(result, Err(DirectTransportError::InexactMapping)));
}

#[test]
fn an_envelope_whose_physics_do_not_match_the_channel_is_refused() {
    let (transport, _sender) = frozen();
    let mut wrong_unit = step_request(ControlChannel::Roll, 0.5);
    wrong_unit.envelope = collective_envelope();
    let mut wrong_reference = step_request(ControlChannel::Vertical, 0.5);
    wrong_reference.envelope = attitude_envelope();

    for stimulus in [wrong_unit, wrong_reference] {
        assert!(matches!(
            transport.prepare_step(&stimulus),
            Err(DirectTransportError::EnvelopePhysics { .. })
        ));
    }
}

#[test]
fn a_normalized_value_outside_the_declared_range_is_refused() {
    let (transport, _sender) = frozen();

    let result = transport.prepare_step(&step_request(ControlChannel::Roll, 1.5));

    assert!(matches!(result, Err(DirectTransportError::Envelope { .. })));
}

#[test]
fn an_exact_step_reaches_its_target_in_one_simulator_sample() {
    let (mut transport, mut sender) = frozen();
    let prepared = transport
        .prepare_step(&step_request(ControlChannel::Roll, 1.0))
        .expect("prepared step");
    let requested = prepared.requested();

    let record = enacted(
        transport
            .enact_blocking(&mut sender, &prepared)
            .expect("enacted step"),
    );

    assert_eq!(sender.transmitted(), [requested], "one command, one sample");
    assert_eq!(record.transmitted, requested);
    assert_eq!(record.effective, requested);
    assert_eq!(record.purpose, DirectCommandPurpose::Step);
    assert_eq!(record.channel, ControlChannel::Roll);
    assert_eq!(record.family, ControlFamily::DirectAttitudeThrust);
    assert_eq!(
        record.times.effective_at_ns,
        record.times.transmitted_at_ns + super::SAMPLE_PERIOD_NS,
        "the effective readback is the exact next sample"
    );
    assert!(record.times.requested_at_ns <= record.times.transmitted_at_ns);
    assert_eq!(record.sender.endpoint, super::sender::ENDPOINT);
    assert_eq!(record.run_intent_digest, digest(9));
    assert_eq!(
        record.transport_identity_digest,
        transport.session().identity_digest()
    );
}

#[test]
fn a_release_sends_the_frozen_baseline_as_one_exact_step() {
    let (mut transport, mut sender) = frozen();
    let baseline = transport.baseline().expect("frozen baseline").setpoint();
    let stimulus = step_request(ControlChannel::Roll, 1.0);
    let step = transport.prepare_step(&stimulus).expect("prepared step");
    transport
        .enact_blocking(&mut sender, &step)
        .expect("enacted step");
    sender.clear_transmitted();

    let release = transport
        .prepare_release(&stimulus)
        .expect("prepared release");
    let record = enacted(
        transport
            .enact_blocking(&mut sender, &release)
            .expect("enacted release"),
    );

    assert_eq!(release.purpose(), DirectCommandPurpose::Release);
    assert_eq!(sender.transmitted(), [baseline]);
    assert_eq!(record.transmitted, baseline);
    assert_eq!(
        record.effective, baseline,
        "direct mode still holds the baseline after the release"
    );
}

#[test]
fn a_changed_prepared_target_fails_before_a_datagram_leaves_the_process() {
    let (mut transport, mut sender) = frozen();
    let prepared = transport
        .prepare_step(&step_request(ControlChannel::Roll, 1.0))
        .expect("prepared step")
        .with_requested_for_test(DirectSetpoint {
            roll_rad: 0.9,
            pitch_rad: 0.0,
            yaw_rad: 0.0,
            collective_force: 0.9,
        });

    let result = transport.enact_blocking(&mut sender, &prepared);

    assert!(matches!(
        result,
        Err(DirectTransportError::ChangedPreparedCommand {
            detail: "the re-derived physical target"
        })
    ));
    assert!(
        sender.transmitted().is_empty(),
        "no datagram may leave for a changed prepared target"
    );
}

#[test]
fn a_substituted_channel_family_or_envelope_is_refused_before_the_send() {
    let (mut transport, mut sender) = frozen();
    let honest = step_request(ControlChannel::Roll, 1.0);
    let mut other_envelope = attitude_envelope();
    other_envelope.positive_endpoint = 0.4;

    let substitutions = [
        step_request(ControlChannel::Pitch, 1.0),
        step_request(ControlChannel::Roll, 0.25),
        {
            let mut stimulus = honest.clone();
            stimulus.envelope = other_envelope;
            stimulus
        },
        {
            let mut stimulus = honest.clone();
            stimulus.family = ControlFamily::OperatorVelocity;
            stimulus
        },
    ];
    for stimulus in substitutions {
        let prepared = transport
            .prepare_step(&honest)
            .expect("prepared step")
            .with_stimulus_for_test(stimulus);

        let result = transport.enact_blocking(&mut sender, &prepared);

        assert!(result.is_err(), "a substituted stimulus must be refused");
        assert!(sender.transmitted().is_empty());
    }
}

#[test]
fn a_stale_run_binding_cannot_reuse_the_direct_baseline_or_target() {
    let (transport, _sender) = frozen();
    let prepared = transport
        .prepare_step(&step_request(ControlChannel::Roll, 1.0))
        .expect("prepared step");

    let mut later_sender = RecordingSender::new();
    let mut later = authorize(&later_sender);
    let mut later_run = baseline_request();
    later_run.run_intent_digest = digest(0x5a);
    later
        .freeze_baseline_blocking(&mut later_sender, &later_run)
        .expect("frozen baseline");
    later_sender.clear_transmitted();

    let result = later.enact_blocking(&mut later_sender, &prepared);

    assert!(matches!(
        result,
        Err(DirectTransportError::ChangedPreparedCommand {
            detail: "the frozen run intent"
        })
    ));
    assert!(later_sender.transmitted().is_empty());
}

#[test]
fn a_command_prepared_without_a_frozen_baseline_is_refused() {
    let sender = RecordingSender::new();
    let transport = authorize(&sender);

    let result = transport.prepare_step(&step_request(ControlChannel::Roll, 1.0));

    assert!(matches!(result, Err(DirectTransportError::NoBaseline)));
}

#[test]
fn an_effective_setpoint_that_left_the_transmitted_target_quarantines_the_run() {
    let mut sender = RecordingSender::new();
    let mut transport = authorize(&sender);
    transport
        .freeze_baseline_blocking(&mut sender, &baseline_request())
        .expect("frozen baseline");
    let prepared = transport
        .prepare_step(&step_request(ControlChannel::Roll, 1.0))
        .expect("prepared step");
    let mut constrained = RecordingSender::new().substituting(DirectSetpoint {
        roll_rad: 0.0,
        pitch_rad: 0.0,
        yaw_rad: 0.0,
        collective_force: HOVER_TRIM,
    });
    // Seed the raw source so the pre-send causal check has an exact sample.
    DirectCommandSender::transmit_exact_blocking(&mut constrained, prepared.baseline())
        .expect("seed the raw source");

    let result = transport.enact_blocking(&mut constrained, &prepared);

    assert!(matches!(
        result,
        Err(DirectTransportError::EffectiveTargetMismatch { .. })
    ));
}
