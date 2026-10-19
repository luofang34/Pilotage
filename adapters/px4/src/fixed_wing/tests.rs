#![allow(clippy::expect_used, clippy::panic)]

//! The fixed-wing uplink against a fake PX4 on a loopback socket: the
//! parameter pinning gate, the stream-before-arm takeoff sequence, the
//! neutral hold until the handover, the stick mapping, and link loss.

use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

use pilotage_adapter_api::{CommandOutcome, FcCommandAck};
use pilotage_mavlink::codec::{
    ManualControlSticks, ParamValue, encode_autopilot_heartbeat, encode_command_ack,
    encode_command_long, encode_param_value,
};
use pilotage_protocol::{ReferenceFrame, VelocityIntent};

use super::params::PINNED;
use super::sticks::{self, max_turn_rate_rps};
use super::{ArmRefusal, FixedWingUplink, HANDOVER_HEIGHT_M, IntentDisposition};

const SYSTEM: u8 = 7;
const MANUAL_CONTROL: u32 = 69;
const PARAM_SET: u32 = 23;
const COMMAND_LONG: u32 = 76;
/// MAV_CMD_USER_1: a command the uplink never sends, used as an in-order
/// end marker on the uplink's own socket.
const PROBE_COMMAND: u16 = 31_000;
const READ_TIMEOUT: Duration = Duration::from_secs(2);

/// One frame the uplink sent, reduced to what the tests check.
#[derive(Debug, Clone, PartialEq)]
enum Sent {
    Sticks(ManualControlSticks),
    ParamSet(String),
    Mode { main: f32, sub: f32 },
    Arm(bool),
    Probe,
    Other(u32),
}

const POSITION_MODE: Sent = Sent::Mode {
    main: 3.0,
    sub: 0.0,
};
const TAKEOFF_MODE: Sent = Sent::Mode {
    main: 4.0,
    sub: 2.0,
};

fn i16_at(payload: &[u8], at: usize) -> i16 {
    i16::from_le_bytes([payload[at], payload[at + 1]])
}

fn f32_at(payload: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(payload[at..at + 4].try_into().expect("four bytes"))
}

fn classify(frame: &[u8]) -> Sent {
    let msg_id = u32::from(frame[7]) | (u32::from(frame[8]) << 8) | (u32::from(frame[9]) << 16);
    let payload = &frame[10..10 + usize::from(frame[1])];
    match msg_id {
        MANUAL_CONTROL => Sent::Sticks(ManualControlSticks {
            x: i16_at(payload, 0),
            y: i16_at(payload, 2),
            z: i16_at(payload, 4),
            r: i16_at(payload, 6),
        }),
        PARAM_SET => Sent::ParamSet(
            String::from_utf8_lossy(&payload[6..22])
                .trim_end_matches('\0')
                .to_owned(),
        ),
        COMMAND_LONG => match u16::from_le_bytes([payload[28], payload[29]]) {
            176 => Sent::Mode {
                main: f32_at(payload, 4),
                sub: f32_at(payload, 8),
            },
            400 => Sent::Arm(f32_at(payload, 0) == 1.0),
            PROBE_COMMAND => Sent::Probe,
            other => Sent::Other(u32::from(other)),
        },
        other => Sent::Other(other),
    }
}

/// The uplink under test, the fake PX4 it talks to, and the uplink's
/// address as the fake PX4 saw it.
struct Harness {
    uplink: FixedWingUplink,
    fc: UdpSocket,
    peer: Option<SocketAddr>,
}

impl Harness {
    fn new() -> Self {
        let fc = UdpSocket::bind("127.0.0.1:0").expect("bind fake FC");
        fc.set_read_timeout(Some(READ_TIMEOUT)).expect("timeout");
        let addr = fc.local_addr().expect("addr");
        let mut uplink = FixedWingUplink::new(addr, SYSTEM, 1).expect("uplink");
        uplink.use_manual_clock();
        Self {
            uplink,
            fc,
            peer: None,
        }
    }

    /// Everything the uplink sent since the last call, oldest first. The
    /// uplink's socket sends a probe last, and loopback UDP from one socket
    /// arrives in order, so receiving the probe means the rest has arrived.
    fn sent(&mut self) -> Vec<Sent> {
        let probe = encode_command_long(0, PROBE_COMMAND, [0.0; 7], SYSTEM, 1);
        self.uplink
            .socket
            .send_to(&probe, self.uplink.target)
            .expect("probe");
        let mut buf = [0u8; 512];
        let mut sent = Vec::new();
        loop {
            let (len, peer) = self
                .fc
                .recv_from(&mut buf)
                .expect("a frame before the probe");
            self.peer = Some(peer);
            match classify(&buf[..len]) {
                Sent::Probe => return sent,
                frame => sent.push(frame),
            }
        }
    }

