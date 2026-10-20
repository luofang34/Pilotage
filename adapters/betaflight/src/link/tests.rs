#![allow(clippy::expect_used, clippy::panic)]

//! The link task against a fake Betaflight on a loopback socket: the
//! first-connect configuration, the RC stream, and the arm switch, all
//! through the real TCP and MSP path.

use std::time::Duration;

use pilotage_adapter_api::FcCommandAck;
use pilotage_msp::message::feature;
use pilotage_msp::{Decoder, Direction, Frame, ModeRange, Version, command};
use pilotage_protocol::CommandOutcome;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, watch};

use super::{ConfigIssue, LinkCommand, LinkReport, Requests, run};
use crate::adapter::ENVELOPE;
use crate::config::BetaflightProfile;
use crate::rc::{ARM_HIGH_US, required_mode_ranges};

/// What the fake flight controller answers with.
#[derive(Clone, Copy)]
struct FakeFc {
    features: u32,
    armed_after_arm_switch: bool,
    /// Never answer a telemetry poll: a flight controller whose serial
    /// task is too slow to reply in time.
    withhold_polls: bool,
}

fn status(armed: bool) -> Vec<u8> {
    let mut payload = vec![0x8c, 0x03, 0, 0, 0x2f, 0];
    payload.extend(u32::from(armed).to_le_bytes());
    payload.extend([0, 0, 0, 4, 0, 0, 30]);
    payload.extend(0u32.to_le_bytes());
    payload.push(0);
    payload
}

fn poll_reply(armed: bool) -> Vec<u8> {
    let mut gps = vec![1, 12];
    gps.extend(473_977_420i32.to_le_bytes());
    gps.extend(85_455_940i32.to_le_bytes());
    gps.extend([0; 6]);
    let entries = [status(armed), vec![0; 6], vec![0; 18], vec![0; 6], gps];
    entries
        .iter()
        .flat_map(|entry| {
            let mut out = vec![u8::try_from(entry.len()).expect("short")];
            out.extend(entry);
            out
        })
        .collect()
}

/// Serves one connection; forwards every request it receives.
async fn serve(listener: TcpListener, fc: FakeFc, seen: mpsc::UnboundedSender<Frame>) {
    let (mut stream, _) = listener.accept().await.expect("accept");
    let mut decoder = Decoder::new();
    let mut buffer = [0u8; 1_024];
    let mut armed = false;
    loop {
        let Ok(len) = stream.read(&mut buffer).await else {
            return;
        };
        if len == 0 {
            return;
        }
        for frame in decoder.feed(&buffer[..len]).into_iter().flatten() {
            if frame.command == command::MSP_SET_RAW_RC {
                let aux1 = u16::from_le_bytes([frame.payload[8], frame.payload[9]]);
                armed = fc.armed_after_arm_switch && aux1 >= 1_700;
            }
            if fc.withhold_polls && frame.command == command::MSP_MULTIPLE_MSP {
                if seen.send(frame).is_err() {
                    return;
                }
                continue;
            }
            let payload = match frame.command {
                command::MSP_API_VERSION => vec![0, 1, 49],
                command::MSP_BOXIDS => vec![0, 1, 2, 5, 3],
                command::MSP_FEATURE_CONFIG => fc.features.to_le_bytes().to_vec(),
                command::MSP_FAILSAFE_CONFIG => vec![15, 60, 0xe8, 0x03, 0, 100, 0, 0],
                command::MSP_MODE_RANGES => vec![0; 80],
                command::MSP_MULTIPLE_MSP => poll_reply(armed),
                _ => vec![],
            };
            let reply = Frame {
                version: Version::V1,
                direction: Direction::Response,
                command: frame.command,
                payload,
            };
            stream
                .write_all(&reply.encode().expect("encodes"))
                .await
                .expect("reply");
            if seen.send(frame).is_err() {
                return;
            }
        }
    }
}

struct Rig {
    seen: mpsc::UnboundedReceiver<Frame>,
    commands: watch::Sender<LinkCommand>,
    reports: watch::Receiver<LinkReport>,
}

async fn rig(fc: FakeFc) -> Rig {
    rig_with(fc, BetaflightProfile::Simulation).await
}

async fn rig_with(fc: FakeFc, profile: BetaflightProfile) -> Rig {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let endpoint = listener.local_addr().expect("addr");
    let (seen_tx, seen) = mpsc::unbounded_channel();
    tokio::spawn(serve(listener, fc, seen_tx));
    let (commands, command_rx) = watch::channel(LinkCommand::default());
    let (report_tx, reports) = watch::channel(LinkReport::default());
    tokio::spawn(run(endpoint, profile, ENVELOPE, command_rx, report_tx));
    Rig {
        seen,
        commands,
        reports,
    }
}

/// The next request the fake saw for `wanted`.
async fn next(rig: &mut Rig, wanted: u16) -> Frame {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let frame = rig.seen.recv().await.expect("fake fc alive");
            if frame.command == wanted {
                return frame;
            }
        }
    })
    .await
    .expect("request arrived")
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

const GOOD: FakeFc = FakeFc {
    features: feature::RX_MSP | feature::GPS,
    armed_after_arm_switch: true,
    withhold_polls: false,
};

