use super::{DjiVideoConfig, FRAME_CAPACITY, SHUTDOWN_TIMEOUT};
use pilotage_adapter_api::RawVideoFrame;
use pilotage_adapter_dji::EncodedVideoStream;
use std::path::PathBuf;
use tokio::{sync::mpsc, task::JoinHandle};

pub(in crate::runtime) fn supervise(
    ffmpeg: PathBuf,
    mut updates: mpsc::Receiver<Option<EncodedVideoStream>>,
) -> (mpsc::Receiver<RawVideoFrame>, JoinHandle<()>) {
    let (output, frames) = mpsc::channel(FRAME_CAPACITY);
    let task = tokio::spawn(async move {
        let mut current: Option<Active> = None;
        loop {
            let update = match &mut current {
                None => updates.recv().await,
                Some(active) => tokio::select! {
                    update = updates.recv() => update,
                    frame = receive(&mut active.frames) => {
                        if let Some(frame) = frame {
                            // The client cannot hold the USB owner or grow the video queue.
                            match output.try_send(frame) {
                                Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => {},
                                Err(mpsc::error::TrySendError::Closed(_)) => break,
                            }
                        } else { stop(&mut current).await; }
                        continue;
                    }
                },
            };
            stop(&mut current).await;
            let Some(update) = update else {
                break;
            };
            let Some(stream) = update else {
                continue;
            };
            let config = DjiVideoConfig {
                ffmpeg: ffmpeg.clone(),
                incarnation: stream.incarnation,
                origin: tokio::time::Instant::from_std(stream.origin),
                keyframe_request: stream.keyframe_request,
            };
            match super::spawn(config, stream.receiver) {
                Ok((frames, task)) => current = Some(Active { frames, task }),
                Err(error) => tracing::error!(%error, "DJI video decoder could not start"),
            }
        }
        stop(&mut current).await;
    });
    (frames, task)
}

async fn receive(frames: &mut mpsc::Receiver<RawVideoFrame>) -> Option<RawVideoFrame> {
    frames.recv().await
}

struct Active {
    frames: mpsc::Receiver<RawVideoFrame>,
    task: JoinHandle<()>,
}

impl Drop for Active {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn stop(current: &mut Option<Active>) {
    if let Some(mut active) = current.take() {
        active.frames.close();
        if tokio::time::timeout(SHUTDOWN_TIMEOUT, &mut active.task)
            .await
            .is_err()
        {
            tracing::error!(task = "dji-video-decoder", "decoder did not exit; aborting");
            active.task.abort();
            (&mut active.task).await.ok();
        }
    }
}
