//! A lease-bound stream. Mode feedback and input lifetime remain independent.

mod admission;

use std::time::Duration;

use crate::flight::{VirtualStickBinding, VirtualStickStream, VirtualStickVelocity};

#[derive(Default)]
pub(super) struct Motion {
    pub(in crate::adapter) binding: Option<VirtualStickBinding>,
    pub(in crate::adapter) sequence: Option<u32>,
    pub(in crate::adapter) latest: Option<(VirtualStickVelocity, Duration)>,
    pub(in crate::adapter) stream: Option<VirtualStickStream>,
    pub(in crate::adapter) stopped: bool,
    pub(in crate::adapter) link_loss: bool,
}

impl Motion {
    pub(super) fn stop(&mut self, now: Duration) {
        self.stopped = true;
        if let Some(stream) = &mut self.stream {
            stream.stop(now);
        }
    }

    pub(super) fn lost(&mut self) {
        self.stopped = true;
        if let Some(stream) = &mut self.stream {
            stream.transport_lost();
        }
    }
}

#[derive(Clone, Copy)]
pub(in crate::adapter) struct MotionInput {
    pub binding: VirtualStickBinding,
    pub sequence: u32,
    pub value: VirtualStickVelocity,
    pub until: Duration,
}
