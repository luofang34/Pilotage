//! The production adapter discovers and controls a payload over its shared UDP link.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use pilotage_adapter_api::{Disposition, SourceRole, VehicleAdapter};
use pilotage_mavlink::codec::{
    COMMAND_LONG_ID, GCS_COMPONENT_ID, GCS_SYSTEM_ID, GIMBAL_MANAGER_SET_ATTITUDE_ID, HEARTBEAT_ID,
    MAV_CMD_SET_MESSAGE_INTERVAL, encode_frame_v2_from,
};
use pilotage_protocol::{ControlAction, ControlIntent, GimbalRateIntent, ScopeId, VehicleId};
use tokio::net::UdpSocket;
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;

use super::tests::scoped;
use super::{ArduPilotAdapter, GIMBAL_SCOPE};
use crate::config::{ArduPilotConfig, ArduPilotProfile};

const SYSTEM: u8 = 23;
const MANAGER: u8 = 191;
const DEVICE: u8 = 154;

struct Harness {
    adapter: ArduPilotAdapter,
    socket: Arc<UdpSocket>,
    owner: watch::Sender<(u8, u8)>,
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl Harness {
    async fn start(owner: (u8, u8)) -> Self {
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("mock FC binds"));
        let mut config = ArduPilotConfig::new(ArduPilotProfile::Simulation).with_gimbal(true);
        config.endpoint = socket.local_addr().expect("mock FC address");
        config.system_id = SYSTEM;
        let adapter = ArduPilotAdapter::start(VehicleId::new(1), config)
            .await
            .expect("production adapter start");
        // The occupied endpoint exercises the supported router-mode handshake.
        let (_, peer) = receive_matching(&socket, |frame| message_id(frame) == HEARTBEAT_ID).await;
        let (owner, receiver) = watch::channel(owner);
        let (stop, task) = stream_telemetry(socket.clone(), peer, receiver);
        receive_stream_request(&socket).await;
        Self {
            adapter,
            socket,
            owner,
            stop,
            task,
        }
    }

    async fn finish(self) {
        self.stop
            .send(())
            .expect("stop the mock telemetry producer");
        tokio::time::timeout(Duration::from_secs(1), self.task)
            .await
            .expect("mock telemetry producer exits")
            .expect("mock telemetry producer succeeds");
    }
}

fn message_id(frame: &[u8]) -> u32 {
    assert!(frame.len() >= 12 && frame[0] == 0xfd);
    u32::from_le_bytes([frame[7], frame[8], frame[9], 0])
}

fn word(frame: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(frame[offset..offset + 2].try_into().unwrap())
}

fn float(frame: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(frame[offset..offset + 4].try_into().unwrap())
}

fn inspect_outgoing(frame: &[u8]) {
    assert_eq!((frame[5], frame[6]), (GCS_SYSTEM_ID, GCS_COMPONENT_ID));
    match message_id(frame) {
        HEARTBEAT_ID | 20 | GIMBAL_MANAGER_SET_ATTITUDE_ID => {}
        COMMAND_LONG_ID => {
            assert!(
                matches!(word(frame, 38), 511 | 512 | 1000 | 1001),
                "payload interaction emitted a flight command"
            );
        }
        id => panic!("unexpected outgoing MAVLink message {id}"),
    }
}

async fn receive_matching(
    socket: &UdpSocket,
    wanted: impl Fn(&[u8]) -> bool,
) -> (Vec<u8>, SocketAddr) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut bytes = [0u8; 256];
    loop {
        let (length, peer) = tokio::time::timeout_at(deadline, socket.recv_from(&mut bytes))
            .await
            .expect("expected an outgoing packet before the deadline")
            .expect("receive the outgoing packet");
        let frame = &bytes[..length];
        inspect_outgoing(frame);
        if wanted(frame) {
            return (frame.to_vec(), peer);
        }
    }
}

async fn receive_stream_request(socket: &UdpSocket) {
    // The last interval request also drains the preceding requests in this batch.
    receive_matching(socket, |frame| {
        assert_ne!(message_id(frame), GIMBAL_MANAGER_SET_ATTITUDE_ID);
        message_id(frame) == COMMAND_LONG_ID
            && word(frame, 38) == MAV_CMD_SET_MESSAGE_INTERVAL
            && float(frame, 10) == 285.0
    })
    .await;
}

