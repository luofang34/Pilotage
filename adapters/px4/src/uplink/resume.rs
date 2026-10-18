//! Resuming the offboard stream for the next lease holder.
//!
//! A flight-scheme switch is release-first: the host neutralizes the
//! released scope before it grants the sibling, and a neutralize stops the
//! stream. The next holder of a vehicle that PX4 still reports armed must
//! fly on without a new arm sequence, so its first admitted frame resumes
//! the stream from the measured state. A silent pilot still stops the
//! stream, and PX4's offboard-loss failsafe still takes over.

use tracing::info;

use super::{ArmPhase, Px4Uplink};

impl Px4Uplink {
    /// Resumes a stream that a link-loss neutralize stopped, while the FC
    /// reports itself `armed`. OFFBOARD is commanded again, because PX4's
    /// offboard-loss failsafe can have left it during the gap. A stream
    /// that a disarm stopped, or that never ran, stays stopped.
    pub(crate) fn resume_after_neutralize(
        &mut self,
        armed: bool,
        yaw_rad: f32,
        vel_ned_mps: Option<[f32; 3]>,
    ) {
        if !self.neutralized || !armed || self.arm_phase != ArmPhase::Idle {
            return;
        }
        self.neutralized = false;
        self.heading_sp_rad = yaw_rad;
        self.last_vel_ned = vel_ned_mps.unwrap_or([0.0; 3]);
        self.arm_phase = ArmPhase::Commanded;
        self.send_velocity(self.last_vel_ned);
        self.command_offboard();
        info!("offboard stream resumed for the next holder of an armed vehicle");
    }
}
