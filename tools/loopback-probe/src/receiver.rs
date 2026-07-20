//! Background task receiving inbound traffic concurrently with the send loop.
//! Telemetry, `Pong`, and `FrameRejected` all arrive as datagrams (the host's
//! control-fast/telemetry class mapping) and are routed by envelope arm.
//!
//! Host-initiated uni streams carry two kinds of traffic (ADR-0005): the
//! dedicated authority-events stream, opened once per connection, and one
//! fresh stream per video frame (media = one uni stream per frame). Every
//! host-initiated uni stream leads with a 1-byte kind tag; this module accepts
//! every such stream in a loop and dispatches each on that tag, spawning a
//! short-lived reader task per video-frame stream so a slow video decode never
//! blocks accepting the next stream.
//!
//! Every hand-off to the run loop's bounded event channel uses `try_send`,
//! never the blocking `send().await`: the channel's consumer (`handle_event`)
//! does inline JPEG decode, and a channel that blocks its producers on a slow
//! consumer would stall delivery of everything behind the slow item —
//! telemetry and `Pong` queued behind video frames would skew latency/RTT
//! measurements instead of being counted as dropped (ADR-0015: a lagging
//! consumer is a correctness signal, not silent). A full channel drops the
//! new event and increments the shared `dropped_events` counter instead.

mod uni_stream;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{Instrument, warn};
use wtransport::{Connection, RecvStream};

use pilotage_protocol::wire;
use prost::Message;

use crate::control_source::elapsed_to_timestamp;
use crate::error::ProbeError;
use crate::telemetry::{TelemetryObservation, observation_from_sample};
use crate::wire_session::{StreamMessage, decode_one};

/// One event the receiver task hands to the run loop.
#[derive(Debug)]
pub enum ReceiverEvent {
    /// A decoded telemetry sample. Boxed: the captured provenance lanes
    /// dwarf the other variants.
    Telemetry(Box<TelemetryObservation>),
    /// A `FrameRejected` notice for a control frame this client sent.
    FrameRejected(pilotage_protocol::FrameRejected),
    /// A `Pong` reply to a `Ping` this client sent, plus the client-local
    /// receive timestamp used to fold it into the RTT estimator.
    Pong {
        /// The decoded `Pong`.
        pong: pilotage_protocol::Pong,
        /// Client-local monotonic timestamp at receipt.
        received_at: pilotage_timing::MonoTimestamp,
    },
    /// An authority/mode event read from the host's dedicated authority-events
    /// stream (ADR-0005). Counted only, to prove the stream is live.
    Authority,
    /// One raw JPEG frame body read off a per-frame video uni stream
    /// (ADR-0005 media = one uni stream per frame). Inter-arrival latency is
    /// measured by the run loop's own wall clock at the point it receives
    /// this event, not by a timestamp carried on the event itself.
    VideoFrame {
        /// Video source this frame came from: 0 = onboard FPV, 1 = chase.
        source_id: u8,
        /// The JPEG bytes (tag, source id, and length prefix already stripped).
        jpeg: Vec<u8>,
    },
}

/// Spawns the receiver task: branches poll datagrams (telemetry, `Pong`,
/// `FrameRejected`), the bootstrap bidi stream's `recv` half (to observe a
/// post-handshake stream close), and every host-initiated uni stream accepted
/// over the connection's lifetime (the dedicated authority-events stream plus
/// one fresh stream per video frame, ADR-0005). Named so an aborted-task audit
/// can identify it (ADR-0015).
///
/// Takes only `recv_stream`; the bidi stream's send half stays in the run
/// loop, kept open but unwritten after the handshake.
///
/// `run_start` is the shared monotonic origin the send loop also stamps from
/// (ADR-0009): receive timestamps must derive from the same `Instant` as the
/// frame/ping send timestamps, or the send/receive difference the metrics take
/// (with a zero clock offset) mixes two independent origins and skews every
/// control-to-telemetry latency and RTT sample.
///
/// Returns the join handle plus a shared counter of events dropped because
/// the bounded `events_tx` channel was full (see module doc); the caller
/// folds it into `RunMetrics` once the run completes.
pub fn spawn_receiver(
    connection: Connection,
    recv_stream: RecvStream,
    run_start: Instant,
    events_tx: mpsc::Sender<ReceiverEvent>,
) -> (JoinHandle<()>, Arc<AtomicU64>) {
    let dropped_events = Arc::new(AtomicU64::new(0));
    let handle = tokio::spawn(
        receiver_loop(
            connection,
            recv_stream,
            run_start,
            events_tx,
            Arc::clone(&dropped_events),
        )
        .instrument(tracing::info_span!("loopback_probe_receiver")),
    );
    (handle, dropped_events)
}

