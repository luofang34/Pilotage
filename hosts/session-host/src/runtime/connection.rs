//! Per-connection transport plumbing: decodes inbound bytes into
//! [`DomainEnvelope`]s for the engine actor and writes the engine's decided
//! [`OutboundMessage`]s/datagrams back onto the wire (ADR-0005).
//!
//! One task per connected client, internally split into a reader half (the
//! bootstrap bidi stream's recv side plus datagrams) and a writer half (the
//! bootstrap bidi stream's send side plus the reliable session-events uni
//! stream), driven concurrently so a backpressured write can never stall
//! inbound control-frame or datagram servicing (ADR-0005/0011: no head-of-line
//! blocking between classes). The engine actor never touches transport types
//! directly, keeping [`SessionEngine`] sans-IO.
//!
//! [`SessionEngine`]: pilotage_session::SessionEngine

use pilotage_session::{ClientKey, DomainEnvelope};
use pilotage_timing::MonoTimestamp;
use std::ops::ControlFlow;
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;
use tracing::{debug, error, warn};
use wtransport::error::SendDatagramError;
use wtransport::{Connection, RecvStream, SendStream};

use crate::runtime::engine_actor::ToEngine;
use crate::runtime::media::MediaHandle;
use crate::runtime::registry::OUTBOUND_QUEUE_CAPACITY;
use crate::runtime::stream_tag::SESSION_EVENTS;
use crate::runtime::wire_codec::{
    InboundBootstrap, OversizedFrame, complete_frame_len, decode_bootstrap_message,
    decode_control_datagram, decode_ping_datagram, encode_video_delivery_state,
};

mod admission;
mod send_blocked;

const CONTROL_STREAM_PRIORITY: i32 = 10;

/// The ADR-0011 message class a datagram send belongs to, carried so a failed
/// transport-level send is counted and first-logged against the right class
/// (ADR-0009/0011: drops are counted per class, never silent).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatagramClass {
    /// Best-effort telemetry fanned out at tick cadence.
    Telemetry,
    /// A `Pong` reply to a client's `Ping` (RTT probe, ADR-0009).
    Pong,
    /// A `FrameRejected` notice returned to a rejected frame's sender.
    FrameRejected,
}

impl DatagramClass {
    /// Stable label for structured logging and the per-class counter table.
    fn as_str(self) -> &'static str {
        match self {
            DatagramClass::Telemetry => "telemetry",
            DatagramClass::Pong => "pong",
            DatagramClass::FrameRejected => "frame_rejected",
        }
    }
}

/// One message the engine actor asks a connection task to write out.
#[derive(Debug, Clone)]
pub enum ToConnection {
    /// Bytes to write, length-delimited, on the reliable session stream
    /// (handshake, lease, action, and adapter-enactment replies — never
    /// authority events, which use ADR-0005's session-events stream).
    BootstrapMessage {
        /// Encoded length-delimited envelope.
        bytes: Vec<u8>,
        /// Whether a successful write proves media capability negotiation.
        opens_media: bool,
    },
    /// Bytes to write, length-delimited, on the reliable session-events stream.
    AuthorityMessage(Vec<u8>),
    /// Bytes to send as a single datagram, tagged with the ADR-0011 class a
    /// failed send is counted against.
    Datagram {
        /// The datagram's message class, for per-class drop accounting.
        class: DatagramClass,
        /// The encoded datagram payload.
        bytes: Vec<u8>,
    },
    /// The engine asked this connection to close.
    Close,
}

