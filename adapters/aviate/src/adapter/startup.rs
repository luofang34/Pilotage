//! Adapter construction: the control-feel checks every adapter runs, and
//! (with `net`) the process start.

use pilotage_control_feel::{FeelMode, FlightFeelProfile, ValidatedFlightFeelProfile};

use crate::error::AviateAdapterError;

#[cfg(feature = "net")]
mod process;

pub(crate) fn validate_adapter_control_feel(
    control_feel: &ValidatedFlightFeelProfile,
) -> Result<(), AviateAdapterError> {
    validate_aviate_profile_bindings(control_feel)?;
    validate_legacy_compatibility_response(control_feel)?;
    if control_feel.profile().hold.require_accel {
        return Err(AviateAdapterError::UnsupportedControlFeel {
            detail: "hold.require_accel needs an acceleration source with provenance".to_owned(),
        });
    }
    Ok(())
}

fn validate_legacy_compatibility_response(
    control_feel: &ValidatedFlightFeelProfile,
) -> Result<(), AviateAdapterError> {
    let profile = control_feel.profile();
    if profile.mode != FeelMode::LegacyCompatibility {
        return Ok(());
    }
    let mut required = FlightFeelProfile::legacy_compatibility();
    required.profile_id.clone_from(&profile.profile_id);
    if profile != &required {
        return Err(AviateAdapterError::UnsupportedControlFeel {
            detail: "legacy-compatibility mode requires the fixed Aviate response fields"
                .to_owned(),
        });
    }
    Ok(())
}

pub(crate) fn validate_aviate_profile_bindings(
    control_feel: &ValidatedFlightFeelProfile,
) -> Result<(), AviateAdapterError> {
    let trusted = FlightFeelProfile::legacy_compatibility();
    // The hover thrust is the one envelope field an airframe owns: a shipped
    // law fitted to a known airframe hovers at that airframe's trim.
    let envelope = control_feel.profile().envelope;
    let known_hover = pilotage_control_feel::AirframeLimits::ALL
        .iter()
        .any(|airframe| airframe.hover_thrust.to_bits() == envelope.direct_hover_thrust.to_bits());
    let mut fitted = trusted.envelope;
    fitted.direct_hover_thrust = envelope.direct_hover_thrust;
    if !known_hover || envelope != fitted {
        return Err(AviateAdapterError::UnsupportedControlFeel {
            detail: "the demand envelope does not match the required Alia envelope".to_owned(),
        });
    }
    if control_feel.profile().bindings != trusted.bindings {
        return Err(AviateAdapterError::UnsupportedControlFeel {
            detail: "the device or flight-controller identity does not match Alia".to_owned(),
        });
    }
    Ok(())
}
