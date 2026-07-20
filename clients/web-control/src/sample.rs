//! The per-tick inputs the runtime evaluates: the raw device sample the JS
//! shell reads from the Gamepad API, and the session state it reads from the
//! DOM, the clock, and the network (lease grants). Neither is owned by the
//! runtime; both are handed in each tick.

/// One physical button as the Standard Gamepad reports it: a pressed flag
/// and an analog value (triggers report `[0, 1]`, digital buttons `0`/`1`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ButtonSample {
    /// Whether the button reads pressed this tick.
    pub pressed: bool,
    /// Analog travel in `[0, 1]` (equal to `1.0` for a pressed digital
    /// button, the analog reading for a trigger).
    pub value: f32,
}

/// A raw device sample: the gamepad axes and buttons for one tick. The
/// runtime indexes into these by the active profile's declared bindings, so
/// no axis or button index ever lives outside a profile.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawSample {
    /// Axis values in `[-1, 1]`, Standard Gamepad order.
    pub axes: Vec<f32>,
    /// Button states, Standard Gamepad order.
    pub buttons: Vec<ButtonSample>,
}

impl RawSample {
    /// The axis at `index`, or `0.0` when the sample is shorter than the
    /// profile expects (a partial pad reads neutral, never out of bounds).
    #[must_use]
    pub fn axis(&self, index: usize) -> f32 {
        self.axes.get(index).copied().unwrap_or(0.0)
    }

    /// Whether the button at `index` reads pressed, or `false` when absent.
    #[must_use]
    pub fn pressed(&self, index: usize) -> bool {
        self.buttons.get(index).is_some_and(|button| button.pressed)
    }

    /// The analog value of the button at `index`, or `0.0` when absent.
    #[must_use]
    pub fn button_value(&self, index: usize) -> f32 {
        self.buttons.get(index).map_or(0.0, |button| button.value)
    }
}

/// The operator's flight-control scheme this tick, chosen from the DOM.
/// Rover releases the gimbal lease; the others hold it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// RC Mode 2 (camera-drone default): left = climb/yaw, right = translate.
    QuadPilot,
    /// Game-native: left = translate, right X = yaw, triggers = climb/descend.
    QuadCruise,
    /// Attitude mode; same stick geometry as pilot.
    Fpv,
    /// Throttle/yaw only; no gimbal.
    Rover,
}

impl Mode {
    /// Parses the DOM `flightMode` value, defaulting unknown values to the
    /// pilot scheme (the safe camera-drone default), never to rover.
    #[must_use]
    pub fn from_str_or_pilot(value: &str) -> Self {
        match value {
            "quad-cruise" => Self::QuadCruise,
            "fpv" => Self::Fpv,
            "rover" => Self::Rover,
            _ => Self::QuadPilot,
        }
    }

    /// Whether this mode operates a gimbal (everything but rover).
    #[must_use]
    pub const fn carries_gimbal(self) -> bool {
        !matches!(self, Self::Rover)
    }
}

/// The session context for one tick: the clock, the mode, connection, and
/// the network-observed gimbal lease state. Lease grant/deny is ingested by
/// the shell from the reliable session stream and handed in — the runtime
/// never reaches the network itself.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SessionState {
    /// The session generation: the shell advances it on every fresh connect,
    /// so the runtime can seed its edge baselines and fire no spurious edge
    /// from a control held across a disconnect/reconnect.
    pub generation: u32,
    /// Monotonic clock in milliseconds (the shell's `performance.now()`).
    pub now_ms: f64,
    /// The operator's selected flight-control scheme.
    pub mode: Mode,
    /// Whether a transport session is live; a dead session emits nothing.
    pub connected: bool,
    /// Whether the gimbal scope lease is currently granted.
    pub lease_granted: bool,
    /// Whether the gimbal scope lease was denied this session (never
    /// re-requested once denied).
    pub lease_denied: bool,
    /// Whether the MOTION scope lease is currently granted on the current
    /// generation. A profile handover releases it, so this drives the runtime's
    /// motion-authority reacquisition: no motion frame publishes until the host
    /// regrants the lease on a fresh generation.
    pub motion_granted: bool,
}
