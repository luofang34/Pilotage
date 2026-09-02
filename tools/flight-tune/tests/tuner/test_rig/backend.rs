use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use flight_tune::{
    AdapterError, ArtifactIdentity, CampaignBackend, Digest, KinematicTruth, MissionCapability,
    MissionDirective, MissionDocument, MissionReference, ReceiptResult, RunExecutionContext,
    RunPreparationReceipt, SampleEvent, ScenarioFrame, ScenarioObservationReceipt, ScenarioRuntime,
    ScenarioRuntimeError, ScenarioStartReceipt, ScenarioStopContext, SessionChallenge,
    SimulatorCapability, SimulatorSessionAcquisition, SimulatorSessionReceipt, TelemetrySample,
    scenario_runtime_identity,
};
use serde::Deserialize;

use super::terminal_head_poison::{TerminalExternalAction, poison_terminal_head};
use super::{FakeHandle, identity};

pub struct FakeBackend {
    state: FakeHandle,
    simulator: ArtifactIdentity,
    airframe: ArtifactIdentity,
    action_port_identity: ArtifactIdentity,
    scenario_runtime_identity: ArtifactIdentity,
}

impl FakeBackend {
    pub fn new(state: FakeHandle) -> Self {
        Self::with_simulator_id(state, "fake-simulator-v1")
    }

    pub fn with_simulator_id(state: FakeHandle, id: &str) -> Self {
        let action_port_identity = identity("vehicle", "fake-controller-v1");
        Self {
            state,
            simulator: identity("simulator", id),
            airframe: identity("airframe", "default-airframe"),
            scenario_runtime_identity: scenario_runtime_identity(&action_port_identity)
                .expect("scenario runtime identity"),
            action_port_identity,
        }
    }

    pub fn with_action_port_identity(state: FakeHandle, content: &str) -> Self {
        let mut backend = Self::new(state);
        backend.action_port_identity = identity("scenario-action-port", content);
        backend.scenario_runtime_identity =
            scenario_runtime_identity(&backend.action_port_identity)
                .expect("scenario runtime identity");
        backend
    }

    /// Names the session acquisition this backend answers for.
    pub fn session_acquisition(&self, session_digest: Digest) -> SimulatorSessionAcquisition {
        SimulatorSessionAcquisition::new(
            session_digest,
            self.simulator.digest,
            self.airframe.digest,
        )
    }
}

impl CampaignBackend for FakeBackend {
    type ScenarioRuntime = Self;

    fn simulator_identity(&self) -> &ArtifactIdentity {
        &self.simulator
    }

    fn airframe_identity(&self) -> &ArtifactIdentity {
        &self.airframe
    }

    fn scenario_runtime(&self) -> &Self::ScenarioRuntime {
        self
    }

    fn scenario_runtime_mut(&mut self) -> &mut Self::ScenarioRuntime {
        self
    }

    fn attest_scenario_runtime_blocking(&self) -> Result<(), AdapterError> {
        let expected = scenario_runtime_identity(&self.action_port_identity)
            .map_err(|error| AdapterError::new(error.to_string()))?;
        if expected == self.scenario_runtime_identity {
            Ok(())
        } else {
            Err(AdapterError::new(
                "the fake scenario runtime identity changed",
            ))
        }
    }

    fn mission_document_blocking(
        &self,
        mission: &MissionReference,
    ) -> Result<MissionDocument, AdapterError> {
        let (bad_content, bad_revision) = {
            let state = self.state.0.borrow();
            (state.bad_mission_content, state.bad_mission_revision)
        };
        if bad_revision {
            return Ok(super::fake_mission_document(super::FAKE_MISSION_IDS[2]));
        }
        let mut document = super::fake_stored_mission(mission)
            .ok_or_else(|| AdapterError::new("the fake backend stores no such mission"))?;
        if bad_content {
            document.execution_policy.retry_limit =
                document.execution_policy.retry_limit.wrapping_add(1);
        }
        Ok(document)
    }

