//! The flight uplink policy without input or output.
//!
//! [`UplinkPolicy`] turns operator stick frames, direct attitude frames,
//! arm and disarm requests, and link-loss holds into the MAVLink frames
//! that the flight controller receives. It shapes demands with the
//! control-feel profile, keeps the post-arm quiet interval, captures and
//! repeats holds, and numbers the frames. It holds no socket and reads no
//! clock: the caller gives the monotonic time of each call and sends the
//! frames that the call returns. A UDP shell and a simulator that writes
//! the bytes into a flight controller's receive ring use the same policy.

mod policy;

pub use policy::{
    AIRSPEED_REPEAT, ActivationStep, AirspeedAnswer, AttitudeOutcome, DirectFrame, DirectRefusal,
    HeadingAltitudeDemand, HeadingState, MAX_DT_S, StickFrame, StickOutcome, UplinkPolicy,
};
