//! The increment-0 acceptance fixture: a fully scripted session covering the
//! ADR-0010 authority state machine end to end against the reference adapter.
//!
//! Increment 0's acceptance is "client core and test host exchange fixture
//! sessions": this module encodes one such session as a [`Script`]. It
//! registers `vehicle.motion` and `vehicle.camera` on one vehicle, grants
//! motion to A and camera to B, drives motion frames through authority into
//! the adapter with stepped execution, rejects a stale-generation frame after
//! a transition, performs a normal A->B handover (offer/accept/confirm) after
//! which A's old-generation frames are fenced out, seizes motion by emergency
//! override from C, then loses C's link (authority release) and neutralizes
//! the adapter, whose speed decays under drag.

use core::time::Duration;

use pilotage_adapter_api::LinkLossPolicy;
use pilotage_authority::{AuthorityClass, AuthorityCommand, LinkState, OverrideReason};
use pilotage_protocol::{
    ControlPayload, Generation, LogicalAxisId, ScopeId, ScopedControlFrame, SequenceNum, SessionId,
    VehicleId,
};
use pilotage_timing::MonoTimestamp;

use crate::session::{Script, ScriptStep};

/// Deterministic seed for the increment-0 session's adapter initial state.
pub const SEED: u64 = 0x00C0_FFEE;

/// The single vehicle the increment-0 session drives.
pub const VEHICLE: VehicleId = VehicleId::new(1);

/// The session id every control frame in the fixture carries.
const SESSION: SessionId = SessionId::new(7);

/// Operator A, initial motion holder and offerer in the normal handover.
pub const OPERATOR_A: pilotage_protocol::PrincipalId = pilotage_protocol::PrincipalId::new(10);
/// Operator B, camera holder and recipient of the motion handover.
pub const OPERATOR_B: pilotage_protocol::PrincipalId = pilotage_protocol::PrincipalId::new(20);
/// Operator C, the emergency-override actor.
pub const OPERATOR_C: pilotage_protocol::PrincipalId = pilotage_protocol::PrincipalId::new(30);

/// The motion control scope registered on the vehicle.
pub const MOTION_SCOPE: &str = "vehicle.motion";
/// The camera control scope registered on the vehicle.
pub const CAMERA_SCOPE: &str = "vehicle.camera";

/// Logical throttle axis id the reference adapter's motion scope accepts.
const THROTTLE_AXIS: u16 = 2;
/// Logical steering axis id the reference adapter's motion scope accepts.
const STEERING_AXIS: u16 = 3;

/// The principals participating in the increment-0 session, for callers that
/// want to assert against holders without re-deriving the ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Principals {
    /// Operator A.
    pub a: pilotage_protocol::PrincipalId,
    /// Operator B.
    pub b: pilotage_protocol::PrincipalId,
    /// Operator C.
    pub c: pilotage_protocol::PrincipalId,
}

impl Principals {
    /// The fixture's fixed principal assignment.
    #[must_use]
    pub const fn increment_zero() -> Self {
        Self {
            a: OPERATOR_A,
            b: OPERATOR_B,
            c: OPERATOR_C,
        }
    }
}

fn motion() -> ScopeId {
    ScopeId::new(MOTION_SCOPE)
}

fn camera() -> ScopeId {
    ScopeId::new(CAMERA_SCOPE)
}

/// Builds a motion control frame at `generation`/`sequence` driving the two
/// motion axes. `sampled_at` is derived from the sequence so frames carry
/// distinct, monotonically increasing timestamps.
fn motion_frame(
    generation: u64,
    sequence: u32,
    throttle: f32,
    steering: f32,
) -> ScopedControlFrame {
    ScopedControlFrame {
        session: SESSION,
        vehicle: VEHICLE,
        scope: motion(),
        generation: Generation::new(generation),
        sequence: SequenceNum::new(sequence),
        sampled_at: MonoTimestamp::from_nanos(u64::from(sequence) * 1_000),
        profile_revision: 1,
        payload: ControlPayload {
            axes: vec![
                (LogicalAxisId::new(THROTTLE_AXIS), throttle),
                (LogicalAxisId::new(STEERING_AXIS), steering),
            ],
            edges: vec![],
        },
    }
}

