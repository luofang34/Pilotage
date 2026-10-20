//! The PX4 parameters the fixed-wing uplink depends on. Under
//! [`crate::config::Px4Profile::Simulation`] it writes them at connect;
//! on a physical vehicle it only reads them ([`crate::pinning`] is the one
//! place either happens). The uplink refuses to arm until every one holds
//! the value below, because the stick mapping and the link-loss behavior
//! both depend on them.

use pilotage_mavlink::codec::ParamValue;

use super::sticks::{
    AIRSPEED_MAX_MPS, AIRSPEED_MIN_MPS, AIRSPEED_TRIM_MPS, CLIMB_RATE_MPS, ROLL_LIMIT_DEG,
    SINK_RATE_MPS,
};

/// The takeoff altitude PX4's Takeoff mode climbs to, m above the
/// takeoff point. It is above [`super::HANDOVER_HEIGHT_M`], so the
/// handover to Position mode happens during the climb, not after a loiter.
pub(crate) const TAKEOFF_ALTITUDE_M: f32 = 30.0;

/// Every required parameter and why:
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
