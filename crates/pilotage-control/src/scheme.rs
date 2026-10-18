//! Which flight schemes a vehicle offers the operator, and how to move
//! between them.
//!
//! The scheme is the operator's choice, bounded by what the host
//! advertises: an assisted (velocity) scheme needs `Velocity` on the
//! velocity scope, angle flight needs `AttitudeThrust` on the direct scope,
//! and acro flight needs `BodyRate` on the direct scope. The vehicle class
//! chooses which schemes fit the airframe and which one comes first. The
//! two scopes form one motion authority group
//! ([`pilotage_protocol::MOTION_SCOPE`] and
//! [`pilotage_protocol::DIRECT_SCOPE`]), so a switch between them is a
//! lease transfer and a switch inside one scope is only a stick remap.

use pilotage_client_session::VehicleCatalog;
use pilotage_protocol::{VehicleClass, wire};

use crate::sample::Mode;

/// One scheme the vehicle offers: the stick mapping, the motion-group
/// member it flies on, and the intent family it sends there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemeChoice {
    /// The stick mapping.
    pub mode: Mode,
    /// The motion-group member scope.
    pub scope: &'static str,
    /// The intent family the scheme sends on that scope.
    pub family: wire::IntentFamily,
}

/// Every scheme a vehicle offers, in the order a picker shows them, and the
/// one a client starts on.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SchemeOffer {
    /// The offered schemes.
    pub choices: Vec<SchemeChoice>,
    /// The scheme to start on; `None` when the vehicle offers no flyable
    /// scheme at all.
    pub default: Option<Mode>,
}

impl SchemeOffer {
    /// The offered choice for `mode`, or `None` when the vehicle does not
    /// offer it.
    #[must_use]
    pub fn choice(&self, mode: Mode) -> Option<&SchemeChoice> {
        self.choices.iter().find(|choice| choice.mode == mode)
    }
}

/// What a client does to move from one offered scheme to another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemeSwitch {
    /// Both schemes fly on the same scope: change the stick mapping and
    /// keep the lease.
    Remap,
    /// The target flies on the other motion-group member: open the neutral
    /// handover ([`crate::ControlRuntime::reactivate`]), release the held
    /// member, and lease `scope`.
    Transfer {
        /// The member scope to lease.
        scope: &'static str,
    },
}

/// Why a scheme switch was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SchemeRefusal {
    /// The vehicle does not advertise what the target scheme needs.
    #[error("the vehicle does not offer the {0:?} scheme")]
    NotOffered(Mode),
}

/// The schemes `vehicle` offers, from its class and the intent families
/// its motion-group scopes advertise.
///
/// A fixed wing offers the plane scheme and a ground or surface vehicle
/// the rover scheme, each on `Velocity`. A multicopter, a VTOL, or a
/// vehicle of no declared class offers both assisted quad schemes on
/// `Velocity`, angle flight on `AttitudeThrust`, and acro on `BodyRate`.
/// The default is the class default ([`Mode::for_vehicle_class`]) when it
/// is offered, and otherwise the first offered scheme.
#[must_use]
pub fn offered_schemes(vehicle: &VehicleCatalog) -> SchemeOffer {
    let advertises = |scope: &str, family: wire::IntentFamily| {
        vehicle
            .scopes
            .iter()
            .filter(|catalog| catalog.scope == scope)
            .flat_map(|catalog| &catalog.intents)
            .any(|intent| intent.family == family as i32)
    };
    let candidates: &[(Mode, wire::IntentFamily)] = match vehicle.vehicle_class {
        Some(VehicleClass::FixedWing) => &[(Mode::Plane, wire::IntentFamily::Velocity)],
        Some(VehicleClass::GroundRover | VehicleClass::SurfaceVessel) => {
            &[(Mode::Rover, wire::IntentFamily::Velocity)]
        }
        Some(VehicleClass::Multicopter | VehicleClass::Vtol) | None => &[
            (Mode::QuadPilot, wire::IntentFamily::Velocity),
            (Mode::QuadCruise, wire::IntentFamily::Velocity),
            (Mode::Fpv, wire::IntentFamily::AttitudeThrust),
            (Mode::Acro, wire::IntentFamily::BodyRate),
        ],
    };
    let choices: Vec<SchemeChoice> = candidates
        .iter()
        .filter(|(mode, family)| advertises(mode.scope(), *family))
        .map(|&(mode, family)| SchemeChoice {
            mode,
            scope: mode.scope(),
            family,
        })
        .collect();
    let class_default = Mode::for_vehicle_class(vehicle.vehicle_class);
    let default = choices
        .iter()
        .find(|choice| choice.mode == class_default)
        .or_else(|| choices.first())
        .map(|choice| choice.mode);
    SchemeOffer { choices, default }
}

/// How to move from the scheme flown now to `to`.
///
/// # Errors
///
/// Returns [`SchemeRefusal::NotOffered`] when the vehicle does not offer
/// `to`: a client never switches into a scheme the host cannot fly, in
/// flight or on the ground.
pub fn plan_scheme_switch(
    offer: &SchemeOffer,
    from: Mode,
    to: Mode,
) -> Result<SchemeSwitch, SchemeRefusal> {
    let target = offer.choice(to).ok_or(SchemeRefusal::NotOffered(to))?;
    if target.scope == from.scope() {
        Ok(SchemeSwitch::Remap)
    } else {
        Ok(SchemeSwitch::Transfer {
            scope: target.scope,
        })
    }
}

#[cfg(test)]
mod tests;
