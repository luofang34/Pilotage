//! The motion authority group's scope identities, shared by every adapter
//! and every client so both sides name the same scope.
//!
//! The two scopes drive one actuator: a host leases, fences, and latches
//! them as one authority, so a client holds at most one of them.

/// The assisted member: `Velocity` intents in Body-FRD. A vehicle holds
/// its position when the demand is zero.
pub const MOTION_SCOPE: &str = "vehicle.motion";
/// The direct member: `AttitudeThrust` (angle flight) and/or `BodyRate`
/// (acro flight) intents with normalized thrust.
pub const DIRECT_SCOPE: &str = "vehicle.motion.direct";
