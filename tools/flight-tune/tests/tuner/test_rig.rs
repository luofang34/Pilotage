use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use flight_tune::{
    AdapterError, ArtifactIdentity, Candidate, CandidateReceipt, Digest, SampleEvent, ScenarioRef,
    ScenarioStartReceipt, SessionChallenge, SimulatorBackend, SimulatorCapability,
    SimulatorSessionReceipt, SimulatorVehicleAdapter, SimulatorVehicleFactory, TelemetrySample,
    VehicleBinding, VehicleBindingReceipt,
};

#[path = "test_rig/scoring.rs"]
mod scoring;

#[allow(unused_imports)]
pub use scoring::{
    EnvelopeGates, ObservedViews, QuadraticMetric, SequenceStrategy, assert_receipt_error,
    candidate, stage,
};

#[derive(Debug, Default)]
pub struct FakeState {
    pub gain: f64,
    pub active_candidate_digest: Option<Digest>,
    pub open_session_count: usize,
    pub prepare_count: usize,
    pub bind_count: usize,
    pub ensure_count: usize,
    pub apply_count: usize,
    pub start_count: usize,
    pub sample_count: usize,
    pub sample_poll_count: usize,
    pub stop_count: usize,
    pub cleanup_count: usize,
    pub metric_observe_count: usize,
    pub gate_begin_count: usize,
    pub gate_evaluate_count: usize,
    pub gate_finish_count: usize,
    pub gate_cancel_count: usize,
    pub metric_begin_count: usize,
    pub metric_finish_count: usize,
    pub metric_cancel_count: usize,
    pub scenario_runs: Vec<(String, u64, f64)>,
    pub lifecycle: Vec<String>,
    pub current_scenario: Option<ScenarioRef>,
    pub current_seed: u64,
    pub next_sequence: u64,
    pub panic_on_prepare: Option<usize>,
    pub panic_on_start: Option<usize>,
    pub panic_on_cleanup: Option<usize>,
    pub change_head_on_prepare: Option<PathBuf>,
    pub bad_candidate_readback_on_ensure: Option<usize>,
    pub bad_candidate_readback_on_apply: Option<usize>,
    pub bad_scenario_readback: bool,
    pub timeout_next_sample: bool,
    pub complete_without_sample: bool,
}

#[derive(Clone)]
pub struct FakeHandle(pub Rc<RefCell<FakeState>>);

impl FakeHandle {
    pub fn new() -> Self {
        Self(Rc::new(RefCell::new(FakeState::default())))
    }
}

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
        _capability: &SimulatorCapability,
        scenario: &ScenarioRef,
        seed: u64,
    ) -> Result<(), AdapterError> {
        let mut state = self.state.0.borrow_mut();
        state.prepare_count = state.prepare_count.wrapping_add(1);
        state.lifecycle.push("prepare".to_owned());
        if state.panic_on_prepare == Some(state.prepare_count) {
            panic!("simulated process stop after AttemptPrepared");
        }
        let head_change = state.change_head_on_prepare.take();
        state.current_scenario = Some(scenario.clone());
        state.current_seed = seed;
        state.next_sequence = 0;
        drop(state);
        if let Some(root) = head_change {
            change_head_digest(&root);
        }
        Ok(())
    }

    fn start_blocking(
        &mut self,
        capability: &SimulatorCapability,
    ) -> Result<ScenarioStartReceipt, AdapterError> {
        let mut state = self.state.0.borrow_mut();
        state.start_count = state.start_count.wrapping_add(1);
        state.lifecycle.push("start".to_owned());
        if state.panic_on_start == Some(state.start_count) {
            panic!("simulated process stop after candidate activation");
        }
        let scenario = state
            .current_scenario
            .clone()
            .ok_or_else(|| AdapterError::new("scenario was not prepared"))?;
        let seed = state.current_seed;
        let gain = state.gain;
        state.scenario_runs.push((scenario.id.clone(), seed, gain));
        let applied_scenario_digest = if state.bad_scenario_readback {
            Digest::from_bytes([98; 32])
        } else {
            scenario.digest
        };
        Ok(ScenarioStartReceipt {
            session_digest: capability.session_digest(),
            applied_scenario_digest,
            seed,
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
            values: BTreeMap::from([("gain".to_owned(), state.gain)]),
        }))
    }

    fn stop_blocking(&mut self) -> Result<(), AdapterError> {
        let mut state = self.state.0.borrow_mut();
        state.stop_count = state.stop_count.wrapping_add(1);
        state.lifecycle.push("stop".to_owned());
        Ok(())
    }

    fn cleanup_blocking(&mut self) -> Result<(), AdapterError> {
        let mut state = self.state.0.borrow_mut();
        state.cleanup_count = state.cleanup_count.wrapping_add(1);
        state.lifecycle.push("cleanup".to_owned());
        if state.panic_on_cleanup == Some(state.cleanup_count) {
            panic!("simulated process stop after outcome publication");
        }
        Ok(())
    }
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

