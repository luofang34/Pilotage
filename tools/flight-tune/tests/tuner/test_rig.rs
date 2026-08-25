use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use flight_tune::{ArtifactIdentity, RunExecutionContext, ScenarioRef};

#[path = "test_rig/backend.rs"]
mod backend;
#[path = "test_rig/cleanup_fault.rs"]
mod cleanup_fault;
#[path = "test_rig/scoring.rs"]
mod scoring;
#[path = "test_rig/vehicle.rs"]
mod vehicle;
#[path = "test_rig/vehicle_state.rs"]
mod vehicle_state;

pub use backend::FakeBackend;
pub use cleanup_fault::FakeCleanupFault;
#[allow(unused_imports)]
pub use scoring::{
    EnvelopeGates, ObservedViews, QuadraticMetric, SequenceStrategy, assert_receipt_error,
    candidate, stage,
};
pub use vehicle::{FakeFactory, FakeVehicle};
pub use vehicle_state::FakeVehicleState;

#[derive(Debug, Default)]
pub struct FakeState {
    pub vehicle: FakeVehicleState,
    pub open_session_count: usize,
    pub prepare_count: usize,
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
    pub transition: FakeTransitionState,
    pub lifecycle: Vec<String>,
    pub current_scenario: Option<ScenarioRef>,
    pub current_seed: u64,
    pub next_sequence: u64,
    pub panic_on_prepare: Option<usize>,
    pub panic_on_start: Option<usize>,
    pub cleanup_fault: FakeCleanupFault,
    pub change_head_on_prepare: Option<PathBuf>,
    pub bad_scenario_readback: bool,
    pub timeout_next_sample: bool,
    pub complete_without_sample: bool,
}

#[derive(Debug, Default)]
pub struct FakeTransitionState {
    pub authorization_count: usize,
    pub checks: Vec<(f64, f64)>,
    pub maximum_delta: Option<f64>,
    pub prepared_contexts: Vec<RunExecutionContext>,
    pub started_contexts: Vec<RunExecutionContext>,
    pub vehicle_contexts: Vec<RunExecutionContext>,
    pub bad_preparation_intent: bool,
    pub bad_start_intent: bool,
    pub bad_vehicle_intent: bool,
}

#[derive(Clone)]
pub struct FakeHandle(pub Rc<RefCell<FakeState>>);

impl FakeHandle {
    pub fn new() -> Self {
        Self(Rc::new(RefCell::new(FakeState::default())))
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
