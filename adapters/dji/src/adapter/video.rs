use std::sync::{Arc, atomic::AtomicBool};
use std::time::Instant;

use pilotage_adapter_api::{GIMBAL_SOURCE_ID, SourceIncarnation, VideoSource};

use super::{DjiAdapter, EncodedVideoChunk};

/// The sole encoded video subscription for one attachment.
pub struct EncodedVideoStream {
    /// Bounded stream segments; a continuity change invalidates decoder references.
    pub receiver: tokio::sync::mpsc::Receiver<EncodedVideoChunk>,
    /// Set to request recovery. Requests coalesce and are rate limited by the owner.
    pub keyframe_request: Arc<AtomicBool>,
    /// The attachment identity shared by measurements and this stream.
    pub incarnation: SourceIncarnation,
    /// Origin for host receive offsets, not the camera capture clock.
    pub origin: Instant,
}

impl DjiAdapter {
    /// Subscribe once to encoded video segments from this connection.
    ///
    /// Subscription does not certify decoded video availability. Host receive
    /// time must not be interpreted as camera sample time.
    pub fn subscribe_encoded_video(&mut self) -> Option<EncodedVideoStream> {
        if self.connection.fault.is_some() {
            return None;
        }
        let receiver = self.connection.input.subscribe_video()?;
        self.video_subscribed = true;
        Some(EncodedVideoStream {
            receiver,
            keyframe_request: self.connection.keyframes.request.clone(),
            incarnation: self.observations.incarnation(),
            origin: self.origin,
        })
    }

    pub(super) fn advertised_video_sources(&self) -> Vec<VideoSource> {
        if self.video_subscribed && self.connection.fault.is_none() {
            vec![VideoSource {
                id: GIMBAL_SOURCE_ID.to_owned(),
                description: "DJI camera".to_owned(),
            }]
        } else {
            Vec::new()
        }
    }
}
