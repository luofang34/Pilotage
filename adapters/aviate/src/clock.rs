//! The adapter's time source. A process host reads the monotonic clock;
//! an embedding host, a simulator or a browser page, sets the time it
//! steps on before each call, so the adapter reads no clock of its own.

use web_time::Instant;

/// Where the adapter's monotonic time comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterClock {
    /// The process monotonic clock.
    System,
    /// A time the host sets with [`Self::set`]; it moves only when the
    /// host moves it.
    Manual(Instant),
}

impl AdapterClock {
    /// The current time on this clock.
    #[must_use]
    pub fn now(&self) -> Instant {
        match self {
            Self::System => Instant::now(),
            Self::Manual(at) => *at,
        }
    }

    /// Sets a manual clock to `now`; a system clock does not change.
    pub fn set(&mut self, now: Instant) {
        if let Self::Manual(at) = self {
            *at = now;
        }
    }
}