    /// Delivers FC frames as one datagram, waits until the uplink's socket
    /// holds it, then runs one maintain pass with `down_m`.
    fn deliver(&mut self, frames: &[&[u8]], down_m: Option<f32>) {
        if self.peer.is_none() {
            self.sent();
        }
        let datagram: Vec<u8> = frames.concat();
        let peer = self.peer.expect("the uplink spoke");
        self.fc.send_to(&datagram, peer).expect("reply");
        let socket = &self.uplink.socket;
        socket.set_nonblocking(false).expect("blocking");
        socket
            .set_read_timeout(Some(READ_TIMEOUT))
            .expect("timeout");
        socket.peek(&mut [0u8; 1]).expect("the reply arrives");
        socket.set_nonblocking(true).expect("nonblocking");
        self.uplink.maintain(down_m);
    }

    fn echo_all_pinned(&mut self) {
        let echoes: Vec<[u8; 37]> = PINNED
            .iter()
            .map(|(name, value)| encode_param_value(0, name, *value, SYSTEM, 1))
            .collect();
        let frames: Vec<&[u8]> = echoes.iter().map(|echo| echo.as_slice()).collect();
        self.deliver(&frames, Some(0.0));
    }

    fn report_mode(&mut self, armed: bool, custom_mode: u32, down_m: Option<f32>) {
        let heartbeat = encode_autopilot_heartbeat(0, armed, custom_mode, SYSTEM, 1);
        self.deliver(&[&heartbeat], down_m);
    }

    fn tick(&mut self, by: Duration, down_m: Option<f32>) {
        self.uplink.advance_clock(by);
        self.uplink.maintain(down_m);
    }
}

/// Pinned, armed, and through Takeoff mode from a takeoff point at 0 m.
fn taking_off() -> Harness {
    let mut harness = Harness::new();
    harness.uplink.maintain(Some(0.0));
    harness.echo_all_pinned();
    harness.uplink.begin_arm(0.0).expect("arm");
    harness.tick(Duration::from_millis(400), Some(0.0));
    let sent = harness.sent();
    assert!(sent.contains(&TAKEOFF_MODE), "{sent:?}");
    harness
}

/// Climbed through the handover and confirmed in Position mode.
fn flying() -> Harness {
    let mut harness = taking_off();
    harness.tick(Duration::from_millis(60), Some(-HANDOVER_HEIGHT_M));
    harness.report_mode(true, 3 << 16, Some(-HANDOVER_HEIGHT_M));
    harness.sent();
    harness
}

fn sticks_of(sent: &[Sent]) -> Vec<ManualControlSticks> {
    sent.iter()
        .filter_map(|frame| match frame {
            Sent::Sticks(sticks) => Some(*sticks),
            _ => None,
        })
        .collect()
}

fn velocity(vx: f32, vy: f32, vz: f32, yaw_rate: f32) -> VelocityIntent {
    VelocityIntent {
        frame: ReferenceFrame::BodyFrd,
        vx,
        vy,
        vz,
        yaw_rate,
    }
}

#[test]
fn every_pinned_parameter_is_written_and_arming_waits_for_their_echo() {
    let mut harness = Harness::new();
    harness.uplink.maintain(Some(0.0));
    let written: Vec<String> = harness
        .sent()
        .into_iter()
        .filter_map(|frame| match frame {
            Sent::ParamSet(name) => Some(name),
            _ => None,
        })
        .collect();
    let pinned: Vec<String> = PINNED.iter().map(|(name, _)| (*name).to_owned()).collect();
    assert_eq!(written, pinned);
    assert_eq!(
        harness.uplink.begin_arm(0.0),
        Err(ArmRefusal::ParametersUnconfirmed)
    );

    // Every echo but one matches; the odd one reports another value (a
    // second writer, or a refused write), which does not pin.
    let mut echoes: Vec<[u8; 37]> = PINNED
        .iter()
        .skip(1)
        .map(|(name, value)| encode_param_value(0, name, *value, SYSTEM, 1))
        .collect();
    echoes.push(encode_param_value(
        0,
        "COM_RC_IN_MODE",
        ParamValue::Int32(3),
        SYSTEM,
        1,
    ));
    let frames: Vec<&[u8]> = echoes.iter().map(|echo| echo.as_slice()).collect();
    harness.deliver(&frames, Some(0.0));
    assert_eq!(
        harness.uplink.begin_arm(0.0),
        Err(ArmRefusal::ParametersUnconfirmed)
    );

    harness.tick(Duration::from_secs(1), Some(0.0));
    let resent = harness.sent();
    assert_eq!(
        resent
            .iter()
            .filter(|frame| matches!(frame, Sent::ParamSet(_)))
            .collect::<Vec<_>>(),
        vec![&Sent::ParamSet("COM_RC_IN_MODE".to_owned())],
        "only the unconfirmed write is retried"
    );
    let echo = encode_param_value(0, "COM_RC_IN_MODE", ParamValue::Int32(1), SYSTEM, 1);
    harness.deliver(&[&echo], Some(0.0));
    assert_eq!(harness.uplink.begin_arm(0.0), Ok(()));
}

