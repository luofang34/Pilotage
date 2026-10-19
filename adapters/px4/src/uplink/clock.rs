//! The uplinks' time source; tests substitute a manually advanced instant
//! so sequencing is exercised without real-time sleeps.

use std::time::Instant;

#[derive(Debug)]
pub(crate) enum UplinkClock {
    System,
    #[cfg(test)]
    Manual(Instant),
}

impl UplinkClock {
    pub(crate) fn now(&self) -> Instant {
        match self {
            Self::System => Instant::now(),
            #[cfg(test)]
            Self::Manual(at) => *at,
        }
    }
}
