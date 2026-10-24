//! The judgment worker: one sampled picture at a time goes to the decision
//! service on a blocking thread; the answer comes back to the lane. A
//! failed or slow judgment is no judgment; it never blocks the lane.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use pilotage_adapter_api::RawVideoFrame;
use pilotage_decision::survey::{FrameContext, FrameJudgment, frame_request, read_frame};
use pilotage_decision::{DecisionClient, Provider};
use pilotage_mission::FrameTicket;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tracing::{debug, warn};

/// The longest one judgment may take.
const JUDGMENT_TIMEOUT: Duration = Duration::from_secs(20);
/// JPEG quality of the picture the service sees.
const JPEG_QUALITY: u8 = 85;

/// A picture's ticket and its judgment, or `None` when the service did not
/// judge it.
type Judged = (FrameTicket, Option<FrameJudgment>);

/// Sends sampled pictures for judgment, one at a time.
pub(super) struct Judge {
    client: Arc<DecisionClient>,
    busy: Arc<AtomicBool>,
    every: Duration,
    height_m: f64,
    last: Option<Instant>,
    results: mpsc::Sender<Judged>,
    receiver: Option<mpsc::Receiver<Judged>>,
}

impl Judge {
    pub(super) fn new(provider: Provider, every: Duration, height_m: f64) -> Self {
        let (results, receiver) = mpsc::channel(8);
        Self {
            client: Arc::new(DecisionClient::new(provider, JUDGMENT_TIMEOUT)),
            busy: Arc::new(AtomicBool::new(false)),
            every,
            height_m,
            last: None,
            results,
            receiver: Some(receiver),
        }
    }

    /// The receiver of answers; the lane polls it.
    pub(super) fn take_results(&mut self) -> mpsc::Receiver<Judged> {
        self.receiver.take().unwrap_or_else(|| mpsc::channel(1).1)
    }

    /// Sends the picture `ticket` names when none is in flight and the
    /// period passed, or at once when `needed`; whether it was sent. A sent
    /// picture always gets an answer.
    pub(super) fn offer(
        &mut self,
        ticket: FrameTicket,
        frame: Arc<RawVideoFrame>,
        needed: bool,
    ) -> bool {
        if (!needed && self.last.is_some_and(|at| at.elapsed() < self.every))
            || self.busy.swap(true, Ordering::AcqRel)
        {
            return false;
        }
        self.last = Some(Instant::now());
        let (client, busy, results) = (
            Arc::clone(&self.client),
            Arc::clone(&self.busy),
            self.results.clone(),
        );
        let context = FrameContext {
            task_id: ticket.task_id.clone(),
            height_m: self.height_m,
            nadir: true,
        };
        tokio::task::spawn_blocking(move || {
            let outcome = jpeg(&frame).and_then(|bytes| {
                let request = frame_request(bytes, &context);
                client
                    .decide_blocking(&request)
                    .map_err(|e| e.to_string())
                    .and_then(|response| {
                        read_frame(&response)
                            .ok_or_else(|| "the answer has no frame judgment".to_owned())
                    })
            });
            busy.store(false, Ordering::Release);
            let judgment = match outcome {
                Ok(judgment) => {
                    debug!(task = %ticket.task_id, frame = ticket.frame, usable = judgment.usable, hazard = judgment.hazard, cover = ?judgment.cover, "picture judged");
                    Some(judgment)
                }
                Err(error) => {
                    warn!(%error, task = %ticket.task_id, frame = ticket.frame, "picture not judged");
                    None
                }
            };
            // One picture is in flight at a time, so the channel has room.
            if results.try_send((ticket, judgment)).is_err() {
                warn!("the lane no longer takes judgments");
            }
        });
        true
    }
}

/// The next answer, or never without a judge.
pub(super) async fn next(results: &mut Option<mpsc::Receiver<Judged>>) -> Option<Judged> {
    match results.as_mut() {
        Some(receiver) => receiver.recv().await,
        None => std::future::pending().await,
    }
}

fn jpeg(frame: &RawVideoFrame) -> Result<Vec<u8>, String> {
    let (width, height) = (
        u16::try_from(frame.width).map_err(|_| "picture too wide")?,
        u16::try_from(frame.height).map_err(|_| "picture too tall")?,
    );
    let mut bytes = Vec::new();
    jpeg_encoder::Encoder::new(&mut bytes, JPEG_QUALITY)
        .encode(&frame.rgb, width, height, jpeg_encoder::ColorType::Rgb)
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}
