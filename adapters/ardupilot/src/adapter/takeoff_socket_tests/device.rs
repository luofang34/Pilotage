use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use pilotage_adapter_api::VehicleAdapter;
use pilotage_mavlink::codec::{COMMAND_LONG_ID, HEARTBEAT_ID, encode_frame_v2_from};
use pilotage_protocol::VehicleId;
use tokio::net::UdpSocket;
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;

use crate::{ArduPilotAdapter, ArduPilotConfig, ArduPilotProfile};

#[derive(Clone, Copy)]
pub(super) struct Observation {
    pub(super) down_m: f32,
    pub(super) altitude_mm: i32,
    pub(super) landed: u8,
    pub(super) ack: Option<u8>,
    pub(super) restart: bool,
}

impl Default for Observation {
    fn default() -> Self {
        Self {
            down_m: 0.0,
            altitude_mm: 5_000,
            landed: 1,
            ack: None,
            restart: false,
        }
    }
}

pub(super) struct Harness {
    pub(super) adapter: ArduPilotAdapter,
    pub(super) observation: watch::Sender<Observation>,
    socket: Arc<UdpSocket>,
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl Harness {
    pub(super) async fn start() -> Self {
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let mut config = ArduPilotConfig::new(ArduPilotProfile::Simulation);
        config.endpoint = socket.local_addr().unwrap();
        config.system_id = 23;
        let mut adapter = ArduPilotAdapter::start(VehicleId::new(1), config)
            .await
            .unwrap();
        let (_, peer) = receive(&socket, |frame| message_id(frame) == HEARTBEAT_ID).await;
        let (observation, receiver) = watch::channel(Observation::default());
        let (stop, task) = stream(socket.clone(), peer, receiver);
        receive(&socket, |frame| {
            message_id(frame) == COMMAND_LONG_ID
                && word(frame, 38) == 511
                && float(frame, 10) == 245.0
        })
        .await;
        let sample = adapter.sample_telemetry();
        assert!(
            sample.samples[0].pose.is_some(),
            "real decoder and EKF authorization"
        );
        Self {
            adapter,
            observation,
            socket,
            stop,
            task,
        }
    }

    pub(super) async fn receive(&self, wanted: impl Fn(&[u8]) -> bool) -> Vec<u8> {
        receive(&self.socket, wanted).await.0
    }

    pub(super) async fn await_source_reset(&self) {
        tokio::time::timeout(Duration::from_secs(6), async {
            loop {
                let reset_ready = {
                    let state = self.adapter.state.lock().unwrap();
                    state.source_resets > 0
                        && pilotage_mavlink::sampling::operational_pose(&state).is_some()
                        && state
                            .fc_reports
                            .relative_altitude
                            .is_some_and(|report| report.stamp.source_epoch == state.source_epoch)
                };
                if reset_ready {
                    return;
                }
                self.receive(|frame| message_id(frame) == HEARTBEAT_ID)
                    .await;
            }
        })
        .await
        .expect("real decoder confirms a new source epoch");
    }

    pub(super) async fn finish(self) {
        self.stop.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), self.task)
            .await
            .unwrap()
            .unwrap();
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

async fn receive(socket: &UdpSocket, wanted: impl Fn(&[u8]) -> bool) -> (Vec<u8>, SocketAddr) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut buffer = [0u8; 512];
    loop {
        let (length, peer) = tokio::time::timeout_at(deadline, socket.recv_from(&mut buffer))
            .await
            .expect("outbound packet before deadline")
            .unwrap();
        let packet = &buffer[..length];
        if wanted(packet) {
            return (packet.to_vec(), peer);
        }
    }
}

fn append(datagram: &mut Vec<u8>, id: u32, crc: u8, payload: &[u8]) {
    let mut buffer = [0u8; 128];
    let count = encode_frame_v2_from(0, id, payload, crc, 23, 1, &mut buffer);
    assert_ne!(count, 0);
    datagram.extend_from_slice(&buffer[..count]);
}

fn telemetry(time: u32, state: Observation) -> Vec<u8> {
    let mut datagram = Vec::new();
    append(&mut datagram, 0, 50, &[4, 0, 0, 0, 2, 3, 129, 4, 3]);
    let mut attitude = [0u8; 32];
    attitude[..4].copy_from_slice(&time.to_le_bytes());
    attitude[4..8].copy_from_slice(&1.0_f32.to_le_bytes());
    append(&mut datagram, 31, 246, &attitude);
    let mut position = [0u8; 28];
    position[..4].copy_from_slice(&time.to_le_bytes());
    position[12..16].copy_from_slice(&state.down_m.to_le_bytes());
    append(&mut datagram, 32, 185, &position);
    let mut ekf = [0u8; 22];
    ekf[20..22].copy_from_slice(&63_u16.to_le_bytes());
    append(&mut datagram, 193, 71, &ekf);
    let mut global = [0u8; 28];
    global[..4].copy_from_slice(&time.to_le_bytes());
    global[16..20].copy_from_slice(&state.altitude_mm.to_le_bytes());
    append(&mut datagram, 33, 104, &global);
    append(&mut datagram, 245, 130, &[0, state.landed]);
    if let Some(result) = state.ack {
        append(
            &mut datagram,
            77,
            143,
            &[22, 0, result, 0, 0, 0, 0, 0, 255, 190],
        );
    }
    datagram
}

fn stream(
    socket: Arc<UdpSocket>,
    peer: SocketAddr,
    observation: watch::Receiver<Observation>,
) -> (oneshot::Sender<()>, JoinHandle<()>) {
    let (stop, mut stopped) = oneshot::channel();
    let task = tokio::spawn(async move {
        // The mock FC emits real wire telemetry at its configured device cadence.
        let mut timer = tokio::time::interval(Duration::from_millis(20));
        let mut time = 120_000u32;
        let mut restarted = false;
        loop {
            tokio::select! {
                biased;
                _ = &mut stopped => return,
                _ = timer.tick() => {
                    let state = *observation.borrow();
                    if state.restart && !restarted {
                        time = 0;
                        restarted = true;
                    }
                    time = time.wrapping_add(20);
                    let datagram = telemetry(time, state);
                    socket.send_to(&datagram, peer).await.unwrap();
                }
            }
        }
    });
    (stop, task)
}
