//! The two ArduCopter parameters direct flight depends on, read from the
//! FC and kept current.
//!
//! - `GUID_OPTIONS` bit 3 makes SET_ATTITUDE_TARGET thrust mean thrust;
//!   without it ArduCopter reads the field as a climb rate. The adapter
//!   sets the bit when it is clear and changes no other bit.
//! - `MOT_THST_HOVER` is the FC's hover thrust, the midpoint of the
//!   canonical thrust scale. ArduCopter learns it in flight, so it is
//!   read again periodically.
//!
//! Until both are known and the bit is set, the direct scope refuses
//! frames: a thrust scale without its hover point would step the motors.

use std::time::{Duration, Instant};

use pilotage_mavlink::codec::ParamValue;
use pilotage_mavlink::{LinkState, OutboundParam, ParamRequest};
use tokio::sync::mpsc;
use tracing::{info, warn};

const GUID_OPTIONS: &str = "GUID_OPTIONS";
const MOT_THST_HOVER: &str = "MOT_THST_HOVER";
/// GUID_OPTIONS bit 3: SetAttitudeTarget interprets Thrust As Thrust.
const THRUST_AS_THRUST: u32 = 1 << 3;
/// How often the parameters are read again.
const REFRESH: Duration = Duration::from_secs(5);
/// A hover thrust outside this range is not believed.
const PLAUSIBLE_HOVER: std::ops::RangeInclusive<f32> = 0.1..=0.8;

/// The parameter state direct flight needs.
#[derive(Debug)]
pub(super) struct DirectParams {
    lane: mpsc::Sender<OutboundParam>,
    system_id: u8,
    component_id: u8,
    last_request: Option<Instant>,
    set_requested: bool,
    hover: Option<f32>,
    thrust_as_thrust: bool,
}

/// An ArduPilot parameter's number. ArduPilot sends integers as the
/// number in the float field, typed INT32.
fn number(value: ParamValue) -> f32 {
    match value {
        ParamValue::Real32(value) => value,
        ParamValue::Int32(bits) => f32::from_bits(bits as u32),
    }
}

impl DirectParams {
    pub(super) fn new(lane: mpsc::Sender<OutboundParam>, system_id: u8, component_id: u8) -> Self {
        Self {
            lane,
            system_id,
            component_id,
            last_request: None,
            set_requested: false,
            hover: None,
            thrust_as_thrust: false,
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
            self.thrust_as_thrust = options & THRUST_AS_THRUST != 0;
            if !self.thrust_as_thrust && !self.set_requested {
                info!(
                    options,
                    "setting GUID_OPTIONS bit 3 so attitude-target thrust is thrust"
                );
                self.send(ParamRequest::Set(
                    GUID_OPTIONS,
                    ParamValue::Real32((options | THRUST_AS_THRUST) as f32),
                ));
                self.set_requested = true;
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

    /// The hover thrust, once the FC is ready for direct flight.
    pub(super) fn ready_hover(&self) -> Option<f32> {
        self.hover.filter(|_| self.thrust_as_thrust)
    }
}

#[cfg(test)]
mod tests;