fn append_frame(datagram: &mut Vec<u8>, component: u8, id: u32, crc: u8, payload: &[u8]) {
    let mut frame = [0u8; 128];
    let length = encode_frame_v2_from(0, id, payload, crc, SYSTEM, component, &mut frame);
    assert_ne!(length, 0);
    datagram.extend_from_slice(&frame[..length]);
}

fn telemetry(time: u32, owner: (u8, u8)) -> Vec<u8> {
    let mut datagram = Vec::new();
    // One datagram makes FC sender learning an acknowledgement of the entire batch.
    append_frame(&mut datagram, 1, 0, 50, &[0, 0, 0, 0, 2, 3, 1, 4, 3]);
    let mut information = [0u8; 33];
    information[..4].copy_from_slice(&time.to_le_bytes());
    information[4..8].copy_from_slice(&(2_u32 | 32 | 64 | 128 | 256 | 512).to_le_bytes());
    for (index, limit) in [f32::NAN, f32::NAN, -1.5, 1.5, -3.0, 3.0]
        .into_iter()
        .enumerate()
    {
        information[8 + index * 4..12 + index * 4].copy_from_slice(&limit.to_le_bytes());
    }
    information[32] = DEVICE;
    append_frame(&mut datagram, MANAGER, 280, 70, &information);
    let mut status = [0u8; 13];
    status[..4].copy_from_slice(&time.to_le_bytes());
    status[8..11].copy_from_slice(&[DEVICE, owner.0, owner.1]);
    append_frame(&mut datagram, MANAGER, 281, 48, &status);
    let mut attitude = [0u8; 40];
    attitude[..4].copy_from_slice(&time.to_le_bytes());
    for (index, value) in [
        0.15_f32.cos(),
        0.0,
        0.15_f32.sin(),
        0.0,
        f32::NAN,
        0.125,
        -0.25,
    ]
    .into_iter()
    .enumerate()
    {
        attitude[4 + index * 4..8 + index * 4].copy_from_slice(&value.to_le_bytes());
    }
    attitude[36..38].copy_from_slice(&12_u16.to_le_bytes());
    attitude[38..40].copy_from_slice(&[GCS_SYSTEM_ID, GCS_COMPONENT_ID]);
    append_frame(&mut datagram, DEVICE, 285, 137, &attitude);
    datagram
}

fn stream_telemetry(
    socket: Arc<UdpSocket>,
    peer: SocketAddr,
    owner: watch::Receiver<(u8, u8)>,
) -> (oneshot::Sender<()>, JoinHandle<()>) {
    let (stop, mut stopped) = oneshot::channel();
    let task = tokio::spawn(async move {
        // This is the simulated device's reporting cadence, not a cache polling loop.
        let mut reports = tokio::time::interval(Duration::from_millis(20));
        let mut time = 0u32;
        loop {
            tokio::select! {
                biased;
                _ = &mut stopped => return,
                _ = reports.tick() => {
                    time = time.wrapping_add(20);
                    let datagram = telemetry(time, *owner.borrow());
                    socket.send_to(&datagram, peer).await.expect("send device reports");
                }
            }
        }
    });
    (stop, task)
}

fn assert_feedback(adapter: &mut ArduPilotAdapter) {
    let batch = adapter.sample_telemetry();
    let sample = batch
        .samples
        .iter()
        .find(|sample| sample.gimbal.is_some())
        .expect("decoded payload feedback");
    assert!(sample.avionics.is_none(), "payload data is not flight pose");
    let attitude = sample.gimbal.unwrap();
    assert_eq!(attitude.stamp.role, SourceRole::PayloadDevice);
    assert_eq!(attitude.stamp.source_id, u64::from(DEVICE) << 8);
    assert_eq!(attitude.rates_rps[1..], [0.125, -0.25]);
    let payload = sample.payload.as_ref().expect("shared payload projection");
    let pitch = payload.pitch.as_ref().unwrap();
    assert!((pitch.angle_rad.unwrap() - 0.3).abs() < 1e-6);
    assert_eq!(pitch.rate_rad_s, Some(0.125));
}

