#![allow(clippy::expect_used, clippy::panic)]

use pilotage_mission_core::{MissionCapability, MissionDirective, ReceiptResult};
use pilotage_trial::{
    ActuatorCondition, CONDITION_SET_SCHEMA_VERSION, CommandLossPolicy,
    ControllerInitializationCondition, HorizontalWind, HoverThrustForceInitialization,
    PlantCondition, SensorAxis, SensorCondition, SensorNoiseLane, TimingCondition, TurbulenceModel,
    WindCondition,
};

use super::*;
use crate::{
    ArtifactIdentity, Digest, RunExecutionContext, ScenarioFrame, ScenarioObservationReceipt,
    ScenarioStopContext,
};

struct DeclaringRuntime {
    identity: ArtifactIdentity,
    mission: Vec<MissionCapability>,
    uncertainty: Vec<BackendCapability>,
    hover_estimator_mode: HoverEstimatorMode,
}

impl DeclaringRuntime {
    fn new(uncertainty: Vec<BackendCapability>, hover_estimator_mode: HoverEstimatorMode) -> Self {
        Self {
            identity: ArtifactIdentity::new("reference-runtime", Digest::from_bytes([3; 32]))
                .expect("runtime identity"),
            mission: vec![MissionCapability::SimulatorTime],
            uncertainty,
            hover_estimator_mode,
        }
    }

    fn silent() -> Self {
        Self {
            identity: ArtifactIdentity::new("silent-runtime", Digest::from_bytes([4; 32]))
                .expect("runtime identity"),
            mission: vec![MissionCapability::SimulatorTime],
            uncertainty: Vec::new(),
            hover_estimator_mode: HoverEstimatorMode::Online,
        }
    }
}

impl ScenarioRuntime for DeclaringRuntime {
    fn identity(&self) -> &ArtifactIdentity {
        &self.identity
    }

    fn capabilities(&self) -> &[MissionCapability] {
        &self.mission
    }

    fn uncertainty_capabilities(&self) -> &[BackendCapability] {
        &self.uncertainty
    }

    fn hover_estimator_mode(&self) -> HoverEstimatorMode {
        self.hover_estimator_mode
    }

    fn prepare_blocking(
        &mut self,
        _document: &pilotage_mission_core::MissionDocument,
        _context: &RunExecutionContext,
    ) -> Result<(), ScenarioRuntimeError> {
        Ok(())
    }

    fn start_blocking(&mut self) -> Result<(), ScenarioRuntimeError> {
        Ok(())
    }

    fn observe_blocking(
        &mut self,
        frame: &ScenarioFrame,
        directive: Option<&MissionDirective>,
    ) -> Result<ScenarioObservationReceipt, ScenarioRuntimeError> {
        Ok(ScenarioObservationReceipt {
            source_sequence: frame.source_sequence,
            action_result: directive.map(|_| ReceiptResult::Succeeded {}),
        })
    }

    fn stop_blocking(
        &mut self,
        _context: &mut ScenarioStopContext,
    ) -> Result<(), ScenarioRuntimeError> {
        Ok(())
    }

    fn cleanup_blocking(&mut self) -> Result<(), ScenarioRuntimeError> {
        Ok(())
    }
}

fn nominal_condition() -> ConditionSet {
    ConditionSet {
        schema_version: CONDITION_SET_SCHEMA_VERSION,
        id: "calm".to_owned(),
        revision: 1,
        seed: 5,
        wind: WindCondition {
            steady: HorizontalWind {
                speed_mps: 0.0,
                direction_deg: 0.0,
            },
            gusts: Vec::new(),
            turbulence: TurbulenceModel::None,
        },
        timing: TimingCondition::nominal(),
        sensor: SensorCondition::nominal(),
        actuator: ActuatorCondition::nominal(),
        controller_initialization: ControllerInitializationCondition::nominal(),
        plant: PlantCondition::nominal(),
    }
}

fn sensor_condition() -> ConditionSet {
    let mut value = nominal_condition();
    value.id = "sensor-noise".to_owned();
    value.sensor = SensorCondition::BoundedNoise {
        lanes: vec![SensorNoiseLane::Gyroscope {
            axis: SensorAxis::X,
            peak_amplitude_rad_s: 0.01,
            update_interval_samples: 5,
        }],
    };
    value
}