    fn project_scenario_frame(
        &mut self,
        sample: &TelemetrySample,
    ) -> Result<ScenarioFrame, AdapterError> {
        Ok(fake_scenario_frame(sample))
    }

    fn open_session_blocking(
        &mut self,
        challenge: &SessionChallenge,
    ) -> Result<SimulatorSessionReceipt, AdapterError> {
        let mut state = self.state.0.borrow_mut();
        state.open_session_count = state.open_session_count.wrapping_add(1);
        state.lifecycle.push("open_session".to_owned());
        state.open_order.push("open_session".to_owned());
        // A backend cannot talk to a runtime the operator has not leased,
        // so a cleanup that released the lease shows up as the next open
        // having nothing to open a session against.
        if state
            .runtime_lease
            .as_ref()
            .is_some_and(|lease| !lease.held)
        {
            return Err(AdapterError::new("no operator runtime lease is held"));
        }
        state.session_open = true;
        let bad_receipt = state.bad_session_receipt;
        drop(state);
        Ok(SimulatorSessionReceipt {
            session_digest: challenge.session_digest(),
            simulator_digest: self.simulator.digest,
            airframe_digest: if bad_receipt {
                Digest::from_bytes([73; 32])
            } else {
                self.airframe.digest
            },
        })
    }

    fn close_session_blocking(
        &mut self,
        acquisition: &SimulatorSessionAcquisition,
    ) -> Result<(), AdapterError> {
        if acquisition.simulator_digest() != self.simulator.digest
            || acquisition.airframe_digest() != self.airframe.digest
            || acquisition.session_digest().is_zero()
        {
            return Err(AdapterError::new(
                "the acquisition names another simulator session",
            ));
        }
        let mut state = self.state.0.borrow_mut();
        state.open_order.push("close_session".to_owned());
        state.session_close_count = state.session_close_count.wrapping_add(1);
        if state.fail_session_close_on == Some(state.session_close_count) {
            return Err(AdapterError::new(
                "the simulator did not acknowledge the session close",
            ));
        }
        state.session_open = false;
        Ok(())
    }

    fn prepare_blocking(
        &mut self,
        capability: &SimulatorCapability,
        context: &RunExecutionContext,
        scenario: &MissionReference,
    ) -> Result<RunPreparationReceipt, AdapterError> {
        let run_intent_digest = context
            .digest()
            .map_err(|error| AdapterError::new(error.to_string()))?;
        let mut state = self.state.0.borrow_mut();
        state.prepare_count = state.prepare_count.wrapping_add(1);
        state.lifecycle.push("prepare".to_owned());
        state.transition.prepared_contexts.push(context.clone());
        if state.panic_on_prepare == Some(state.prepare_count) {
            panic!("simulated process stop after AttemptPrepared");
        }
        let head_change = state.change_head_on_prepare.take();
        state.current_scenario = Some(scenario.clone());
        state.current_seed = context.seed();
        state.next_sequence = 0;
        let receipt_intent = if state.transition.bad_preparation_intent {
            Digest::from_bytes([97; 32])
        } else {
            run_intent_digest
        };
        drop(state);
        if let Some(root) = head_change {
            change_head_digest(&root);
        }
        Ok(RunPreparationReceipt {
            session_digest: capability.session_digest(),
            run_intent_digest: receipt_intent,
        })
    }

