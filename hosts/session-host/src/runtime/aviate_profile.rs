//! Session-profile selection for the Aviate adapter (LINK-04):
//! fail-closed environment parsing and per-profile link configuration.

use std::env::VarError;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use pilotage_adapter_aviate::{ALIA250_DEFAULT_CONTROL_FEEL_JSON, AviateProfile, LinkConfig};
use pilotage_control_feel::ValidatedFlightFeelProfile;

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

/// Loads the selected Aviate control-feel artifact.
///
/// # Errors
///
/// This function returns a typed error if a physical session selects an
/// explicit path.
/// It returns a typed error if the host cannot read the path.
/// It returns a typed error if the artifact is invalid.
pub(crate) fn control_feel_from_env_blocking(
    profile: AviateProfile,
    value: Option<OsString>,
) -> Result<ValidatedFlightFeelProfile, HostError> {
    match value.map(PathBuf::from) {
        Some(path) if profile == AviateProfile::Physical => {
            Err(HostError::AviatePhysicalControlFeelOverride { path })
        }
        Some(path) => control_feel_from_path_blocking(&path),
        None => ValidatedFlightFeelProfile::from_json_str(ALIA250_DEFAULT_CONTROL_FEEL_JSON)
            .map_err(|source| HostError::AviateDefaultControlFeelInvalid { source }),
    }
}

fn control_feel_from_path_blocking(path: &Path) -> Result<ValidatedFlightFeelProfile, HostError> {
    let text =
        std::fs::read_to_string(path).map_err(|source| HostError::AviateControlFeelRead {
            path: path.to_path_buf(),
            source,
        })?;
    ValidatedFlightFeelProfile::from_json_str(&text).map_err(|source| {
        HostError::AviateControlFeelInvalid {
            path: path.to_path_buf(),
            source,
        }
    })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use std::env::VarError;
    use std::ffi::OsString;

    use pilotage_adapter_aviate::{AviateProfile, ResetPolicy};

    use super::{control_feel_from_env_blocking, link_config, profile_from_env};
    use crate::error::HostError;

    /// Every shipped shaped profile is one this host will actually launch with.
    ///
    /// The launcher names a file in the environment and the vehicle comes up
    /// flying whatever it holds. A file the host refuses is a launch that dies
    /// at startup; a file it accepts but reads as a different mode is worse,
    /// because the vehicle flies a law nobody chose and nothing says so.
    #[test]
    fn every_shipped_shaped_profile_loads_as_the_mode_it_names() {
        let profiles =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../adapters/aviate/profiles");
        for (file, mode) in [
            (
                "alia250-shaped-precision-v1.json",
                pilotage_control_feel::FeelMode::Precision,
            ),
            (
                "alia250-shaped-balanced-v1.json",
                pilotage_control_feel::FeelMode::Balanced,
            ),
            (
                "alia250-shaped-agile-v1.json",
                pilotage_control_feel::FeelMode::Agile,
            ),
            // The X500's, which the launcher names by path for a gazebo
            // session. This host is what reads that path, so a file it
            // cannot load is a session that does not start.
            (
                "x500-shaped-precision-v1.json",
                pilotage_control_feel::FeelMode::Precision,
            ),
            (
                "x500-shaped-balanced-v1.json",
                pilotage_control_feel::FeelMode::Balanced,
            ),
            (
                "x500-shaped-agile-v1.json",
                pilotage_control_feel::FeelMode::Agile,
            ),
        ] {
            let path = profiles.join(file);
            let loaded = control_feel_from_env_blocking(
                AviateProfile::Simulation,
                Some(OsString::from(path.as_os_str())),
            )
            .unwrap_or_else(|error| panic!("{file} must load: {error}"));
            assert_eq!(loaded.profile().mode, mode, "{file} loaded as another mode");
            // And it is a shaped law rather than the one that steps.
            assert!(
                loaded.profile().horizontal.neutral.dwell_ms > 0,
                "{file} has no dwell"
            );
            assert!(
                loaded.profile().horizontal.dynamics.release_accel < 1_000.0,
                "{file} still steps on release"
            );
        }
    }

    /// A physical vehicle refuses a named profile outright.
    #[test]
    fn a_physical_vehicle_refuses_a_named_profile() {
        let path = OsString::from("/nonexistent/profile.json");
        let refused = control_feel_from_env_blocking(AviateProfile::Physical, Some(path))
            .expect_err("refused");
        assert!(matches!(
            refused,
            HostError::AviatePhysicalControlFeelOverride { .. }
        ));
    }

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

    #[test]
    fn absent_override_selects_the_checked_default_artifact() {
        let loaded = control_feel_from_env_blocking(AviateProfile::Simulation, None)
            .expect("checked default");
        assert_eq!(
            loaded.profile(),
            &pilotage_control_feel::FlightFeelProfile::legacy_compatibility()
        );
    }

    #[test]
    fn physical_refuses_an_explicit_control_feel_path_before_io() {
        let path = std::path::PathBuf::from("unqualified-control-feel.json");
        let result = control_feel_from_env_blocking(
            AviateProfile::Physical,
            Some(path.clone().into_os_string()),
        );
        assert!(matches!(
            result,
            Err(HostError::AviatePhysicalControlFeelOverride { path: refused })
                if refused == path
        ));
    }

    #[test]
    fn explicit_control_feel_artifact_is_loaded_and_validated() {
        let path = std::env::temp_dir().join(format!(
            "pilotage-feel-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        let profile = pilotage_control_feel::FlightFeelProfile::legacy_compatibility();
        let text = serde_json::to_string(&profile).expect("profile JSON");
        std::fs::write(&path, text).expect("write profile");

        let loaded = control_feel_from_env_blocking(
            AviateProfile::Simulation,
            Some(path.clone().into_os_string()),
        )
        .expect("load profile");

        assert_eq!(loaded.profile(), &profile);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn an_unknown_control_feel_field_fails_closed() {
        let path = std::env::temp_dir().join(format!(
            "pilotage-invalid-feel-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        let profile = pilotage_control_feel::FlightFeelProfile::legacy_compatibility();
        let mut value = serde_json::to_value(profile).expect("profile value");
        value
            .as_object_mut()
            .expect("profile object")
            .insert("unknown".to_owned(), serde_json::Value::Bool(true));
        std::fs::write(&path, serde_json::to_vec(&value).expect("profile JSON"))
            .expect("write profile");

        let result = control_feel_from_env_blocking(
            AviateProfile::Simulation,
            Some(path.clone().into_os_string()),
        );

        assert!(matches!(
            result,
            Err(HostError::AviateControlFeelInvalid { .. })
        ));
        std::fs::remove_file(path).ok();
    }
}