#[test]
fn first_connect_writes_the_missing_aux_ranges_and_saves_them() {
    runtime().block_on(async {
        let mut rig = rig(GOOD).await;
        let wanted = required_mode_ranges();
        for (slot, range) in wanted.iter().enumerate() {
            let set = next(&mut rig, command::MSP_SET_MODE_RANGE).await;
            assert_eq!(set.payload[0], u8::try_from(slot).expect("slot"));
            assert_eq!(
                ModeRange::decode_all(&set.payload[1..]).first(),
                Some(range)
            );
        }
        next(&mut rig, command::MSP_EEPROM_WRITE).await;
        next(&mut rig, command::MSP_MULTIPLE_MSP).await;
        assert!(rig.reports.borrow().issue.is_none());
    });
}

#[test]
fn a_physical_flight_controller_is_never_written_and_a_differing_range_blocks_arming() {
    runtime().block_on(async {
        let mut rig = rig_with(GOOD, BetaflightProfile::Physical).await;
        let mut before_poll = Vec::new();
        loop {
            let frame = tokio::time::timeout(Duration::from_secs(5), rig.seen.recv())
                .await
                .expect("request arrived")
                .expect("fake fc alive");
            if frame.command == command::MSP_MULTIPLE_MSP {
                break;
            }
            before_poll.push(frame.command);
        }
        assert!(
            !before_poll.contains(&command::MSP_SET_MODE_RANGE)
                && !before_poll.contains(&command::MSP_EEPROM_WRITE),
            "a physical flight controller got a write: {before_poll:?}"
        );
        let issue = rig.reports.borrow().issue;
        let Some(ConfigIssue::ModeRange { slot, wanted, .. }) = issue else {
            panic!("the differing range blocks flight: {issue:?}");
        };
        assert_eq!(slot, 0);
        assert_eq!(Some(&wanted), required_mode_ranges().first());
        assert!(
            issue
                .map(|issue| issue.to_string())
                .is_some_and(|detail| detail.contains("aux mode range 0")),
            "{issue:?}"
        );
    });
}

#[test]
fn rc_keeps_flowing_while_a_telemetry_reply_is_outstanding() {
    runtime().block_on(async {
        let mut rig = rig(FakeFc {
            withhold_polls: true,
            ..GOOD
        })
        .await;
        next(&mut rig, command::MSP_MULTIPLE_MSP).await;
        // The poll is never answered; the RC frames still come, until the
        // link gives the reply up and drops the connection.
        for _ in 0..3 {
            let rc = next(&mut rig, command::MSP_SET_RAW_RC).await;
            assert_eq!(rc.payload.len(), 16, "eight channels");
        }
    });
}

#[test]
fn the_arm_request_raises_aux1_with_the_throttle_cut() {
    runtime().block_on(async {
        let mut rig = rig(GOOD).await;
        next(&mut rig, command::MSP_SET_RAW_RC).await;
        rig.commands.send_modify(|command| {
            command.requests = Requests {
                arm: 1,
                ..Requests::default()
            };
        });
        let raised = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let rc = next(&mut rig, command::MSP_SET_RAW_RC).await;
                if u16::from_le_bytes([rc.payload[8], rc.payload[9]]) == ARM_HIGH_US {
                    return rc;
                }
            }
        })
        .await
        .expect("AUX1 went high");
        let throttle = u16::from_le_bytes([raised.payload[4], raised.payload[5]]);
        assert_eq!(throttle, 1_000, "arming needs the throttle cut");
        let mut reports = rig.reports.clone();
        tokio::time::timeout(
            Duration::from_secs(2),
            reports.wait_for(|report| report.latest.is_some_and(|o| o.snapshot.armed)),
        )
        .await
        .expect("armed in time")
        .expect("link alive");
        let accepted = Some(FcCommandAck {
            arm: true,
            outcome: CommandOutcome::Accepted,
        });
        tokio::time::timeout(
            Duration::from_secs(2),
            reports.wait_for(|report| report.last_command == accepted),
        )
        .await
        .expect("the arm verdict arrives in time")
        .expect("link alive");
    });
}

#[test]
fn a_udp_receiver_is_a_typed_issue_that_keeps_the_switch_down() {
    runtime().block_on(async {
        let mut rig = rig(FakeFc {
            features: feature::RX_UDP | feature::RX_MSP,
            armed_after_arm_switch: true,
            withhold_polls: false,
        })
        .await;
        let mut reports = rig.reports.clone();
        let report = tokio::time::timeout(
            Duration::from_secs(5),
            reports.wait_for(|report| report.issue.is_some()),
        )
        .await
        .expect("issue in time")
        .expect("link alive")
        .clone();
        assert!(matches!(
            report.issue,
            Some(ConfigIssue::ReceiverNotMsp { .. })
        ));
        rig.commands.send_modify(|command| command.requests.arm = 1);
        for _ in 0..10 {
            let rc = next(&mut rig, command::MSP_SET_RAW_RC).await;
            assert_ne!(
                u16::from_le_bytes([rc.payload[8], rc.payload[9]]),
                ARM_HIGH_US
            );
        }
    });
}
