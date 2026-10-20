//! Modular boot-clock ordering; reconnect, rather than a late packet, establishes reboot.

#[derive(Debug, Clone, Copy)]
pub(super) struct BootClock {
    latest: Option<u32>,
    pub(super) epoch: u64,
}

impl Default for BootClock {
    fn default() -> Self {
        Self {
            latest: None,
            epoch: 1,
        }
    }
}

pub(super) fn advances(time: u32, previous: u32) -> bool {
    let delta = time.wrapping_sub(previous);
    delta != 0 && delta < (1_u32 << 31)
}

impl BootClock {
    pub(super) fn admits_new(&self, time: u32) -> bool {
        self.latest.is_none_or(|previous| advances(time, previous))
    }

    pub(super) fn accepts_epoch(&self, time: u32) -> bool {
        let mut copy = *self;
        copy.observe(time).is_some()
    }

    /// Accept timestamps in this epoch, including independently delayed report streams.
    pub(super) fn observe(&mut self, time: u32) -> Option<bool> {
        let Some(previous) = self.latest else {
            self.latest = Some(time);
            return Some(false);
        };
        if time.wrapping_sub(previous) == (1_u32 << 31) {
            return None;
        }
        if advances(time, previous) {
            let wrapped = time < previous;
            if wrapped {
                self.epoch = self.epoch.wrapping_add(1);
            }
            self.latest = Some(time);
            return Some(wrapped);
        }
        if time > previous {
            // A high timestamp arriving after wrap belongs to the preceding epoch.
            return None;
        }
        Some(false)
    }
}
