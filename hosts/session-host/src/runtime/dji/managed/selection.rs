#[derive(Default)]
pub(super) struct Selection {
    attempted: Option<u64>,
    deferred: Option<u64>,
}

impl Selection {
    pub(super) fn observe(&mut self, ids: &[u64]) -> Option<u64> {
        let [id] = ids else {
            if ids.len() > 1 {
                tracing::warn!(
                    count = ids.len(),
                    "multiple DJI attachments; selection required"
                );
            }
            self.deferred = None;
            return None;
        };
        if self.attempted == Some(*id) {
            return self.deferred.take();
        }
        self.attempted = Some(*id);
        self.deferred = None;
        Some(*id)
    }

    pub(super) fn defer(&mut self, selected: Option<u64>) {
        self.deferred = selected;
    }
    pub(super) fn take_deferred(&mut self) -> Option<u64> {
        self.deferred.take()
    }
}

#[cfg(test)]
mod tests;
