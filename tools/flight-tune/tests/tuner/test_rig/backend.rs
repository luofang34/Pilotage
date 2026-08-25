use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use flight_tune::{
    AdapterError, ArtifactIdentity, Digest, RunExecutionContext, RunPreparationReceipt,
    SampleEvent, ScenarioRef, ScenarioStartReceipt, SessionChallenge, SimulatorBackend,
    SimulatorCapability, SimulatorSessionReceipt, TelemetrySample,
};
use serde::Deserialize;

use super::terminal_head_poison::{TerminalExternalAction, poison_terminal_head};
use super::{FakeHandle, identity};

pub struct FakeBackend {
    state: FakeHandle,
    simulator: ArtifactIdentity,
    airframe: ArtifactIdentity,
}

impl FakeBackend {
    pub fn new(state: FakeHandle) -> Self {
        Self::with_simulator_id(state, "fake-simulator-v1")
    }

    pub fn with_simulator_id(state: FakeHandle, id: &str) -> Self {
        Self {
            state,
            simulator: identity("simulator", id),
            airframe: identity("airframe", "default-airframe"),
        }
    }
}

impl SimulatorBackend for FakeBackend {
    fn simulator_identity(&self) -> &ArtifactIdentity {
        &self.simulator
    }

    fn airframe_identity(&self) -> &ArtifactIdentity {
        &self.airframe
    }

    fn open_session_blocking(
        &mut self,
        challenge: &SessionChallenge,
    ) -> Result<SimulatorSessionReceipt, AdapterError> {
        let mut state = self.state.0.borrow_mut();
        state.open_session_count = state.open_session_count.wrapping_add(1);
        state.lifecycle.push("open_session".to_owned());
        drop(state);
        Ok(SimulatorSessionReceipt {
            session_digest: challenge.session_digest(),
            simulator_digest: self.simulator.digest,
            airframe_digest: self.airframe.digest,
        })
    }

    fn prepare_blocking(
        &mut self,
        capability: &SimulatorCapability,
        context: &RunExecutionContext,
        scenario: &ScenarioRef,
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
        let scenario = state
            .current_scenario
            .clone()
            .ok_or_else(|| AdapterError::new("scenario was not prepared"))?;
        let seed = state.current_seed;
        let gain = state.vehicle.gain;
        state.scenario_runs.push((scenario.id.clone(), seed, gain));
        let applied_scenario_digest = if state.bad_scenario_readback {
            Digest::from_bytes([98; 32])
        } else {
            scenario.digest
        };
        let receipt_intent = if state.transition.bad_start_intent {
            Digest::from_bytes([96; 32])
        } else {
            run_intent_digest
        };
        Ok(ScenarioStartReceipt {
            session_digest: capability.session_digest(),
            applied_scenario_digest,
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
        Ok(SampleEvent::Sample(TelemetrySample {
            sequence,
            elapsed_ms: 10,
            values: BTreeMap::from([("gain".to_owned(), state.vehicle.gain)]),
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