/// Forwards one event to the run loop via `try_send`, incrementing
/// `dropped_events` (and warning) if the bounded channel is full rather than
/// awaiting free capacity (see module doc).
fn forward_event(
    events_tx: &mpsc::Sender<ReceiverEvent>,
    dropped_events: &AtomicU64,
    event: ReceiverEvent,
) {
    if let Err(source) = events_tx.try_send(event) {
        dropped_events.fetch_add(1, Ordering::Relaxed);
        match source {
            mpsc::error::TrySendError::Full(_) => {
                warn!("event channel full; dropping event instead of stalling delivery");
            }
            mpsc::error::TrySendError::Closed(_) => {
                warn!("event channel closed while forwarding event");
            }
        }
    }
}

/// Runs the datagram and bidi-stream receive branches, and accepts every
/// host-initiated uni stream, until the connection closes or the event
/// channel's receiver is dropped.
async fn receiver_loop(
    connection: Connection,
    mut recv_stream: RecvStream,
    start: Instant,
    events_tx: mpsc::Sender<ReceiverEvent>,
    dropped_events: Arc<AtomicU64>,
) {
    let mut stream_buf = Vec::new();
    let mut read_chunk = [0u8; 4096];
    let mut uni_streams = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            datagram = connection.receive_datagram() => {
                match datagram {
                    Ok(datagram) => handle_datagram(&datagram, start, &events_tx, &dropped_events),
                    Err(source) => {
                        warn!(%source, "datagram receive failed; stopping receiver");
                        break;
                    }
                }
            }
            read = recv_stream.read(&mut read_chunk) => {
                match read {
                    Ok(Some(count)) => {
                        stream_buf.extend_from_slice(&read_chunk[..count]);
                        drain_stream_frames(&mut stream_buf, start, &events_tx, &dropped_events);
                    }
                    Ok(None) => {
                        warn!("bidi stream closed; stopping receiver");
                        break;
                    }
                    Err(source) => {
                        warn!(%source, "bidi stream read failed; stopping receiver");
                        break;
                    }
                }
            }
            accepted = connection.accept_uni() => {
                match accepted {
                    Ok(uni) => {
                        uni_streams.spawn(uni_stream::read_one_uni_stream(
                            uni,
                            start,
                            events_tx.clone(),
                            Arc::clone(&dropped_events),
                        ));
                    }
                    Err(source) => {
                        warn!(%source, "stopped accepting uni streams");
                        break;
                    }
                }
            }
            Some(joined) = uni_streams.join_next(), if !uni_streams.is_empty() => {
                if let Err(error) = joined {
                    warn!(%error, "uni-stream reader task panicked");
                }
            }
        }
    }
    uni_streams.shutdown().await;
}

/// Decodes one datagram and forwards it as the matching [`ReceiverEvent`],
/// dropping (with a warning) anything this probe does not expect on the
/// datagram channel.
///
/// The host carries telemetry, `Pong`, and `FrameRejected` all on the
/// control-fast/telemetry datagram channel (ADR-0005's class mapping; see
/// the host's `to_connection_message`/`RejectFrame` handling), distinguished
/// only by the envelope's payload arm — so this routes on that arm rather
/// than assuming every datagram is telemetry.
fn handle_datagram(
    datagram: &wtransport::datagram::Datagram,
    start: std::time::Instant,
    events_tx: &mpsc::Sender<ReceiverEvent>,
    dropped_events: &AtomicU64,
) {
    let received_at = elapsed_to_timestamp(start.elapsed());
    match decode_datagram_event(&datagram.payload(), received_at) {
        Ok(event) => forward_event(events_tx, dropped_events, event),
        Err(source) => warn!(%source, "dropping undecodable datagram"),
    }
}