async fn assert_rate_and_neutral(adapter: &mut ArduPilotAdapter, socket: &UdpSocket) {
    let rate = scoped(
        GIMBAL_SCOPE,
        Some(ControlIntent::GimbalRate(GimbalRateIntent {
            pitch_rate: -0.4,
            yaw_rate: 0.2,
        })),
        vec![],
    );
    assert_eq!(
        adapter.apply_control(&rate).disposition,
        Disposition::Accepted
    );
    let (frame, _) = receive_matching(socket, |frame| {
        message_id(frame) == GIMBAL_MANAGER_SET_ATTITUDE_ID
    })
    .await;
    assert_eq!(&frame[42..45], &[SYSTEM, MANAGER, DEVICE]);
    assert_eq!(u32::from_le_bytes(frame[10..14].try_into().unwrap()), 12);
    assert!(float(&frame, 30).is_nan());
    assert_eq!((float(&frame, 34), float(&frame, 38)), (-0.4, 0.2));
    let neutral = scoped(GIMBAL_SCOPE, None, vec![ControlAction::GimbalRecenter]);
    assert_eq!(
        adapter.apply_control(&neutral).disposition,
        Disposition::Accepted
    );
    let (frame, _) = receive_matching(socket, |frame| {
        message_id(frame) == COMMAND_LONG_ID && word(frame, 38) == 1000
    })
    .await;
    assert_eq!(&frame[40..42], &[SYSTEM, MANAGER]);
    assert_eq!(
        (float(&frame, 26), float(&frame, 34)),
        (2.0, f32::from(DEVICE))
    );
    assert!(float(&frame, 10).is_nan());
    assert!(float(&frame, 14).is_nan());
}

#[test]
fn production_start_routes_discovered_payload_feedback_and_commands_on_the_shared_socket() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime")
        .block_on(async {
            let mut harness = Harness::start((GCS_SYSTEM_ID, GCS_COMPONENT_ID)).await;
            assert_feedback(&mut harness.adapter);
            assert_rate_and_neutral(&mut harness.adapter, &harness.socket).await;
            harness.finish().await;
        });
}

#[test]
fn revoking_a_claim_before_motion_releases_the_selected_manager_without_a_rate_command() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime")
        .block_on(async {
            let mut harness = Harness::start((0, 0)).await;
            let rate = scoped(
                GIMBAL_SCOPE,
                Some(ControlIntent::GimbalRate(GimbalRateIntent {
                    pitch_rate: -0.4,
                    yaw_rate: 0.0,
                })),
                vec![],
            );
            assert!(matches!(
                harness.adapter.apply_control(&rate).disposition,
                Disposition::Rejected(_)
            ));
            let (claim, _) = receive_matching(&harness.socket, |frame| {
                assert_ne!(message_id(frame), GIMBAL_MANAGER_SET_ATTITUDE_ID);
                message_id(frame) == COMMAND_LONG_ID && word(frame, 38) == 1001
            })
            .await;
            assert_eq!(&claim[40..42], &[SYSTEM, MANAGER]);
            assert_eq!(
                (float(&claim, 10), float(&claim, 14), float(&claim, 34)),
                (
                    f32::from(GCS_SYSTEM_ID),
                    f32::from(GCS_COMPONENT_ID),
                    f32::from(DEVICE)
                )
            );
            harness
                .owner
                .send((GCS_SYSTEM_ID, GCS_COMPONENT_ID))
                .unwrap();
            receive_stream_request(&harness.socket).await;
            assert_feedback(&mut harness.adapter);
            harness
                .adapter
                .invalidate_pending_commands(VehicleId::new(1), &ScopeId::new(GIMBAL_SCOPE));
            harness.adapter.sample_telemetry();
            let (release, _) = receive_matching(&harness.socket, |frame| {
                assert_ne!(message_id(frame), GIMBAL_MANAGER_SET_ATTITUDE_ID);
                message_id(frame) == COMMAND_LONG_ID && word(frame, 38) == 1001
            })
            .await;
            assert_eq!(&release[40..42], &[SYSTEM, MANAGER]);
            assert_eq!(
                (
                    float(&release, 10),
                    float(&release, 14),
                    float(&release, 34)
                ),
                (-3.0, -3.0, f32::from(DEVICE))
            );
            harness.finish().await;
        });
}
