//! Orchestrates one probe run: connect, handshake, concurrent send/receive
//! loops, the stale-generation fencing probe, and the summary print.

use std::time::{Duration, Instant};

use pilotage_protocol::{ScopedControlFrame, SequenceNum, VehicleId};
use tokio::sync::mpsc;
use tracing::{info, warn};
use wtransport::Connection;

use crate::cli::Args;
use crate::error::ProbeError;
use crate::metrics::RunMetrics;
use crate::receiver::{ReceiverEvent, spawn_receiver};
use crate::sender::run_send_loop;
use crate::summary::print_summary;
use crate::transport::{client_endpoint, connect, handshake, validate_loopback_url};
use crate::video::VideoStats;

/// The vehicle this probe always targets; this tool has no per-run vehicle
/// selection flag (out of scope per the task), so a fixed id keeps the
/// `vehicle.motion` lease request unambiguous.
const PROBE_VEHICLE: VehicleId = VehicleId::new(1);
/// Latency-histogram capacity: comfortably above `rate * seconds` for any
/// realistic run, so eviction only ever triggers on a run long enough that
/// eviction is the intended behavior (bounded memory, ADR-0009).
const HISTOGRAM_CAPACITY: usize = 100_000;
/// Bounded channel capacity between the receiver task and the run loop;
/// sized so a burst of telemetry does not need unbounded buffering. A full
/// channel drops the newly arrived event via `try_send` and counts it
/// (`receiver::forward_event`) rather than blocking delivery of everything
/// queued behind a slow consumer (ADR-0015: a lagging consumer is a
/// correctness signal, not silent).
const EVENT_CHANNEL_CAPACITY: usize = 4096;

/// Runs the full probe: connect, handshake, timed send/receive loop, the
/// fencing probe, then prints the summary.
///
/// # Errors
///
/// Returns a [`ProbeError`] if the connection, handshake, or transport I/O
/// fails. A rejected lease or fencing probe outcome is reported in the
/// summary, not as an error, since a `FrameRejected` reply is this probe's
/// expected, successful outcome for that step.
pub async fn run(args: &Args) -> Result<(), ProbeError> {
    validate_loopback_url(&args.url)?;
    let endpoint = client_endpoint()?;
    let connection = connect(&endpoint, &args.url).await?;
    // The bootstrap send half is kept open for the whole run (dropping it
    // would half-close the stream); handshake replies and control/ping
    // traffic now flow on datagrams, so it is not written to after the
    // handshake.
    let (outcome, _send_stream, recv_stream) = handshake(&connection, PROBE_VEHICLE).await?;
    info!(
        session = outcome.welcome.session.as_u64(),
        generation = outcome.lease.generation.as_u64(),
        "vehicle.motion lease granted"
    );

    // The send loop and the receiver task must stamp send and receive
    // timestamps from the same monotonic origin (ADR-0009): the metrics
    // subtract them with a zero clock offset, so a second `Instant` origin in
    // the receiver would offset every latency and RTT sample by the gap
    // between the two origins.
    let run_start = Instant::now();

    let (events_tx, mut events_rx) = mpsc::channel(EVENT_CHANNEL_CAPACITY);
    // The receiver task accepts every host-initiated uni stream itself (the
    // dedicated authority-events stream plus one fresh stream per video
    // frame, ADR-0005), so no uni stream is accepted here.
    let (receiver_handle, dropped_events) =
        spawn_receiver(connection.clone(), recv_stream, run_start, events_tx);

    let mut metrics = RunMetrics::new(HISTOGRAM_CAPACITY);
    let mut video = VideoStats::new();
    let session = outcome.welcome.session;
    let generation = outcome.lease.generation;
    let run_budget = Duration::from_secs(args.seconds);

    let last_sequence = run_send_loop(
        &connection,
        args,
        session,
        generation,
        run_start,
        run_budget,
        &mut events_rx,
        &mut metrics,
        &mut video,
    )
    .await?;

    let fencing_confirmed = send_stale_generation_probe(
        &connection,
        session,
        generation,
        last_sequence,
        run_start,
        &mut events_rx,
        &mut metrics,
        &mut video,
    )
    .await;

    drain_pending_events(&mut events_rx, &mut metrics, &mut video);
    receiver_handle.abort();
    metrics.dropped_events = dropped_events.load(std::sync::atomic::Ordering::Relaxed);

    if let Some(dir) = &args.save_frames {
        crate::save_frames::save_proof_frames(&video, dir).await;
    }

    print_summary(&metrics, &video, run_start.elapsed(), fencing_confirmed);
    Ok(())
}