fn hover_condition() -> ConditionSet {
    let mut value = nominal_condition();
    value.id = "hover-trim".to_owned();
    value.controller_initialization.hover_thrust_force =
        HoverThrustForceInitialization::ScaleBaseline {
            scale_basis_points: 9_000,
        };
    value
}

fn admission(capabilities: Vec<BackendCapability>, mode: HoverEstimatorMode) -> ConditionAdmission {
    ConditionAdmission::new(UncertaintyDeclaration::new(capabilities, mode))
}

#[test]
fn a_runtime_that_declares_nothing_still_runs_a_nominal_condition() {
    let runtime = DeclaringRuntime::silent();
    let admission = ConditionAdmission::new(UncertaintyDeclaration::from_runtime(&runtime));

    admission
        .prepare(&nominal_condition())
        .expect("a nominal condition keeps the current behavior");
    admission
        .admit_live(&nominal_condition(), &runtime)
        .expect("a nominal condition arms");
}

#[test]
fn preparation_refuses_a_non_nominal_request_before_discovery() {
    let admission = ConditionAdmission::new(UncertaintyDeclaration::from_runtime(
        &DeclaringRuntime::silent(),
    ));

    assert!(matches!(
        admission.prepare(&sensor_condition()),
        Err(ScenarioRuntimeError::UnsupportedCondition { .. })
    ));
    assert!(matches!(
        admission.prepare(&hover_condition()),
        Err(ScenarioRuntimeError::UnsupportedCondition { .. })
    ));
}

#[test]
fn a_prepared_frozen_mode_refuses_a_live_online_runtime() {
    let admission = admission(
        vec![BackendCapability::HoverTrimUncertainty],
        HoverEstimatorMode::Frozen,
    );
    let live = DeclaringRuntime::new(
        vec![BackendCapability::HoverTrimUncertainty],
        HoverEstimatorMode::Online,
    );

    admission
        .prepare(&hover_condition())
        .expect("preparation admits the known declaration");
    assert!(matches!(
        admission.admit_live(&hover_condition(), &live),
        Err(ScenarioRuntimeError::ChangedHoverEstimatorMode {
            prepared: "frozen",
            live: "online",
        })
    ));
}

#[test]
fn a_capability_set_that_changes_after_preparation_fails_closed() {
    let admission = admission(
        vec![BackendCapability::SensorPerturbation],
        HoverEstimatorMode::Online,
    );
    let live = DeclaringRuntime::new(Vec::new(), HoverEstimatorMode::Online);

    admission
        .prepare(&sensor_condition())
        .expect("preparation admits the known declaration");
    assert!(matches!(
        admission.admit_live(&sensor_condition(), &live),
        Err(ScenarioRuntimeError::ChangedUncertaintyCapabilities { .. })
    ));
}

#[test]
fn an_agreeing_live_runtime_arms_the_requested_condition() {
    let admission = admission(
        vec![BackendCapability::HoverTrimUncertainty],
        HoverEstimatorMode::Disabled,
    );
    let live = DeclaringRuntime::new(
        vec![BackendCapability::HoverTrimUncertainty],
        HoverEstimatorMode::Disabled,
    );

    admission.prepare(&hover_condition()).expect("preparation");
    admission
        .admit_live(&hover_condition(), &live)
        .expect("the live declaration agrees");
    assert_eq!(
        admission.declared().hover_estimator_mode(),
        HoverEstimatorMode::Disabled
    );
    assert_eq!(
        admission.declared().capabilities(),
        [BackendCapability::HoverTrimUncertainty]
    );
}

#[test]
fn a_command_hold_request_needs_its_own_capability() {
    let mut value = nominal_condition();
    value.id = "command-loss".to_owned();
    value.actuator.command_loss = CommandLossPolicy::SeededZeroOrderHold {
        fraction_basis_points: 100,
        decision_interval_samples: 100,
    };
    let authority_only = admission(
        vec![BackendCapability::ActuatorAuthority],
        HoverEstimatorMode::Online,
    );

    assert!(matches!(
        authority_only.prepare(&value),
        Err(ScenarioRuntimeError::UnsupportedCondition { .. })
    ));
    admission(
        vec![BackendCapability::CommandHold],
        HoverEstimatorMode::Online,
    )
    .prepare(&value)
    .expect("the exact capability admits the request");
}
