//! Live check of the `fdm_packet` frame conventions against a running
//! Betaflight SITL (built with `ENABLE_GAZEBO_BRIDGE=0`).
//!
//! Ignored by default. Start `betaflight_SITL.elf` with nothing else on
//! UDP 9002/9003 and TCP 5761, then run:
//!
//! ```text
//! PILOTAGE_BETAFLIGHT_SITL=1 cargo test -p pilotage-multirotor-fdm \
//!     --test betaflight_conventions -- --ignored --nocapture
//! ```
//!
//! The test streams kinematic poses (no motor feedback) and reads
//! Betaflight's own attitude estimate back over MSP: if the gyro,
//! accelerometer, or quaternion field had a wrong sign, the estimate
//! would settle on the wrong attitude or fight itself.
#![allow(clippy::expect_used, clippy::panic)]

use std::io::{Read, Write};
use std::net::{TcpStream, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use pilotage_msp::{Attitude, Decoder, Frame, Version, command};
use pilotage_multirotor_fdm::quad::GRAVITY;
use pilotage_multirotor_fdm::{FdmPacket, Origin, PacketClock, QuadState, Quat, Vec3};

const ORIGIN: Origin = Origin {
    lat_deg: 47.397_742,
    lon_deg: 8.545_594,
    alt_m: 488.0,
};

/// The pose the streaming thread sends: attitude and body rate.
type Pose = (Quat, Vec3);

fn live() -> bool {
    std::env::var("PILOTAGE_BETAFLIGHT_SITL").as_deref() == Ok("1")
}

/// Streams state packets at 500 Hz for whatever pose arrives on `poses`;
/// a constant body rate is integrated into the attitude.
fn stream(poses: mpsc::Receiver<Pose>, stop: Arc<AtomicBool>) {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("bind");
    let (mut attitude, mut rates) = (Quat::IDENTITY, Vec3::ZERO);
    let start = Instant::now();
    let dt = 0.002;
    let wall = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |since| since.as_secs_f64());
    let mut clock = PacketClock::new(wall, 1.0 / dt);
    let mut steps: u32 = 0;
    while !stop.load(Ordering::Relaxed) {
        if let Ok((q, w)) = poses.try_recv() {
            (attitude, rates) = (q, w);
        }
        attitude = attitude.integrate(rates, dt);
        let mut state = QuadState::landed(Vec3::ZERO, 0.0);
        state.position.z = -20.0;
        state.attitude = attitude;
        state.rates = rates;
        state.specific_force = attitude.rotate_inverse(Vec3::new(0.0, 0.0, -GRAVITY));
        state.on_ground = false;
        let packet = FdmPacket::from_state(clock.tick(), &state, &ORIGIN);
        socket
            .send_to(&packet.to_bytes(), "127.0.0.1:9003")
            .expect("send");
        steps = steps.wrapping_add(1);
        let next = start + Duration::from_secs_f64(dt) * steps;
        if let Some(wait) = next.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
    }
}

struct Msp {
    stream: TcpStream,
    decoder: Decoder,
}

impl Msp {
    fn connect() -> Self {
        let stream = TcpStream::connect("127.0.0.1:5761").expect("MSP on TCP 5761");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("timeout");
        Self {
            stream,
            decoder: Decoder::new(),
        }
    }

    fn attitude(&mut self) -> Attitude {
        let request = Frame::request(Version::V1, command::MSP_ATTITUDE, vec![])
            .encode()
            .expect("encodes");
        self.stream.write_all(&request).expect("write");
        let mut buffer = [0u8; 256];
        loop {
            let len = self.stream.read(&mut buffer).expect("MSP reply");
            for frame in self.decoder.feed(&buffer[..len]).into_iter().flatten() {
                if frame.command == command::MSP_ATTITUDE {
                    return Attitude::decode(&frame.payload).expect("attitude");
                }
            }
        }
    }

