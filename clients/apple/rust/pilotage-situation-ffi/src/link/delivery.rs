//! The delivery seam between the driver and the shell.
//!
//! The driver loop must never wait on foreign code: while a Swift
//! callback runs, the loop is not pumping the transport, the connection
//! stops returning flow-control credit, and the host's send path wedges
//! — no lease response, no action result, no authority event. The field
//! log reads "peer stopped consuming".
//!
//! One dedicated thread makes every observer call. Events queue without
//! loss; state frames and video frames are latest-wins per lane, because
//! a newer picture supersedes an older one by definition.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};

use super::observer::LinkObserver;
use super::records::LinkEvent;

/// What one delivery carries.
enum Delivery {
    Event(LinkEvent),
    StateFrame {
        frame: Vec<u8>,
        accepted_at_ms: u64,
    },
    Video {
        source_id: u8,
        codec: String,
        payload: Vec<u8>,
    },
}

/// Latest-wins slots plus a lossless event queue.
#[derive(Default)]
struct Pending {
    events: Vec<LinkEvent>,
    state_frame: Option<(Vec<u8>, u64)>,
    video: HashMap<u8, (String, Vec<u8>)>,
    closed: bool,
}

/// The driver's handle: enqueues without ever blocking on the shell.
pub(crate) struct DeliveryQueue {
    pending: Arc<(Mutex<Pending>, Condvar)>,
}

impl DeliveryQueue {
    /// Starts the delivery thread against one observer.
    pub(crate) fn start(observer: Arc<dyn LinkObserver>) -> Self {
        let pending = Arc::new((Mutex::new(Pending::default()), Condvar::new()));
        let worker = Arc::clone(&pending);
        // A plain thread, deliberately outside the tokio runtime: a slow
        // shell callback here can stall only this thread.
        std::thread::Builder::new()
            .name("pilotage-link-delivery".into())
            .spawn(move || deliver_loop(&worker, observer.as_ref()))
            .ok();
        Self { pending }
    }

    fn push(&self, delivery: Delivery) {
        let (lock, signal) = &*self.pending;
        let Ok(mut pending) = lock.lock() else {
            return;
        };
        match delivery {
            Delivery::Event(event) => pending.events.push(event),
            Delivery::StateFrame {
                frame,
                accepted_at_ms,
            } => pending.state_frame = Some((frame, accepted_at_ms)),
            Delivery::Video {
                source_id,
                codec,
                payload,
            } => {
                pending.video.insert(source_id, (codec, payload));
            }
        }
        signal.notify_one();
    }

    /// One typed event; never dropped.
    pub(crate) fn event(&self, event: LinkEvent) {
        self.push(Delivery::Event(event));
    }

    /// The newest state frame; an undelivered older one is superseded.
    pub(crate) fn state_frame(&self, frame: Vec<u8>, accepted_at_ms: u64) {
        self.push(Delivery::StateFrame {
            frame,
            accepted_at_ms,
        });
    }

    /// The newest video frame for one source; older undelivered ones are
    /// superseded.
    pub(crate) fn video(&self, source_id: u8, codec: String, payload: Vec<u8>) {
        self.push(Delivery::Video {
            source_id,
            codec,
            payload,
        });
    }
}

impl Drop for DeliveryQueue {
    fn drop(&mut self) {
        let (lock, signal) = &*self.pending;
        if let Ok(mut pending) = lock.lock() {
            pending.closed = true;
        }
        signal.notify_one();
    }
}

fn deliver_loop(shared: &(Mutex<Pending>, Condvar), observer: &dyn LinkObserver) {
    let (lock, signal) = shared;
    loop {
        let batch = {
            let Ok(mut pending) = lock.lock() else {
                return;
            };
            loop {
                if pending.closed {
                    return;
                }
                if !pending.events.is_empty()
                    || pending.state_frame.is_some()
                    || !pending.video.is_empty()
                {
                    break;
                }
                pending = match signal.wait(pending) {
                    Ok(next) => next,
                    Err(_) => return,
                };
            }
            (
                std::mem::take(&mut pending.events),
                pending.state_frame.take(),
                std::mem::take(&mut pending.video),
            )
        };
        // The lock is released: the shell can take as long as it likes
        // without holding up a single driver enqueue.
        let (events, state_frame, video) = batch;
        for event in events {
            observer.on_event(event);
        }
        if let Some((frame, accepted_at_ms)) = state_frame {
            observer.on_state_frame(frame, accepted_at_ms);
        }
        for (source_id, (codec, payload)) in video {
            observer.on_video_frame(source_id, codec, payload);
        }
    }
}
