use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use pilotage_adapter_api::{
    CalibrationId, CaptureClockMapping, GIMBAL_CAMERA, MeasurementClock, SourceIncarnation,
};
use pilotage_adapter_dji::EncodedVideoChunk;
use tokio::sync::mpsc;

use super::{DjiVideoConfig, DjiVideoError, spawn};

// The verified Mac route includes SPS, PPS, an IDR, and following pictures.
const STREAM: &[u8] = include_bytes!("fixtures/mini3-mac.h264");
const DEADLINE: Duration = Duration::from_secs(10);

#[cfg(test)]
fn config() -> DjiVideoConfig {
    DjiVideoConfig {
        ffmpeg: PathBuf::from("ffmpeg"),
        incarnation: SourceIncarnation::new([71; 16]),
        origin: tokio::time::Instant::now(),
        keyframe_request: Arc::new(AtomicBool::new(false)),
    }
}

#[cfg(test)]
async fn feed(sender: &mpsc::Sender<EncodedVideoChunk>, continuity: u64) {
    for bytes in STREAM.chunks(2040) {
        sender
            .send(EncodedVideoChunk {
                received_at: Duration::from_nanos(123),
                continuity,
                bytes: bytes.to_vec(),
            })
            .await
            .unwrap();
    }
}

#[test]
#[ignore = "requires ffmpeg on PATH; run with --ignored"]
fn a_real_mac_h264_stream_decodes_and_discontinuity_starts_a_new_epoch() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            tokio::time::timeout(DEADLINE, async {
                let config = config();
                let request = config.keyframe_request.clone();
                let (sender, input) = mpsc::channel(64);
                let (mut frames, task) = spawn(config, input).unwrap();
                feed(&sender, 0).await;
                let frame = frames.recv().await.unwrap();
                assert_eq!((frame.width, frame.height), (1280, 720));
                assert_eq!(frame.rgb.len(), 1280 * 720 * 3);
                assert!(frame.rgb.windows(2).any(|pair| pair[0] != pair[1]));
                assert_eq!(frame.source_id, GIMBAL_CAMERA);
                assert_eq!(frame.capture.stamp.clock, MeasurementClock::HostMonotonic);
                assert_eq!(frame.capture.mapping, CaptureClockMapping::Unavailable);
                assert_eq!(frame.capture.calibration_id, CalibrationId::NONE);
                assert_eq!(
                    frame.capture.stamp.source_incarnation,
                    SourceIncarnation::new([71; 16])
                );
                assert_eq!(frame.capture.stamp.source_epoch, 0);
                assert_ne!(frame.capture.stamp.acquired_at_ns, 123);
                assert!(request.swap(false, Ordering::AcqRel));
                let ((), fresh) = tokio::join!(feed(&sender, 1), async {
                    loop {
                        let frame = frames.recv().await.unwrap();
                        if frame.capture.stamp.source_epoch == 1 {
                            break frame;
                        }
                    }
                });
                assert_eq!(
                    fresh.capture.stamp.source_incarnation,
                    frame.capture.stamp.source_incarnation
                );
                assert!(request.load(Ordering::Acquire));
                drop(sender);
                task.await.unwrap();
                while frames.try_recv().is_ok() {}
                assert!(frames.recv().await.is_none());
            })
            .await
            .unwrap();
        });
}

#[test]
#[ignore = "requires ffmpeg on PATH; run with --ignored"]
fn closing_the_output_stops_an_idle_decoder_and_joins_its_owner() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let (sender, input) = mpsc::channel(1);
            let (frames, task) = spawn(config(), input).unwrap();
            drop(frames);
            tokio::time::timeout(DEADLINE, task).await.unwrap().unwrap();
            assert!(sender.is_closed());
        });
}

#[test]
fn a_missing_decoder_fails_before_a_video_task_is_created() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let mut config = config();
            config.ffmpeg = PathBuf::from("/missing-pilotage-dji-decoder/ffmpeg");
            let (_, input) = mpsc::channel(1);
            assert!(matches!(
                spawn(config, input),
                Err(DjiVideoError::Spawn { .. })
            ));
        });
}

#[test]
#[ignore = "requires ffmpeg on PATH; run with --ignored"]
fn an_oversized_encoded_segment_closes_the_stream_without_a_frame() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let (sender, input) = mpsc::channel(1);
            let (mut frames, task) = spawn(config(), input).unwrap();
            sender
                .send(EncodedVideoChunk {
                    received_at: Duration::ZERO,
                    continuity: 0,
                    bytes: vec![0; 65536],
                })
                .await
                .unwrap();
            tokio::time::timeout(DEADLINE, task).await.unwrap().unwrap();
            assert!(frames.recv().await.is_none());
            assert!(sender.is_closed());
        });
}