/// Verifies the first Hello before opening the reliable session-events stream.
/// Drives the admitted connection until it closes or the engine asks it to close.
/// Runs the reader half (bootstrap-stream reads and datagrams) and the
/// writer half (bootstrap-stream and authority-stream writes plus datagram
/// sends) concurrently until either exits.
pub async fn run_connection(
    connection: Connection,
    client: ClientKey,
    start: Instant,
    to_engine: mpsc::Sender<ToEngine>,
    media: Option<MediaHandle>,
    admission: crate::runtime::Admission,
) {
    let Some(admission::AdmittedBootstrap {
        send,
        recv,
        hello,
        pending,
        may_approve,
    }) = admission::admit(&connection, &admission).await
    else {
        return;
    };
    let mut event_send = match connection.open_uni().await {
        Ok(opening) => match opening.await {
            Ok(stream) => stream,
            Err(error) => {
                warn!(%error, "session-events stream failed to open");
                return;
            }
        },
        Err(error) => {
            warn!(%error, "failed to request session-events stream");
            return;
        }
    };
    // Every host-initiated uni stream leads with its kind tag so a reader can
    // tell reliable session events from video frames; the event stream emits
    // its tag once, before any envelope.
    if event_send.write_all(&[SESSION_EVENTS]).await.is_err() {
        warn!("session-events stream closed before its kind tag was written");
        return;
    }
    send.set_priority(CONTROL_STREAM_PRIORITY);
    event_send.set_priority(CONTROL_STREAM_PRIORITY);

    let (outbound_tx, outbound_rx) = mpsc::channel(OUTBOUND_QUEUE_CAPACITY);
    if to_engine
        .send(ToEngine::ClientConnected {
            client,
            sender: outbound_tx,
        })
        .await
        .is_err()
    {
        if let Some(media) = &media {
            media.deregister(client);
        }
        return;
    }
    if !admission::introduce(&to_engine, client, hello, may_approve, start).await {
        return;
    }

    tokio::select! {
        () = run_reader(&connection, recv, pending, client, start, &to_engine, media.as_ref()) => {}
        () = run_writer(
            &connection,
            client,
            send,
            event_send,
            outbound_rx,
            media.as_ref(),
        ) => {}
        () = send_blocked::run_send_blocked_watch(&connection, client.as_u64()) => {}
    }

    if let Some(media) = &media {
        media.deregister(client);
    }
    forward(&to_engine, client, DomainEnvelope::Disconnect, start).await;
}

