use pilotage_trial::AppliedWind;

use crate::Digest;

/// One confirmed X-Plane trial state change.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionReceipt {
    /// The plugin stored the scenario and condition identities.
    Configured {
        /// The trial generation.
        generation: u64,
    },
    /// The plugin started its truth stream.
    Started {
        /// The trial generation.
        generation: u64,
        /// Simulator time at the start.
        sim_time_s: f64,
        /// Simulator reset generation at the start.
        reset_generation: u64,
    },
    /// The plugin stopped its truth stream.
    Stopped {
        /// The trial generation.
        generation: u64,
        /// The complete sample count.
        sample_count: u64,
        /// Simulator time at the stop.
        sim_time_s: f64,
    },
    /// The simulator completed one accepted reset.
    ResetComplete {
        /// The reset command generation.
        generation: u64,
        /// The new simulator reset generation.
        reset_generation: u64,
        /// The simulator time after the reset.
        sim_time_s: f64,
    },
    /// The plugin applied one deterministic wind request.
    WindApplied {
        /// The active trial generation.
        generation: u64,
        /// The condition update generation.
        condition_generation: u32,
        /// The requested resolved wind.
        requested: AppliedWind,
        /// The actual local wind speed.
        actual_speed_mps: f64,
        /// The actual local wind direction.
        actual_direction_deg: f64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum State {
    Connected,
    Configured {
        generation: u64,
        scenario: Digest,
        condition: Digest,
    },
    Active {
        generation: u64,
        reset_generation: u64,
    },
    Stopped,
}
