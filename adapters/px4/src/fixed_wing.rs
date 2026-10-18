//! PX4 fixed-wing command uplink: Position mode flown through
//! MANUAL_CONTROL, with an automatic takeoff on arm.
//!
//! PX4 fixed-wing offboard accepts only position setpoints
//! (`src/modules/fw_mode_manager/FixedWingModeManager.cpp`), so the
//! multicopter velocity-offboard path cannot fly a plane. Instead this
//! uplink streams MAVLink joystick sticks in Position mode, where PX4
//! itself holds airspeed, height rate, and course (see [`sticks`]).
//!
//! Arming is a sequence: the neutral stick stream starts first, so PX4
//! sees manual control present at the arm instant (a source already lost
//! at arming would exempt the flight from the manual-control-loss
//! failsafe); then Takeoff mode and arm; then, once the plane climbs
//! through [`HANDOVER_HEIGHT_M`], Position mode. Until that handover the
//! sticks stay neutral whatever the intent, because a stick deflection in
//! Takeoff mode would make PX4 abandon the takeoff.
//!
//! "Neutralize" stops the stream: PX4's manual-control-loss failsafe then
//! holds the plane in a loiter (`NAV_RCL_ACT`, pinned in [`params`]).

use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use pilotage_adapter_api::{CommandOutcome, FcCommandAck};
use pilotage_mavlink::codec::{
    FcMessage, FrameSource, GCS_COMPONENT_ID, GCS_SYSTEM_ID, MAV_CMD_COMPONENT_ARM_DISARM,
    MAV_CMD_DO_SET_MODE, ManualControlSticks, encode_arm_command, encode_command_long,
    encode_gcs_heartbeat, encode_manual_control, encode_param_set, parse_datagram,
};
use pilotage_mavlink::command_outcome;
use pilotage_protocol::VelocityIntent;
use tracing::{info, warn};

use crate::uplink::clock::UplinkClock;

mod params;
pub(crate) mod sticks;
#[cfg(test)]
mod tests;

/// Height above the takeoff point at which the uplink hands the climbing
/// plane from Takeoff mode to Position mode, m. Safely above the runway,
/// and below the pinned takeoff altitude.
pub(crate) const HANDOVER_HEIGHT_M: f32 = 20.0;
const HEARTBEAT_PERIOD: Duration = Duration::from_secs(1);
/// How long neutral sticks flow before Takeoff mode and arm are commanded.
const STREAM_WARMUP: Duration = Duration::from_millis(300);
/// Stick resend period. PX4 declares manual control lost after
/// `COM_RC_LOSS_T` (0.5 s), so the stream stays well inside it.
const STREAM_PERIOD: Duration = Duration::from_millis(50);
/// How often an unconfirmed Position-mode switch is commanded again.
const MODE_RETRY: Duration = Duration::from_secs(1);

const BASE_MODE_CUSTOM: f32 = 1.0;
const PX4_MAIN_MODE_POSCTL: u8 = 3;
const PX4_MAIN_MODE_AUTO: u8 = 4;
const PX4_SUB_MODE_AUTO_TAKEOFF: u8 = 2;

/// Where the fixed-wing arm sequence stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// No stream.
    Idle,
    /// Neutral sticks flowing; Takeoff mode and arm pending.
    Warmup,
    /// Takeoff commanded; sticks held neutral until the handover height.
    TakingOff,
    /// Position mode commanded; awaiting the FC's confirmation.
    HandingOver,
    /// Position mode active; intents are flown.
    Flying,
}

/// What became of a velocity intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IntentDisposition {
    /// Flown exactly.
    Flown,
    /// Flown after clamping, or dropped to neutral before the handover.
    Constrained,
    /// No arm sequence is active, so nothing left the process.
    UplinkIdle,
}

/// Why the uplink refused to start the arm sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArmRefusal {
    /// The pinned PX4 parameters are not all confirmed yet.
    ParametersUnconfirmed,
}

