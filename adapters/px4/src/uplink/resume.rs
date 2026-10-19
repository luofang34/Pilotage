//! Resuming the offboard stream for a newer lease holder.
//!
//! A voluntary scope transfer (a flight-scheme switch inside the flight
//! authority group) is release-first: the host engages Neutralize on the
//! revoked scope before it grants the sibling, and PX4's Neutralize ends
//! the stream. The holder of the newer lease on a vehicle PX4 still reports
//! armed must fly on without a fresh arm sequence. A stale or
//! same-generation holder never revives the stream, and a silent pilot
//! still stops it.

use std::time::Instant;

use pilotage_protocol::Generation;
use tracing::info;

use super::{ArmPhase, OFFBOARD_WARMUP, Px4Uplink};

impl Px4Uplink {
    /// Records the lease generation of an admitted flight frame, then
    /// resumes a stream that a link-loss neutralize stopped when that
    /// generation is newer than the neutralized one and the FC reports
    /// itself `armed`. Like the arm path, the resumed stream sends
    /// setpoints first and asks for OFFBOARD after the warmup, because
    /// PX4's offboard-loss failsafe can have left it during the gap.
    pub(crate) fn admit_flight_frame(
        &mut self,
        generation: Generation,
        armed: bool,
        yaw_rad: f32,
        vel_ned_mps: Option<[f32; 3]>,
    ) {
        self.last_generation = Some(generation);
        let newer = self
            .neutralized_generation
            .is_some_and(|neutralized| generation > neutralized);
        if !newer || !armed || self.arm_phase != ArmPhase::Idle {
            return;
        }
        self.neutralized_generation = None;
        self.heading_sp_rad = yaw_rad;
        self.last_vel_ned = vel_ned_mps.unwrap_or([0.0; 3]);
        self.arm_phase = ArmPhase::Commanded;
        self.send_velocity(self.last_vel_ned);
        self.resume_offboard_at = Some(self.clock.now() + OFFBOARD_WARMUP);
        info!(
            generation = generation.as_u64(),
            "offboard stream resumed for the newer holder of an armed vehicle"
        );
    }

    /// Sends the resumed stream's OFFBOARD request once its warmup ends.
    pub(super) fn request_offboard_when_due(&mut self, now: Instant) {
        if self.resume_offboard_at.is_some_and(|due| now >= due) {
            self.resume_offboard_at = None;
            if self.arm_phase != ArmPhase::Idle {
                self.command_offboard();
            }
        }
    }

    /// Whether a resumed stream still owes its OFFBOARD request, for tests.
    #[cfg(test)]
    pub(crate) fn offboard_request_pending(&self) -> bool {
        self.resume_offboard_at.is_some()
    }
}
