//! The timed control-frame send loop: samples input at `--rate` Hz, sends
//! each frame as a datagram, and correlates outbound sends with inbound
//! telemetry/rejection/pong events to fill in `RunMetrics`. `Ping` also
//! travels as a control-fast datagram (ADR-0005's class mapping); the host
//! decodes RTT pings on the datagram channel, not the bootstrap stream.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use pilotage_protocol::{
    ControlPayload, Generation, Ping, ScopeId, ScopedControlFrame, SequenceNum, SessionId,
};
use pilotage_timing::{ClockOffset, estimated_age};
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;
use tracing::warn;
use wtransport::Connection;

use crate::cli::Args;
use crate::control_source::{HidSource, elapsed_to_timestamp};
use crate::error::ProbeError;
use crate::metrics::RunMetrics;
use crate::pipeline::{Pipeline, load_radiomaster_pocket_profile};
use crate::receiver::ReceiverEvent;
use crate::synthetic;
use crate::wire_session::encode_ping_datagram;

/// Ping cadence: independent of the control-frame rate since RTT tracking
/// only needs a modest sample rate, not per-tick probing.
const PING_INTERVAL: Duration = Duration::from_millis(200);
/// A local clock never has a nonzero offset from itself: every timestamp
/// this probe compares (send vs. receive) is sampled from the same
/// process's monotonic clock (see `metrics` module doc), so every
/// `estimated_age` call in this file uses a zero offset by construction,
/// not as a stand-in for a real cross-endpoint estimate.
const ZERO_OFFSET: ClockOffset = ClockOffset::from_nanos(0);

/// One tick's outstanding control frame, kept only long enough to match it
/// against the first telemetry change that follows it, in send order.
struct PendingFrame {
    sent_at: pilotage_timing::MonoTimestamp,
}

/// Upper bound on outstanding unmatched frames queued in `SendLoopState::pending`.
/// A telemetry-update rate slower than the send rate would otherwise grow this
/// queue without limit; once full, the oldest unmatched frame is evicted and
/// counted via `RunMetrics::control_to_telemetry_backlog_dropped` rather than
/// silently discarded (ADR-0015: a lagging consumer is a correctness signal).
const PENDING_QUEUE_CAPACITY: usize = 1024;

/// Bundles the mutable per-run state `run_send_loop` threads through each
/// tick, keeping the function's own argument list within the workspace's
/// per-function limits.
struct SendLoopState {
    hid_source: Option<HidSource>,
    pipeline: Option<Pipeline>,
    sequence: SequenceNum,
    pending: VecDeque<PendingFrame>,
    last_pose: Option<(f32, f32, f32)>,
    /// Receive timestamp of the most recent telemetry sample folded in.
    /// Frames sent at or before this instant were already in flight when the
    /// last telemetry arrived without changing the pose, so a later pose
    /// change cannot causally belong to them; they are evicted rather than
    /// matched (see `handle_event`).
    last_telemetry_at: Option<pilotage_timing::MonoTimestamp>,
    ping_nonce: u64,
}

/// Runs the timed send loop for `args.seconds` at `args.rate` Hz, folding
/// every observed event into `metrics`. Returns the last sequence number
/// sent, so the caller's stale-generation probe can pick the next one.
///
/// # Errors
///
/// Returns a [`ProbeError`] if opening the HID device (`--hid`) fails, or a
/// datagram send fails outright.
#[allow(clippy::too_many_arguments)]
pub async fn run_send_loop(
    connection: &Connection,
    args: &Args,
    session: SessionId,
    generation: Generation,
    run_start: Instant,
    run_budget: Duration,
    events_rx: &mut mpsc::Receiver<ReceiverEvent>,
    metrics: &mut RunMetrics,
) -> Result<SequenceNum, ProbeError> {
    let mut state = SendLoopState {
        hid_source: args.hid.then(|| HidSource::open(run_start)).transpose()?,
        pipeline: args
            .hid
            .then(load_radiomaster_pocket_profile)
            .transpose()?
            .map(Pipeline::new),
        sequence: SequenceNum::new(0),
        pending: VecDeque::new(),
        last_pose: None,
        last_telemetry_at: None,
        ping_nonce: 0,
    };

    let period = Duration::from_secs(1)
        .checked_div(u32::try_from(args.rate).unwrap_or(u32::MAX))
        .unwrap_or(Duration::from_millis(1));
    let mut tick = tokio::time::interval(period);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut ping_tick = tokio::time::interval(PING_INTERVAL);
    ping_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);

    while run_start.elapsed() < run_budget {
        tokio::select! {
            _ = tick.tick() => {
                send_one_frame(connection, session, generation, run_start, &mut state, metrics)?;
            }
            _ = ping_tick.tick() => {
                send_ping(connection, run_start, &mut state);
            }
            Some(event) = events_rx.recv() => {
                handle_event(event, &mut state, metrics);
            }
        }
    }
    Ok(state.sequence)
}