#[test]
fn arm_streams_neutral_sticks_before_takeoff_mode_and_the_arm_command() {
    let mut harness = Harness::new();
    harness.uplink.maintain(Some(-0.5));
    harness.echo_all_pinned();
    harness.sent();
    harness.uplink.begin_arm(-0.5).expect("arm");
    harness.tick(Duration::from_millis(100), Some(-0.5));
    let early = harness.sent();
    assert!(!early.contains(&TAKEOFF_MODE), "warmup first: {early:?}");
    harness.tick(Duration::from_millis(250), Some(-0.5));
    let sent = [early, harness.sent()].concat();
    let position = |wanted: &Sent| sent.iter().position(|frame| frame == wanted);
    let takeoff = position(&TAKEOFF_MODE).expect("Takeoff mode commanded");
    let arm = position(&Sent::Arm(true)).expect("arm commanded");
    let first_sticks = sent
        .iter()
        .position(|frame| matches!(frame, Sent::Sticks(_)))
        .expect("sticks streamed");
    assert!(first_sticks < takeoff && takeoff < arm, "order: {sent:?}");
    assert!(
        sticks_of(&sent)
            .iter()
            .all(|sticks| *sticks == sticks::NEUTRAL)
    );
}

#[test]
fn intents_stay_neutral_until_the_handover_then_position_mode_flies_them() {
    let mut harness = taking_off();
    let turn = velocity(0.0, 0.0, 0.0, max_turn_rate_rps());
    assert_eq!(
        harness.uplink.apply_velocity(&turn),
        IntentDisposition::Constrained
    );
    harness.tick(Duration::from_millis(60), Some(-(HANDOVER_HEIGHT_M - 1.0)));
    let sent = harness.sent();
    assert!(
        sticks_of(&sent)
            .iter()
            .all(|sticks| *sticks == sticks::NEUTRAL),
        "{sent:?}"
    );
    assert!(!sent.contains(&POSITION_MODE));

    harness.tick(Duration::from_millis(60), Some(-HANDOVER_HEIGHT_M));
    let sent = harness.sent();
    assert!(
        sent.contains(&POSITION_MODE),
        "Position mode commanded at the handover height: {sent:?}"
    );
    assert_eq!(
        harness.uplink.apply_velocity(&turn),
        IntentDisposition::Constrained,
        "still neutral until the FC confirms Position mode"
    );

    harness.report_mode(true, 3 << 16, Some(-HANDOVER_HEIGHT_M));
    harness.sent();
    assert_eq!(
        harness.uplink.apply_velocity(&turn),
        IntentDisposition::Flown
    );
    assert_eq!(
        sticks_of(&harness.sent()),
        vec![ManualControlSticks {
            y: 1000,
            ..sticks::NEUTRAL
        }]
    );
}

#[test]
fn an_unconfirmed_position_mode_switch_is_commanded_again() {
    let mut harness = taking_off();
    harness.tick(Duration::from_millis(60), Some(-HANDOVER_HEIGHT_M));
    assert!(harness.sent().contains(&POSITION_MODE));
    harness.tick(Duration::from_millis(500), Some(-HANDOVER_HEIGHT_M));
    assert!(!harness.sent().contains(&POSITION_MODE));
    harness.tick(Duration::from_millis(500), Some(-HANDOVER_HEIGHT_M));
    assert!(harness.sent().contains(&POSITION_MODE));
}

