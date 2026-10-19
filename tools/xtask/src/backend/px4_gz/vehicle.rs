//! The vehicles the px4-gz backend launches (`--fc px4-gz:<model>`).
//!
//! Each model names one PX4 airframe and one Pilotage world that
//! includes the matching gz model under the name PX4 attaches to, with
//! the Pilotage camera rig welded to it. The airframe and the world
//! must agree: PX4's gz bridge reads and drives `/model/<name>/...`, so
//! a mismatch boots cleanly and then flies nothing.

use crate::error::XtaskError;

/// A vehicle model px4-gz can launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Px4Model {
    /// The x500 quadcopter with the CGO3 3-axis gimbal (airframe 4019).
    #[default]
    X500Gimbal,
    /// The RC Cessna fixed-wing (airframe 4003).
    RcCessna,
}

/// How the vehicle flies, told to the host so the adapter maps the
/// unified velocity intent onto the right PX4 setpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Airframe {
    /// Hovers and follows body velocity directly.
    Multicopter,
    /// Flies forward: velocity maps to airspeed, climb, and turn.
    FixedWing,
}

impl Airframe {
    /// The `PILOTAGE_PX4_AIRFRAME` value.
    pub(crate) fn as_env_value(self) -> &'static str {
        match self {
            Airframe::Multicopter => "multicopter",
            Airframe::FixedWing => "fixed_wing",
        }
    }
}

impl Px4Model {
    /// Every model, in the order help and errors list them.
    pub(crate) const ALL: [Px4Model; 2] = [Px4Model::X500Gimbal, Px4Model::RcCessna];

    /// The name after `px4-gz:`.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Px4Model::X500Gimbal => "x500_gimbal",
            Px4Model::RcCessna => "rc_cessna",
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
                backend: "px4-gz",
                model: name.to_owned(),
                known: Self::ALL.iter().map(|model| model.name()).collect(),
            })
    }

    /// PX4's autostart id for the airframe (`PX4_SYS_AUTOSTART`).
    pub(crate) fn sys_autostart(self) -> &'static str {
        match self {
            // MNT_MODE_IN/OUT = MAVLink Gimbal Protocol v2, which the PX4
            // adapter's `vehicle.gimbal` scope drives (GIM-04).
            Px4Model::X500Gimbal => "4019",
            Px4Model::RcCessna => "4003",
        }
    }

    /// PX4's model selector for the airframe (`PX4_SIM_MODEL`).
    pub(crate) fn sim_model(self) -> &'static str {
        match self {
            Px4Model::X500Gimbal => "gz_x500_gimbal",
            Px4Model::RcCessna => "gz_rc_cessna",
        }
    }

    /// The name the world gives the vehicle, which PX4 attaches to
    /// (`PX4_GZ_MODEL_NAME`).
    pub(crate) fn model_name(self) -> &'static str {
        match self {
            Px4Model::X500Gimbal => "x500_0",
            Px4Model::RcCessna => "rc_cessna_0",
        }
    }

    /// The Pilotage world that includes this vehicle, relative to the
    /// repository root.
    pub(crate) fn world(self) -> &'static str {
        match self {
            Px4Model::X500Gimbal => "sim/worlds/px4_flightdeck.sdf",
            Px4Model::RcCessna => "sim/worlds/px4_rc_cessna_flightdeck.sdf",
        }
    }

    /// Whether the vehicle carries a gimbal the host may advertise.
    pub(crate) fn has_gimbal(self) -> bool {
        matches!(self, Px4Model::X500Gimbal)
    }

    /// How the vehicle flies.
    pub(crate) fn airframe(self) -> Airframe {
        match self {
            Px4Model::X500Gimbal => Airframe::Multicopter,
            Px4Model::RcCessna => Airframe::FixedWing,
        }
    }
}
