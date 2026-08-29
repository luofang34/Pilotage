//! The reference trial a host builds when it commands one stimulus.
//!
//! A trial that only observes states no control meaning, so nothing scoped to
//! a control family can be written for it. This is the smallest trial that
//! does: one phase that commands one stimulus on one channel, with the
//! versioned physical envelope that says what the normalized value asks for.

use pilotage_mission_core::{ControlChannel, ControlFamily, StimulusEnvelope};
use pilotage_trial::{
    BackendCapability, Comparison, Phase, PhaseAction, PhaseCondition, SCENARIO_SCHEMA_VERSION,
    Scenario, Waveform,
};

use super::ScenarioRuntimeError;

/// One reference stimulus, in the contract types a campaign host speaks.
#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceStimulus {
    /// The physical control family the trial commands.
    pub family: ControlFamily,
    /// The control channel the trial commands.
    pub channel: ControlChannel,
    /// The versioned physical envelope of the normalized range.
    pub envelope: StimulusEnvelope,
    /// The normalized value the trial holds.
    pub normalized_value: f64,
}

/// Creates the declarative stimulus scenario a reference backend commands.
///
/// The phase declares contact state as well as its family capability. A
/// campaign's first hard gate reads a contact signal, so a backend that
/// cannot report one cannot execute this trial, and the refusal happens
/// during capability admission rather than one sample at a time.
///
/// # Errors
///
/// Returns [`ScenarioRuntimeError`] when the stimulus cannot be projected
/// into the trial contract.
pub fn reference_stimulus_scenario(
    id: &str,
    completion_time_ns: u64,
    stimulus: &ReferenceStimulus,
) -> Result<Scenario, ScenarioRuntimeError> {
    let family: pilotage_trial::ControlFamily =
        super::document::transcode_contract("control family", &stimulus.family)?;
    let channel: pilotage_trial::ControlChannel =
        super::document::transcode_contract("control channel", &stimulus.channel)?;
    let envelope: pilotage_trial::StimulusEnvelope =
        super::document::transcode_contract("stimulus envelope", &stimulus.envelope)?;
    let mapping = family.mapping();
    Ok(Scenario {
        schema_version: SCENARIO_SCHEMA_VERSION,
        id: id.to_owned(),
        revision: 1,
        phases: vec![Phase {
            id: "stimulate".to_owned(),
            max_sim_time_ns: completion_time_ns.saturating_add(1_000_000_000),
            required_capabilities: vec![
                BackendCapability::SimulatorTime,
                BackendCapability::ContactState,
                family.capability(),
            ],
            entry_conditions: vec![PhaseCondition::Always],
            action: PhaseAction::Stimulus {
                family,
                channel,
                mapping,
                envelope,
                waveform: Waveform::Step {
                    value: stimulus.normalized_value,
                },
            },
            exit_conditions: vec![PhaseCondition::SimulatorTime {
                comparison: Comparison::GreaterOrEqual,
                value_ns: completion_time_ns,
            }],
            abort_conditions: Vec::new(),
        }],
    })
}
