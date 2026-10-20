//! ArduCopter GUIDED-mode command path: mode changes, arm and disarm,
//! explicit observed takeoff, and body-frame velocity setpoints.
//!
//! Every frame rides the link's own socket through its typed lanes: an
//! ArduPilot SITL serial port answers only the address it sends to.
//!
//! The flight phase exists because ArduCopter does not take off from a
//! velocity setpoint in GUIDED: a landed, armed copter needs
//! MAV_CMD_NAV_TAKEOFF, and a velocity setpoint sent while that takeoff
//! runs switches the guided sub-mode and aborts it. So setpoints wait
//! until the takeoff reaches its altitude.

use std::time::Instant;

use tokio::sync::{mpsc, watch};
use tracing::{info, warn};

use pilotage_mavlink::ardupilot::ArduCopterMode;
use pilotage_mavlink::codec::{MAV_CMD_COMPONENT_ARM_DISARM, MAV_CMD_DO_SET_MODE};
use pilotage_mavlink::{BodyVelocityDemand, OffboardSetpoint, OutboundCommand};

/// Full-envelope horizontal velocity demand, the same as the PX4 and
/// Aviate adapters advertise so every quad feels alike to the operator.
pub const MAX_HORIZONTAL_MPS: f32 = 3.0;
/// Full-envelope climb and descent rate demand.
pub const MAX_VERTICAL_MPS: f32 = 1.5;
/// Full-envelope yaw rate demand (~52°/s).
pub const MAX_YAW_RATE_RPS: f32 = 0.9;
/// Height above the arm point the takeoff climbs to before velocity
/// control starts.
pub const TAKEOFF_ALTITUDE_M: f32 = 2.5;
/// MAV_MODE_FLAG_CUSTOM_MODE_ENABLED: param 1 of MAV_CMD_DO_SET_MODE.
const BASE_MODE_CUSTOM: f32 = 1.0;

/// Where the vehicle is in the arm, takeoff, fly, land cycle, as this
/// adapter commanded it.
#[derive(Debug, Clone, Copy, PartialEq)]
enum FlightPhase {
    /// No airborne procedure or manual direct control has been established.
    Ground,
    /// Airborne under velocity control.
    Flying,
    /// LAND mode commanded; setpoints are not sent.
    Landing,
}

/// What a velocity intent did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VelocityOutcome {
    /// A setpoint was published; `constrained` when the demand was clamped
    /// to the envelope.
    Streamed { constrained: bool },
    /// A takeoff is climbing; the intent waits.
    TakeoffInProgress,
    /// A failed or revoked procedure still blocks motion.
    TakeoffBlocked,
    /// Landed: only an explicit takeoff action may launch.
    HeldOnGround,
    /// Landing: the FC owns the descent.
    Landing,
    /// The FC reports itself disarmed.
    NotArmed,
    /// The lane to the link is gone.
    LaneClosed,
}

/// What a disarm action became.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DisarmOutcome {
    /// Landed: the disarm command was sent.
    Disarmed,
    /// Airborne: ArduCopter refuses a disarm in flight, so LAND mode was
    /// commanded instead and the FC disarms once it has landed.
    LandCommanded,
}

/// The GUIDED-mode command path over the link's lanes.
#[derive(Debug)]
pub struct ArduPilotUplink {
    commands: mpsc::Sender<OutboundCommand>,
    velocity: watch::Sender<Option<OffboardSetpoint>>,
    system_id: u8,
    component_id: u8,
    phase: FlightPhase,
    takeoff: Option<takeoff::TakeoffProcedure>,
    takeoff_revision: u64,
    flight_state: Option<std::sync::Arc<std::sync::Mutex<pilotage_mavlink::LinkState>>>,
    // The latest arm (true) or disarm (false) command and when it left,
    // so the verdict and the FC's status texts pair with it.
    last_arm_command: Option<(bool, Instant)>,
    send_failures: u64,
    // The FC thrust the last direct setpoint carried and when, so the
    // next one slews from it; `None` after a velocity setpoint, so a
    // switch into direct flight starts from hover thrust.
    direct_thrust: Option<(f32, Instant)>,
}

impl ArduPilotUplink {
    pub(crate) fn new(
        commands: mpsc::Sender<OutboundCommand>,
        velocity: watch::Sender<Option<OffboardSetpoint>>,
        system_id: u8,
        component_id: u8,
    ) -> Self {
        Self {
            commands,
            velocity,
            system_id,
            component_id,
            phase: FlightPhase::Ground,
            takeoff: None,
            takeoff_revision: 0,
            flight_state: None,
            last_arm_command: None,
            send_failures: 0,
            direct_thrust: None,
        }
    }

    /// Total refused sends, for enactment-truth counter deltas.
    pub(crate) fn send_failures(&self) -> u64 {
        self.send_failures
    }

    /// The latest arm or disarm command and when it was sent.
    pub(crate) fn last_arm_command(&self) -> Option<(bool, Instant)> {
        self.last_arm_command
    }

