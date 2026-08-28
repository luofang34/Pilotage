//! Document-level checks for the physical stimulus contract.

use crate::{
    CodecError, ControlChannel, ControlFamily, ExecutionTarget, MissionAction, MissionCapability,
    PhysicalUnit, ReferenceRule, StimulusEnvelope, StimulusError, TrialAction, ValidationError,
    Waveform,
};

use super::{document_for, operator_envelope, recalculate};

fn direct_envelope() -> StimulusEnvelope {
    StimulusEnvelope {
        id: "bench.direct.pitch".to_owned(),
        revision: 1,
        unit: PhysicalUnit::Radians,
        reference: ReferenceRule::EffectiveSetpointAtEntry,
        negative_endpoint: -0.25,
        neutral: 0.0,
        positive_endpoint: 0.25,
    }
}

fn stimulate(family: ControlFamily, envelope: StimulusEnvelope) -> MissionAction {
    MissionAction::Trial(TrialAction::Stimulate {
        family,
        channel: ControlChannel::Pitch,
        mapping: family.mapping(),
        envelope,
        waveform: Waveform::Step { value: 0.2 },
    })
}

#[test]
fn a_stimulus_phase_declares_its_control_family_capability() {
    for (family, envelope, capability) in [
        (
            ControlFamily::OperatorVelocity,
            operator_envelope(),
            MissionCapability::OperatorVelocityControl,
        ),
        (
            ControlFamily::DirectAttitudeThrust,
            direct_envelope(),
            MissionCapability::DirectAttitudeThrustControl,
        ),
    ] {
        let action = stimulate(family, envelope);
        assert_eq!(action.required_capability(), Some(capability));
        let mut document = document_for(action, ExecutionTarget::Simulator)
            .expect("the stimulus mission must be valid");
        document.phases[0]
            .required_capabilities
            .retain(|value| *value != capability);
        recalculate(&mut document);
        assert!(matches!(
            document.validate(),
            Err(ValidationError::UndeclaredCapability { .. })
        ));
    }
}

#[test]
fn a_document_stimulus_rejects_a_substituted_unit() {
    let mut envelope = operator_envelope();
    envelope.unit = PhysicalUnit::Radians;

    let result = document_for(
        stimulate(ControlFamily::OperatorVelocity, envelope),
        ExecutionTarget::Simulator,
    );

    assert!(matches!(
        result,
        Err(CodecError::Validation(ValidationError::InvalidStimulus {
            source: StimulusError::UnitMismatch { .. },
            ..
        }))
    ));
}

#[test]
fn a_changed_control_family_changes_the_content_digest() {
    let operator = document_for(
        stimulate(ControlFamily::OperatorVelocity, operator_envelope()),
        ExecutionTarget::Simulator,
    )
    .expect("the operator mission must be valid");
    let direct = document_for(
        stimulate(ControlFamily::DirectAttitudeThrust, direct_envelope()),
        ExecutionTarget::Simulator,
    )
    .expect("the direct mission must be valid");

    assert_ne!(
        operator.identity.content_digest,
        direct.identity.content_digest
    );
}

#[test]
fn a_changed_envelope_changes_the_content_digest_and_the_envelope_digest() {
    let base = operator_envelope();
    let mut wider = base.clone();
    wider.positive_endpoint = 6.0;
    let document = document_for(
        stimulate(ControlFamily::OperatorVelocity, base.clone()),
        ExecutionTarget::Simulator,
    )
    .expect("the base mission must be valid");
    let changed = document_for(
        stimulate(ControlFamily::OperatorVelocity, wider.clone()),
        ExecutionTarget::Simulator,
    )
    .expect("the changed mission must be valid");

    assert_ne!(
        base.canonical_digest().expect("base envelope digest"),
        wider.canonical_digest().expect("wider envelope digest")
    );
    assert_ne!(
        document.identity.content_digest,
        changed.identity.content_digest
    );
}