/// Decodes one datagram envelope and maps its payload arm onto the matching
/// [`ReceiverEvent`]. `TelemetrySample`, `Pong`, and `FrameRejected` all
/// arrive on this channel from the host; any other arm is unexpected here.
fn decode_datagram_event(
    bytes: &[u8],
    received_at: pilotage_timing::MonoTimestamp,
) -> Result<ReceiverEvent, ProbeError> {
    let envelope = wire::Envelope::decode(bytes).map_err(|source| ProbeError::Decode {
        source: pilotage_protocol::DecodeError::Prost {
            message: "pilotage.v1.Envelope",
            source,
        },
    })?;
    let payload = envelope.payload.ok_or_else(|| ProbeError::Protocol {
        message: "datagram envelope carried no payload".to_string(),
    })?;
    match payload {
        wire::envelope::Payload::TelemetrySample(sample) => Ok(ReceiverEvent::Telemetry(Box::new(
            observation_from_sample(&sample, received_at),
        ))),
        wire::envelope::Payload::Pong(pong) => Ok(ReceiverEvent::Pong {
            pong: pilotage_protocol::Pong::try_from(pong).map_err(|source| ProbeError::Decode {
                source: pilotage_protocol::DecodeError::Convert(source),
            })?,
            received_at,
        }),
        wire::envelope::Payload::FrameRejected(rejected) => Ok(ReceiverEvent::FrameRejected(
            pilotage_protocol::FrameRejected::try_from(rejected).map_err(|source| {
                ProbeError::Decode {
                    source: pilotage_protocol::DecodeError::Convert(source),
                }
            })?,
        )),
        other => Err(ProbeError::Protocol {
            message: format!("unexpected envelope payload arm on datagram channel: {other:?}"),
        }),
    }
}

/// Decodes every complete length-delimited frame currently buffered from
/// the bidi stream, forwarding `FrameRejected`/`Pong` and warning on
/// anything else; leaves a trailing partial frame in `buf` for the next
/// read.
fn drain_stream_frames(
    buf: &mut Vec<u8>,
    start: std::time::Instant,
    events_tx: &mpsc::Sender<ReceiverEvent>,
    dropped_events: &AtomicU64,
) {
    loop {
        match decode_one(buf) {
            Ok((message, consumed)) => {
                buf.drain(..consumed);
                forward_stream_message(message, start, events_tx, dropped_events);
            }
            Err(_) => return,
        }
    }
}