    fn command(&mut self, command: u16, params: [f32; 7]) -> bool {
        let outbound = OutboundCommand {
            payload_write: None,
            receipt: None,
            fence: None,
            send_before: None,
            command,
            params,
            target_system: self.system_id,
            target_component: self.component_id,
        };
        if self.commands.try_send(outbound).is_err() {
            self.send_failures = self.send_failures.wrapping_add(1);
            warn!(command, "ArduPilot command lane refused a command");
            return false;
        }
        true
    }

    fn set_mode(&mut self, mode: ArduCopterMode) {
        let custom_mode = mode.custom_mode() as f32;
        self.command(
            MAV_CMD_DO_SET_MODE,
            [BASE_MODE_CUSTOM, custom_mode, 0.0, 0.0, 0.0, 0.0, 0.0],
        );
    }

    /// Selects GUIDED and arms. ArduCopter runs its own pre-arm checks
    /// (EKF, GPS, ...), so readiness is the FC's verdict, never guessed
    /// here: a refusal arrives as the arm acknowledgement plus the FC's
    /// `PreArm:` status texts. An arm while already armed and landing
    /// takes the vehicle back into GUIDED velocity control.
    pub(crate) fn arm(&mut self, armed: bool, now: Instant) {
        self.set_mode(ArduCopterMode::Guided);
        self.command(
            MAV_CMD_COMPONENT_ARM_DISARM,
            [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        );
        self.last_arm_command = Some((true, now));
        self.phase = match (armed, self.phase) {
            (true, FlightPhase::Landing) => FlightPhase::Flying,
            (true, phase) => phase,
            (false, _) => FlightPhase::Ground,
        };
        info!("GUIDED and arm commanded");
    }

    /// Disarms on the ground, or lands when airborne (see
    /// [`DisarmOutcome::LandCommanded`]).
    pub(crate) fn disarm(&mut self, now: Instant) -> DisarmOutcome {
        self.interrupt_takeoff(now, pilotage_protocol::FlightProcedureReason::Interrupted);
        let procedure_may_run = self.takeoff_may_have_written();
        match self.phase {
            _ if procedure_may_run => {
                self.set_mode(ArduCopterMode::Land);
                self.phase = FlightPhase::Landing;
                DisarmOutcome::LandCommanded
            }
            FlightPhase::Flying | FlightPhase::Landing => {
                self.set_mode(ArduCopterMode::Land);
                self.phase = FlightPhase::Landing;
                info!("disarm while airborne: LAND commanded");
                DisarmOutcome::LandCommanded
            }
            FlightPhase::Ground => {
                self.command(
                    MAV_CMD_COMPONENT_ARM_DISARM,
                    [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                );
                self.last_arm_command = Some((false, now));
                info!("disarm commanded");
                DisarmOutcome::Disarmed
            }
        }
    }

    /// Follows the FC's arm report: a disarmed vehicle is on the ground,
    /// whatever was commanded before.
    pub(crate) fn observe_armed(&mut self, armed: bool) {
        if !armed && self.phase != FlightPhase::Ground {
            info!(phase = ?self.phase, "FC reports disarmed: back to ground phase");
            self.phase = FlightPhase::Ground;
        }
    }

    /// Applies one body-frame velocity intent (m/s forward, right, down;
    /// rad/s clockwise), already clamped to the envelope by the caller.
    pub(crate) fn apply_velocity(
        &mut self,
        demand: [f32; 4],
        constrained: bool,
        armed: bool,
    ) -> VelocityOutcome {
        self.observe_armed(armed);
        if !armed {
            return VelocityOutcome::NotArmed;
        }
        if self.takeoff_blocks_motion() {
            return if self.takeoff_active() {
                VelocityOutcome::TakeoffInProgress
            } else {
                VelocityOutcome::TakeoffBlocked
            };
        }
        match self.phase {
            FlightPhase::Landing => VelocityOutcome::Landing,
            FlightPhase::Ground => VelocityOutcome::HeldOnGround,
            FlightPhase::Flying => self.stream(demand, constrained),
        }
    }

    fn stream(&mut self, demand: [f32; 4], constrained: bool) -> VelocityOutcome {
        self.direct_thrust = None;
        let published =
            self.velocity
                .send(Some(OffboardSetpoint::BodyVelocity(BodyVelocityDemand {
                    vel_frd_mps: [demand[0], demand[1], demand[2]],
                    yaw_rate_rps: demand[3],
                    target_system: self.system_id,
                    target_component: self.component_id,
                })));
        if published.is_err() {
            self.send_failures = self.send_failures.wrapping_add(1);
            return VelocityOutcome::LaneClosed;
        }
        VelocityOutcome::Streamed { constrained }
    }

    /// Revokes queued launch work and publishes one zero-velocity attempt when
    /// motion may remain active. Publication does not establish a physical stop.
    pub(crate) fn neutralize(&mut self) {
        self.cancel_takeoff(Instant::now());
        let procedure_may_run = self.takeoff_may_have_written();
        if self.phase == FlightPhase::Flying || procedure_may_run {
            let outcome = self.stream([0.0; 4], false);
            info!(
                ?outcome,
                "zero velocity published for link-loss interruption"
            );
        }
    }
}

mod direct;
mod takeoff;
pub(crate) use direct::DirectOutcome;
pub use direct::{MAX_BODY_RATE_RPS, MAX_DIRECT_TILT_RAD};
#[cfg(test)]
mod tests;
