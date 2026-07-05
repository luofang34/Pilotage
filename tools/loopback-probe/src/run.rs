//! Orchestrates one probe run: connect, handshake, concurrent send/receive
//! loops, the stale-generation fencing probe, and the summary print.

use std::time::{Duration, Instant};

use pilotage_protocol::{ScopedControlFrame, SequenceNum, VehicleId};
use tokio::sync::mpsc;
use tracing::{info, warn};
use wtransport::{Connection, RecvStream};

use crate::cli::Args;
use crate::error::ProbeError;
use crate::metrics::RunMetrics;
use crate::receiver::{ReceiverEvent, spawn_receiver};
use crate::sender::run_send_loop;
use crate::summary::print_summary;
use crate::transport::{client_endpoint, connect, handshake, validate_loopback_url};

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
/// channel drops the oldest-pending event and counts it (ADR-0015: a
/// lagging consumer is a correctness signal, not silent).
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

    // The host opens a dedicated uni stream toward the client for reliable
    // ordered authority events (ADR-0005); accept it so those events are read
    // off their own stream rather than left unserviced.
    let authority_stream = accept_authority_stream(&connection).await?;

    // The send loop and the receiver task must stamp send and receive
    // timestamps from the same monotonic origin (ADR-0009): the metrics
    // subtract them with a zero clock offset, so a second `Instant` origin in
    // the receiver would offset every latency and RTT sample by the gap
    // between the two origins.
    let run_start = Instant::now();

    let (events_tx, mut events_rx) = mpsc::channel(EVENT_CHANNEL_CAPACITY);
    let receiver_handle = spawn_receiver(
        connection.clone(),
        recv_stream,
        authority_stream,
        run_start,
        events_tx,
    );

    let mut metrics = RunMetrics::new(HISTOGRAM_CAPACITY);
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
    )
    .await;

    drain_pending_events(&mut events_rx, &mut metrics);
    receiver_handle.abort();

    print_summary(&metrics, fencing_confirmed);
    Ok(())
}

/// Accepts the host's dedicated authority-events uni stream (ADR-0005).
///
/// # Errors
///
/// Returns [`ProbeError::BidiStream`] if the stream cannot be accepted; the
/// host opens it immediately after the connection is established, so a failure
/// here means the session did not come up as expected.
async fn accept_authority_stream(connection: &Connection) -> Result<RecvStream, ProbeError> {
    connection
        .accept_uni()
        .await
        .map_err(|source| ProbeError::BidiStream {
            message: format!("authority-events stream not accepted: {source}"),
        })
}

/// Sends one control frame carrying a generation strictly older than the
/// lease's granted generation, then waits briefly for the matching
/// `FrameRejected` as end-to-end fencing proof.
async fn send_stale_generation_probe(
    connection: &Connection,
    session: pilotage_protocol::SessionId,
    current_generation: pilotage_protocol::Generation,
    last_sequence: SequenceNum,
    run_start: Instant,
    events_rx: &mut mpsc::Receiver<ReceiverEvent>,
    metrics: &mut RunMetrics,
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

    wait_for_rejection(events_rx, stale_sequence, metrics).await
}

/// Waits up to a short fixed deadline for a `FrameRejected` matching
/// `sequence` to arrive on the receiver channel, folding it (and any other
/// event observed while waiting) into `metrics`.
async fn wait_for_rejection(
    events_rx: &mut mpsc::Receiver<ReceiverEvent>,
    sequence: SequenceNum,
    metrics: &mut RunMetrics,
) -> bool {
    const FENCING_WAIT: Duration = Duration::from_millis(500);
    let deadline = Instant::now() + FENCING_WAIT;
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
            Ok(Some(ReceiverEvent::Telemetry(_))) => {
                metrics.telemetry_received = metrics.telemetry_received.saturating_add(1);
            }
            Ok(Some(ReceiverEvent::Pong { .. } | ReceiverEvent::Authority)) | Err(_) => {}
            Ok(None) => return false,
        }
    }
}

/// Drains any events still queued after the run loop and fencing probe
/// finish, folding trailing telemetry/rejection counts into `metrics`
/// rather than discarding them silently.
fn drain_pending_events(events_rx: &mut mpsc::Receiver<ReceiverEvent>, metrics: &mut RunMetrics) {
    while let Ok(event) = events_rx.try_recv() {
        match event {
            ReceiverEvent::Telemetry(_) => {
                metrics.telemetry_received = metrics.telemetry_received.wrapping_add(1);
            }
            ReceiverEvent::FrameRejected(_) => {
                metrics.frames_rejected = metrics.frames_rejected.saturating_add(1);
            }
            ReceiverEvent::Pong { .. } | ReceiverEvent::Authority => {}
        }
    }
}
