use super::*;

/// Queued reference to a Turret command, never a reusable write permission.
#[derive(Debug, Clone)]
pub struct PayloadWrite {
    pub(super) channel: PayloadChannel,
    pub(super) identity: CommandIdentity,
}

impl PartialEq for PayloadWrite {
    fn eq(&self, other: &Self) -> bool {
        self.identity == other.identity && Arc::ptr_eq(&self.channel.state, &other.channel.state)
    }
}

impl PayloadWrite {
    /// Revalidate and perform one nonblocking write while authority cannot change.
    /// Readiness waits must complete before this method is called.
    pub(crate) fn write(
        &self,
        expected: usize,
        operation: impl FnOnce() -> std::io::Result<usize>,
    ) -> std::io::Result<bool> {
        let mut state = self
            .channel
            .state
            .lock()
            .map_err(|_| std::io::Error::other("payload executor lock poisoned"))?;
        if state
            .command
            .is_none_or(|command| command.identity != self.identity)
        {
            return Ok(false);
        }
        let now = self
            .channel
            .tick(Instant::now())
            .map_err(std::io::Error::other)?;
        match state.executor.prepare_send(now) {
            Ok(Some(command)) if command.identity == self.identity => {}
            _ => {
                state.note(now);
                return Ok(false);
            }
        }
        match operation() {
            Ok(written) if written == expected => {
                let now = self
                    .channel
                    .tick(Instant::now())
                    .map_err(std::io::Error::other)?;
                let sent = state
                    .executor
                    .record_sent(self.identity, now)
                    .map_err(std::io::Error::other)?;
                state.effect(sent.current.effect);
                state.note(now);
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Err(error),
            result => {
                let now = self
                    .channel
                    .tick(Instant::now())
                    .map_err(std::io::Error::other)?;
                let failure = state
                    .executor
                    .record_write_failure(self.identity, now)
                    .map_err(std::io::Error::other)?;
                state.effect(failure.attempted.effect);
                if let Some(active) = failure.active {
                    state.effect(active.effect);
                }
                state.note(now);
                Err(result.err().unwrap_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::WriteZero, "partial MAVLink datagram")
                }))
            }
        }
    }
}
