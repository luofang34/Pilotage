//! Ending the offboard stream on the FC's word, not on the request.
//!
//! PX4 refuses a disarm it judges unsafe, such as one a touchdown outruns
//! its land detector. A stream stopped at the request leaves a vehicle that
//! is still armed in OFFBOARD with no setpoints, and the offboard-loss
//! failsafe then flies it: a landed vehicle climbs out to return home. The
//! stream therefore continues until the FC answers the disarm or reports
//! itself disarmed. PX4's own auto-disarm after landing ends a refused
//! disarm the same way.

use tracing::info;

use pilotage_mavlink::codec::encode_arm_command;

use super::{ArmPhase, Px4Uplink};

impl Px4Uplink {
    /// Commands a disarm with the setpoint stream held at zero velocity until
    /// the FC confirms it. Stick frames still fly meanwhile, so a vehicle
    /// whose disarm was refused stays under control.
    pub fn send_disarm(&mut self) {
        self.leave_direct();
        let frame = encode_arm_command(
            self.seq,
            false,
            self.expected_system_id,
            self.expected_component_id,
        );
        self.send(&frame);
        if self.arm_phase != ArmPhase::Idle {
            self.arm_phase = ArmPhase::Disarming;
        }
        self.neutralized_generation = None;
        self.resume_offboard_at = None;
        self.warmup_since = None;
        self.last_vel_ned = [0.0; 3];
        self.last_sent_arm = Some(false);
        self.last_arm_ack = None;
        info!("disarm sent; setpoint stream held until the FC confirms");
    }

    /// Ends a pending disarm once the FC's heartbeat reports it disarmed,
    /// which also covers a lost ack and PX4's auto-disarm after landing.
    pub(crate) fn observe_fc_armed(&mut self, armed: Option<bool>) {
        if armed == Some(false) {
            self.finish_disarm();
        }
    }

    pub(super) fn finish_disarm(&mut self) {
        if self.arm_phase == ArmPhase::Disarming {
            self.arm_phase = ArmPhase::Idle;
            info!("FC confirmed the disarm; setpoint stream stopped");
        }
    }
}
