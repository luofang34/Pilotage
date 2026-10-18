//! The PX4 parameters the fixed-wing uplink pins at connect, and the
//! confirmation that each write took effect.
//!
//! PX4 echoes every PARAM_SET with a PARAM_VALUE holding the value it now
//! stores; a parameter counts as pinned only when that echo matches. The
//! uplink refuses to arm until every one is pinned, because the stick
//! mapping and the link-loss behavior both depend on them.

use std::time::{Duration, Instant};

use pilotage_mavlink::codec::{PARAM_ID_LEN, ParamValue, param_id};

use super::sticks::{
    AIRSPEED_MAX_MPS, AIRSPEED_MIN_MPS, AIRSPEED_TRIM_MPS, CLIMB_RATE_MPS, ROLL_LIMIT_DEG,
    SINK_RATE_MPS,
};

/// How long an unconfirmed write waits before it is sent again.
const RETRY_PERIOD: Duration = Duration::from_secs(1);

/// The takeoff altitude PX4's Takeoff mode climbs to, m above the
/// takeoff point. It is above [`super::HANDOVER_HEIGHT_M`], so the
/// handover to Position mode happens during the climb, not after a loiter.
pub(crate) const TAKEOFF_ALTITUDE_M: f32 = 30.0;

/// Every pinned parameter and why:
///
/// - `COM_RC_IN_MODE = 1`: manual control from MAVLink only. SITL has no
///   RC receiver, and this makes MANUAL_CONTROL the source whose loss is
///   the link-loss signal.
/// - `NAV_RCL_ACT = 1`: on manual-control loss, Hold (loiter at the
///   present position). This is the fixed-wing form of the neutral hold.
/// - `COM_RCL_EXCEPT = 2`: manual-control loss does not interrupt the auto
///   modes (Takeoff, Hold, Return), so a link lost during the takeoff
///   roll completes the takeoff and loiters instead of entering Hold on
///   the runway.
/// - `FW_POS_STK_CONF = 2`: throttle stick sets airspeed, pitch stick
///   sets height rate; [`super::sticks`] depends on this layout.
/// - the airspeeds, `FW_R_LIM`, and the climb and sink rates: the envelope
///   the capability advertisement states.
/// - `MIS_TAKEOFF_ALT`: the altitude Takeoff mode climbs to.
pub(crate) const PINNED: [(&str, ParamValue); 11] = [
    ("COM_RC_IN_MODE", ParamValue::Int32(1)),
    ("NAV_RCL_ACT", ParamValue::Int32(1)),
    ("COM_RCL_EXCEPT", ParamValue::Int32(2)),
    ("FW_POS_STK_CONF", ParamValue::Int32(2)),
    ("FW_AIRSPD_MIN", ParamValue::Real32(AIRSPEED_MIN_MPS)),
    ("FW_AIRSPD_TRIM", ParamValue::Real32(AIRSPEED_TRIM_MPS)),
    ("FW_AIRSPD_MAX", ParamValue::Real32(AIRSPEED_MAX_MPS)),
    ("FW_R_LIM", ParamValue::Real32(ROLL_LIMIT_DEG)),
    ("FW_T_CLMB_R_SP", ParamValue::Real32(CLIMB_RATE_MPS)),
    ("FW_T_SINK_R_SP", ParamValue::Real32(SINK_RATE_MPS)),
    ("MIS_TAKEOFF_ALT", ParamValue::Real32(TAKEOFF_ALTITUDE_M)),
];

/// Which pinned parameters the FC has confirmed.
#[derive(Debug)]
pub(crate) struct ParamPinning {
    confirmed: [bool; PINNED.len()],
    last_sent: Option<Instant>,
}

impl ParamPinning {
    pub(crate) fn new() -> Self {
        Self {
            confirmed: [false; PINNED.len()],
            last_sent: None,
        }
    }

    /// Whether every pinned parameter is confirmed.
    pub(crate) fn all_confirmed(&self) -> bool {
        self.confirmed.iter().all(|confirmed| *confirmed)
    }

    /// The writes due now: every unconfirmed parameter, at most once per
    /// [`RETRY_PERIOD`].
    pub(crate) fn due(&mut self, now: Instant) -> Vec<(&'static str, ParamValue)> {
        if self.all_confirmed()
            || self
                .last_sent
                .is_some_and(|at| now.duration_since(at) < RETRY_PERIOD)
        {
            return Vec::new();
        }
        self.last_sent = Some(now);
        PINNED
            .iter()
            .zip(self.confirmed.iter())
            .filter(|(_, confirmed)| !**confirmed)
            .map(|(pinned, _)| *pinned)
            .collect()
    }

    /// Folds one PARAM_VALUE in. A matching value confirms the parameter;
    /// a different value (another writer, or a refused write) withdraws
    /// the confirmation so the write is sent again.
    pub(crate) fn observe(&mut self, id: [u8; PARAM_ID_LEN], value: ParamValue) {
        for ((name, pinned), confirmed) in PINNED.iter().zip(self.confirmed.iter_mut()) {
            if param_id(name) != id {
                continue;
            }
            let matches = same_value(*pinned, value);
            if matches != *confirmed {
                tracing::info!(param = name, ?value, pinned = matches, "PX4 parameter echo");
            }
            *confirmed = matches;
        }
    }
}

/// Exact comparison: the pinned values are sent verbatim, so the echo
/// returns the same bits.
fn same_value(pinned: ParamValue, echoed: ParamValue) -> bool {
    match (pinned, echoed) {
        (ParamValue::Int32(a), ParamValue::Int32(b)) => a == b,
        (ParamValue::Real32(a), ParamValue::Real32(b)) => a.to_bits() == b.to_bits(),
        _ => false,
    }
}