/// Sends one control frame carrying a generation strictly older than the
/// lease's granted generation, then waits briefly for the matching
/// `FrameRejected` as end-to-end fencing proof.
#[allow(clippy::too_many_arguments)]
async fn send_stale_generation_probe(
    connection: &Connection,
    session: pilotage_protocol::SessionId,
    current_generation: pilotage_protocol::Generation,
    last_sequence: SequenceNum,
    run_start: Instant,
    events_rx: &mut mpsc::Receiver<ReceiverEvent>,
    metrics: &mut RunMetrics,
    video: &mut VideoStats,
) -> bool {
    let stale_generation =
        pilotage_protocol::Generation::new(current_generation.as_u64().wrapping_sub(1));
    let stale_sequence = last_sequence.next();
    let frame = ScopedControlFrame {
        session,
        vehicle: PROBE_VEHICLE,
        scope: pilotage_protocol::ScopeId::new("vehicle.motion"),
        generation: stale_generation,
        sequence: stale_sequence,
        sampled_at: crate::control_source::elapsed_to_timestamp(run_start.elapsed()),
        profile_revision: 0,
        payload: pilotage_protocol::ControlPayload::default(),
    };
    let bytes = pilotage_protocol::encode_control_frame_envelope(&frame);
    if let Err(source) = connection.send_datagram(bytes) {
        warn!(%source, "failed to send stale-generation fencing probe frame");
        return false;
    }
    metrics.frames_sent = metrics.frames_sent.saturating_add(1);

    wait_for_rejection(events_rx, stale_sequence, metrics, video).await
}

/// Waits up to a short fixed deadline for a `FrameRejected` matching
/// `sequence` to arrive on the receiver channel, folding it (and any other
/// event observed while waiting) into `metrics`/`video`.
async fn wait_for_rejection(
    events_rx: &mut mpsc::Receiver<ReceiverEvent>,
    sequence: SequenceNum,
    metrics: &mut RunMetrics,
    video: &mut VideoStats,
) -> bool {
    const FENCING_WAIT: Duration = Duration::from_millis(500);
    let deadline = Instant::now() + FENCING_WAIT;
    let mut last_video_at: Option<Instant> = None;
    loop {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return false;
        };
        match tokio::time::timeout(remaining, events_rx.recv()).await {
            Ok(Some(ReceiverEvent::FrameRejected(rejected))) => {
                metrics.frames_rejected = metrics.frames_rejected.saturating_add(1);
                if rejected.sequence == sequence {
                    return true;
                }
            }
            Ok(Some(ReceiverEvent::Telemetry(observation))) => {
                metrics.telemetry_received = metrics.telemetry_received.saturating_add(1);
                metrics.last_pose = Some(observation.pose);
            }
            Ok(Some(ReceiverEvent::VideoFrame { source_id, jpeg })) => {
                fold_video_frame(source_id, &jpeg, video, &mut last_video_at);
            }
            Ok(Some(ReceiverEvent::Pong { .. } | ReceiverEvent::Authority)) | Err(_) => {}
            Ok(None) => return false,
        }
    }
}

/// Drains any events still queued after the run loop and fencing probe
/// finish, folding trailing telemetry/rejection/video counts into
/// `metrics`/`video` rather than discarding them silently.
fn drain_pending_events(
    events_rx: &mut mpsc::Receiver<ReceiverEvent>,
    metrics: &mut RunMetrics,
    video: &mut VideoStats,
) {
    let mut last_video_at: Option<Instant> = None;
    while let Ok(event) = events_rx.try_recv() {
        match event {
            ReceiverEvent::Telemetry(observation) => {
                metrics.telemetry_received = metrics.telemetry_received.wrapping_add(1);
                metrics.last_pose = Some(observation.pose);
            }
            ReceiverEvent::FrameRejected(_) => {
                metrics.frames_rejected = metrics.frames_rejected.saturating_add(1);
            }
            ReceiverEvent::VideoFrame { source_id, jpeg } => {
                fold_video_frame(source_id, &jpeg, video, &mut last_video_at);
            }
            ReceiverEvent::Pong { .. } | ReceiverEvent::Authority => {}
        }
    }
}

/// Folds one arrived video frame into `video`, recording the inter-arrival
/// gap against `last_video_at` (this call's own local timing, independent of
/// the frame's `received_at` field — both derive from the same wall clock, so
/// either would do; this uses a plain `Instant` since the gap is all that
/// matters here, not an absolute timestamp).
fn fold_video_frame(
    source_id: u8,
    jpeg: &[u8],
    video: &mut VideoStats,
    last_video_at: &mut Option<Instant>,
) {
    let now = Instant::now();
    let gap = last_video_at.map(|previous| now.duration_since(previous));
    *last_video_at = Some(now);
    video.record(source_id, jpeg, gap);
}
