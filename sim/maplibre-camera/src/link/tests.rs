#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration;

use pilotage_sim_video::read_envelope;
use pilotage_sim_video::wire::{
    BridgeCameraCommand, BridgeEnvelope, BridgeFrame, BridgeViewPose, bridge_envelope::Payload,
};
use prost::Message;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;

use super::spawn;

/// A missing delivery fails the test here instead of hanging it.
const DEADLINE: Duration = Duration::from_secs(10);

fn pose(sim_time_ns: u64) -> BridgeViewPose {
    BridgeViewPose {
        sim_time_ns,
        latitude_deg: 40.5,
        longitude_deg: -74.7,
        altitude_m: 80.0,
        vertical_datum: 2,
        q_w: 1.0,
        q_x: 0.0,
        q_y: 0.0,
        q_z: 0.0,
        gimbal: None,
    }
}

fn frame(sim_time_ns: u64) -> BridgeFrame {
    BridgeFrame {
        width: 1,
        height: 1,
        pixel_format: "RGB_INT8".to_owned(),
        sim_time_ns,
        rgb: vec![1, 2, 3],
        camera_id: 2,
        calibration_id: 0x4D4C_0201,
    }
}

#[test]
fn the_link_dials_the_host_folds_its_state_and_returns_frames() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("address").port();
        let (mut ends, task) = spawn(port);
        let (mut host, _) = tokio::time::timeout(DEADLINE, listener.accept())
            .await
            .expect("the producer dials before the deadline")
            .expect("accept");

        let command = BridgeCameraCommand {
            mode: 2,
            pan_rad: 0.3,
            tilt_rad: -0.4,
            zoom_detent: 1,
        };
        for payload in [Payload::CameraCommand(command), Payload::ViewPose(pose(42))] {
            let envelope = BridgeEnvelope {
                payload: Some(payload),
            };
            host.write_all(&envelope.encode_length_delimited_to_vec())
                .await
                .expect("write");
        }
        tokio::time::timeout(DEADLINE, async {
            while ends
                .poses
                .borrow_and_update()
                .is_none_or(|got| got.pose.sim_time_ns != 42)
            {
                ends.poses.changed().await.expect("link alive");
            }
        })
        .await
        .expect("the pose arrives");
        assert_eq!(*ends.commands.borrow(), Some(command));

        assert!(ends.offer(frame(42)));
        let returned = tokio::time::timeout(DEADLINE, read_envelope(&mut host))
            .await
            .expect("a frame before the deadline")
            .expect("read")
            .expect("an envelope");
        assert_eq!(returned.payload, Some(Payload::Frame(frame(42))));
        task.abort();
    });
}
