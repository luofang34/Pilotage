//! One receive owner for the command and encoded video streams.

use std::time::Duration;

use crate::logiclink::{CONTROL_CHANNEL, LogicLinkParser, VIDEO_CHANNEL};
use crate::{DumlFrame, DumlParser};

use super::DjiFault;

// Small USB envelopes need enough slots for a startup burst before decoding.
// Each envelope is at most 65535 bytes, so queued payloads remain below 4 MiB.
const VIDEO_BUDGET_CHUNKS: usize = 64;

/// Encoded video bytes received from one DJI connection.
///
/// This is a stream segment, not an image or a complete NAL unit. The decoder
/// must select the codec for its verified device profile. No camera calibration
/// or device sample time is implied.
#[derive(Debug)]
pub struct EncodedVideoChunk {
    /// Host receive time relative to this adapter's connection start.
    pub received_at: Duration,
    /// Changes when a bounded receive queue discards stream data.
    ///
    /// A decoder must discard reference state when this changes and wait for a
    /// new independently decodable frame. This value is local to one connection.
    pub continuity: u64,
    /// Exact bytes from the video channel.
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Framing {
    #[default]
    Duml,
    LogicLink,
}

impl Framing {
    pub(super) fn probe_sender(self) -> u8 {
        match self {
            Self::Duml => 10,
            Self::LogicLink => 2,
        }
    }

    pub(super) fn rc_version_request(self) -> [u8; 13] {
        let mut request = turret_dji::rc_version_request();
        // LogicLink replies return to the mobile-app address, while CDC uses the PC address.
        request[4] = self.probe_sender();
        let checksum = turret_dji::frame_checksum(&request[..11]).to_le_bytes();
        request[11..].copy_from_slice(&checksum);
        request
    }

    pub(super) fn command(self, bytes: &[u8]) -> Vec<u8> {
        match self {
            Self::Duml => bytes.to_vec(),
            Self::LogicLink => crate::logiclink::encode_duml(bytes),
        }
    }
}

pub(super) struct Input {
    framing: Framing,
    mux: Option<LogicLinkParser>,
    duml: DumlParser,
    video: VideoQueue,
    unknown_envelopes: u64,
}

impl Input {
    pub(super) fn new(framing: Framing) -> Self {
        Self {
            framing,
            mux: (framing == Framing::LogicLink).then(LogicLinkParser::new),
            duml: DumlParser::new(),
            video: VideoQueue::default(),
            unknown_envelopes: 0,
        }
    }

    pub(super) fn framing(&self) -> Framing {
        self.framing
    }

    pub(super) fn command_boundary(&self) -> bool {
        // An incomplete video envelope does not make the DUML stream partial.
        self.duml.buffered_bytes() == 0
            && self.mux.as_ref().is_none_or(|mux| {
                mux.buffered_bytes() == 0 || mux.pending_channel() == Some(VIDEO_CHANNEL)
            })
    }

    pub(super) fn receive(
        &mut self,
        bytes: &[u8],
        received_at: Duration,
    ) -> Result<Vec<DumlFrame>, DjiFault> {
        let before = self.duml.stats();
        let mut commands = Vec::new();
        if let Some(mux) = &mut self.mux {
            mux.push(bytes, |frame| match frame.channel {
                CONTROL_CHANNEL => self
                    .duml
                    .push(frame.payload, |command| commands.push(command)),
                VIDEO_CHANNEL => self.video.receive(frame.payload, received_at),
                _ => self.unknown_envelopes = self.unknown_envelopes.wrapping_add(1),
            })
            .map_err(DjiFault::LogicLink)?;
        } else {
            self.duml.push(bytes, |frame| commands.push(frame));
        }
        if before.discarded_bytes != self.duml.stats().discarded_bytes
            || commands.iter().any(|frame| frame.version() != 1)
        {
            return Err(DjiFault::Framing);
        }
        Ok(commands)
    }

    #[cfg(test)]
    pub(super) fn take_video(&mut self) -> Option<EncodedVideoChunk> {
        self.video.receiver.as_mut()?.try_recv().ok()
    }

    pub(super) fn subscribe_video(
        &mut self,
    ) -> Option<tokio::sync::mpsc::Receiver<EncodedVideoChunk>> {
        (self.framing == Framing::LogicLink)
            .then(|| self.video.receiver.take())
            .flatten()
    }

    pub(super) fn close(&mut self) {
        self.video.sender = None;
    }

    pub(super) fn dropped_video_bytes(&self) -> u64 {
        self.video.dropped_bytes
    }

    pub(super) fn unknown_envelopes(&self) -> u64 {
        self.unknown_envelopes
    }
}

struct VideoQueue {
    sender: Option<tokio::sync::mpsc::Sender<EncodedVideoChunk>>,
    receiver: Option<tokio::sync::mpsc::Receiver<EncodedVideoChunk>>,
    continuity: u64,
    dropped_bytes: u64,
}

impl Default for VideoQueue {
    fn default() -> Self {
        let (sender, receiver) = tokio::sync::mpsc::channel(VIDEO_BUDGET_CHUNKS);
        Self {
            sender: Some(sender),
            receiver: Some(receiver),
            continuity: 0,
            dropped_bytes: 0,
        }
    }
}

impl VideoQueue {
    fn receive(&mut self, bytes: &[u8], received_at: Duration) {
        let Some(sender) = &self.sender else {
            return;
        };
        if bytes.is_empty() || sender.is_closed() {
            return;
        }
        let chunk = EncodedVideoChunk {
            received_at,
            continuity: self.continuity,
            bytes: bytes.to_vec(),
        };
        if let Err(tokio::sync::mpsc::error::TrySendError::Full(chunk)) = sender.try_send(chunk) {
            self.dropped_bytes = self.dropped_bytes.wrapping_add(chunk.bytes.len() as u64);
            self.continuity = self.continuity.wrapping_add(1);
            tracing::warn!(
                dropped_bytes = self.dropped_bytes,
                continuity = self.continuity,
                "DJI encoded video queue overflow; decoder must reacquire a keyframe"
            );
        }
    }
}

#[cfg(test)]
mod tests;
