//! The canonical signals the Aviate vehicle port adds to a neutral frame.
//!
//! The simulator projection carries truth. The vehicle port carries what
//! only the vehicle knows: the normalized value it commanded, the exact
//! setpoint field it transmitted, and the link and estimator states. Every
//! signal is stated once, so a frame cannot carry two values for one
//! selector.

use std::collections::BTreeMap;

use flight_tune::{
    CanonicalTelemetryKey, ControlChannel, ControlValueField, ObservedSignal, ReferenceFrame,
    ScenarioFrame, SignalSelector,
};

use super::AviateRuntimeError;
use super::math::require_finite;
use super::quality::boolean;

/// What the vehicle port commanded and observed on one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VehicleSignals {
    /// The normalized value the active stimulus commanded, when one is active.
    pub normalized_command: Option<f64>,
    /// The control channel the active stimulus commands, when one is active.
    pub channel: Option<ControlChannel>,
    /// The exact attitude setpoint the direct path transmitted, in radians.
    pub transmitted_attitude_rad: Option<f64>,
    /// Whether the commanded value reached its declared envelope endpoint.
    pub saturated: bool,
    /// Whether the vehicle command link is valid.
    pub link_valid: bool,
    /// Whether the vehicle estimator is valid.
    pub estimator_valid: bool,
}

impl VehicleSignals {
    /// The canonical signals this frame adds to the neutral projection.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when a commanded value is not finite.
    pub fn observed(&self) -> Result<Vec<ObservedSignal>, AviateRuntimeError> {
        let mut signals = Vec::with_capacity(2);
        if let (Some(value), Some(channel)) = (self.normalized_command, self.channel) {
            signals.push(ObservedSignal {
                selector: SignalSelector::NormalizedControl { channel },
                value: require_finite("normalized command", value)?,
            });
        }
        if let Some(value) = self.transmitted_attitude_rad {
            signals.push(ObservedSignal {
                selector: SignalSelector::TransmittedSetpoint {
                    field: ControlValueField::AttitudeThrust {
                        expected_frame: ReferenceFrame::BodyFrd,
                    },
                },
                value: require_finite("transmitted setpoint", value)?,
            });
        }
        Ok(signals)
    }

    /// The canonical telemetry values the vehicle port is answerable for.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when a commanded value is not finite.
    pub fn canonical_values(
        &self,
        recovered: bool,
    ) -> Result<BTreeMap<String, f64>, AviateRuntimeError> {
        let command = self.normalized_command.unwrap_or(0.0);
        let effort = require_finite("actuator effort", command)?;
        Ok(BTreeMap::from([
            (
                CanonicalTelemetryKey::ActuatorEffort.as_str().to_owned(),
                effort,
            ),
            (
                CanonicalTelemetryKey::ActuatorSaturated.as_str().to_owned(),
                boolean(self.saturated),
            ),
            (
                CanonicalTelemetryKey::CommandLinkValid.as_str().to_owned(),
                boolean(self.link_valid),
            ),
            (
                CanonicalTelemetryKey::CommandPrimary.as_str().to_owned(),
                effort,
            ),
            (
                CanonicalTelemetryKey::EstimatorValid.as_str().to_owned(),
                boolean(self.estimator_valid),
            ),
            (
                CanonicalTelemetryKey::Recovered.as_str().to_owned(),
                boolean(recovered),
            ),
        ]))
    }
}

/// Reads the link and estimator states one frame reports.
///
/// A frame that states neither is not a frame the vehicle port can act on:
/// arming, stimulating, and releasing all depend on knowing them.
///
/// # Errors
///
/// Returns [`AviateRuntimeError`] when the frame omits a required state.
pub fn require_vehicle_states(frame: &ScenarioFrame) -> Result<(bool, bool), AviateRuntimeError> {
    let link_valid = frame
        .link_valid
        .ok_or(AviateRuntimeError::IncompleteFrame {
            field: "control-link validity",
        })?;
    let estimator_valid = frame
        .estimator_valid
        .ok_or(AviateRuntimeError::IncompleteFrame {
            field: "estimator validity",
        })?;
    Ok((link_valid, estimator_valid))
}
