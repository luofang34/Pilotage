//! Bounded ownership for allocated WebTransport streams whose header flush
//! exceeds the frame deadline.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use pilotage_session::ClientKey;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::{error, warn};

use super::{FrameStream, StreamError};

pub(super) const MAX_PENDING_OPEN_REAPERS: usize = 8;

pub(super) struct OpenReapers {
    client: ClientKey,
    source_id: u8,
    slots: Arc<Semaphore>,
    active: Arc<AtomicUsize>,
    total_spawned: u64,
}

impl OpenReapers {
    pub(super) fn new(client: ClientKey, source_id: u8) -> Self {
        Self {
            client,
            source_id,
            slots: Arc::new(Semaphore::new(MAX_PENDING_OPEN_REAPERS)),
            active: Arc::new(AtomicUsize::new(0)),
            total_spawned: 0,
        }
    }

    pub(super) async fn reserve(&self) -> Option<OwnedSemaphorePermit> {
        if self.slots.available_permits() == 0 {
            warn!(
                client = self.client.as_u64(),
                source_id = self.source_id,
                pending_reapers = self.active.load(Ordering::Acquire),
                reaper_bound = MAX_PENDING_OPEN_REAPERS,
                "video open reaper bound reached; waiting before allocating another stream"
            );
        }
        self.slots.clone().acquire_owned().await.ok()
    }

    pub(super) fn own<F, S>(&mut self, completion: Pin<Box<F>>, permit: OwnedSemaphorePermit)
    where
        F: Future<Output = Result<S, StreamError>> + Send + 'static,
        S: FrameStream + Send + 'static,
    {
        self.total_spawned = self.total_spawned.wrapping_add(1);
        let reaper_id = self.total_spawned;
        let active = Arc::clone(&self.active);
        let pending = active.fetch_add(1, Ordering::AcqRel).saturating_add(1);
        let client = self.client.as_u64();
        let source_id = self.source_id;
        warn!(
            client,
            source_id,
            reaper_id,
            pending_reapers = pending,
            total_reapers = self.total_spawned,
            "video stream header flush exceeded its deadline; reaper owns allocated stream"
        );
        drop(tokio::spawn(async move {
            reap(completion, permit, active, client, source_id, reaper_id).await;
        }));
    }

    #[cfg(test)]
    pub(super) fn available_slots(&self) -> usize {
        self.slots.available_permits()
    }
}

async fn reap<F, S>(
    completion: Pin<Box<F>>,
    permit: OwnedSemaphorePermit,
    active: Arc<AtomicUsize>,
    client: u64,
    source_id: u8,
    reaper_id: u64,
) where
    F: Future<Output = Result<S, StreamError>> + Send,
    S: FrameStream,
{
    match completion.await {
        Ok(mut stream) => {
            stream.reset();
            warn!(
                client,
                source_id, reaper_id, "video open reaper reset allocated stream"
            );
        }
        Err(error) => {
            warn!(client, source_id, reaper_id, %error, "video open reaper observed opening failure");
        }
    }
    let remaining = active.fetch_sub(1, Ordering::AcqRel).saturating_sub(1);
    drop(permit);
    if remaining >= MAX_PENDING_OPEN_REAPERS {
        error!(
            client,
            source_id, reaper_id, remaining, "video open reaper count invariant failed"
        );
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic)]

    use std::sync::atomic::Ordering;

    use tokio::sync::{mpsc, oneshot};

    use super::*;

    struct MockStream {
        reset_events: mpsc::UnboundedSender<()>,
    }

    impl FrameStream for MockStream {
        async fn write_all(&mut self, _buf: &[u8]) -> Result<(), StreamError> {
            Ok(())
        }

        async fn finish(&mut self) -> Result<(), StreamError> {
            Ok(())
        }

        fn reset(&mut self) {
            self.reset_events.send(()).ok();
        }
    }

    #[tokio::test]
    async fn pending_open_reapers_are_bounded_counted_and_reset_on_completion() {
        let mut reapers = OpenReapers::new(ClientKey::new(9), 3);
        let (reset_tx, mut reset_rx) = mpsc::unbounded_channel();
        let mut releases = Vec::with_capacity(MAX_PENDING_OPEN_REAPERS);

        for _ in 0..MAX_PENDING_OPEN_REAPERS {
            let permit = reapers.reserve().await.expect("reaper slot is available");
            let (release_tx, release_rx) = oneshot::channel();
            releases.push(release_tx);
            let reset_events = reset_tx.clone();
            reapers.own(
                Box::pin(async move {
                    release_rx.await.expect("test releases the open");
                    Ok(MockStream { reset_events })
                }),
                permit,
            );
        }

        assert_eq!(reapers.available_slots(), 0, "the configured bound is hard");
        assert_eq!(
            reapers.active.load(Ordering::Acquire),
            MAX_PENDING_OPEN_REAPERS,
            "every detached open is counted"
        );
        assert_eq!(
            reapers.total_spawned, MAX_PENDING_OPEN_REAPERS as u64,
            "the lifetime count includes every detached open"
        );

        for release in releases {
            release.send(()).expect("reaper still owns the open");
        }
        for _ in 0..MAX_PENDING_OPEN_REAPERS {
            reset_rx
                .recv()
                .await
                .expect("each open resolves to a reset");
        }
    }
}
