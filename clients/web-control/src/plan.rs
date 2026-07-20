//! What one evaluated tick asks the JS shell to do. The shell executes these
//! actions verbatim — send a datagram control frame, or request/release a
//! reliable-stream lease — and holds no control policy of its own.

/// The velocity-control scope: the four flight axes.
pub const MOTION_SCOPE: &str = "vehicle.motion";
/// The gimbal pointing scope, leased and fenced independently of flight.
pub const GIMBAL_SCOPE: &str = "vehicle.gimbal";

/// Canonical `roll` axis id.
pub const AXIS_ROLL: u16 = 0;
/// Canonical `pitch` axis id (also the gimbal pitch demand).
pub const AXIS_PITCH: u16 = 1;
/// Canonical `throttle` axis id.
pub const AXIS_THROTTLE: u16 = 2;
/// Canonical `yaw` axis id (also the gimbal yaw demand).
pub const AXIS_YAW: u16 = 3;
/// Gimbal-scope button whose press recenters the gimbal.
pub const GIMBAL_NEUTRAL_BUTTON: u16 = 0;
/// The `pressed` button-edge code carried on a control frame.
pub const BUTTON_EDGE_PRESSED: u8 = 1;

/// One scope's control frame for this tick: normalized axis demands plus any
/// button edges. The shell stamps sequence/generation/time and encodes it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Frame {
    /// `(axis_id, value)` demands, values in `[-1, 1]`.
    pub axes: Vec<(u16, f32)>,
    /// `(button_id, edge_code)` edges fired this tick.
    pub edges: Vec<(u16, u8)>,
}

/// A reliable-stream lease action for the gimbal scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseAction {
    /// Request the gimbal lease (a flight mode entering, debounced).
    Request,
    /// Release the gimbal lease (rover mode, or an activation handover).
    Release,
}

/// The complete outcome of one evaluated tick. Absent fields mean "send
/// nothing on that channel this tick".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ControlPlan {
    /// The motion (flight) control frame, when connected.
    pub motion: Option<Frame>,
    /// The gimbal rate frame, when the lease is held (a continuous stream,
    /// zero-rate while idle, is the scope's liveness).
    pub gimbal: Option<Frame>,
    /// A gimbal-lease request or release, when the plan calls for one.
    pub lease: Option<LeaseAction>,
    /// A motion-lease request or release, cycled with the gimbal lease across a
    /// profile activation (the scheme remaps flight, so the motion generation
    /// must be fenced and reacquired too).
    pub motion_lease: Option<LeaseAction>,
    /// A human-readable scheme label for the DOM readout (never control).
    pub label: Option<&'static str>,
    /// A typed arm edge fired this tick. The shell maps it to the wire's
    /// LOGICAL arm button — the runtime never emits a physical button index,
    /// so rebinding the arm control cannot silently disable arming.
    pub arm: bool,
    /// A typed disarm edge fired this tick.
    pub disarm: bool,
    /// The gimbal quasimode is capturing the right stick right now (the
    /// modifier is held while the gimbal lease is active), so the HUD can show
    /// capture even at a centered stick — #167's LT-descend suppression stays
    /// visible regardless of stick deflection.
    pub capture_active: bool,
}

/// The outcome of an [`crate::ControlRuntime::activate`] call: the immediate
/// transition the shell must perform. Installation may be deferred until the
/// captured controls return neutral; `installed` reports whether the new
/// profile is live yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationPlan {
    /// Whether the candidate is already the active profile (a first install,
    /// or captured controls were already neutral). When `false` the handover
    /// completes on a later tick once captured controls return neutral.
    pub installed: bool,
    /// The runtime's session activation revision after this call (advances
    /// with `wrapping_add(1)` on each install).
    pub activation_revision: u32,
    /// Whether the shell must emit neutral frames for the affected scopes as
    /// part of the handover.
    pub emit_neutral: bool,
    /// Whether the shell must release the gimbal lease as part of the
    /// handover (it is reacquired through normal lease planning on resume).
    pub release_gimbal_lease: bool,
    /// Whether the shell must also release the motion lease for the handover,
    /// because the candidate remaps flight (reacquired on resume).
    pub release_motion_lease: bool,
}