/// Maps a decoded [`StreamMessage`] onto the [`ReceiverEvent`]s this probe
/// forwards to the run loop, discarding (with a warning) message kinds it
/// has no use for.
fn forward_stream_message(
    message: StreamMessage,
    start: std::time::Instant,
    events_tx: &mpsc::Sender<ReceiverEvent>,
    dropped_events: &AtomicU64,
) {
    let event = match message {
        StreamMessage::FrameRejected(rejected) => ReceiverEvent::FrameRejected(rejected),
        StreamMessage::Pong(pong) => ReceiverEvent::Pong {
            pong,
            received_at: elapsed_to_timestamp(start.elapsed()),
        },
        StreamMessage::AuthorityEvent(_) => ReceiverEvent::Authority,
        StreamMessage::ServerWelcome(_) | StreamMessage::LeaseResponse(_) => {
            warn!("unexpected handshake message arrived after handshake completed");
            return;
        }
    };
    forward_event(events_tx, dropped_events, event);
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{ReceiverEvent, decode_datagram_event};
    use pilotage_protocol::wire;
    use pilotage_timing::MonoTimestamp;
    use prost::Message;

    fn encode(payload: wire::envelope::Payload) -> Vec<u8> {
        wire::Envelope {
            schema_version: 1,
            payload: Some(payload),
        }
        .encode_to_vec()
    }

    #[test]
    fn routes_telemetry_arm() {
        let bytes = encode(wire::envelope::Payload::TelemetrySample(
            wire::TelemetrySample {
                vehicle: Some(wire::VehicleId { value: 1 }),
                tick: Some(wire::SimTick { value: 0 }),
                observed_at: Some(wire::MonoTimestamp { nanos: 0 }),
                pose: Some(wire::Pose2d {
                    x_m: 1.0,
                    y_m: 2.0,
                    heading_rad: 0.0,
                }),
                velocity: None,
                avionics: None,
                sim_truth: None,
                fc_state: None,
                gimbal: None,
            },
        ));
        let event = decode_datagram_event(&bytes, MonoTimestamp::from_nanos(5)).expect("telemetry");
        assert!(matches!(event, ReceiverEvent::Telemetry(_)));
    }

    #[test]
    fn round_trip_capture_retains_role_lanes() {
        // Encode -> transport bytes -> decode -> observation: the
        // capture must retain the truth and FC-state lanes with their
        // provenance intact end to end.
        let stamp = wire::MeasurementStamp {
            role: wire::SourceRole::SimulationTruth as i32,
            integrity: wire::SourceIntegrity::Unprotected as i32,
            source_id: 1,
            source_epoch: 2,
            sequence: 40,
            acquired_at_ns: 1_000_000,
            clock: wire::MeasurementClock::Simulation as i32,
            source_incarnation: vec![0x11; 16],
        };
        let fc_stamp = wire::MeasurementStamp {
            role: wire::SourceRole::FcState as i32,
            integrity: wire::SourceIntegrity::ChecksummedOnly as i32,
            source_id: (255 << 8) | 190,
            clock: wire::MeasurementClock::HostMonotonic as i32,
            ..stamp.clone()
        };
        let bytes = encode(wire::envelope::Payload::TelemetrySample(
            wire::TelemetrySample {
                vehicle: Some(wire::VehicleId { value: 1 }),
                tick: Some(wire::SimTick { value: 1_000_000 }),
                observed_at: Some(wire::MonoTimestamp { nanos: 5 }),
                pose: None,
                velocity: None,
                avionics: None,
                sim_truth: Some(Box::new(wire::SimTruthState {
                    pos_n_m: 2.0,
                    pos_e_m: 1.0,
                    pos_d_m: -3.0,
                    valid_flags: 0b1101,
                    stamp: Some(stamp),
                    ..Default::default()
                })),
                fc_state: Some(Box::new(wire::FcState {
                    arm_state: 2,
                    stamp: Some(fc_stamp),
                })),
                gimbal: None,
            },
        ));
        let event = decode_datagram_event(&bytes, MonoTimestamp::from_nanos(9)).expect("telemetry");
        let ReceiverEvent::Telemetry(observation) = event else {
            panic!("expected telemetry, got {event:?}");
        };
        let truth = observation.sim_truth.expect("truth captured");
        assert_eq!(truth.pos_ned_m, (2.0, 1.0, -3.0));
        assert_eq!(
            truth.provenance.role,
            wire::SourceRole::SimulationTruth as i32
        );
        assert_eq!(
            truth.provenance.integrity,
            wire::SourceIntegrity::Unprotected as i32
        );
        let fc_state = observation.fc_state.expect("fc state captured");
        assert_eq!(fc_state.arm_state, 2);
        assert_eq!(fc_state.provenance.source_id, (255 << 8) | 190);
        assert_eq!(
            fc_state.provenance.clock,
            wire::MeasurementClock::HostMonotonic as i32
        );
    }

    #[test]
    fn routes_pong_arm() {
        let bytes = encode(wire::envelope::Payload::Pong(wire::Pong {
            nonce: 7,
            echoed_sender_sent_at: Some(wire::MonoTimestamp { nanos: 1 }),
            responder_sent_at: Some(wire::MonoTimestamp { nanos: 2 }),
        }));
        let event = decode_datagram_event(&bytes, MonoTimestamp::from_nanos(9)).expect("pong");
        match event {
            ReceiverEvent::Pong { pong, received_at } => {
                assert_eq!(pong.nonce, 7);
                assert_eq!(received_at, MonoTimestamp::from_nanos(9));
            }
            _ => panic!("expected Pong"),
        }
    }

    #[test]
    fn routes_frame_rejected_arm() {
        let bytes = encode(wire::envelope::Payload::FrameRejected(
            wire::FrameRejected {
                vehicle: Some(wire::VehicleId { value: 1 }),
                scope: Some(wire::ScopeId {
                    value: "vehicle.motion".to_string(),
                }),
                sequence: Some(wire::SequenceNum { value: 42 }),
                current_generation: Some(wire::Generation { value: 3 }),
                reason: wire::FrameRejectionReason::StaleGeneration as i32,
            },
        ));
        let event =
            decode_datagram_event(&bytes, MonoTimestamp::from_nanos(0)).expect("frame rejected");
        match event {
            ReceiverEvent::FrameRejected(rejected) => {
                assert_eq!(rejected.sequence.as_u32(), 42);
            }
            _ => panic!("expected FrameRejected"),
        }
    }

    #[test]
    fn rejects_unexpected_arm() {
        let bytes = encode(wire::envelope::Payload::Ping(wire::Ping {
            nonce: 1,
            sender_sent_at: Some(wire::MonoTimestamp { nanos: 0 }),
        }));
        let err = decode_datagram_event(&bytes, MonoTimestamp::from_nanos(0))
            .expect_err("Ping is not a datagram-channel arm");
        assert!(matches!(err, super::ProbeError::Protocol { .. }));
    }
}