    /// Polls until `accept` has held for one second without a break, or
    /// `within` passes; returns the last attitude either way. A pose
    /// change is a step, so the estimate passes through the target on its
    /// way and must stay there to count.
    fn settle(&mut self, within: Duration, accept: impl Fn(&Attitude) -> bool) -> Attitude {
        let deadline = Instant::now() + within;
        let mut held_since: Option<Instant> = None;
        loop {
            let attitude = self.attitude();
            let now = Instant::now();
            held_since = if accept(&attitude) {
                held_since.or(Some(now))
            } else {
                None
            };
            let held = held_since.is_some_and(|since| now - since >= Duration::from_secs(1));
            if held || now >= deadline {
                return attitude;
            }
        }
    }
}

fn near(value: i16, target: i16, tolerance: i16) -> bool {
    (value - target).abs() <= tolerance
}

#[test]
#[ignore = "needs a running Betaflight SITL; set PILOTAGE_BETAFLIGHT_SITL=1"]
fn betaflight_reads_the_packet_frames_as_the_model_means_them() {
    if !live() {
        return;
    }
    tracing_subscriber::fmt().with_test_writer().try_init().ok();
    let (poses, receiver) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let streamer = {
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || stream(receiver, stop))
    };
    let mut msp = Msp::connect();
    let settle = Duration::from_secs(20);

    poses
        .send((Quat::from_euler(0.0, 0.0, 0.0), Vec3::ZERO))
        .expect("pose");
    let level = msp.settle(settle, |a| {
        near(a.roll_decidegrees, 0, 20) && near(a.pitch_decidegrees, 0, 20)
    });
    tracing::info!("level: {level:?}");

    poses
        .send((Quat::from_euler(20f64.to_radians(), 0.0, 0.0), Vec3::ZERO))
        .expect("pose");
    let right_down = msp.settle(settle, |a| {
        near(a.roll_decidegrees, 200, 20) && near(a.pitch_decidegrees, 0, 20)
    });
    tracing::info!("roll right 20 deg: {right_down:?}");

    poses
        .send((Quat::from_euler(0.0, 15f64.to_radians(), 0.0), Vec3::ZERO))
        .expect("pose");
    let nose_up = msp.settle(settle, |a| {
        near(a.pitch_decidegrees.abs(), 150, 20) && near(a.roll_decidegrees, 0, 20)
    });
    tracing::info!("nose up 15 deg: {nose_up:?}");

    poses
        .send((Quat::from_euler(0.0, 0.0, 90f64.to_radians()), Vec3::ZERO))
        .expect("pose");
    let east = msp.settle(settle, |a| {
        near(a.heading_degrees, 90, 3) && near(a.roll_decidegrees, 0, 20)
    });
    tracing::info!("heading east: {east:?}");

    let yaw_rate = Vec3::new(0.0, 0.0, 30f64.to_radians());
    poses
        .send((Quat::from_euler(0.0, 0.0, 90f64.to_radians()), yaw_rate))
        .expect("pose");
    let before = msp.attitude();
    let turn_start = Instant::now();
    let turning = msp.settle(Duration::from_secs(2), |_| {
        turn_start.elapsed() >= Duration::from_secs(2)
    });
    tracing::info!("after 2 s at +30 deg/s yaw: {before:?} -> {turning:?}");

    stop.store(true, Ordering::Relaxed);
    streamer.join().expect("streamer");

    assert!(near(level.roll_decidegrees, 0, 20) && near(level.pitch_decidegrees, 0, 20));
    assert!(near(right_down.roll_decidegrees, 200, 20), "{right_down:?}");
    assert!(near(right_down.pitch_decidegrees, 0, 20), "{right_down:?}");
    assert!(
        near(nose_up.pitch_decidegrees, -150, 20),
        "nose up reads negative: {nose_up:?}"
    );
    assert!(near(nose_up.roll_decidegrees, 0, 20), "{nose_up:?}");
    assert!(near(east.heading_degrees, 90, 3), "{east:?}");
    let turned = (turning.heading_degrees - before.heading_degrees).rem_euclid(360);
    assert!(
        (40..=80).contains(&turned),
        "a clockwise turn raises the heading: {turned}"
    );
}
