//! Wire fixtures and receive barriers for camera discovery on the production link.

use std::net::SocketAddr;
use std::time::Duration;

use pilotage_adapter_api::VehicleAdapter;
use pilotage_mavlink::codec::{
    COMMAND_LONG_ID, GCS_COMPONENT_ID, GCS_SYSTEM_ID, HEARTBEAT_ID, MAV_CMD_SET_CAMERA_ZOOM,
    MAV_CMD_SET_MESSAGE_INTERVAL, encode_frame_v2_from,
};
use pilotage_protocol::VehicleId;
use tokio::net::UdpSocket;

use super::super::ArduPilotAdapter;
use crate::config::{ArduPilotConfig, ArduPilotProfile};

pub(super) const SYSTEM: u8 = 23;
pub(super) const DIRECT: (u8, u8) = (100, 0);
pub(super) const ATTACHED: (u8, u8) = (1, 2);
pub(super) const SECOND_ATTACHED: (u8, u8) = (1, 1);

pub(super) struct Harness {
    pub(super) adapter: ArduPilotAdapter,
    pub(super) socket: UdpSocket,
    peer: SocketAddr,
    time: u32,
}

impl Harness {
    pub(super) async fn start() -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("camera router binds");
        let mut config = ArduPilotConfig::new(ArduPilotProfile::Simulation);
        assert!(
            !config.gimbal,
            "camera discovery must not need gimbal opt-in"
        );
        config.endpoint = socket.local_addr().expect("router address");
        config.system_id = SYSTEM;
        let adapter = ArduPilotAdapter::start(VehicleId::new(1), config)
            .await
            .expect("start production adapter");
        assert!(
            adapter
                .capabilities()
                .vehicles
                .iter()
                .flat_map(|vehicle| &vehicle.scopes)
                .all(|scope| !scope.scope.as_str().starts_with("vehicle.camera."))
        );
        let (_, peer) = receive(&socket, |frame| message_id(frame) == HEARTBEAT_ID).await;
        let harness = Self {
            adapter,
            socket,
            peer,
            time: 100,
        };
        harness.send_reports(64, false).await;
        stream_barrier(&harness.socket).await;
        harness
    }

    pub(super) async fn replace_reports(&mut self, flags: u32, wrong_source: bool) {
        // A new sender address makes a stream request an acknowledgement that the
        // complete datagram was decoded. No cache polling or timing assumption is used.
        self.socket = UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("new router address");
        self.time = self.time.wrapping_add(100);
        self.send_reports(flags, wrong_source).await;
        stream_barrier(&self.socket).await;
    }

    async fn send_reports(&self, flags: u32, wrong_source: bool) {
        let mut datagram = Vec::new();
        append(
            &mut datagram,
            SYSTEM,
            1,
            0,
            50,
            &[0, 0, 0, 0, 2, 3, 1, 4, 3],
        );
        camera(&mut datagram, SYSTEM, DIRECT, self.time, flags, 25.0);
        camera(&mut datagram, SYSTEM, ATTACHED, self.time, 64, 40.0);
        camera(&mut datagram, SYSTEM, SECOND_ATTACHED, self.time, 64, 60.0);
        if wrong_source {
            let foreign_time = self.time.wrapping_add(1);
            camera(
                &mut datagram,
                SYSTEM.wrapping_add(1),
                DIRECT,
                foreign_time,
                64,
                99.0,
            );
            camera(
                &mut datagram,
                SYSTEM.wrapping_add(1),
                (101, 0),
                foreign_time,
                64,
                99.0,
            );
        }
        self.socket
            .send_to(&datagram, self.peer)
            .await
            .expect("send actual camera frames");
    }
}

pub(super) async fn stream_barrier(socket: &UdpSocket) {
    receive(socket, |frame| {
        message_id(frame) == COMMAND_LONG_ID
            && word(frame, 38) == MAV_CMD_SET_MESSAGE_INTERVAL
            && float(frame, 10) == 245.0
    })
    .await;
}

pub(super) async fn receive(
    socket: &UdpSocket,
    wanted: impl Fn(&[u8]) -> bool,
) -> (Vec<u8>, SocketAddr) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    let mut bytes = [0_u8; 512];
    loop {
        let (length, peer) = tokio::time::timeout_at(deadline, socket.recv_from(&mut bytes))
            .await
            .expect("outgoing packet before deadline")
            .expect("read production wire");
        let frame = &bytes[..length];
        inspect(frame);
        if wanted(frame) {
            return (frame.to_vec(), peer);
        }
    }
}

pub(super) fn message_id(frame: &[u8]) -> u32 {
    assert!(frame.len() >= 12 && frame[0] == 0xfd);
    u32::from_le_bytes([frame[7], frame[8], frame[9], 0])
}

pub(super) fn word(frame: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(frame[offset..offset + 2].try_into().unwrap())
}

pub(super) fn float(frame: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(frame[offset..offset + 4].try_into().unwrap())
}

pub(super) fn zoom(frame: &[u8]) -> bool {
    message_id(frame) == COMMAND_LONG_ID && word(frame, 38) == MAV_CMD_SET_CAMERA_ZOOM
}

fn inspect(frame: &[u8]) {
    assert_eq!((frame[5], frame[6]), (GCS_SYSTEM_ID, GCS_COMPONENT_ID));
    match message_id(frame) {
        HEARTBEAT_ID | 20 => {}
        COMMAND_LONG_ID => assert!(
            matches!(word(frame, 38), 511 | 512 | 531),
            "camera interaction emitted a flight command"
        ),
        id => panic!("camera interaction emitted unexpected message {id}"),
    }
}

fn camera(datagram: &mut Vec<u8>, system: u8, key: (u8, u8), time: u32, flags: u32, percent: f32) {
    let mut information = [0_u8; 237];
    information[..4].copy_from_slice(&time.to_le_bytes());
    information[20..24].copy_from_slice(&flags.to_le_bytes());
    information[236] = key.1;
    append(datagram, system, key.0, 259, 92, &information);
    let mut settings = [0_u8; 14];
    settings[..4].copy_from_slice(&time.to_le_bytes());
    settings[5..9].copy_from_slice(&percent.to_le_bytes());
    settings[13] = key.1;
    append(datagram, system, key.0, 260, 146, &settings);
}

fn append(datagram: &mut Vec<u8>, system: u8, component: u8, id: u32, crc: u8, payload: &[u8]) {
    let mut bytes = [0_u8; 300];
    let length = encode_frame_v2_from(0, id, payload, crc, system, component, &mut bytes);
    assert_ne!(length, 0);
    datagram.extend_from_slice(&bytes[..length]);
}
