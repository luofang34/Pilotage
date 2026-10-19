//! The vehicles the px4-jsbsim backend launches
//! (`--fc px4-jsbsim:<model>`).
//!
//! Each names a sidecar aircraft and the PX4 airframe PX4 ships for a
//! JSBSim vehicle of that kind. PX4 sources stay untouched: where the
//! sidecar's vehicle differs from the airframe's, a `PX4_PARAM_` override
//! says so at start-up.

use crate::backend::px4_gz::vehicle::Airframe;
use crate::error::XtaskError;
use crate::site::Ground;

/// A vehicle model px4-jsbsim can launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum JsbsimModel {
    /// The x500-class quadcopter (airframe 3010, x500 rotor geometry).
    #[default]
    Quad,
    /// The Rascal 110 fixed-wing (airframe 1033, which PX4 tunes for it).
    Plane,
}

/// The x500's rotor layout and hover thrust, over airframe 3010's own
/// (which describes PX4's larger `quadrotor_x` JSBSim model): the Gazebo
/// x500 airframe's rotor positions and the sidecar motor model's hover
/// command.
const X500_PARAMS: [(&str, &str); 9] = [
    ("CA_ROTOR0_PX", "0.174"),
    ("CA_ROTOR0_PY", "0.174"),
    ("CA_ROTOR1_PX", "-0.174"),
    ("CA_ROTOR1_PY", "-0.174"),
    ("CA_ROTOR2_PX", "0.174"),
    ("CA_ROTOR2_PY", "-0.174"),
    ("CA_ROTOR3_PX", "-0.174"),
    ("CA_ROTOR3_PY", "0.174"),
    ("MPC_THR_HOVER", "0.71"),
];

impl JsbsimModel {
    /// Every model, in the order help and errors list them.
    pub(crate) const ALL: [JsbsimModel; 2] = [JsbsimModel::Quad, JsbsimModel::Plane];

    /// The name after `px4-jsbsim:`.
    pub(crate) fn name(self) -> &'static str {
        match self {
            JsbsimModel::Quad => "quad",
            JsbsimModel::Plane => "plane",
        }
    }

    /// Resolves a model name, fail-closed.
    ///
    /// # Errors
    ///
    /// Returns [`XtaskError::UnknownModel`] for a name no model has.
    pub(crate) fn parse(name: &str) -> Result<Self, XtaskError> {
        Self::ALL
            .into_iter()
            .find(|model| model.name() == name)
            .ok_or_else(|| XtaskError::UnknownModel {
                backend: "px4-jsbsim",
                model: name.to_owned(),
                known: Self::ALL.iter().map(|model| model.name()).collect(),
            })
    }

    /// The sidecar's `--aircraft`.
    pub(crate) fn sidecar_aircraft(self) -> &'static str {
        match self {
            JsbsimModel::Quad => "x500",
            JsbsimModel::Plane => "rascal",
        }
    }

    /// PX4's autostart id for the airframe (`PX4_SYS_AUTOSTART`).
    pub(crate) fn sys_autostart(self) -> &'static str {
        match self {
            JsbsimModel::Quad => "3010",
            JsbsimModel::Plane => "1033",
        }
    }

    /// The parameters the airframe gets on top of its defaults.
    pub(crate) fn params(self) -> &'static [(&'static str, &'static str)] {
        match self {
            JsbsimModel::Quad => &X500_PARAMS,
            JsbsimModel::Plane => &[],
        }
    }

    /// How the vehicle flies.
    pub(crate) fn airframe(self) -> Airframe {
        match self {
            JsbsimModel::Quad => Airframe::Multicopter,
            JsbsimModel::Plane => Airframe::FixedWing,
        }
    }

    /// Where on a site it starts.
    pub(crate) fn ground(self) -> Ground {
        match self.airframe() {
            Airframe::FixedWing => Ground::Runway,
            Airframe::Multicopter => Ground::Field,
        }
    }
}
