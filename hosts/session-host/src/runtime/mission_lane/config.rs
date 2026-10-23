//! The lane's configuration, read once from the environment at startup.
//!
//! - `PILOTAGE_LANE_NAME`: the lane's name in its status; default the
//!   adapter's name.
//! - `PILOTAGE_LANE_SIMULATED`: `1` or `0`, overriding whether the vehicle
//!   is a simulation; the default follows the adapter.
//! - `PILOTAGE_MISSION_OWNSHIP`, `PILOTAGE_MISSION_TARGET`: as for an
//!   environment mission; they also apply to plans the lane compiles.
//! - `PILOTAGE_LANE_ENERGY_WH`, `PILOTAGE_LANE_CRUISE_POWER_W`: the energy
//!   model admission checks a reserve against; absent, it checks none.
//! - `PILOTAGE_DECISION`: the judgment service, a provider specification
//!   such as `http://127.0.0.1:8761/v1/systemone` (see
//!   [`pilotage_decision::Provider::from_spec`]); absent, pictures are
//!   counted and not judged.
//! - `PILOTAGE_DECISION_EVERY_S`: seconds between two judgments; default 6.

use std::time::Duration;

use pilotage_decision::Provider;
use pilotage_mission::{ExecutionTarget, LaneConfig, OwnshipSource, SurveyDefaults};

use crate::error::HostError;
use crate::runtime::automation::MissionPlan;
use crate::runtime::options::{env_var, parse_execution_target, parse_ownship_source};

/// Default seconds between two judgments of one lane.
const DEFAULT_ASSESS_EVERY_S: f64 = 6.0;

/// The lane and its services.
#[derive(Debug, Clone)]
pub(crate) struct LaneRuntimeConfig {
    /// The sans-IO lane's configuration.
    pub lane: LaneConfig,
    /// The judgment service, when one is configured.
    pub decision: Option<Provider>,
    /// Least time between two judgments.
    pub assess_every: Duration,
    /// The camera height the judge is told, metres.
    pub judge_height_m: f64,
    /// Whether unstamped planar poses are simulator state.
    pub planar_pose_is_truth: bool,
}

impl LaneRuntimeConfig {
    /// Reads the configuration for a host whose adapter is `adapter`.
    ///
    /// # Errors
    /// [`HostError`] for a variable with a value the host does not accept.
    pub(crate) fn from_env(
        adapter: &str,
        simulated: bool,
        planar_pose_is_truth: bool,
    ) -> Result<Self, HostError> {
        let simulated = match env_var("PILOTAGE_LANE_SIMULATED")?.as_deref() {
            None => simulated,
            Some("1") => true,
            Some("0") => false,
            Some(value) => return Err(invalid("PILOTAGE_LANE_SIMULATED", value)),
        };
        let ownship = env_var("PILOTAGE_MISSION_OWNSHIP")?
            .map(|value| parse_ownship_source(&value))
            .transpose()?
            .unwrap_or_else(OwnshipSource::vehicle_estimate);
        let default_target = if simulated {
            ExecutionTarget::Simulator
        } else {
            ExecutionTarget::RealVehicle
        };
        let target = env_var("PILOTAGE_MISSION_TARGET")?
            .map(|value| parse_execution_target(&value))
            .transpose()?
            .unwrap_or(default_target);
        let positive = |name: &'static str| -> Result<Option<f64>, HostError> {
            env_var(name)?
                .map(|value| {
                    value
                        .parse::<f64>()
                        .ok()
                        .filter(|v| v.is_finite() && *v > 0.0)
                        .ok_or_else(|| invalid(name, &value))
                })
                .transpose()
        };
        let decision = env_var("PILOTAGE_DECISION")?
            .map(|spec| {
                Provider::from_spec(&spec)
                    .map_err(|error| invalid("PILOTAGE_DECISION", &error.to_string()))
            })
            .transpose()?;
        let defaults = SurveyDefaults::default();
        Ok(Self {
            lane: LaneConfig {
                name: env_var("PILOTAGE_LANE_NAME")?.unwrap_or_else(|| adapter.to_owned()),
                simulated,
                execution_strategy: "host-guided-velocity".to_owned(),
                template: MissionPlan::lane_template(ownship, target),
                defaults,
                usable_energy_wh: positive("PILOTAGE_LANE_ENERGY_WH")?,
                cruise_power_w: positive("PILOTAGE_LANE_CRUISE_POWER_W")?,
                recovery_policy: "hold in place; the operator lands".to_owned(),
            },
            decision,
            assess_every: Duration::from_secs_f64(
                positive("PILOTAGE_DECISION_EVERY_S")?.unwrap_or(DEFAULT_ASSESS_EVERY_S),
            ),
            judge_height_m: defaults.height_m,
            planar_pose_is_truth,
        })
    }
}

fn invalid(variable: &'static str, value: &str) -> HostError {
    HostError::LaneConfig {
        variable,
        value: value.to_owned(),
    }
}
