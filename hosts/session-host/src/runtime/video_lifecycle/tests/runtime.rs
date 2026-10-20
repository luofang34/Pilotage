use pilotage_adapter_api::{
    CaptureClockMapping, FrameStamper, MeasurementClock, RawVideoFrame, SourceIncarnation,
    UnstampedFrame,
};
use pilotage_adapter_reference::ReferenceAdapter;
use tokio::sync::{mpsc, oneshot};
use wtransport::{Endpoint, Identity, ServerConfig};

use crate::runtime::engine_actor::ToEngine;
use crate::runtime::{
    HOST_VEHICLE, RuntimeOptions, build_engine, media, run_with_media_until_shutdown,
};

use super::{SHUTDOWN_TIMEOUT, VideoTasks, block_on, pending_task};

#[test]
fn live_host_keeps_video_past_the_shutdown_budget_until_shutdown_is_requested() {
    block_on(async {
        tokio::time::pause();
        let identity = Identity::self_signed(["127.0.0.1"]).expect("identity");
        let endpoint = Endpoint::server(
            ServerConfig::builder()
                .with_bind_address(([127, 0, 0, 1], 0).into())
                .with_identity(identity)
                .build(),
        )
        .expect("local endpoint");
        let adapter = ReferenceAdapter::from_seed(HOST_VEHICLE, 0);
        let engine = build_engine(&adapter, RuntimeOptions::default());
        let start = tokio::time::Instant::now();
        let (frames, input) = mpsc::channel(1);
        let (media, media_task) = media::spawn_media_task(input, start);
        let media_abort = media_task.abort_handle();
        let (ready, mut started) = mpsc::channel(1);
        let (producer, exited) = pending_task(ready);
        let producer_abort = producer.abort_handle();
        let tasks = VideoTasks::new(media_task, Some(("test-decoder", producer)));
        let (engine_tx, engine_rx) = mpsc::channel(1);
        let (shutdown, shutdown_rx) = oneshot::channel();
        let host = tokio::spawn(run_with_media_until_shutdown(
            endpoint,
            engine,
            adapter,
            media,
            tasks,
            engine_tx.clone(),
            engine_rx,
            shutdown_rx,
            start,
        ));
        started.recv().await.expect("producer started");
        engine_barrier(&engine_tx).await;
        tokio::time::advance(SHUTDOWN_TIMEOUT * 2).await;
        engine_barrier(&engine_tx).await;
        frames.send(frame()).await.expect("live media input");
        frames
            .send(frame())
            .await
            .expect("media still drains input");
        assert!(!media_abort.is_finished());
        assert!(!producer_abort.is_finished());
        assert!(!host.is_finished());
        shutdown.send(()).expect("host shutdown");
        drop(engine_tx);
        tokio::time::timeout(SHUTDOWN_TIMEOUT * 3, host)
            .await
            .expect("bounded host teardown")
            .expect("host joined");
        exited.await.expect("producer canceled during teardown");
        assert!(media_abort.is_finished());
        assert!(producer_abort.is_finished());
    });
}

#[cfg(test)]
async fn engine_barrier(engine: &mpsc::Sender<ToEngine>) {
    let (reply, received) = oneshot::channel();
    engine
        .send(ToEngine::DumpLatencySummary { reply })
        .await
        .expect("engine running");
    received.await.expect("engine processed barrier");
}

#[cfg(test)]
fn frame() -> RawVideoFrame {
    FrameStamper::new(
        SourceIncarnation::new([27; 16]),
        MeasurementClock::HostMonotonic,
        CaptureClockMapping::Unavailable,
        std::collections::BTreeMap::new(),
    )
    .stamp(UnstampedFrame {
        width: 1,
        height: 1,
        pixel_format: "RGB_INT8".to_owned(),
        time_ns: 0,
        rgb: vec![0, 1, 2],
        camera_id: 0,
        calibration_id: None,
    })
}