/// The UDP MAVLink uplink to a fixed-wing PX4's onboard instance.
#[derive(Debug)]
pub struct FixedWingUplink {
    socket: UdpSocket,
    target: SocketAddr,
    system_id: u8,
    component_id: u8,
    seq: u8,
    clock: UplinkClock,
    params: params::ParamPinning,
    phase: Phase,
    sticks: ManualControlSticks,
    last_stick_sent: Option<Instant>,
    warmup_since: Option<Instant>,
    // Whether the warmup ends in Position mode (a re-arm while airborne)
    // rather than in a takeoff.
    resume_airborne: bool,
    last_mode_sent: Option<Instant>,
    // NED down at the arm instant: the takeoff point the handover height
    // is measured from.
    ground_down_m: Option<f32>,
    // The FC's own report, from its heartbeat on this socket.
    fc_custom_mode: Option<u32>,
    fc_armed: Option<bool>,
    last_heartbeat_sent: Option<Instant>,
    last_sent_arm: Option<bool>,
    last_arm_ack: Option<FcCommandAck>,
    send_failures: u64,
}

impl FixedWingUplink {
    /// Binds an ephemeral socket toward PX4's onboard command endpoint,
    /// addressing the given MAVLink system and component.
    ///
    /// # Errors
    ///
    /// Returns the socket bind error.
    pub fn new(target: SocketAddr, system_id: u8, component_id: u8) -> std::io::Result<Self> {
        let socket = UdpSocket::bind("127.0.0.1:0")?;
        socket.set_nonblocking(true)?;
        info!(%target, system_id, "PX4 fixed-wing uplink ready");
        Ok(Self {
            socket,
            target,
            system_id,
            component_id,
            seq: 0,
            clock: UplinkClock::System,
            params: params::ParamPinning::new(),
            phase: Phase::Idle,
            sticks: sticks::NEUTRAL,
            last_stick_sent: None,
            warmup_since: None,
            resume_airborne: false,
            last_mode_sent: None,
            ground_down_m: None,
            fc_custom_mode: None,
            fc_armed: None,
            last_heartbeat_sent: None,
            last_sent_arm: None,
            last_arm_ack: None,
            send_failures: 0,
        })
    }

    /// Total refused sends, for enactment-truth counter deltas.
    #[must_use]
    pub fn send_failures(&self) -> u64 {
        self.send_failures
    }

    /// The FC's answer to the most recent arm/disarm command.
    #[must_use]
    pub fn last_arm_ack(&self) -> Option<FcCommandAck> {
        self.last_arm_ack
    }

    fn send(&mut self, frame: &[u8]) {
        if self.socket.send_to(frame, self.target).is_err() {
            self.send_failures = self.send_failures.wrapping_add(1);
            if self.send_failures == 1 || self.send_failures.is_multiple_of(100) {
                warn!(failures = self.send_failures, target = %self.target, "PX4 fixed-wing uplink send failed");
            }
        }
        self.seq = self.seq.wrapping_add(1);
    }

    /// Periodic upkeep at telemetry cadence: FC replies, the GCS
    /// heartbeat, parameter pinning, the stick stream, and the arm
    /// sequence's phase steps. `down_m` is the latest NED down estimate.
    pub(crate) fn maintain(&mut self, down_m: Option<f32>) {
        self.drain_fc_replies();
        let now = self.clock.now();
        if self
            .last_heartbeat_sent
            .is_none_or(|at| now.duration_since(at) >= HEARTBEAT_PERIOD)
        {
            self.last_heartbeat_sent = Some(now);
            let frame = encode_gcs_heartbeat(self.seq);
            self.send(&frame);
        }
        for (name, value) in self.params.due(now) {
            let frame = encode_param_set(self.seq, name, value, self.system_id, self.component_id);
            self.send(&frame);
        }
        self.step_phase(now, down_m);
        if self.phase != Phase::Idle
            && self
                .last_stick_sent
                .is_none_or(|at| now.duration_since(at) >= STREAM_PERIOD)
        {
            self.send_sticks();
        }
    }

    /// Each phase step runs in order, so a step that enters the next phase
    /// lets that phase act in the same pass.
    fn step_phase(&mut self, now: Instant, down_m: Option<f32>) {
        if self.phase == Phase::Warmup
            && self
                .warmup_since
                .is_some_and(|since| now.duration_since(since) >= STREAM_WARMUP)
        {
            if self.resume_airborne {
                self.phase = Phase::HandingOver;
                self.last_mode_sent = None;
            } else {
                self.command_takeoff_and_arm();
            }
        }
        if self.phase == Phase::TakingOff {
            let height = self
                .ground_down_m
                .zip(down_m)
                .map(|(ground, down)| ground - down);
            if height.is_some_and(|height| height >= HANDOVER_HEIGHT_M) {
                info!(?height, "fixed-wing climbed through the handover height");
                self.phase = Phase::HandingOver;
                self.last_mode_sent = None;
            }
        }
        if self.phase == Phase::HandingOver {
            if self.fc_main_mode() == Some(PX4_MAIN_MODE_POSCTL) {
                info!("PX4 confirmed Position mode; intents are flown");
                self.phase = Phase::Flying;
            } else if self
                .last_mode_sent
                .is_none_or(|at| now.duration_since(at) >= MODE_RETRY)
            {
                self.last_mode_sent = Some(now);
                self.send_mode(PX4_MAIN_MODE_POSCTL, 0);
            }
        }
    }

