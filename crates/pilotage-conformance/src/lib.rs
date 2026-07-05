//! Shared fixtures and behavioral test suites that every adapter and
//! platform port must pass (ADR-0002).
//!
//! The centerpiece is [`ScriptedSession`]: a sans-IO harness that drives an
//! [`AuthorityEngine`] and a [`ReferenceAdapter`] through a fixed
//! [`Script`] of authority commands, control frames, and adapter steps,
//! recording every observable outcome as an ordered [`SessionEvent`] log
//! (ADR-0010, ADR-0012). The increment-0 acceptance fixture —
//! [`increment_zero_script`] — exercises registration, grant, stepped
//! control, staleness rejection, normal handover, emergency override, and
//! link-loss release end to end.
//!
//! Because the authority engine and reference adapter are both sans-IO
//! (ADR-0002), replaying the same script from the same seed reproduces the
//! event log and the adapter trajectory exactly; the conformance tests
//! assert that determinism, that authority effects and control frames
//! round-trip through the `pilotage-protocol` wire types, and that a
//! mid-session snapshot/restore converges with the uninterrupted run.
//!
//! [`AuthorityEngine`]: pilotage_authority::AuthorityEngine
//! [`ReferenceAdapter`]: pilotage_adapter_reference::ReferenceAdapter

mod checkpoint;
mod event;
mod fixture;
mod roundtrip;
mod session;
mod staleness;

pub use checkpoint::{TrajectoryCheckpoint, increment_zero_checkpoints};
pub use event::{FrameOutcome, SessionEvent};
pub use fixture::{
    CAMERA_SCOPE, MOTION_SCOPE, OPERATOR_A, OPERATOR_B, OPERATOR_C, Principals, SEED, VEHICLE,
    increment_zero_script,
};
pub use roundtrip::{RoundTripError, authority_event_roundtrips, control_frame_roundtrips};
pub use session::{Script, ScriptStep, ScriptedSession};
pub use staleness::aged_frame_is_stale;
