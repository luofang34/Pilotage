#![allow(clippy::expect_used, clippy::panic)]
use super::encode_jpeg;
use crate::runtime::stream_tag::{FOURCC_MJPEG, VIDEO_FRAME_V2, frame_video_payload_v2};
use pilotage_adapter_api::{
    CalibrationId, CameraId, CaptureClockMapping, MeasurementClock, MeasurementStamp,
    SourceIncarnation, SourceIntegrity, SourceRole, VideoCaptureStamp,
};
use pilotage_adapter_gazebo::RawVideoFrame;
use pilotage_timing::SimTick;

fn capture_stamp() -> VideoCaptureStamp {
    VideoCaptureStamp {
        stamp: MeasurementStamp {
            role: SourceRole::VideoCapture,
            integrity: SourceIntegrity::Unprotected,
            source_id: 1,
            source_incarnation: SourceIncarnation::new([5; 16]),
            source_epoch: 0,
            sequence: 3,
            acquired_at_ns: 999,
            clock: MeasurementClock::Simulation,
        },
        camera_id: CameraId(1),
        calibration_id: CalibrationId::NONE,
        mapping: CaptureClockMapping::identity(MeasurementClock::Simulation),
    }
}

/// Builds a synthetic RGB frame with a simple gradient so the encoder has
/// real (non-constant) pixel data to work with.
fn synthetic_rgb(width: u32, height: u32) -> RawVideoFrame {
    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height {
        for x in 0..width {
            rgb.push((x % 256) as u8);
            rgb.push((y % 256) as u8);
            rgb.push(((x + y) % 256) as u8);
        }
    }
    RawVideoFrame {
        source_id: 0,
        width,
        height,
        pixel_format: "RGB_INT8".to_owned(),
        tick: SimTick::new(0),
        rgb,
        capture: capture_stamp(),
    }
}

#[test]
fn encodes_frame_and_v2_body_carries_the_jpeg() {
    let frame = synthetic_rgb(16, 12);
    let jpeg = encode_jpeg(&frame).expect("synthetic RGB frame encodes to JPEG");
    // A JPEG stream begins with the SOI marker 0xFFD8 and ends with EOI
    // 0xFFD9; check both so a garbage encode is caught.
    assert_eq!(&jpeg[..2], &[0xFF, 0xD8], "JPEG starts with SOI");
    assert_eq!(&jpeg[jpeg.len() - 2..], &[0xFF, 0xD9], "JPEG ends with EOI");

    // Frame the JPEG exactly as the media task writes it after the tag: a
    // v2 capture-identity body (ADR-0020). The full on-wire unit leads with
    // the v2 kind tag, then the header, codec, length prefix, and payload.
    let body = frame_video_payload_v2(1, &frame.capture, 10, 20, FOURCC_MJPEG, &jpeg)
        .expect("JPEG frames into a v2 body");
    assert_eq!(body[0], 1, "leads with the chase source id");
    assert_eq!(
        &body[body.len() - jpeg.len()..],
        jpeg.as_slice(),
        "JPEG trails intact"
    );

    let mut wire = vec![VIDEO_FRAME_V2];
    wire.extend_from_slice(&body);
    assert_eq!(wire[0], VIDEO_FRAME_V2, "leads with the v2 video kind tag");
}

#[test]
fn non_rgb_frame_is_skipped() {
    let mut frame = synthetic_rgb(4, 4);
    frame.pixel_format = "BGR_INT8".to_owned();
    assert!(encode_jpeg(&frame).is_none());
}

#[test]
fn a_writer_exit_respawns_within_the_bound_then_retires() {
    use super::{MAX_WRITER_RESPAWNS, SinkAction, on_writer_exit};
    for exits in 0..MAX_WRITER_RESPAWNS {
        assert_eq!(
            on_writer_exit(exits),
            SinkAction::Respawn,
            "exit {exits} respawns"
        );
    }
    assert_eq!(on_writer_exit(MAX_WRITER_RESPAWNS), SinkAction::Retire);
    assert_eq!(on_writer_exit(MAX_WRITER_RESPAWNS + 1), SinkAction::Retire);
}

mod sink_transitions {
    use std::sync::Arc;

    use tokio::sync::mpsc;

    use super::super::{
        ClientSink, EncodedFrame, MAX_WRITER_RESPAWNS, SinkDelivery, deliver_to_sink, fully_retired,
    };
    use super::capture_stamp;

    fn encoded() -> EncodedFrame {
        EncodedFrame {
            jpeg: Arc::new(vec![0xff, 0xd8]),
            capture: capture_stamp(),
            received_at_ns: 1,
        }
    }

    fn live_sink(writer_exits: u32) -> (ClientSink, mpsc::Receiver<EncodedFrame>) {
        let (frames, rx) = mpsc::channel(1);
        (
            ClientSink::Live {
                frames,
                dropped: 0,
                writer_exits,
            },
            rx,
        )
    }

    fn no_respawn(_exits: u32) -> ClientSink {
        panic!("this transition must not respawn")
    }

    #[test]
    fn a_live_sink_delivers_then_counts_a_full_slot() {
        let (mut sink, mut rx) = live_sink(0);
        assert_eq!(
            deliver_to_sink(&mut sink, &encoded(), no_respawn),
            SinkDelivery::Delivered
        );
        assert_eq!(
            deliver_to_sink(&mut sink, &encoded(), no_respawn),
            SinkDelivery::DroppedFull(1),
            "a busy writer drops-to-latest and counts"
        );
        assert!(rx.try_recv().is_ok(), "the delivered frame is in the slot");
    }

    #[test]
    fn a_closed_sink_respawns_and_the_frame_reaches_the_fresh_writer() {
        let (mut sink, rx) = live_sink(0);
        drop(rx); // the writer task exited
        let mut fresh_rx = None;
        let delivery = deliver_to_sink(&mut sink, &encoded(), |exits| {
            let (fresh, rx) = live_sink(exits);
            fresh_rx = Some(rx);
            fresh
        });
        assert_eq!(delivery, SinkDelivery::Respawned(1));
        assert!(
            fresh_rx.expect("respawned").try_recv().is_ok(),
            "the triggering frame is not lost: the fresh writer gets it"
        );
    }

    #[test]
    fn the_exit_bound_retires_the_source_and_a_retired_sink_skips() {
        let (mut sink, rx) = live_sink(MAX_WRITER_RESPAWNS);
        drop(rx);
        assert_eq!(
            deliver_to_sink(&mut sink, &encoded(), no_respawn),
            SinkDelivery::Retired(MAX_WRITER_RESPAWNS)
        );
        assert!(matches!(sink, ClientSink::Retired));
        assert_eq!(
            deliver_to_sink(&mut sink, &encoded(), no_respawn),
            SinkDelivery::Skipped
        );
    }

    #[test]
    fn a_client_is_fully_retired_only_when_every_source_is() {
        let mut sources = std::collections::BTreeMap::new();
        assert!(!fully_retired(&sources), "no sources is not retirement");
        let (live, _rx) = live_sink(0);
        sources.insert(0u8, ClientSink::Retired);
        sources.insert(1u8, live);
        assert!(
            !fully_retired(&sources),
            "one live source keeps the client (other sources survive a retirement)"
        );
        sources.insert(1u8, ClientSink::Retired);
        assert!(fully_retired(&sources));
    }
}