    fn fc_main_mode(&self) -> Option<u8> {
        self.fc_custom_mode.map(|mode| ((mode >> 16) & 0xff) as u8)
    }

    /// Starts the arm sequence with the neutral stream warmup. A plane the
    /// FC already reports armed and at least [`HANDOVER_HEIGHT_M`] above the
    /// takeoff point (or the estimator origin) is airborne, a re-arm after
    /// link loss: the warmup ends in Position mode, which PX4 refuses until
    /// it sees manual control again. Otherwise it ends in the takeoff.
    pub(crate) fn begin_arm(&mut self, down_m: f32) -> Result<(), ArmRefusal> {
        if !self.params.all_confirmed() {
            return Err(ArmRefusal::ParametersUnconfirmed);
        }
        self.sticks = sticks::NEUTRAL;
        let ground = self.ground_down_m.unwrap_or(0.0);
        self.resume_airborne = self.fc_armed == Some(true) && ground - down_m >= HANDOVER_HEIGHT_M;
        self.last_sent_arm = Some(true);
        if self.resume_airborne {
            // No arm command goes out: PX4 already reports the plane armed,
            // which is the answer the operator's ARM asks for.
            info!("fixed-wing already airborne; resuming in Position mode");
            self.last_arm_ack = Some(FcCommandAck {
                arm: true,
                outcome: CommandOutcome::Accepted,
            });
        } else {
            // The arm goes out only after the warmup; the order is in work.
            info!("fixed-wing arm sequence: neutral stick stream started");
            self.ground_down_m = Some(down_m);
            self.last_arm_ack = Some(FcCommandAck {
                arm: true,
                outcome: CommandOutcome::InProgress,
            });
        }
        self.phase = Phase::Warmup;
        self.warmup_since = Some(self.clock.now());
        self.send_sticks();
        Ok(())
    }

    fn command_takeoff_and_arm(&mut self) {
        self.send_mode(PX4_MAIN_MODE_AUTO, PX4_SUB_MODE_AUTO_TAKEOFF);
        let arm = encode_arm_command(self.seq, true, self.system_id, self.component_id);
        self.send(&arm);
        self.phase = Phase::TakingOff;
        // The arm is in work until PX4 answers it. The takeoff that follows
        // is a separate phase: the arm answer does not wait for it.
        self.last_sent_arm = Some(true);
        self.last_arm_ack = Some(FcCommandAck {
            arm: true,
            outcome: CommandOutcome::InProgress,
        });
        info!("fixed-wing arm sequence: Takeoff mode and arm commanded");
    }

    fn send_mode(&mut self, main: u8, sub: u8) {
        let frame = encode_command_long(
            self.seq,
            MAV_CMD_DO_SET_MODE,
            [
                BASE_MODE_CUSTOM,
                f32::from(main),
                f32::from(sub),
                0.0,
                0.0,
                0.0,
                0.0,
            ],
            self.system_id,
            self.component_id,
        );
        self.send(&frame);
    }

    /// Applies one velocity intent. Before the handover the sticks stay
    /// neutral and a non-neutral intent reports constrained.
    pub(crate) fn apply_velocity(&mut self, velocity: &VelocityIntent) -> IntentDisposition {
        let (sticks, constrained) = sticks::sticks_from_velocity(velocity);
        match self.phase {
            Phase::Idle => IntentDisposition::UplinkIdle,
            Phase::Flying => {
                self.sticks = sticks;
                self.send_sticks();
                if constrained {
                    IntentDisposition::Constrained
                } else {
                    IntentDisposition::Flown
                }
            }
            Phase::Warmup | Phase::TakingOff | Phase::HandingOver => {
                if sticks == sticks::NEUTRAL && !constrained {
                    IntentDisposition::Flown
                } else {
                    IntentDisposition::Constrained
                }
            }
        }
    }

