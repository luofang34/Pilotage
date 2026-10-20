use crate::error::XtaskError;

/// Which session profile the host runs (mirrors the host's fail-closed
/// `PILOTAGE_AVIATE_PROFILE` vocabulary exactly).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// FC estimate + FC state; no truth source.
    Physical,
    /// Estimate + FC state, plus the truth oracle when attachable.
    Simulation,
    /// Truth stream only; no uplink, no operational control.
    OracleOnly,
}

impl Profile {
    /// The value passed through to `PILOTAGE_AVIATE_PROFILE`.
    pub fn as_env_value(self) -> &'static str {
        match self {
            Profile::Physical => "physical",
            Profile::Simulation => "simulation",
            Profile::OracleOnly => "oracle-only",
        }
    }

    pub(super) fn parse(value: &str) -> Result<Self, XtaskError> {
        match value {
            "physical" => Ok(Profile::Physical),
            "simulation" => Ok(Profile::Simulation),
            "oracle-only" => Ok(Profile::OracleOnly),
            _ => Err(XtaskError::UnknownProfile {
                value: value.to_owned(),
            }),
        }
    }
}
