//! Real QUIC soak for aggregate three-source shedding and datagram priority.

#![allow(clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use pilotage_adapter_api::{
    CalibrationId, CameraId, CaptureClockMapping, MeasurementClock, MeasurementStamp,
    SourceIncarnation, SourceIntegrity, SourceRole, VideoCaptureStamp,
};
use pilotage_adapter_gazebo::RawVideoFrame;
use pilotage_protocol::wire;
use pilotage_session::ClientKey;
use pilotage_timing::SimTick;
use tokio::sync::{mpsc, watch};
use tokio::time::{Instant, MissedTickBehavior, timeout};
use wtransport::config::QuicTransportConfig;
use wtransport::quinn::VarInt as QuinnVarInt;
use wtransport::{ClientConfig, Connection, Endpoint, Identity, ServerConfig, VarInt};

use super::spawn_media_task;

const IO_BOUND: Duration = Duration::from_secs(8);
const VIDEO_DRAIN_PERIOD: Duration = Duration::from_millis(40);
const FRAME_PERIOD: Duration = Duration::from_micros(11_111);
const TELEMETRY_PERIOD: Duration = Duration::from_millis(5);
const TELEMETRY_SAMPLES: u32 = 100;

async fn connected_pair() -> (Connection, Connection) {
    let identity =
        Identity::self_signed(["localhost", "127.0.0.1"]).expect("test identity constructs");
    let server_config = ServerConfig::builder()
        .with_bind_address(([127, 0, 0, 1], 0).into())
        .with_identity(identity)
        .build();
    let server = Endpoint::server(server_config).expect("server endpoint binds");
    let server_addr = server.local_addr().expect("server has a local address");
    let mut client_config = ClientConfig::builder()
        .with_bind_default()
        .with_no_cert_validation()
        .build();
    let mut transport = QuicTransportConfig::default();
    transport.max_concurrent_uni_streams(QuinnVarInt::from_u32(4));
    client_config
        .quic_config_mut()
        .transport_config(Arc::new(transport));
    let client = Endpoint::client(client_config).expect("client endpoint binds");
    let url = format!("https://127.0.0.1:{}/media-budget-test", server_addr.port());
    let accept = async {
        server
            .accept()
            .await
            .await
            .expect("session request arrives")
            .accept()
            .await
            .expect("server accepts session")
    };
    let connect = async { client.connect(url).await.expect("client connects") };
    timeout(IO_BOUND, async { tokio::join!(accept, connect) })
        .await
        .expect("connection setup stays bounded")
}

fn video_frame(source_id: u8) -> RawVideoFrame {
    let width = 320_u32;
    let height = 240_u32;
    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height {
        for x in 0..width {
            rgb.push((x % 256) as u8);
            rgb.push((y % 256) as u8);
            rgb.push(((x + y + u32::from(source_id)) % 256) as u8);
        }
    }
    RawVideoFrame {
        source_id,
        width,
        height,
        pixel_format: "RGB_INT8".to_owned(),
        tick: SimTick::new(0),
        rgb,
        capture: capture_stamp(source_id),
    }
}

fn capture_stamp(source_id: u8) -> VideoCaptureStamp {
    VideoCaptureStamp {
        stamp: MeasurementStamp {
            role: SourceRole::VideoCapture,
            integrity: SourceIntegrity::Unprotected,
            source_id: u64::from(source_id),
            source_incarnation: SourceIncarnation::new([source_id; 16]),
            source_epoch: 0,
            sequence: 0,
            acquired_at_ns: 0,
            clock: MeasurementClock::Simulation,
        },
        camera_id: CameraId(u32::from(source_id)),
        calibration_id: CalibrationId::NONE,
        mapping: CaptureClockMapping::identity(MeasurementClock::Simulation),
    }
}

async fn produce_video(mut stop: watch::Receiver<bool>, frames: mpsc::Sender<RawVideoFrame>) {
    let templates = [video_frame(0), video_frame(1), video_frame(2)];
    let mut ticker = tokio::time::interval(FRAME_PERIOD);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut index = 0_usize;
    loop {
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    return;
                }
            }
            _ = ticker.tick() => {
                let mut frame = templates[index % templates.len()].clone();
                frame.capture.stamp.sequence = u32::try_from(index).unwrap_or(u32::MAX);
                if frames.send(frame).await.is_err() {
                    return;
                }
                index = index.wrapping_add(1);
            }
        }
    }
}

async fn drain_video_slowly(connection: Connection, mut stop: watch::Receiver<bool>) {
    let mut ticker = tokio::time::interval(VIDEO_DRAIN_PERIOD);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        let stream = tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    return;
                }
                continue;
            }
            stream = connection.accept_uni() => stream,
        };
        let Ok(mut stream) = stream else {
            return;
        };
        ticker.tick().await;
        let mut buf = [0_u8; 16 * 1024];
        while let Ok(Some(_)) = stream.read(&mut buf).await {}
    }
}

async fn await_stable_degradation(
    status: &mut watch::Receiver<wire::VideoDeliveryState>,
) -> wire::VideoDeliveryState {
    timeout(IO_BOUND, async {
        loop {
            status
                .changed()
                .await
                .expect("media status sender stays live");
            let state = *status.borrow_and_update();
            if state.mode == i32::from(wire::VideoDeliveryMode::Degraded)
                && state.budget_bytes_per_second <= 500_000
            {
                return state;
            }
        }
    })
    .await
    .expect("slow client drives the budget to a stable degraded rate")
}

async fn verify_loss_free_datagrams(server: &Connection, client: &Connection) {
    let send = async {
        let mut ticker = tokio::time::interval(TELEMETRY_PERIOD);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        for sequence in 0..TELEMETRY_SAMPLES {
            ticker.tick().await;
            server
                .send_datagram(sequence.to_le_bytes())
                .expect("small telemetry datagram queues");
        }
    };
    let receive = async {
        for expected in 0..TELEMETRY_SAMPLES {
            let datagram = client
                .receive_datagram()
                .await
                .expect("telemetry connection remains live");
            assert_eq!(datagram.payload().as_ref(), expected.to_le_bytes());
        }
    };
    timeout(IO_BOUND, async { tokio::join!(send, receive) })
        .await
        .expect("telemetry remains loss-free during video shedding");
}

#[tokio::test]
async fn three_source_slow_client_converges_without_losing_telemetry() {
    let (server, client) = connected_pair().await;
    let (frames_tx, frames_rx) = mpsc::channel(4);
    let (media, media_task) = spawn_media_task(frames_rx, Instant::now());
    let mut status = media.register(ClientKey::new(17), server.clone());
    let (stop_tx, stop_rx) = watch::channel(false);
    let producer = tokio::spawn(produce_video(stop_rx.clone(), frames_tx));
    let drainer = tokio::spawn(drain_video_slowly(client.clone(), stop_rx));

    let degraded = await_stable_degradation(&mut status).await;
    assert!(degraded.budget_bytes_per_second > 0, "video remains active");
    verify_loss_free_datagrams(&server, &client).await;
    assert_ne!(
        status.borrow().mode,
        i32::from(wire::VideoDeliveryMode::Suspended),
        "a slowly draining client converges without full suspension"
    );

    stop_tx.send(true).expect("workers retain stop receivers");
    producer.await.expect("video producer joins");
    drainer.await.expect("video drainer joins");
    server.close(VarInt::from_u32(0), b"test complete");
    drop(media);
    timeout(IO_BOUND, media_task)
        .await
        .expect("media task shutdown stays bounded")
        .expect("media task joins");
}