pub struct FakeVehicle {
    state: FakeHandle,
}

impl SimulatorVehicleAdapter for FakeVehicle {
    fn ensure_candidate_blocking(
        &mut self,
        capability: &SimulatorCapability,
        candidate: &Candidate,
        candidate_digest: Digest,
    ) -> Result<CandidateReceipt, AdapterError> {
        let gain = candidate
            .parameters()
            .get("gain")
            .copied()
            .ok_or_else(|| AdapterError::new("candidate has no gain"))?;
        let mut state = self.state.0.borrow_mut();
        state.ensure_count = state.ensure_count.wrapping_add(1);
        let wrote_candidate = state.active_candidate_digest != Some(candidate_digest);
        if wrote_candidate {
            state.gain = gain;
            state.active_candidate_digest = Some(candidate_digest);
            state.apply_count = state.apply_count.wrapping_add(1);
            state.lifecycle.push("apply".to_owned());
        }
        let bad_readback = state.bad_candidate_readback_on_ensure == Some(state.ensure_count)
            || (wrote_candidate
                && state.bad_candidate_readback_on_apply == Some(state.apply_count));
        let readback = if bad_readback {
            Digest::from_bytes([99; 32])
        } else {
            candidate_digest
        };
        Ok(CandidateReceipt {
            session_digest: capability.session_digest(),
            requested_digest: candidate_digest,
            applied_digest: candidate_digest,
            readback_digest: readback,
        })
    }
}

pub struct FakeFactory {
    state: FakeHandle,
    identity: ArtifactIdentity,
    allow_binding: bool,
}

impl FakeFactory {
    pub fn new(state: FakeHandle) -> Self {
        Self {
            state,
            identity: identity("vehicle", "fake-controller-v1"),
            allow_binding: true,
        }
    }

    pub fn hardware_like(state: FakeHandle) -> Self {
        Self {
            state,
            identity: identity("vehicle", "hardware-like-controller"),
            allow_binding: false,
        }
    }
}

impl SimulatorVehicleFactory for FakeFactory {
    type Adapter = FakeVehicle;

    fn vehicle_identity(&self) -> &ArtifactIdentity {
        &self.identity
    }

    fn bind_blocking(
        self,
        capability: &SimulatorCapability,
    ) -> Result<VehicleBinding<Self::Adapter>, AdapterError> {
        let mut state = self.state.0.borrow_mut();
        state.bind_count = state.bind_count.wrapping_add(1);
        drop(state);
        if !self.allow_binding {
            return Err(AdapterError::new(
                "hardware-like adapter has no simulator session binding",
            ));
        }
        capability.bind_vehicle(
            FakeVehicle { state: self.state },
            VehicleBindingReceipt {
                session_digest: capability.session_digest(),
                vehicle_digest: self.identity.digest,
            },
        )
    }
}

fn identity(id: &str, content: &str) -> ArtifactIdentity {
    ArtifactIdentity::from_text(id, content).expect("artifact identity")
}

pub struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    pub fn new(label: &str) -> Self {
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = test_root().join(format!("flight-tune-{label}-{}-{time}", std::process::id()));
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn test_root() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        PathBuf::from("/private/tmp")
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::env::temp_dir()
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).ok();
    }
}