#[test]
fn neutralize_sends_neutral_once_then_falls_silent() {
    let mut harness = flying();
    harness
        .uplink
        .apply_velocity(&velocity(0.0, 0.0, 0.0, max_turn_rate_rps()));
    harness.sent();
    harness.uplink.neutralize();
    assert_eq!(sticks_of(&harness.sent()), vec![sticks::NEUTRAL]);
    for _ in 0..20 {
        harness.tick(Duration::from_millis(60), Some(-HANDOVER_HEIGHT_M));
    }
    let sent = harness.sent();
    assert!(
        sticks_of(&sent).is_empty(),
        "the stream stays stopped: {sent:?}"
    );
    assert_eq!(
        harness.uplink.apply_velocity(&velocity(1.0, 0.0, 0.0, 0.0)),
        IntentDisposition::UplinkIdle
    );
}

#[test]
fn re_arming_an_airborne_plane_resumes_position_mode_without_a_takeoff() {
    let mut harness = flying();
    harness.uplink.neutralize();
    let loiter = (3 << 24) | (4 << 16);
    harness.report_mode(true, loiter, Some(-(HANDOVER_HEIGHT_M + 10.0)));
    harness.sent();
    harness
        .uplink
        .begin_arm(-(HANDOVER_HEIGHT_M + 10.0))
        .expect("re-arm");
    harness.uplink.maintain(Some(-(HANDOVER_HEIGHT_M + 10.0)));
    let early = harness.sent();
    assert!(
        !early.contains(&POSITION_MODE),
        "the stream warms up first: {early:?}"
    );
    harness.tick(
        Duration::from_millis(400),
        Some(-(HANDOVER_HEIGHT_M + 10.0)),
    );
    let sent = [early, harness.sent()].concat();
    assert!(sent.contains(&POSITION_MODE), "{sent:?}");
    assert!(!sent.contains(&TAKEOFF_MODE), "{sent:?}");
    assert!(!sent.contains(&Sent::Arm(true)), "{sent:?}");
}

#[test]
fn disarm_stops_the_stream() {
    let mut harness = taking_off();
    harness.uplink.send_disarm();
    assert!(harness.sent().contains(&Sent::Arm(false)));
    harness.tick(Duration::from_millis(200), Some(0.0));
    let sent = harness.sent();
    assert!(sticks_of(&sent).is_empty(), "{sent:?}");
}

#[test]
fn a_refused_arm_ends_the_takeoff_sequence() {
    let mut harness = taking_off();
    let refused = encode_command_ack(0, 400, 1, SYSTEM, 1);
    harness.deliver(&[&refused], Some(0.0));
    harness.sent();
    harness.tick(Duration::from_millis(200), Some(0.0));
    let sent = harness.sent();
    assert!(sticks_of(&sent).is_empty(), "{sent:?}");
    assert_eq!(
        harness.uplink.apply_velocity(&velocity(0.0, 0.0, 0.0, 0.0)),
        IntentDisposition::UplinkIdle
    );
}

#[test]
fn the_arm_answer_is_in_progress_until_px4_answers_and_ignores_the_takeoff() {
    let mut harness = Harness::new();
    harness.uplink.maintain(Some(0.0));
    harness.echo_all_pinned();
    harness.uplink.begin_arm(0.0).expect("arm");
    let in_progress = Some(FcCommandAck {
        arm: true,
        outcome: CommandOutcome::InProgress,
    });
    assert_eq!(harness.uplink.last_arm_ack(), in_progress, "warmup");
    harness.tick(Duration::from_millis(400), Some(0.0));
    assert_eq!(harness.uplink.last_arm_ack(), in_progress, "arm sent");
    let accepted = encode_command_ack(0, 400, 0, SYSTEM, 1);
    harness.deliver(&[&accepted], Some(0.0));
    // The answer is final while the takeoff is still on the runway.
    assert_eq!(
        harness.uplink.last_arm_ack(),
        Some(FcCommandAck {
            arm: true,
            outcome: CommandOutcome::Accepted,
        })
    );
}

#[test]
fn a_refused_arm_answers_with_the_typed_refusal() {
    let mut harness = taking_off();
    let refused = encode_command_ack(0, 400, 1, SYSTEM, 1);
    harness.deliver(&[&refused], Some(0.0));
    assert_eq!(
        harness.uplink.last_arm_ack(),
        Some(FcCommandAck {
            arm: true,
            outcome: CommandOutcome::TemporarilyRejected,
        })
    );
}

#[test]
fn an_airborne_re_arm_answers_accepted_without_an_arm_command() {
    let mut harness = flying();
    harness.uplink.neutralize();
    harness
        .uplink
        .begin_arm(-(HANDOVER_HEIGHT_M + 10.0))
        .expect("re-arm");
    assert_eq!(
        harness.uplink.last_arm_ack(),
        Some(FcCommandAck {
            arm: true,
            outcome: CommandOutcome::Accepted,
        })
    );
}