/// Services inbound bootstrap-stream reads and datagrams until the
/// connection closes, forwarding each decoded message to the engine actor.
///
/// Kept as its own future (not interleaved with outbound writes in one
/// `select!`) so a client that never drains its receive side cannot starve
/// this side's servicing of fresh control datagrams (finding: a
/// `send.write_all().await` sharing a select arm with `recv`/datagram would
/// park both while the write stalls).
async fn run_reader(
    connection: &Connection,
    mut recv: RecvStream,
    mut pending: Vec<u8>,
    client: ClientKey,
    start: Instant,
    to_engine: &mpsc::Sender<ToEngine>,
    media: Option<&MediaHandle>,
) {
    let mut read_buf = vec![0u8; 64 * 1024];
    if !matches!(
        drain_bootstrap_frames(&mut pending, client, start, to_engine, media, connection).await,
        Ok(ControlFlow::Continue(()))
    ) {
        return;
    }
    loop {
        tokio::select! {
            read = recv.read(&mut read_buf) => {
                match read {
                    Ok(Some(count)) => {
                        pending.extend_from_slice(&read_buf[..count]);
                        match
                            drain_bootstrap_frames(
                                &mut pending,
                                client,
                                start,
                                to_engine,
                                media,
                                connection,
                            ).await
                        {
                            Ok(ControlFlow::Continue(())) => {}
                            Ok(ControlFlow::Break(())) => break,
                            Err(error) => {
                                warn!(%error, "closing connection: oversized bootstrap frame");
                                break;
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        warn!(%error, "bootstrap stream read failed");
                        break;
                    }
                }
            }
            datagram = connection.receive_datagram() => {
                match datagram {
                    Ok(datagram) => {
                        handle_datagram(&datagram, client, start, to_engine).await;
                    }
                    Err(error) => {
                        debug!(%error, "connection closed while awaiting datagram");
                        break;
                    }
                }
            }
        }
    }
}

/// Services the engine actor's outbound queue until it closes or a write
/// fails, writing bootstrap-stream and authority-stream messages and sending
/// datagrams. Runs independently of [`run_reader`] so a slow/backpressured
/// write never delays servicing the read side.
async fn run_writer(
    connection: &Connection,
    client: ClientKey,
    mut send: SendStream,
    mut event_send: SendStream,
    mut outbound_rx: mpsc::Receiver<ToConnection>,
    media: Option<&MediaHandle>,
) {
    let mut datagram_drops = DatagramDropCounters::default();
    let mut media_status: Option<watch::Receiver<pilotage_protocol::wire::VideoDeliveryState>> =
        None;
    loop {
        let message = if let Some(status) = media_status.as_mut() {
            tokio::select! {
                message = outbound_rx.recv() => message,
                changed = status.changed() => {
                    if changed.is_err() {
                        media_status = None;
                        continue;
                    }
                    let bytes = encode_video_delivery_state(*status.borrow_and_update());
                    if event_send.write_all(&bytes).await.is_err() {
                        break;
                    }
                    continue;
                }
            }
        } else {
            outbound_rx.recv().await
        };
        let Some(message) = message else {
            break;
        };
        match message {
            ToConnection::BootstrapMessage { bytes, opens_media } => {
                if send.write_all(&bytes).await.is_err() {
                    break;
                }
                if opens_media && let Some(media) = media {
                    media_status = Some(media.register(client, connection.clone()));
                }
            }
            ToConnection::AuthorityMessage(bytes) => {
                if event_send.write_all(&bytes).await.is_err() {
                    break;
                }
            }
            ToConnection::Datagram { class, bytes } => {
                send_datagram_counted(connection, class, bytes, &mut datagram_drops);
            }
            ToConnection::Close => break,
        }
    }
}

/// Per-connection, per-datagram-class counters for transport-level send
/// failures, so a dropped telemetry/`Pong`/`FrameRejected` datagram is counted
/// against its own class and never silently discarded (ADR-0009/0011: drops
/// are counted per class).
#[derive(Debug, Default)]
struct DatagramDropCounters {
    telemetry: u64,
    pong: u64,
    frame_rejected: u64,
}

impl DatagramDropCounters {
    /// Records one drop for `class`, returning the class's running total.
    fn record(&mut self, class: DatagramClass) -> u64 {
        let counter = match class {
            DatagramClass::Telemetry => &mut self.telemetry,
            DatagramClass::Pong => &mut self.pong,
            DatagramClass::FrameRejected => &mut self.frame_rejected,
        };
        *counter = counter.wrapping_add(1);
        *counter
    }
}

/// Sends one datagram on `connection`, counting and logging a failed send
/// rather than silently discarding it (ADR-0009: "drops are counted, never
/// silent" applies to transport-level datagram loss, not only the mpsc-full
/// case the engine actor already counts).
///
/// The first drop of each class is logged at `error` (a class that cannot send
/// at all is a genuine fault worth surfacing loudly once per connection);
/// subsequent drops of that class stay at `warn` so a persistently-too-large
/// or disconnected peer does not flood the log.
fn send_datagram_counted(
    connection: &Connection,
    class: DatagramClass,
    bytes: Vec<u8>,
    drops: &mut DatagramDropCounters,
) {
    let Err(error) = connection.send_datagram(bytes) else {
        return;
    };
    let total = drops.record(class);
    let detail = match error {
        SendDatagramError::TooLarge => {
            "payload exceeds connection's datagram size limit".to_owned()
        }
        SendDatagramError::NotConnected | SendDatagramError::UnsupportedByPeer => error.to_string(),
    };
    if total == 1 {
        error!(
            class = class.as_str(),
            total_dropped = total,
            detail,
            "datagram send dropped (first for this class on this connection)"
        );
    } else {
        warn!(
            class = class.as_str(),
            total_dropped = total,
            detail,
            "datagram send dropped"
        );
    }
}

/// Sends one decoded message to the engine actor, discarding a closed-actor
/// send failure intentionally: the connection task is tearing down either
/// way, and there is no reply path to report the failure to.
async fn forward(
    to_engine: &mpsc::Sender<ToEngine>,
    client: ClientKey,
    message: DomainEnvelope,
    start: Instant,
) {
    forward_at(to_engine, client, message, now_from(start)).await;
}

/// Like [`forward`], but with an already-sampled receive timestamp, so a
/// caller that must consult `now` before forwarding (control frames, whose
/// `sampled_at` is rebased against it) does not sample the clock twice and
/// compare a frame against a `now` a few microseconds later than the one it
/// was rebased against.
async fn forward_at(
    to_engine: &mpsc::Sender<ToEngine>,
    client: ClientKey,
    message: DomainEnvelope,
    now: MonoTimestamp,
) {
    to_engine
        .send(ToEngine::ClientMessage {
            client,
            message: Box::new(message),
            now,
        })
        .await
        .ok();
}

/// Decodes as many complete length-delimited envelopes as `pending` holds,
/// forwarding each to the engine actor and leaving any partial tail buffered.
///
/// [`complete_frame_len`] gates each iteration on a full frame being
/// buffered, so a decode failure past that point is a genuinely malformed
/// envelope rather than a stream read landing mid-frame.
///
/// Returns `Err` when a client declares a frame larger than
/// [`MAX_BOOTSTRAP_FRAME_LEN`]; the caller closes the connection instead of
/// letting `pending` grow toward the attacker-chosen size.
async fn drain_bootstrap_frames(
    pending: &mut Vec<u8>,
    client: ClientKey,
    start: Instant,
    to_engine: &mpsc::Sender<ToEngine>,
    media: Option<&MediaHandle>,
    connection: &Connection,
) -> Result<ControlFlow<()>, OversizedFrame> {
    loop {
        let frame_len = match complete_frame_len(pending)? {
            Some(frame_len) => frame_len,
            None => return Ok(ControlFlow::Continue(())),
        };
        match decode_bootstrap_message(&pending[..frame_len]) {
            Ok((InboundBootstrap::Engine(message), _rest)) => {
                if matches!(*message, DomainEnvelope::Hello(_)) {
                    connection.close(
                        wtransport::VarInt::from_u32(
                            pilotage_protocol::ADMISSION_REJECTED_CLOSE_CODE,
                        ),
                        b"admission rejected",
                    );
                    pending.clear();
                    return Ok(ControlFlow::Break(()));
                }
                forward(to_engine, client, *message, start).await;
            }
            Ok((InboundBootstrap::Mission(intent), _rest)) => {
                let command = ToEngine::MissionIntent { client, intent };
                if to_engine.send(command).await.is_err() {
                    return Ok(ControlFlow::Break(()));
                }
            }
            Ok((InboundBootstrap::MediaAttach, _rest)) => {
                if let Some(media) = media {
                    media.reattach(client, connection.clone());
                    debug!(
                        client = client.as_u64(),
                        "media attachment requested on live session"
                    );
                }
            }
            Err(error) => {
                warn!(%error, "dropping malformed bootstrap-stream envelope");
            }
        }
        pending.drain(..frame_len);
    }
}

/// Decodes one datagram as either a control frame or a `Ping`, forwarding the
/// decoded message to the engine actor (control frames) or answering
/// directly (`Ping`, which is a pure echo the connection task can compute
/// without a round trip through the engine).
async fn handle_datagram(
    payload: &[u8],
    client: ClientKey,
    start: Instant,
    to_engine: &mpsc::Sender<ToEngine>,
) {
    if let Ok(frame) = decode_control_datagram(payload) {
        let now = now_from(start);
        forward_at(to_engine, client, DomainEnvelope::Frame(frame), now).await;
        return;
    }
    if let Ok(ping) = decode_ping_datagram(payload) {
        // The Pong reply flows back through the engine actor (SendToClient
        // action, ADR-0009's responder-time stamp comes from the driver's
        // shared clock), so this task only forwards the decoded `Ping`.
        forward(to_engine, client, DomainEnvelope::Ping(ping), start).await;
        return;
    }
    debug!("dropping unrecognized datagram payload");
}

fn now_from(start: Instant) -> MonoTimestamp {
    let nanos = u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX);
    MonoTimestamp::from_nanos(nanos)
}