/// Builds and sends one control frame for the next sequence number.
fn send_one_frame(
    connection: &Connection,
    session: SessionId,
    generation: Generation,
    run_start: Instant,
    state: &mut SendLoopState,
    metrics: &mut RunMetrics,
) -> Result<(), ProbeError> {
    state.sequence = state.sequence.next();
    let sampled_at = elapsed_to_timestamp(run_start.elapsed());
    let (payload, profile_revision) = build_payload(state, run_start)?;
    let frame = ScopedControlFrame {
        session,
        vehicle: pilotage_protocol::VehicleId::new(1),
        scope: ScopeId::new("vehicle.motion"),
        generation,
        sequence: state.sequence,
        sampled_at,
        profile_revision,
        payload,
    };
    let bytes = pilotage_protocol::encode_control_frame_envelope(&frame);
    connection
        .send_datagram(bytes)
        .map_err(|source| ProbeError::DatagramSend {
            message: source.to_string(),
        })?;
    metrics.frames_sent = metrics.frames_sent.saturating_add(1);
    if state.pending.len() >= PENDING_QUEUE_CAPACITY {
        state.pending.pop_front();
        metrics.control_to_telemetry_backlog_dropped = metrics
            .control_to_telemetry_backlog_dropped
            .saturating_add(1);
    }
    state.pending.push_back(PendingFrame {
        sent_at: sampled_at,
    });
    Ok(())
}

/// Logical axis ids the reference adapter's `vehicle.motion` scope accepts
/// (`throttle` and `yaw`). The RadioMaster Pocket profile normalizes all
/// eight stick/aux axes; the adapter rejects a frame carrying any axis
/// outside its scope (`UnknownAxis`), so the HID payload is filtered to these
/// before it goes on the wire.
const MOTION_SCOPE_AXES: [u16; 2] = [2, 3];

/// Produces this tick's payload and profile revision from whichever source
/// is active (`--hid` or the synthetic sine generator).
fn build_payload(
    state: &mut SendLoopState,
    run_start: Instant,
) -> Result<(ControlPayload, u32), ProbeError> {
    match (&mut state.hid_source, &mut state.pipeline) {
        (Some(source), Some(pipeline)) => {
            let sample = source.sample()?;
            let mut payload = pipeline.normalize(&sample)?;
            payload
                .axes
                .retain(|(axis, _)| MOTION_SCOPE_AXES.contains(&axis.as_u16()));
            Ok((payload, pipeline.profile_revision()))
        }
        _ => {
            // Toggle a button0 edge every 200 ticks so the run also
            // exercises edge-event encoding, not only continuous axes.
            let edge = state
                .sequence
                .as_u32()
                .is_multiple_of(200)
                .then_some(pilotage_protocol::ButtonEdge::Pressed);
            let payload = synthetic::payload_at(run_start.elapsed(), edge)?;
            Ok((payload, 0))
        }
    }
}

/// Sends one `Ping` as a control-fast datagram. The host decodes RTT pings
/// on the datagram channel (`decode_ping_datagram`), not the bootstrap
/// stream, and answers with a `Pong` datagram; reply matching happens in the
/// receiver task, which folds the `Pong` into `metrics.rtt` via
/// [`ReceiverEvent::Pong`]. A send failure only logs, since a single missed
/// ping does not abort the run.
fn send_ping(connection: &Connection, run_start: Instant, state: &mut SendLoopState) {
    state.ping_nonce = state.ping_nonce.wrapping_add(1);
    let ping = Ping {
        nonce: state.ping_nonce,
        sender_sent_at: elapsed_to_timestamp(run_start.elapsed()),
    };
    if let Err(source) = connection.send_datagram(encode_ping_datagram(&ping)) {
        warn!(%source, "ping datagram send failed");
    }
}

/// Folds one receiver-task event into `metrics`. Telemetry pose changes are
/// matched against the earliest frame sent *after* the previous telemetry
/// sample; rejections and pongs are handled independently of the pending
/// queue.
fn handle_event(event: ReceiverEvent, state: &mut SendLoopState, metrics: &mut RunMetrics) {
    match event {
        ReceiverEvent::Telemetry(observation) => {
            metrics.telemetry_received = metrics.telemetry_received.saturating_add(1);
            fold_telemetry(&observation, state, metrics);
        }
        ReceiverEvent::FrameRejected(_) => {
            metrics.frames_rejected = metrics.frames_rejected.saturating_add(1);
        }
        ReceiverEvent::Pong { pong, received_at } => {
            let rtt = estimated_age(received_at, pong.echoed_sender_sent_at, ZERO_OFFSET);
            metrics.rtt.record(rtt);
        }
        ReceiverEvent::Authority => {}
    }
}

/// Matches a telemetry observation against the frame that plausibly caused it.
///
/// A pose change can only be caused by a frame sent *after* the previous
/// telemetry sample (an earlier frame was already reflected in that sample's
/// pose). During a still period the pose is unchanged while many frames are
/// sent, so those frames pile up in `pending`; matching the oldest of them
/// (FIFO from the front) against a later pose change would record the latency
/// of a frame sent seconds earlier. Instead, evict every frame sent at or
/// before the previous telemetry's receive time first, then match the earliest
/// remaining frame — the first frame sent after the last observation, i.e. the
/// one that could have produced this change.
fn fold_telemetry(
    observation: &crate::telemetry::TelemetryObservation,
    state: &mut SendLoopState,
    metrics: &mut RunMetrics,
) {
    if let Some(watermark) = state.last_telemetry_at {
        while state
            .pending
            .front()
            .is_some_and(|frame| frame.sent_at <= watermark)
        {
            state.pending.pop_front();
            metrics.control_to_telemetry_backlog_dropped = metrics
                .control_to_telemetry_backlog_dropped
                .saturating_add(1);
        }
    }
    let changed = state.last_pose.replace(observation.pose) != Some(observation.pose);
    if changed && let Some(frame) = state.pending.pop_front() {
        let age = estimated_age(observation.received_at, frame.sent_at, ZERO_OFFSET);
        metrics.control_to_telemetry.record(age);
    }
    state.last_telemetry_at = Some(observation.received_at);
}
