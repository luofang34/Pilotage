use std::sync::{Arc, Mutex};
use std::time::Instant;

/// A complete transport-write fact, independent of device acknowledgement.
#[derive(Debug, Default, Clone)]
pub struct WriteReceipt(Arc<Mutex<Option<Instant>>>);

impl PartialEq for WriteReceipt {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl WriteReceipt {
    /// Read a retained write time without refreshing it.
    #[must_use]
    pub fn sent_at_cached(&self) -> Option<Instant> {
        self.0.lock().ok().and_then(|value| *value)
    }

    pub(crate) fn record_sent(&self, at: Instant) {
        if let Ok(mut value) = self.0.lock() {
            *value = Some(at);
        }
    }
}
