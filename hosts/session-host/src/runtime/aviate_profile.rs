//! Session-profile selection for the Aviate adapter (LINK-04):
//! fail-closed environment parsing and per-profile link configuration.

use std::env::VarError;

use pilotage_adapter_aviate::{AviateProfile, LinkConfig};

use crate::error::HostError;

/// Parses `PILOTAGE_AVIATE_PROFILE` fail-closed. Absence selects the
/// default `Simulation` profile; every PRESENT value must name a known
/// profile exactly (`physical`, `simulation`, `oracle-only`) or startup
/// fails with a typed error. A typo in a physical deployment must never
/// fail open into the simulation profile.
pub(crate) fn profile_from_env(
    value: Result<String, VarError>,
) -> Result<AviateProfile, HostError> {
    match value {
        Err(VarError::NotPresent) => Ok(AviateProfile::Simulation),
        Ok(text) => match text.as_str() {
            "physical" => Ok(AviateProfile::Physical),
            "simulation" => Ok(AviateProfile::Simulation),
            "oracle-only" => Ok(AviateProfile::OracleOnly),
            _ => Err(HostError::AviateProfile { value: text }),
        },
        Err(VarError::NotUnicode(raw)) => Err(HostError::AviateProfile {
            value: raw.to_string_lossy().into_owned(),
        }),
    }
}

/// The link configuration a profile runs. Simulation-family profiles
/// enable the bounded simulator reset heuristic; `Physical` stays
/// conservative — a boot-clock regression in replayable telemetry never
/// infers a reboot.
pub(crate) fn link_config(profile: AviateProfile) -> LinkConfig {
    match profile {
        AviateProfile::Physical => LinkConfig::physical(),
        AviateProfile::Simulation | AviateProfile::OracleOnly => LinkConfig::simulator(),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use std::env::VarError;
    use std::ffi::OsString;

    use pilotage_adapter_aviate::{AviateProfile, ResetPolicy};

    use super::{link_config, profile_from_env};
    use crate::error::HostError;

    #[test]
    fn absent_variable_selects_the_default_simulation_profile() {
        let profile = profile_from_env(Err(VarError::NotPresent)).expect("default");
        assert_eq!(profile, AviateProfile::Simulation);
    }

    #[test]
    fn every_known_profile_value_parses_exactly() {
        for (value, expected) in [
            ("physical", AviateProfile::Physical),
            ("simulation", AviateProfile::Simulation),
            ("oracle-only", AviateProfile::OracleOnly),
        ] {
            let profile = profile_from_env(Ok(value.to_owned())).expect(value);
            assert_eq!(profile, expected);
        }
    }

    #[test]
    fn unknown_values_fail_startup_instead_of_failing_open() {
        // A physical-deployment typo must never become Simulation.
        for value in ["phyiscal", "Physical", "sim", "oracle_only", ""] {
            let refusal = profile_from_env(Ok(value.to_owned()));
            assert!(
                matches!(refusal, Err(HostError::AviateProfile { value: ref v }) if v == value),
                "{value:?} must be refused, got {refusal:?}"
            );
        }
    }

    #[test]
    fn non_unicode_values_fail_startup_with_a_typed_error() {
        #[cfg(unix)]
        let raw = {
            use std::os::unix::ffi::OsStringExt;
            OsString::from_vec(vec![0x66, 0xFF, 0x6F])
        };
        #[cfg(not(unix))]
        let raw = OsString::from("\u{FFFD}garbage");
        let refusal = profile_from_env(Err(VarError::NotUnicode(raw)));
        assert!(matches!(refusal, Err(HostError::AviateProfile { .. })));
    }

    #[test]
    fn physical_runs_the_conservative_reset_policy() {
        assert_eq!(
            link_config(AviateProfile::Physical).reset_policy,
            ResetPolicy::Conservative
        );
        assert_eq!(
            link_config(AviateProfile::Simulation).reset_policy,
            ResetPolicy::SimulatorHeuristic
        );
    }
}
