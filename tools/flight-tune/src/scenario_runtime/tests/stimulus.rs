//! Control-family admission and projection checks.

use std::cell::Cell;
use std::rc::Rc;

use pilotage_mission_core::{MissionAction, MissionCapability, TrialAction};
use pilotage_trial::{
    BackendCapability, Comparison, ControlChannel, ControlFamily, Phase, PhaseAction,
    PhaseCondition, PhysicalUnit, ReferenceRule, SCENARIO_SCHEMA_VERSION, Scenario,
    StimulusEnvelope, Waveform,
};

use super::{
    CampaignMissionRuntime, ReferenceRuntime, ScenarioRuntimeError, context,
    mission_document_from_scenario, navigation, start,
};
use crate::{ArtifactIdentity, Digest};

#[test]
fn an_unsupported_control_family_precedes_action_port_mutation() {
    for (family, capability) in [
        (
            ControlFamily::OperatorVelocity,
            MissionCapability::OperatorVelocityControl,
        ),
        (
            ControlFamily::DirectAttitudeThrust,
            MissionCapability::DirectAttitudeThrustControl,
        ),
    ] {
        let document = mission_document_from_scenario(
            &stimulus_scenario(family, ControlChannel::Roll),
            navigation(),
            0,
            1_000_000,
        )
        .expect("project scenario");
        let identity = ArtifactIdentity::new("runtime", Digest::from_bytes([9; 32]))
            .expect("runtime identity");
        let mutations = Rc::new(Cell::new(0));
        let mut action_port = ReferenceRuntime::tracked(identity.clone(), mutations.clone())
            .with_capability(MissionCapability::SimulatorControl);

        let result = CampaignMissionRuntime::start_blocking(
            document.clone(),
            start(&document),
            &identity,
            &mut action_port,
            &context(),
        );

        match result.map(|_| ()) {
            Err(ScenarioRuntimeError::MissingCapability {
                capability: missing,
                ..
            }) => assert_eq!(missing, capability),
            other => panic!("{family:?} must be refused before mutation: {other:?}"),
        }
        assert_eq!(mutations.get(), 0);
    }
}

#[test]
fn a_generic_runtime_admits_each_declared_control_family() {
    for (family, capability) in [
        (
            ControlFamily::OperatorVelocity,
            MissionCapability::OperatorVelocityControl,
        ),
        (
            ControlFamily::DirectAttitudeThrust,
            MissionCapability::DirectAttitudeThrustControl,
        ),
    ] {
        let document = mission_document_from_scenario(
            &stimulus_scenario(family, ControlChannel::Yaw),
            navigation(),
            0,
            1_000_000,
        )
        .expect("project scenario");
        let identity = ArtifactIdentity::new("runtime", Digest::from_bytes([9; 32]))
            .expect("runtime identity");
        let action_port = ReferenceRuntime::new(identity).with_capability(capability);

        CampaignMissionRuntime::attest_capabilities(&document, &action_port)
            .unwrap_or_else(|error| panic!("{family:?} must be admitted: {error}"));
    }
}

#[test]
fn projection_carries_the_control_family_and_envelope_without_loss() {
    let scenario = stimulus_scenario(
        ControlFamily::DirectAttitudeThrust,
        ControlChannel::Vertical,
    );
    let authored = trial_envelope(
        ControlFamily::DirectAttitudeThrust,
        ControlChannel::Vertical,
    );

    let document = mission_document_from_scenario(&scenario, navigation(), 0, 1_000_000)
        .expect("project stimulus");

    assert!(
        document.phases[0]
            .required_capabilities
            .contains(&MissionCapability::DirectAttitudeThrustControl)
    );
    let MissionAction::Trial(TrialAction::Stimulate {
        family,
        channel,
        mapping,
        envelope,
        ..
    }) = &document.phases[0].action
    else {
        panic!("the projection must keep the stimulate action");
    };
    assert_eq!(
        *family,
        pilotage_mission_core::ControlFamily::DirectAttitudeThrust
    );
    assert_eq!(*channel, pilotage_mission_core::ControlChannel::Vertical);
    assert_eq!(
        *mapping,
        pilotage_mission_core::StimulusMapping::AffineExact
    );
    assert_eq!(
        envelope
            .canonical_digest()
            .expect("document envelope digest")
            .as_bytes(),
        authored
            .canonical_digest()
            .expect("authored envelope digest")
            .as_bytes(),
        "the projection keeps the envelope identity"
    );
}

#[test]
fn a_stimulus_phase_does_not_gain_the_neutral_simulator_control_capability() {
    let scenario = stimulus_scenario(ControlFamily::OperatorVelocity, ControlChannel::Pitch);

    let document = mission_document_from_scenario(&scenario, navigation(), 0, 1_000_000)
        .expect("project stimulus");

    assert!(
        !document.phases[0]
            .required_capabilities
            .contains(&MissionCapability::SimulatorControl),
        "the family capability replaces the neutral simulator-control capability"
    );
}

fn trial_envelope(family: ControlFamily, channel: ControlChannel) -> StimulusEnvelope {
    match (family, channel) {
        (ControlFamily::OperatorVelocity, ControlChannel::Yaw) => StimulusEnvelope {
            id: "reference.operator.yaw".to_owned(),
            revision: 1,
            unit: PhysicalUnit::RadiansPerSecond,
            reference: ReferenceRule::Zero,
            negative_endpoint: -1.2,
            neutral: 0.0,
            positive_endpoint: 1.2,
        },
        (ControlFamily::OperatorVelocity, _) => StimulusEnvelope {
            id: "reference.operator.linear".to_owned(),
            revision: 1,
            unit: PhysicalUnit::MetersPerSecond,
            reference: ReferenceRule::Zero,
            negative_endpoint: -4.0,
            neutral: 0.0,
            positive_endpoint: 4.0,
        },
        (ControlFamily::DirectAttitudeThrust, ControlChannel::Vertical) => StimulusEnvelope {
            id: "reference.direct.collective".to_owned(),
            revision: 2,
            unit: PhysicalUnit::NormalizedCollectiveForce,
            reference: ReferenceRule::IdentifiedHoverTrim,
            negative_endpoint: -0.2,
            neutral: 0.05,
            positive_endpoint: 0.4,
        },
        (ControlFamily::DirectAttitudeThrust, _) => StimulusEnvelope {
            id: "reference.direct.attitude".to_owned(),
            revision: 1,
            unit: PhysicalUnit::Radians,
            reference: ReferenceRule::EffectiveSetpointAtEntry,
            negative_endpoint: -0.3,
            neutral: 0.0,
            positive_endpoint: 0.3,
        },
    }
}

fn stimulus_scenario(family: ControlFamily, channel: ControlChannel) -> Scenario {
    Scenario {
        schema_version: SCENARIO_SCHEMA_VERSION,
        id: "stimulus-scenario".to_owned(),
        revision: 1,
        phases: vec![Phase {
            id: "stimulus".to_owned(),
            max_sim_time_ns: 2_000_000,
            required_capabilities: vec![BackendCapability::SimulatorTime, family.capability()],
            entry_conditions: vec![PhaseCondition::Always],
            action: PhaseAction::Stimulus {
                family,
                channel,
                mapping: family.mapping(),
                envelope: trial_envelope(family, channel),
                waveform: Waveform::Pulse {
                    value: 0.25,
                    duration_ns: 1_000_000,
                },
            },
            exit_conditions: vec![PhaseCondition::SimulatorTime {
                comparison: Comparison::GreaterOrEqual,
                value_ns: 1_000_000,
            }],
            abort_conditions: Vec::new(),
        }],
    }
}