    fn start_blocking(
        &mut self,
        capability: &SimulatorCapability,
        context: &RunExecutionContext,
    ) -> Result<ScenarioStartReceipt, AdapterError> {
        let run_intent_digest = context
            .digest()
            .map_err(|error| AdapterError::new(error.to_string()))?;
        let mut state = self.state.0.borrow_mut();
        state.start_count = state.start_count.wrapping_add(1);
        state.lifecycle.push("start".to_owned());
        if state.transition.prepared_contexts.last() != Some(context) {
            return Err(AdapterError::new(
                "started run intent differs from prepared run intent",
            ));
        }
        state.transition.started_contexts.push(context.clone());
        if state.panic_on_start == Some(state.start_count) {
            panic!("simulated process stop after candidate activation");
        }
        if state.start_count <= state.fail_starts_through {
            return Err(AdapterError::new(
                "simulated execution failure after candidate activation",
            ));
        }
        let scenario = state
            .current_scenario
            .clone()
            .ok_or_else(|| AdapterError::new("scenario was not prepared"))?;
        let seed = state.current_seed;
        let gain = state.vehicle.gain;
        state
            .scenario_runs
            .push((scenario.revision_id.clone(), seed, gain));
        let applied_mission_content_digest = if state.bad_scenario_readback {
            Digest::from_bytes([98; 32])
        } else {
            scenario.content_digest
        };
        let receipt_intent = if state.transition.bad_start_intent {
            Digest::from_bytes([96; 32])
        } else {
            run_intent_digest
        };
        Ok(ScenarioStartReceipt {
            session_digest: capability.session_digest(),
            applied_mission_content_digest,
            seed,
            run_intent_digest: receipt_intent,
        })
    }

    fn sample_blocking(&mut self, _timeout: Duration) -> Result<SampleEvent, AdapterError> {
        let mut state = self.state.0.borrow_mut();
        state.sample_poll_count = state.sample_poll_count.wrapping_add(1);
        if state.complete_without_sample {
            return Ok(SampleEvent::Complete);
        }
        if state.timeout_next_sample {
            state.timeout_next_sample = false;
            return Ok(SampleEvent::TimedOut);
        }
        if state.next_sequence > 0 {
            return Ok(SampleEvent::Complete);
        }
        let sequence = state.next_sequence;
        state.next_sequence = state.next_sequence.wrapping_add(1);
        state.sample_count = state.sample_count.wrapping_add(1);
        state.lifecycle.push("sample".to_owned());
        let values = BTreeMap::from([("gain".to_owned(), state.vehicle.gain)]);
        let head_change = state.change_head_on_sample.take();
        let object_change = state.change_object_on_sample.take();
        drop(state);
        if let Some(root) = head_change {
            change_head_digest(&root);
        }
        if let Some(object) = object_change {
            change_object_bytes(&object);
        }
        Ok(SampleEvent::Sample(TelemetrySample {
            sequence,
            elapsed_ms: 10,
            values,
        }))
    }

    fn stop_blocking(&mut self) -> Result<(), AdapterError> {
        let (expected, head_poison, fail) = {
            let mut state = self.state.0.borrow_mut();
            state.stop_count = state.stop_count.wrapping_add(1);
            state.lifecycle.push("stop".to_owned());
            let expected = state.expected_head_event_on_stop.take();
            let head_poison = state
                .terminal
                .take_head_poison(TerminalExternalAction::SimulatorStop);
            (expected, head_poison, state.terminal.fail_simulator_stop)
        };
        if let Some((root, event)) = expected {
            assert_head_event(&root, &event);
        }
        poison_terminal_head(head_poison);
        if fail {
            return Err(AdapterError::new(
                "the reference simulator stop operation failed",
            ));
        }
        Ok(())
    }

    fn cleanup_blocking(&mut self) -> Result<(), AdapterError> {
        let mut state = self.state.0.borrow_mut();
        state.cleanup_count = state.cleanup_count.wrapping_add(1);
        state.lifecycle.push("cleanup".to_owned());
        state.cleanup_fault.finish(state.cleanup_count)
    }
}

impl ScenarioRuntime for FakeBackend {
    fn identity(&self) -> &ArtifactIdentity {
        &self.scenario_runtime_identity
    }

    fn capabilities(&self) -> &[MissionCapability] {
        &[
            MissionCapability::SimulatorTime,
            MissionCapability::ContactState,
            MissionCapability::OperatorVelocityControl,
            MissionCapability::DirectAttitudeThrustControl,
        ]
    }

    fn prepare_blocking(
        &mut self,
        _document: &MissionDocument,
        _context: &RunExecutionContext,
    ) -> Result<(), ScenarioRuntimeError> {
        let head_change = self
            .state
            .0
            .borrow_mut()
            .change_head_on_action_prepare
            .take();
        if let Some(root) = head_change {
            change_head_digest(&root);
        }
        Ok(())
    }