    fn send_sticks(&mut self) {
        self.last_stick_sent = Some(self.clock.now());
        let frame = encode_manual_control(self.seq, self.sticks, self.system_id);
        self.send(&frame);
    }

    /// Disarms and stops the stick stream.
    pub(crate) fn send_disarm(&mut self) {
        let frame = encode_arm_command(self.seq, false, self.system_id, self.component_id);
        self.send(&frame);
        self.stop_stream();
        self.last_sent_arm = Some(false);
        self.last_arm_ack = None;
        info!("disarm sent; stick stream stopped");
    }

    /// Link-loss enactment: one final neutral sample, then silence, so
    /// PX4's manual-control-loss failsafe takes the plane into a loiter.
    pub(crate) fn neutralize(&mut self) {
        self.sticks = sticks::NEUTRAL;
        self.send_sticks();
        self.stop_stream();
        info!("neutralized: stick stream stopped; PX4 manual-control-loss failsafe takes over");
    }

    fn stop_stream(&mut self) {
        self.phase = Phase::Idle;
        self.sticks = sticks::NEUTRAL;
        self.warmup_since = None;
    }

    /// Drains FC replies off the command socket: PX4 answers a command,
    /// and echoes a parameter write, to the endpoint that sent it.
    fn drain_fc_replies(&mut self) {
        let mut buf = [0u8; 512];
        let mut messages: Vec<(FrameSource, FcMessage)> = Vec::new();
        while let Ok((len, _)) = self.socket.recv_from(&mut buf) {
            messages.clear();
            parse_datagram(buf.get(..len).unwrap_or(&[]), &mut messages);
            for (source, message) in &messages {
                if source.system_id == self.system_id && source.component_id == self.component_id {
                    self.apply_fc_reply(*message);
                }
            }
        }
    }

    fn apply_fc_reply(&mut self, message: FcMessage) {
        match message {
            FcMessage::Heartbeat { armed, custom_mode } => {
                if self.fc_custom_mode != Some(custom_mode) {
                    info!(
                        main_mode = (custom_mode >> 16) & 0xff,
                        sub_mode = custom_mode >> 24,
                        armed,
                        "PX4 flight mode"
                    );
                }
                self.fc_armed = Some(armed);
                self.fc_custom_mode = Some(custom_mode);
            }
            FcMessage::ParamValue { param_id, value } => self.params.observe(param_id, value),
            FcMessage::CommandAck {
                command,
                result,
                target_system,
                target_component,
            } => {
                let addressed_to_us = (target_system == 0 && target_component == 0)
                    || (target_system == GCS_SYSTEM_ID && target_component == GCS_COMPONENT_ID);
                if addressed_to_us {
                    self.apply_command_ack(command, result);
                }
            }
            _ => {}
        }
    }

    /// Pairs an arm/disarm ack with the latest arm or disarm sent; the
    /// ack carries only the command id (the same limit as the multicopter
    /// uplink, see `Px4Uplink`).
    fn apply_command_ack(&mut self, command: u16, result: u8) {
        if command == MAV_CMD_DO_SET_MODE && result != 0 {
            warn!(result, "PX4 refused a fixed-wing mode command");
        }
        if command != MAV_CMD_COMPONENT_ARM_DISARM {
            return;
        }
        let Some(arm) = self.last_sent_arm else {
            return;
        };
        let outcome = command_outcome(result);
        if !outcome.is_refusal() {
            info!(arm, ?outcome, "FC answered the arm/disarm command");
        } else {
            warn!(arm, ?outcome, "FC REFUSED the arm/disarm command");
            // A refused arm leaves a disarmed plane on the ground: the takeoff
            // sequence ends, so the operator's next ARM starts it afresh.
            if arm {
                self.stop_stream();
            }
        }
        self.last_arm_ack = Some(FcCommandAck { arm, outcome });
    }

    /// Switches to the manual clock, for tests.
    #[cfg(test)]
    pub(crate) fn use_manual_clock(&mut self) {
        self.clock = UplinkClock::Manual(Instant::now());
    }

    /// Advances the manual clock, for tests.
    #[cfg(test)]
    pub(crate) fn advance_clock(&mut self, by: Duration) {
        if let UplinkClock::Manual(at) = &mut self.clock {
            *at += by;
        }
    }
}
