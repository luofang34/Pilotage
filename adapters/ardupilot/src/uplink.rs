//! ArduCopter GUIDED-mode command path: mode changes, arm and disarm,
//! the takeoff a climb intent starts, and body-frame velocity setpoints.
//!
//! Every frame rides the link's own socket through its typed lanes: an
//! ArduPilot SITL serial port answers only the address it sends to.
//!
//! The flight phase exists because ArduCopter does not take off from a
//! velocity setpoint in GUIDED: a landed, armed copter needs
//! MAV_CMD_NAV_TAKEOFF, and a velocity setpoint sent while that takeoff
//! runs switches the guided sub-mode and aborts it. So setpoints wait
//! until the takeoff reaches its altitude.

use std::time::{Duration, Instant};

use tokio::sync::{mpsc, watch};
use tracing::{info, warn};

use pilotage_mavlink::ardupilot::ArduCopterMode;
use pilotage_mavlink::codec::{
    MAV_CMD_COMPONENT_ARM_DISARM, MAV_CMD_DO_SET_MODE, MAV_CMD_NAV_TAKEOFF,
};
use pilotage_mavlink::{BodyVelocityDemand, OutboundCommand};

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
/// A climb demand at or above this rate, while landed, starts a takeoff.
/// A third of the envelope keeps a resting stick from lifting off.
const CLIMB_INTENT_MPS: f32 = MAX_VERTICAL_MPS / 3.0;
/// The takeoff counts as complete this close to its altitude.
const TAKEOFF_MARGIN_M: f32 = 0.5;
/// Velocity control starts after this long even if the takeoff altitude
/// was not observed, so a stalled estimate cannot hold control forever.
const TAKEOFF_TIMEOUT: Duration = Duration::from_secs(10);
/// MAV_MODE_FLAG_CUSTOM_MODE_ENABLED: param 1 of MAV_CMD_DO_SET_MODE.
const BASE_MODE_CUSTOM: f32 = 1.0;

/// Where the vehicle is in the arm, takeoff, fly, land cycle, as this
/// adapter commanded it.
#[derive(Debug, Clone, Copy, PartialEq)]
enum FlightPhase {
    /// On the ground: a climb intent starts a takeoff.
    Ground,
    /// NAV_TAKEOFF is running; setpoints wait.
    TakingOff {
        started_at: Instant,
        start_down_m: f32,
    },
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
    /// The intent started a takeoff.
    TakeoffCommanded,
    /// A takeoff is climbing; the intent waits.
    TakeoffInProgress,
    /// Landed with no climb intent: nothing to send.
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
    velocity: watch::Sender<Option<BodyVelocityDemand>>,
    system_id: u8,
    component_id: u8,
    phase: FlightPhase,
    // The latest arm (true) or disarm (false) command and when it left,
    // so the verdict and the FC's status texts pair with it.
    last_arm_command: Option<(bool, Instant)>,
    send_failures: u64,
}

impl ArduPilotUplink {
    pub(crate) fn new(
        commands: mpsc::Sender<OutboundCommand>,
        velocity: watch::Sender<Option<BodyVelocityDemand>>,
        system_id: u8,
        component_id: u8,
    ) -> Self {
        Self {
            commands,
            velocity,
            system_id,
            component_id,
            phase: FlightPhase::Ground,
            last_arm_command: None,
            send_failures: 0,
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

    fn command(&mut self, command: u16, params: [f32; 7]) {
        let outbound = OutboundCommand {
            command,
            params,
            target_system: self.system_id,
            target_component: self.component_id,
        };
        if self.commands.try_send(outbound).is_err() {
            self.send_failures = self.send_failures.wrapping_add(1);
            warn!(command, "ArduPilot command lane refused a command");
        }
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
        match self.phase {
            FlightPhase::TakingOff { .. } | FlightPhase::Flying | FlightPhase::Landing => {
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
    /// `down_m` is the estimated NED down position.
    pub(crate) fn apply_velocity(
        &mut self,
        demand: [f32; 4],
        constrained: bool,
        armed: bool,
        down_m: f32,
        now: Instant,
    ) -> VelocityOutcome {
        self.observe_armed(armed);
        if !armed {
            return VelocityOutcome::NotArmed;
        }
        match self.phase {
            FlightPhase::Landing => VelocityOutcome::Landing,
            FlightPhase::Ground => {
                // + down is descent: a climb demand is negative.
                if -demand[2] < CLIMB_INTENT_MPS {
                    return VelocityOutcome::HeldOnGround;
                }
                self.command(
                    MAV_CMD_NAV_TAKEOFF,
                    [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, TAKEOFF_ALTITUDE_M],
                );
                self.phase = FlightPhase::TakingOff {
                    started_at: now,
                    start_down_m: down_m,
                };
                info!(
                    altitude_m = TAKEOFF_ALTITUDE_M,
                    "climb intent on the ground: takeoff commanded"
                );
                VelocityOutcome::TakeoffCommanded
            }
            FlightPhase::TakingOff {
                started_at,
                start_down_m,
            } => {
                let climbed = start_down_m - down_m;
                let reached = climbed >= TAKEOFF_ALTITUDE_M - TAKEOFF_MARGIN_M;
                if !reached && now.duration_since(started_at) < TAKEOFF_TIMEOUT {
                    return VelocityOutcome::TakeoffInProgress;
                }
                info!(climbed_m = climbed, "takeoff complete: velocity control");
                self.phase = FlightPhase::Flying;
                self.stream(demand, constrained)
            }
            FlightPhase::Flying => self.stream(demand, constrained),
        }
    }

    fn stream(&mut self, demand: [f32; 4], constrained: bool) -> VelocityOutcome {
        let published = self.velocity.send(Some(BodyVelocityDemand {
            vel_frd_mps: [demand[0], demand[1], demand[2]],
            yaw_rate_rps: demand[3],
            target_system: self.system_id,
            target_component: self.component_id,
        }));
        if published.is_err() {
            self.send_failures = self.send_failures.wrapping_add(1);
            return VelocityOutcome::LaneClosed;
        }
        VelocityOutcome::Streamed { constrained }
    }

    /// Link-loss enactment: one zero setpoint when flying, then nothing.
    /// ArduCopter holds the last setpoint for GUID_TIMEOUT (3 s by
    /// default) and then stops by itself, so a zero setpoint makes the
    /// hover immediate, and the silence that follows is the FC-side
    /// guarantee. The parameter is left at its default.
    pub(crate) fn neutralize(&mut self) {
        if self.phase == FlightPhase::Flying {
            let outcome = self.stream([0.0; 4], false);
            info!(?outcome, "neutralized: zero velocity sent, stream stopped");
        }
    }
}

#[cfg(test)]
mod tests;
