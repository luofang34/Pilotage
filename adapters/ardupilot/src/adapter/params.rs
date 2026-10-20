//! The two ArduCopter parameters direct flight depends on, read from the
//! FC and kept current.
//!
//! - `GUID_OPTIONS` bit 3 makes SET_ATTITUDE_TARGET thrust mean thrust;
//!   without it ArduCopter reads the field as a climb rate. Under
//!   [`ArduPilotProfile::Simulation`] the adapter sets the bit when it is
//!   clear and changes no other bit. A physical vehicle's parameters are
//!   never written: this module is the only place a write can come from,
//!   and only with the [`SimulationWrites`] proof the simulation profile
//!   alone yields. On a physical vehicle with the bit clear, direct flight
//!   is refused with a [`DirectRefusal`] naming the parameter.
//! - `MOT_THST_HOVER` is the FC's hover thrust, the midpoint of the
//!   canonical thrust scale. ArduCopter learns it in flight, so it is
//!   read again periodically.
//!
//! Until both are known and the bit is set, the direct scope refuses
//! frames: a thrust scale without its hover point would step the motors.
//! Assisted (velocity) flight depends on neither, so arming does not wait
//! for them.

use std::time::{Duration, Instant};

use pilotage_mavlink::codec::{ParamEncoding, ParamValue};
use pilotage_mavlink::{LinkState, OutboundParam, ParamRequest};
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::config::ArduPilotProfile;

const GUID_OPTIONS: &str = "GUID_OPTIONS";
const MOT_THST_HOVER: &str = "MOT_THST_HOVER";
/// GUID_OPTIONS bit 3: SetAttitudeTarget interprets Thrust As Thrust.
const THRUST_AS_THRUST: u32 = 1 << 3;
/// How often the parameters are read again.
const REFRESH: Duration = Duration::from_secs(5);
/// A hover thrust outside this range is not believed.
const PLAUSIBLE_HOVER: std::ops::RangeInclusive<f32> = 0.1..=0.8;

/// The proof that parameter writes are permitted: constructible only from
/// [`ArduPilotProfile::Simulation`], inside this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SimulationWrites(());

impl SimulationWrites {
    fn for_profile(profile: ArduPilotProfile) -> Option<Self> {
        match profile {
            ArduPilotProfile::Simulation => Some(Self(())),
            ArduPilotProfile::Physical => None,
        }
    }
}

/// Why the direct scope refuses frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum DirectRefusal {
    /// The FC has not reported both parameters yet, or has not confirmed
    /// the simulation's write.
    #[error("ArduCopter hover thrust or GUID_OPTIONS not confirmed yet")]
    Unconfirmed,
    /// A physical vehicle with GUID_OPTIONS bit 3 clear.
    #[error(
        "ArduCopter parameter GUID_OPTIONS is {options}; direct flight needs bit 3 set ({expected}) so attitude-target thrust is thrust (set it on the vehicle: the adapter never writes a physical vehicle's parameters)"
    )]
    ThrustAsThrustClear {
        /// The stored GUID_OPTIONS.
        options: u32,
        /// The same options with bit 3 set.
        expected: u32,
    },
}

/// The parameter state direct flight needs.
#[derive(Debug)]
pub(super) struct DirectParams {
    writes: Option<SimulationWrites>,
    lane: mpsc::Sender<OutboundParam>,
    system_id: u8,
    component_id: u8,
    last_request: Option<Instant>,
    set_requested: bool,
    hover: Option<f32>,
    options: Option<u32>,
}

/// ArduPilot stores an integer parameter's value, not its bytes, in the
/// float field.
const ENCODING: ParamEncoding = ParamEncoding::CCast;

pub(super) fn number(value: ParamValue) -> f32 {
    ENCODING.number(value) as f32
}

impl DirectParams {
    pub(super) fn new(
        profile: ArduPilotProfile,
        lane: mpsc::Sender<OutboundParam>,
        system_id: u8,
        component_id: u8,
    ) -> Self {
        Self {
            writes: SimulationWrites::for_profile(profile),
            lane,
            system_id,
            component_id,
            last_request: None,
            set_requested: false,
            hover: None,
            options: None,
        }
    }

    fn send(&self, request: ParamRequest) {
        let outbound = OutboundParam {
            request,
            target_system: self.system_id,
            target_component: self.component_id,
        };
        if self.lane.try_send(outbound).is_err() {
            warn!(?request, "ArduPilot parameter lane refused a request");
        }
    }

    /// Folds the latest reported values and keeps the reads going.
    pub(super) fn maintain(&mut self, latest: &LinkState, now: Instant) {
        if let Some(report) = latest.fc_reports.param(MOT_THST_HOVER) {
            let hover = number(report.value);
            let believed = PLAUSIBLE_HOVER.contains(&hover).then_some(hover);
            if believed != self.hover {
                info!(hover, "ArduCopter hover thrust");
                self.hover = believed;
            }
        }
        if let Some(report) = latest.fc_reports.param(GUID_OPTIONS) {
            let options = number(report.value) as u32;
            self.options = Some(options);
            if options & THRUST_AS_THRUST == 0 && !self.set_requested {
                self.set_thrust_as_thrust(options);
            }
        }
        if self
            .last_request
            .is_none_or(|at| now.duration_since(at) >= REFRESH)
        {
            self.last_request = Some(now);
            self.send(ParamRequest::Read(MOT_THST_HOVER));
            self.send(ParamRequest::Read(GUID_OPTIONS));
        }
    }

    /// Sets bit 3 under simulation; on a physical vehicle only says why
    /// direct flight stays refused.
    fn set_thrust_as_thrust(&mut self, options: u32) {
        self.set_requested = true;
        let Some(SimulationWrites(())) = self.writes else {
            warn!(
                options,
                "GUID_OPTIONS bit 3 is clear on a physical vehicle; direct flight is refused until it is set on the vehicle"
            );
            return;
        };
        info!(
            options,
            "setting GUID_OPTIONS bit 3 so attitude-target thrust is thrust"
        );
        self.send(ParamRequest::Set(
            GUID_OPTIONS,
            ENCODING.integer((options | THRUST_AS_THRUST) as i32),
        ));
    }

    /// The hover thrust, once the FC is ready for direct flight, or why
    /// it is not.
    pub(super) fn ready_hover(&self) -> Result<f32, DirectRefusal> {
        match (self.options, self.hover) {
            (Some(options), _) if options & THRUST_AS_THRUST == 0 && self.writes.is_none() => {
                Err(DirectRefusal::ThrustAsThrustClear {
                    options,
                    expected: options | THRUST_AS_THRUST,
                })
            }
            (Some(options), Some(hover)) if options & THRUST_AS_THRUST != 0 => Ok(hover),
            _ => Err(DirectRefusal::Unconfirmed),
        }
    }
}

#[cfg(test)]
mod tests;
