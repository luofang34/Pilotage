//! Canonical control-frame vocabulary (ADR-0007, ADR-0009, ADR-0011).

use pilotage_timing::MonoTimestamp;

use crate::ids::{Generation, ScopeId, SequenceNum, SessionId, VehicleId};
use crate::intent::{ControlAction, ControlIntent};

/// Identifies a logical continuous axis (e.g. throttle, steering) in the
/// canonical input model, independent of any physical device layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LogicalAxisId(u16);

impl LogicalAxisId {
    /// Constructs a logical axis identifier from a raw value.
    #[must_use]
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    /// Returns the identifier as a raw `u16`.
    #[must_use]
    pub const fn as_u16(&self) -> u16 {
        self.0
    }
}

/// Identifies a logical button (e.g. horn, headlights) in the canonical
/// input model, independent of any physical device layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LogicalButtonId(u16);

impl LogicalButtonId {
    /// Constructs a logical button identifier from a raw value.
    #[must_use]
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    /// Returns the identifier as a raw `u16`.
    #[must_use]
    pub const fn as_u16(&self) -> u16 {
        self.0
    }
}

/// A button state transition delivered as an explicit edge event rather than
/// a sampled level, per ADR-0009's one-shot consumption semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ButtonEdge {
    /// The button transitioned from released to pressed.
    Pressed,
    /// The button transitioned from pressed to released.
    Released,
}

/// The logical input state carried by a single control frame.
///
/// Axis values follow the convention `[-1.0, 1.0]`, with `0.0` as the neutral
/// (centered/idle) position; asymmetric physical ranges are normalized into
/// this convention upstream in `pilotage-input`. Continuous axes use
/// latest-valid-value semantics; button edges are explicit one-shot events.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ControlPayload {
    /// Current value of each logical axis present in this frame, in
    /// `[-1.0, 1.0]`.
    pub axes: Vec<(LogicalAxisId, f32)>,
    /// Button edge events observed since the previous frame.
    pub edges: Vec<(LogicalButtonId, ButtonEdge)>,
}

/// A control frame scoped to a single session, vehicle, and control scope,
/// carrying the fencing metadata the session host uses to accept or reject
/// it (ADR-0006, ADR-0009).
#[derive(Debug, Clone, PartialEq)]
pub struct ScopedControlFrame {
    /// Session the frame was sampled under.
    pub session: SessionId,
    /// Vehicle the frame targets.
    pub vehicle: VehicleId,
    /// Control scope the frame targets (e.g. `"vehicle.motion"`).
    pub scope: ScopeId,
    /// Fencing generation the sender believes is current for `scope`.
    pub generation: Generation,
    /// Sequence number for ordering within `scope`.
    pub sequence: SequenceNum,
    /// Client-local monotonic sample timestamp (`transport_time`).
    pub sampled_at: MonoTimestamp,
    /// Revision of the device profile used to normalize this frame.
    pub profile_revision: u32,
    /// The sender's monotonic profile ACTIVATION revision (advances on every
    /// profile install), binding this frame to the activation announced via
    /// `ProfileActivation` — distinct from the profile document's own
    /// `profile_revision`.
    pub activation_revision: u32,
    /// The legacy untyped logical input state (ADR-0007). A frame carries
    /// EXACTLY ONE command representation: a non-empty payload OR the typed
    /// `intent`/`actions`; both or neither is rejected by the session host.
    pub payload: ControlPayload,
    /// The typed control intent this frame commands (CTRL-01). Must belong to
    /// a family the vehicle advertises for `scope`.
    pub intent: Option<ControlIntent>,
    /// Typed discrete actions carried by this frame, as one-shot events.
    pub actions: Vec<ControlAction>,
}

impl ScopedControlFrame {
    /// Whether this frame carries the legacy numeric representation (any
    /// axis or edge). Presence is decided by CONTENT, not wire-field
    /// presence, so different encoders cannot disagree about it.
    #[must_use]
    pub fn carries_payload(&self) -> bool {
        !self.payload.axes.is_empty() || !self.payload.edges.is_empty()
    }

    /// Whether this frame carries the typed representation (an intent or at
    /// least one action).
    #[must_use]
    pub fn carries_typed(&self) -> bool {
        self.intent.is_some() || !self.actions.is_empty()
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{
        ButtonEdge, ControlPayload, Generation, LogicalAxisId, LogicalButtonId, MonoTimestamp,
        ScopeId, ScopedControlFrame, SequenceNum, SessionId, VehicleId,
    };

    #[test]
    fn control_payload_default_is_empty() {
        let payload = ControlPayload::default();
        assert!(payload.axes.is_empty());
        assert!(payload.edges.is_empty());
    }

    #[test]
    fn scoped_control_frame_holds_all_fields() {
        let payload = ControlPayload {
            axes: vec![(LogicalAxisId::new(0), 0.5)],
            edges: vec![(LogicalButtonId::new(1), ButtonEdge::Pressed)],
        };
        let frame = ScopedControlFrame {
            session: SessionId::new(1),
            vehicle: VehicleId::new(2),
            scope: ScopeId::new("vehicle.motion"),
            generation: Generation::new(3),
            sequence: SequenceNum::new(4),
            sampled_at: MonoTimestamp::from_nanos(5),
            profile_revision: 6,
            activation_revision: 0,
            payload: payload.clone(),
            intent: None,
            actions: vec![],
        };
        assert_eq!(frame.session.as_u64(), 1);
        assert_eq!(frame.payload, payload);
    }
}
