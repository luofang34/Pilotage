//! Background task receiving inbound traffic concurrently with the send loop.
//! Telemetry, `Pong`, and `FrameRejected` all arrive as datagrams (the host's
//! control-fast/telemetry class mapping) and are routed by envelope arm; the
//! bidi-stream branch remains only to observe a post-handshake stream close.

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
    /// A decoded telemetry sample.
    Telemetry(TelemetryObservation),
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
}

/// Spawns the receiver task: one branch polls datagrams (telemetry, `Pong`,
/// `FrameRejected`), the other reads the bootstrap bidi stream's `recv` half
/// so a post-handshake stream close is observed. Named so an aborted-task
/// audit can identify it (ADR-0015).
///
/// Takes only `recv_stream`; the bidi stream's send half stays in the run
/// loop, kept open but unwritten after the handshake.
///
/// `run_start` is the shared monotonic origin the send loop also stamps from
/// (ADR-0009): receive timestamps must derive from the same `Instant` as the
/// frame/ping send timestamps, or the send/receive difference the metrics take
/// (with a zero clock offset) mixes two independent origins and skews every
/// control-to-telemetry latency and RTT sample.
pub fn spawn_receiver(
    connection: Connection,
    recv_stream: RecvStream,
    authority_stream: RecvStream,
    run_start: Instant,
    events_tx: mpsc::Sender<ReceiverEvent>,
) -> JoinHandle<()> {
    tokio::spawn(
        receiver_loop(
            connection,
            recv_stream,
            authority_stream,
            run_start,
            events_tx,
        )
        .instrument(tracing::info_span!("loopback_probe_receiver")),
    )
}

/// Runs the datagram, bidi-stream, and authority-stream receive branches
/// until the connection closes or the event channel's receiver is dropped.
async fn receiver_loop(
    connection: Connection,
    mut recv_stream: RecvStream,
    mut authority_stream: RecvStream,
    start: Instant,
    events_tx: mpsc::Sender<ReceiverEvent>,
) {
    let mut stream_buf = Vec::new();
    let mut authority_buf = Vec::new();
    let mut read_chunk = [0u8; 4096];
    let mut authority_chunk = [0u8; 4096];
    loop {
        tokio::select! {
            datagram = connection.receive_datagram() => {
                match datagram {
                    Ok(datagram) => handle_datagram(&datagram, start, &events_tx).await,
                    Err(source) => {
                        warn!(%source, "datagram receive failed; stopping receiver");
                        return;
                    }
                }
            }
            read = recv_stream.read(&mut read_chunk) => {
                match read {
                    Ok(Some(count)) => {
                        stream_buf.extend_from_slice(&read_chunk[..count]);
                        drain_stream_frames(&mut stream_buf, start, &events_tx).await;
                    }
                    Ok(None) => {
                        warn!("bidi stream closed; stopping receiver");
                        return;
                    }
                    Err(source) => {
                        warn!(%source, "bidi stream read failed; stopping receiver");
                        return;
                    }
                }
            }
            read = authority_stream.read(&mut authority_chunk) => {
                match read {
                    Ok(Some(count)) => {
                        authority_buf.extend_from_slice(&authority_chunk[..count]);
                        drain_stream_frames(&mut authority_buf, start, &events_tx).await;
                    }
                    Ok(None) => {
                        warn!("authority-events stream closed; stopping receiver");
                        return;
                    }
                    Err(source) => {
                        warn!(%source, "authority-events stream read failed; stopping receiver");
                        return;
                    }
                }
            }
        }
    }
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
async fn handle_datagram(
    datagram: &wtransport::datagram::Datagram,
    start: std::time::Instant,
    events_tx: &mpsc::Sender<ReceiverEvent>,
) {
    let received_at = elapsed_to_timestamp(start.elapsed());
    match decode_datagram_event(&datagram.payload(), received_at) {
        Ok(event) => {
            if events_tx.send(event).await.is_err() {
                warn!("event channel closed while forwarding datagram event");
            }
        }
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
        wire::envelope::Payload::TelemetrySample(sample) => Ok(ReceiverEvent::Telemetry(
            observation_from_sample(&sample, received_at),
        )),
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
async fn drain_stream_frames(
    buf: &mut Vec<u8>,
    start: std::time::Instant,
    events_tx: &mpsc::Sender<ReceiverEvent>,
) {
    loop {
        match decode_one(buf) {
            Ok((message, consumed)) => {
                buf.drain(..consumed);
                forward_stream_message(message, start, events_tx).await;
            }
            Err(_) => return,
        }
    }
}

/// Maps a decoded [`StreamMessage`] onto the [`ReceiverEvent`]s this probe
/// forwards to the run loop, discarding (with a warning) message kinds it
/// has no use for.
async fn forward_stream_message(
    message: StreamMessage,
    start: std::time::Instant,
    events_tx: &mpsc::Sender<ReceiverEvent>,
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
    if events_tx.send(event).await.is_err() {
        warn!("event channel closed while forwarding stream message");
    }
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
            },
        ));
        let event = decode_datagram_event(&bytes, MonoTimestamp::from_nanos(5)).expect("telemetry");
        assert!(matches!(event, ReceiverEvent::Telemetry(_)));
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
