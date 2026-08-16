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

/// What one host-initiated stream turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
enum StreamKind {
    /// Tag byte not yet seen.
    Untagged,
    /// Reliable session events: length-delimited envelopes follow.
    SessionEvents,
    /// A kind this engine does not consume. Bytes are discarded; the
    /// stream stays classified so it cannot be misread later.
    Unsupported(u8),
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

    /// Feeds received bytes and returns every complete session-event
    /// envelope they finish. Bytes on unsupported streams are dropped.
    pub(crate) fn receive(&mut self, stream: StreamId, bytes: &[u8]) -> Vec<wire::Envelope> {
        let entry = self
            .streams
            .entry(stream)
            .or_insert((StreamKind::Untagged, Vec::new()));
        let (kind, pending) = entry;
        let mut bytes = bytes;
        if *kind == StreamKind::Untagged {
            let Some((&tag, rest)) = bytes.split_first() else {
                return Vec::new();
            };
            *kind = match tag {
                SESSION_EVENTS_TAG => StreamKind::SessionEvents,
                other => StreamKind::Unsupported(other),
            };
            bytes = rest;
        }
        match kind {
            StreamKind::SessionEvents => {
                pending.extend_from_slice(bytes);
                drain_envelopes(pending)
            }
            _ => Vec::new(),
        }
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