    fn start_blocking(&mut self) -> Result<(), ScenarioRuntimeError> {
        let mut state = self.state.0.borrow_mut();
        state.scenario_action_start_count = state.scenario_action_start_count.wrapping_add(1);
        state.lifecycle.push("scenario_action_start".to_owned());
        Ok(())
    }

    fn observe_blocking(
        &mut self,
        frame: &ScenarioFrame,
        directive: Option<&MissionDirective>,
    ) -> Result<ScenarioObservationReceipt, ScenarioRuntimeError> {
        let mut state = self.state.0.borrow_mut();
        state.scenario_action_observe_count = state.scenario_action_observe_count.wrapping_add(1);
        state.lifecycle.push("scenario_action_observe".to_owned());
        drop(state);
        Ok(ScenarioObservationReceipt {
            source_sequence: frame.source_sequence,
            action_result: directive.map(|_| ReceiptResult::Succeeded {}),
        })
    }

    fn stop_blocking(
        &mut self,
        _context: &mut ScenarioStopContext,
    ) -> Result<(), ScenarioRuntimeError> {
        let mut state = self.state.0.borrow_mut();
        state.scenario_action_stop_count = state.scenario_action_stop_count.wrapping_add(1);
        state.lifecycle.push("scenario_action_stop".to_owned());
        Ok(())
    }

    fn cleanup_blocking(&mut self) -> Result<(), ScenarioRuntimeError> {
        let mut state = self.state.0.borrow_mut();
        state.scenario_action_cleanup_count = state.scenario_action_cleanup_count.wrapping_add(1);
        state.lifecycle.push("scenario_action_cleanup".to_owned());
        Ok(())
    }
}

fn fake_scenario_frame(sample: &TelemetrySample) -> ScenarioFrame {
    ScenarioFrame {
        source_sequence: sample.sequence,
        simulator_time_ns: sample.elapsed_ms.saturating_mul(1_000_000),
        trial_time_ns: sample.elapsed_ms.saturating_mul(1_000_000),
        lifecycle: None,
        ground_contact: Some(false),
        crashed: Some(false),
        link_valid: Some(true),
        estimator_valid: Some(true),
        truth: KinematicTruth {
            position_ned_m: [0.0; 3],
            velocity_ned_mps: [0.0; 3],
            acceleration_ned_mps2: [0.0; 3],
            attitude_wxyz: [1.0, 0.0, 0.0, 0.0],
            body_rates_rps: [0.0; 3],
        },
        applied_conditions: BTreeMap::new(),
        canonical_signals: Vec::new(),
    }
}

#[derive(Deserialize)]
struct HeadPointer {
    digest: Digest,
}

fn assert_head_event(root: &Path, expected: &str) {
    let head_bytes = std::fs::read(root.join("HEAD.json")).expect("read journal head");
    let head: HeadPointer = serde_json::from_slice(&head_bytes).expect("decode journal head");
    let entry_path = root.join("entries").join(format!("{}.json", head.digest));
    let entry_bytes = std::fs::read(entry_path).expect("read journal entry");
    let entry: serde_json::Value =
        serde_json::from_slice(&entry_bytes).expect("decode journal entry");
    assert_eq!(
        entry
            .get("event")
            .and_then(|event| event.get("event"))
            .and_then(serde_json::Value::as_str),
        Some(expected)
    );
}

fn change_head_digest(root: &Path) {
    let head = root.join("HEAD.json");
    let mut bytes = std::fs::read(&head).expect("read journal head");
    let digest_tail = bytes.len().checked_sub(3).expect("HEAD digest byte");
    bytes[digest_tail] = if bytes[digest_tail] == b'0' {
        b'1'
    } else {
        b'0'
    };
    std::fs::write(head, bytes).expect("change journal head");
}

fn change_object_bytes(path: &Path) {
    let mut bytes = std::fs::read(path).expect("read journal object");
    bytes.push(b' ');
    std::fs::write(path, bytes).expect("change journal object");
}
