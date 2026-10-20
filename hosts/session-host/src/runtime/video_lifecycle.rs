//! Keep video tasks owned through graceful shutdown and cancellation.

use tokio::task::JoinHandle;

use super::shutdown::SHUTDOWN_TIMEOUT;

struct NamedTask {
    name: &'static str,
    task: JoinHandle<()>,
    joined: bool,
}

pub(super) struct VideoTasks {
    tasks: Vec<NamedTask>,
}

impl VideoTasks {
    pub(super) fn new(
        media: JoinHandle<()>,
        producer: Option<(&'static str, JoinHandle<()>)>,
    ) -> Self {
        let mut tasks = Vec::with_capacity(2);
        if let Some((name, task)) = producer {
            tasks.push(NamedTask {
                name,
                task,
                joined: false,
            });
        }
        tasks.push(NamedTask {
            name: "media",
            task: media,
            joined: false,
        });
        Self { tasks }
    }

    pub(super) async fn finish(mut self) {
        if tokio::time::timeout(SHUTDOWN_TIMEOUT, self.join_all())
            .await
            .is_err()
        {
            for task in &self.tasks {
                if !task.joined && !task.task.is_finished() {
                    tracing::error!(task = task.name, "video shutdown timed out; aborting task");
                    task.task.abort();
                }
            }
            self.join_all().await;
        }
    }

    async fn join_all(&mut self) {
        for task in &mut self.tasks {
            if task.joined {
                continue;
            }
            match (&mut task.task).await {
                Ok(()) => tracing::debug!(task = task.name, "video task joined"),
                Err(error) if error.is_cancelled() => {
                    tracing::debug!(task = task.name, "video task cancellation joined");
                }
                Err(error) => {
                    tracing::error!(task = task.name, %error, "video task failed");
                }
            }
            task.joined = true;
        }
    }
}

impl Drop for VideoTasks {
    fn drop(&mut self) {
        for task in &self.tasks {
            if !task.joined && !task.task.is_finished() {
                tracing::warn!(task = task.name, "video owner dropped; aborting task");
                task.task.abort();
            }
        }
    }
}

#[cfg(test)]
mod tests;