/// The increment-0 acceptance script (see the module documentation for the
/// full narrative).
///
/// Generation bookkeeping for `vehicle.motion` across the script: `Grant`
/// advances 0->1, the handover `Accept` advances 1->2, the emergency
/// `EmergencyOverride` advances 2->3, and the link-loss release advances
/// 3->4. Frames are built at the generation live when they are routed, and
/// the two deliberately stale frames (A at gen 1 after the handover, C at
/// gen 2 after the override) are fenced out.
#[must_use]
pub fn increment_zero_script() -> Script {
    let ttl = Duration::from_secs(10);
    let steps = vec![
        // Registration of both scopes on the one vehicle.
        ScriptStep::Command(AuthorityCommand::RegisterScope {
            vehicle: VEHICLE,
            scope: motion(),
        }),
        ScriptStep::Command(AuthorityCommand::RegisterScope {
            vehicle: VEHICLE,
            scope: camera(),
        }),
        // Grant motion to A (gen -> 1) and camera to B (independent scope).
        ScriptStep::Command(AuthorityCommand::Grant {
            vehicle: VEHICLE,
            scope: motion(),
            to: OPERATOR_A,
        }),
        ScriptStep::Command(AuthorityCommand::Grant {
            vehicle: VEHICLE,
            scope: camera(),
            to: OPERATOR_B,
        }),
        // A drives motion at gen 1: forward and to port, then step 10 ticks.
        ScriptStep::Frame(motion_frame(1, 1, 1.0, 0.5)),
        ScriptStep::Step(10),
        // Normal handover A -> B: offer, accept (atomic commit, gen -> 2),
        // then the three-call confirmations.
        ScriptStep::Command(AuthorityCommand::Offer {
            vehicle: VEHICLE,
            scope: motion(),
            from: OPERATOR_A,
            to: OPERATOR_B,
            ttl,
        }),
        ScriptStep::Command(AuthorityCommand::Accept {
            vehicle: VEHICLE,
            scope: motion(),
            by: OPERATOR_B,
            expected_generation: Generation::new(1),
        }),
        ScriptStep::Command(AuthorityCommand::ConfirmIHave {
            vehicle: VEHICLE,
            scope: motion(),
            by: OPERATOR_B,
        }),
        ScriptStep::Command(AuthorityCommand::ConfirmYouHave {
            vehicle: VEHICLE,
            scope: motion(),
            by: OPERATOR_A,
        }),
        // A's old-generation frame after the transition is fenced out.
        ScriptStep::Frame(motion_frame(1, 2, 1.0, 0.0)),
        // B drives motion at gen 2: forward and to starboard, then step.
        ScriptStep::Frame(motion_frame(2, 3, 0.8, -0.5)),
        ScriptStep::Step(10),
        // Emergency override by C (gen -> 3).
        ScriptStep::Command(AuthorityCommand::EmergencyOverride {
            vehicle: VEHICLE,
            scope: motion(),
            by: OPERATOR_C,
            authority_class: AuthorityClass::Supervisor,
            reason: OverrideReason::new("range safety takeover"),
        }),
        // A frame at the pre-override generation is fenced out.
        ScriptStep::Frame(motion_frame(2, 4, 1.0, 0.0)),
        // C drives motion at gen 3: full throttle straight, then step.
        ScriptStep::Frame(motion_frame(3, 5, 1.0, 0.0)),
        ScriptStep::Step(10),
        // Link loss on the effective holder C: authority releases the scope
        // (gen -> 4) and the adapter neutralizes, decaying speed under drag.
        ScriptStep::Command(AuthorityCommand::HolderLinkChanged {
            vehicle: VEHICLE,
            scope: motion(),
            principal: OPERATOR_C,
            state: LinkState::Lost,
        }),
        ScriptStep::LinkLossPolicy {
            vehicle: VEHICLE,
            policy: Some(LinkLossPolicy::Neutralize),
        },
        ScriptStep::Step(10),
    ];
    Script {
        vehicle: VEHICLE,
        seed: SEED,
        steps,
    }
}
