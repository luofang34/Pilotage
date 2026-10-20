//! The host's view-pose lane reaches a producer as `BridgeViewPose`
//! envelopes without displacing camera commands, and a burst of poses
//! leaves the producer holding the newest one.
#![allow(clippy::expect_used, clippy::panic)]

use pilotage_sim_video::BridgeClient;
use pilotage_sim_video::read_envelope;
use pilotage_sim_video::wire::{
    BridgeCameraCommand, BridgeGimbalPose, BridgeViewPose, bridge_envelope::Payload,
};
use tokio::net::{TcpListener, TcpStream};

fn pose(sim_time_ns: u64) -> BridgeViewPose {
    BridgeViewPose {
        sim_time_ns,
        latitude_deg: 40.577_186,
        longitude_deg: -74.738_192,
        altitude_m: 120.0,
        vertical_datum: 2,
        q_w: 1.0,
        q_x: 0.0,
        q_y: 0.0,
        q_z: 0.0,
        gimbal: Some(BridgeGimbalPose {
            pan_rad: 0.5,
            tilt_rad: -0.3,
            zoom_detent: 1,
        }),
    }
}

/// A connected host client and the producer end of its socket.
async fn link() -> (BridgeClient, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address");
    let (producer, accepted) = tokio::join!(TcpStream::connect(address), listener.accept());
    let (host_side, _) = accepted.expect("accept");
    (
        BridgeClient::connect_stream_for_test(host_side),
        producer.expect("connect"),
    )
}

/// A missing lane fails the test at this deadline instead of hanging it.
const READ_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);

async fn next_payload(producer: &mut TcpStream) -> Payload {
    tokio::time::timeout(READ_DEADLINE, read_envelope(producer))
        .await
        .expect("an envelope before the deadline")
        .expect("read")
        .expect("an envelope")
        .payload
        .expect("a payload")
}

#[test]
fn poses_and_camera_commands_reach_the_producer_on_their_own_lanes() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let (client, mut producer) = link().await;

        assert!(client.try_send_view_pose(pose(1_000)));
        assert_eq!(
            next_payload(&mut producer).await,
            Payload::ViewPose(pose(1_000))
        );

        // A pose burst coalesces to its newest value; a camera command
        // published among it still arrives.
        for sim_time_ns in 2_000..2_100 {
            assert!(client.try_send_view_pose(pose(sim_time_ns)));
        }
        let command = BridgeCameraCommand {
            mode: 2,
            pan_rad: 0.25,
            tilt_rad: -0.5,
            zoom_detent: 2,
        };
        assert!(client.try_send_camera_command(command));
        let mut newest_pose = None;
        let mut saw_command = false;
        while !(saw_command && newest_pose == Some(2_099)) {
            match next_payload(&mut producer).await {
                Payload::ViewPose(received) => {
                    assert!(
                        newest_pose.is_none_or(|previous| received.sim_time_ns > previous),
                        "poses arrive oldest first"
                    );
                    newest_pose = Some(received.sim_time_ns);
                }
                Payload::CameraCommand(received) => {
                    assert_eq!(received, command);
                    saw_command = true;
                }
                other => panic!("unexpected payload {other:?}"),
            }
        }
    });
}

#[test]
fn a_pose_to_a_departed_producer_is_reported_undelivered() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let (client, producer) = link().await;
        drop(producer);
        // The writer learns of the close on its first failed write and
        // exits; every later publish reports the lane gone.
        while client.try_send_view_pose(pose(1)) {
            tokio::task::yield_now().await;
        }
        assert!(!client.try_send_view_pose(pose(2)));
    });
}
