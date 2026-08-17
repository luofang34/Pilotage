//! Classification and reassembly of host-initiated streams.
//!
//! Every host-initiated unidirectional stream leads with a one-byte kind
//! tag (ADR-0005). The engine consumes that byte, classifies the stream,
//! and reassembles length-delimited envelopes across reads. An unknown tag
//! fails that one stream closed — its bytes are discarded — without
//! touching any other stream.

use std::collections::BTreeMap;

use pilotage_protocol::wire;

use crate::event::StreamId;

/// Tag prefixing reliable session events (authority transitions and
/// video-delivery state). Mirrors the host's `stream_tag::SESSION_EVENTS`.
pub(crate) const SESSION_EVENTS_TAG: u8 = 0x01;

/// Tag prefixing a long-lived per-source video stream: repeated
/// big-endian u32 byte counts, each followed by one v2 frame body.
/// Mirrors the host's `stream_tag::VIDEO_STREAM_V3`.
pub(crate) const VIDEO_STREAM_V3_TAG: u8 = 0x04;

/// The largest video record a stream may claim. A desynchronized parse
/// reads arbitrary bytes as the length prefix, and an unbounded claim
/// makes the reassembly buffer grow at line rate forever — then the
/// one record that finally completes is handed across the FFI as a
/// gigabyte allocation the process dies inside. No real frame comes
/// within two orders of magnitude of this bound.
pub(crate) const MAX_VIDEO_RECORD_BYTES: usize = 32 * 1024 * 1024;

/// What one read of a stream produced.
#[derive(Debug, Default)]
pub(crate) struct StreamOutput {
    /// Complete session-event envelopes.
    pub envelopes: Vec<wire::Envelope>,
    /// Complete video frame bodies (v2 layout), in arrival order.
    pub video_bodies: Vec<Vec<u8>>,
    /// A video stream claimed a record of this many bytes and was
    /// failed closed for it.
    pub corrupt_video_claim: Option<usize>,
}

/// What one host-initiated stream turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
enum StreamKind {
    /// Tag byte not yet seen.
    Untagged,
    /// Reliable session events: length-delimited envelopes follow.
    SessionEvents,
    /// A per-source video stream: length-prefixed v2 frame bodies.
    Video,
    /// A kind this engine does not consume. Bytes are discarded; the
    /// stream stays classified so it cannot be misread later.
    Unsupported(u8),
    /// A stream whose framing went bad (an impossible record length).
    /// Its bytes cannot be trusted again; everything further is
    /// discarded until the host cycles the stream.
    Corrupt,
}

/// Per-stream reassembly state.
#[derive(Debug, Default)]
pub(crate) struct StreamTable {
    streams: BTreeMap<StreamId, (StreamKind, Vec<u8>)>,
}

impl StreamTable {
    /// Registers a freshly opened stream.
    pub(crate) fn opened(&mut self, stream: StreamId) {
        self.streams
            .insert(stream, (StreamKind::Untagged, Vec::new()));
    }

    /// Drops a closed stream's state.
    pub(crate) fn closed(&mut self, stream: StreamId) {
        self.streams.remove(&stream);
    }

    /// Clears every stream, for a transport loss.
    pub(crate) fn reset(&mut self) {
        self.streams.clear();
    }

    /// Feeds received bytes and returns what they completed. Bytes on
    /// unsupported streams are dropped.
    pub(crate) fn receive(&mut self, stream: StreamId, bytes: &[u8]) -> StreamOutput {
        let entry = self
            .streams
            .entry(stream)
            .or_insert((StreamKind::Untagged, Vec::new()));
        let (kind, pending) = entry;
        let mut bytes = bytes;
        let mut output = StreamOutput::default();
        if *kind == StreamKind::Untagged {
            let Some((&tag, rest)) = bytes.split_first() else {
                return output;
            };
            *kind = match tag {
                SESSION_EVENTS_TAG => StreamKind::SessionEvents,
                VIDEO_STREAM_V3_TAG => StreamKind::Video,
                other => StreamKind::Unsupported(other),
            };
            bytes = rest;
        }
        match kind {
            StreamKind::SessionEvents => {
                pending.extend_from_slice(bytes);
                output.envelopes = drain_envelopes(pending);
            }
            StreamKind::Video => {
                pending.extend_from_slice(bytes);
                match drain_video_records(pending) {
                    Ok(bodies) => output.video_bodies = bodies,
                    Err(claimed) => {
                        // Fail this one stream closed and give its
                        // memory back; other streams stand.
                        *kind = StreamKind::Corrupt;
                        pending.clear();
                        pending.shrink_to_fit();
                        output.corrupt_video_claim = Some(claimed);
                    }
                }
            }
            _ => {}
        }
        output
    }
}

/// Pops every complete `[u32 BE length][body]` video record off the
/// front of `pending`, leaving any partial tail in place.
///
/// # Errors
///
/// Returns the claimed length when it exceeds
/// [`MAX_VIDEO_RECORD_BYTES`]: the framing is broken and nothing after
/// it on this stream can be trusted.
pub(crate) fn drain_video_records(pending: &mut Vec<u8>) -> Result<Vec<Vec<u8>>, usize> {
    let mut bodies = Vec::new();
    loop {
        let Some(prefix) = pending.get(..4) else {
            return Ok(bodies);
        };
        let len = u32::from_be_bytes([prefix[0], prefix[1], prefix[2], prefix[3]]) as usize;
        if len > MAX_VIDEO_RECORD_BYTES {
            return Err(len);
        }
        let Some(body) = pending.get(4..4 + len) else {
            return Ok(bodies);
        };
        bodies.push(body.to_vec());
        pending.drain(..4 + len);
    }
}

/// Pops every complete length-delimited envelope off the front of
/// `pending`, leaving any partial tail in place. A malformed length prefix
/// cannot be told apart from an incomplete one here; the caller's timeout
/// discipline owns that case.
pub(crate) fn drain_envelopes(pending: &mut Vec<u8>) -> Vec<wire::Envelope> {
    let mut envelopes = Vec::new();
    while let Ok((envelope, rest)) =
        pilotage_protocol::decode_envelope_length_delimited(pending.as_slice())
    {
        let consumed = pending.len() - rest.len();
        pending.drain(..consumed);
        envelopes.push(envelope);
    }
    envelopes
}

/// The total bytes parked in reassembly buffers, the gauge a leak hunt
/// reads first.
impl StreamTable {
    pub(crate) fn pending_bytes(&self) -> usize {
        self.streams
            .values()
            .map(|(_, pending)| pending.len())
            .sum()
    }
}
