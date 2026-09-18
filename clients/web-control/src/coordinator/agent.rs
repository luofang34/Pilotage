//! The automation input source (ADR-0042).
//!
//! An agent is one more input source of the control runtime, beside the
//! keyboard and a pad. It takes the same transactional path into control: a
//! neutral handover, the retained lease, and an advanced activation revision
//! that the announcement carries with the agent's own identity. The agent
//! states flight demands directly. It gives way on the first operator input.

use pilotage_input::content_digest;

use super::{ControlCoordinator, InputSource, PendingSwap};
use crate::sample::{ButtonSample, DirectDemand, RawSample};

/// The revision of the automation source contract that the announcement
/// carries as the device revision.
const AGENT_SOURCE_REVISION: u32 = 1;

/// The identity that an engaged agent announces in place of a device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AgentIdentity {
    pub(super) profile_id: String,
    pub(super) digest: [u8; 32],
}

impl AgentIdentity {
    pub(super) const fn revision() -> u32 {
        AGENT_SOURCE_REVISION
    }
}

/// What an agent asks for in one tick.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AgentInput {
    /// The flight demand.
    pub demand: DirectDemand,
    /// True in the tick that asks for an arm.
    pub arm: bool,
    /// True in the tick that asks for a disarm.
    pub disarm: bool,
}

/// How one agent tick ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentTick {
    /// The sample carries the agent's demand.
    Flown,
    /// The operator touched a control. The agent is disengaged, and the
    /// sample carries the operator's input.
    OperatorOverride,
    /// No agent is engaged. The sample carries the operator's input.
    NotEngaged,
}

impl ControlCoordinator {
    /// Engages an agent as the input source, under the identity that it
    /// announces. Returns `false` for an empty identity or before a scheme is
    /// active: there is then nothing to hand control over from. An engaged
    /// agent keeps its identity: a different identity is refused, because it
    /// would change the announcement with no handover and no new revision.
    pub fn engage_agent(&mut self, profile_id: &str) -> bool {
        if profile_id.trim().is_empty() || self.runtime.active_flight_buttons().is_none() {
            return false;
        }
        if self.agent_engaged() {
            return self
                .agent
                .as_ref()
                .is_some_and(|agent| agent.profile_id == profile_id);
        }
        self.agent_resume = self
            .pending
            .as_ref()
            .and_then(|pending| pending.source)
            .unwrap_or(self.active_source);
        self.agent = Some(AgentIdentity {
            profile_id: profile_id.to_owned(),
            digest: content_digest(profile_id.as_bytes()),
        });
        self.swap(PendingSwap {
            pad: None,
            keyboard: None,
            source: Some(InputSource::Agent),
        });
        true
    }

    /// Returns control to the operator's device through the same handover:
    /// the pad when one is selected, the keyboard when none is. The
    /// announcement then names the device that drives.
    pub fn disengage_agent(&mut self) {
        if !self.agent_engaged() {
            return;
        }
        self.swap(PendingSwap {
            pad: None,
            keyboard: None,
            source: Some(self.agent_resume),
        });
    }

    /// Whether an agent is the input source, or will be when the open
    /// handover installs.
    #[must_use]
    pub fn agent_engaged(&self) -> bool {
        self.pending
            .as_ref()
            .and_then(|pending| pending.source)
            .unwrap_or(self.active_source)
            == InputSource::Agent
    }

    /// Builds the sample for one tick of an engaged agent. The operator's
    /// keyboard and pad are read first: any input there disengages the agent,
    /// and the sample is then the operator's.
    pub fn agent_sample(
        &mut self,
        pad_axes: &[f32],
        pad_buttons: &[ButtonSample],
        input: AgentInput,
        out: &mut RawSample,
    ) -> AgentTick {
        let engaged = self.agent_engaged();
        self.stage.key_sample(out);
        let keys_neutral = self.runtime.operator_input_neutral(out);
        if keys_neutral {
            self.stage.pad_sample(pad_axes, pad_buttons, out);
        }
        if !engaged {
            return AgentTick::NotEngaged;
        }
        if !keys_neutral || !self.runtime.operator_input_neutral(out) {
            self.disengage_agent();
            return AgentTick::OperatorOverride;
        }
        let (arm, disarm) = self.runtime.active_flight_buttons().unwrap_or((0, 0));
        let slots = usize::from(arm.max(disarm)).saturating_add(1);
        out.axes.clear();
        out.buttons.clear();
        out.buttons.resize(slots, ButtonSample::default());
        press(out, usize::from(arm), input.arm);
        press(out, usize::from(disarm), input.disarm);
        out.direct = Some(input.demand.bounded());
        AgentTick::Flown
    }
}

fn press(sample: &mut RawSample, slot: usize, pressed: bool) {
    if let Some(button) = sample.buttons.get_mut(slot) {
        *button = ButtonSample {
            pressed,
            value: if pressed { 1.0 } else { 0.0 },
        };
    }
}
