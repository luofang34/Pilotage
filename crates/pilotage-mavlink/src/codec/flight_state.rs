//! Flight-state reports that do not define a navigation origin.

/// GLOBAL_POSITION_INT message id.
pub const GLOBAL_POSITION_INT_ID: u32 = 33;
/// EXTENDED_SYS_STATE message id.
pub const EXTENDED_SYS_STATE_ID: u32 = 245;

/// Flight controller's landed detector state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LandedState {
    /// The flight controller cannot state whether the vehicle is landed.
    Undefined,
    /// The vehicle is on the ground.
    OnGround,
    /// The vehicle is in the air.
    InAir,
    /// The vehicle is taking off.
    Takeoff,
    /// The vehicle is landing.
    Landing,
}

impl LandedState {
    pub(super) fn decode(value: u8) -> Self {
        match value {
            1 => Self::OnGround,
            2 => Self::InAir,
            3 => Self::Takeoff,
            4 => Self::Landing,
            _ => Self::Undefined,
        }
    }
}
