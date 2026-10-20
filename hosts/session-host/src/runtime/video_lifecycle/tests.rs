use std::future::Future;

use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use super::{SHUTDOWN_TIMEOUT, VideoTasks};

mod runtime;

struct ExitNotice(Option<oneshot::Sender<()>>);

impl Drop for ExitNotice {
    fn drop(&mut self) {
        if let Some(notice) = self.0.take() {
            notice.send(()).ok();
        }
    }
}

#[cfg(test)]
fn block_on(future: impl Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(future);
}

#[cfg(test)]
fn pending_task(ready: mpsc::Sender<()>) -> (JoinHandle<()>, oneshot::Receiver<()>) {
    let (exited, receiver) = oneshot::channel();
    let notice = ExitNotice(Some(exited));
    let task = tokio::spawn(async move {
        let _notice = notice;
        ready.send(()).await.expect("ready receiver");
        std::future::pending::<()>().await;
    });
    (task, receiver)
}

#[test]
fn cancellation_before_the_owner_first_poll_stops_both_tasks() {
    block_on(async {
        let (ready, mut received) = mpsc::channel(2);
        let (media, media_exited) = pending_task(ready.clone());
        let (producer, producer_exited) = pending_task(ready);
        received.recv().await.expect("first started");
        received.recv().await.expect("second started");
        let tasks = VideoTasks::new(media, Some(("test-decoder", producer)));
        let owner = tokio::spawn(async move {
            std::future::pending::<()>().await;
            tasks.finish().await;
        });
        owner.abort();
        assert!(owner.await.expect_err("owner canceled").is_cancelled());
        media_exited.await.expect("media canceled");
        producer_exited.await.expect("producer canceled");
    });
}

#[test]
fn cancellation_while_joining_stops_both_tasks() {
    block_on(async {
        let (ready, mut received) = mpsc::channel(2);
        let (media, media_exited) = pending_task(ready.clone());
        let (producer, producer_exited) = pending_task(ready);
        received.recv().await.expect("first started");
        received.recv().await.expect("second started");
        let tasks = VideoTasks::new(media, Some(("test-decoder", producer)));
        let (joining, started) = oneshot::channel();
        let owner = tokio::spawn(async move {
            joining.send(()).expect("join signal");
            tasks.finish().await;
        });
        started.await.expect("joining");
        owner.abort();
        assert!(owner.await.expect_err("owner canceled").is_cancelled());
        media_exited.await.expect("media canceled");
        producer_exited.await.expect("producer canceled");
    });
}

#[test]
fn shutdown_deadline_aborts_and_joins_all_unfinished_video_tasks() {
    block_on(async {
        tokio::time::pause();
        let (ready, mut received) = mpsc::channel(2);
        let (media, media_exited) = pending_task(ready.clone());
        let (producer, producer_exited) = pending_task(ready);
        received.recv().await.expect("first started");
        received.recv().await.expect("second started");
        let tasks = VideoTasks::new(media, Some(("test-decoder", producer)));
        let start = tokio::time::Instant::now();
        tasks.finish().await;
        assert!(start.elapsed() >= SHUTDOWN_TIMEOUT);
        assert!(start.elapsed() < SHUTDOWN_TIMEOUT * 2);
        media_exited.await.expect("media canceled");
        producer_exited.await.expect("producer canceled");
    });
}

#[test]
fn a_normal_finish_joins_both_tasks_without_waiting_for_the_deadline() {
    block_on(async {
        tokio::time::pause();
        let (media_done, media_received) = oneshot::channel();
        let (producer_done, producer_received) = oneshot::channel();
        let media = tokio::spawn(async move {
            media_done.send(()).expect("media result");
        });
        let producer = tokio::spawn(async move {
            producer_done.send(()).expect("producer result");
        });
        let start = tokio::time::Instant::now();
        VideoTasks::new(media, Some(("test-decoder", producer)))
            .finish()
            .await;
        media_received.await.expect("media completed");
        producer_received.await.expect("producer completed");
        assert_eq!(start.elapsed(), std::time::Duration::ZERO);
    });
}
