#![allow(clippy::expect_used, clippy::panic)]

use tokio::sync::mpsc;

use super::session_video;
use crate::error::HostError;

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(future)
}

#[test]
fn an_adapter_that_produces_video_refuses_the_sim_camera() {
    let (engine_tx, _engine_rx) = mpsc::channel(1);
    let (_frames_tx, frames) = mpsc::channel(1);
    let refused = block_on(session_video("px4", Some(frames), Some(1), &engine_tx));
    assert!(
        matches!(
            refused,
            Err(HostError::SimCameraConflict { adapter: "px4" })
        ),
        "two producers would claim one camera source"
    );
}

#[test]
fn without_a_sim_camera_port_the_adapter_keeps_its_own_video() {
    let (engine_tx, _engine_rx) = mpsc::channel(1);
    let (_frames_tx, frames) = mpsc::channel(1);
    let video = block_on(session_video("px4", Some(frames), None, &engine_tx)).expect("video");
    assert!(video.frames.is_some() && video.producer.is_none());
    let video = block_on(session_video("ardupilot", None, None, &engine_tx)).expect("no video");
    assert!(video.frames.is_none() && video.producer.is_none());
}

#[cfg(feature = "sim")]
#[test]
fn a_held_sim_camera_port_fails_startup() {
    block_on(async {
        let held = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = held.local_addr().expect("address").port();
        let (engine_tx, _engine_rx) = mpsc::channel(1);
        let refused = session_video("ardupilot", None, Some(port), &engine_tx).await;
        assert!(
            matches!(refused, Err(HostError::SimCameraBind { port: refused, .. }) if refused == port),
            "a camera that can never attach must not start silently"
        );
    });
}
