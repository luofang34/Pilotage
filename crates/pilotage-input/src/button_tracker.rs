//! Button edge detection from successive raw bitmasks (ADR-0007).
//!
//! Control frames carry button state as one-shot edges rather than sampled
//! levels (see `pilotage_protocol::ButtonEdge`), so a held button produces
//! exactly one `Pressed` edge and one `Released` edge, not a `Pressed` edge
//! every frame.

use pilotage_protocol::ButtonEdge;

/// Tracks a device's previous button bitmask and emits edges when the
/// current mask differs from it.
///
/// Source-index bits, not logical button IDs: callers resolve source index
/// to `LogicalButtonId` via the profile's `buttons` table before or after
/// calling this tracker, as convenient.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ButtonTracker {
    previous: u64,
}

impl ButtonTracker {
    /// Constructs a tracker with all buttons assumed released.
    #[must_use]
    pub const fn new() -> Self {
        Self { previous: 0 }
    }

    /// Constructs a tracker seeded with a known previous mask (e.g. when
    /// resuming a session mid-stream).
    #[must_use]
    pub const fn with_previous_mask(previous: u64) -> Self {
        Self { previous }
    }

    /// Computes edges between the tracker's stored previous mask and
    /// `current`, then updates the stored mask to `current`.
    ///
    /// Returns `(source_index, edge)` pairs in ascending source-index order.
    pub fn update(&mut self, current: u64) -> Vec<(u8, ButtonEdge)> {
        let changed = self.previous ^ current;
        let mut edges = Vec::new();
        if changed != 0 {
            for index in 0..64u8 {
                let bit = 1u64 << index;
                if changed & bit == 0 {
                    continue;
                }
                let edge = if current & bit != 0 {
                    ButtonEdge::Pressed
                } else {
                    ButtonEdge::Released
                };
                edges.push((index, edge));
            }
        }
        self.previous = current;
        edges
    }

    /// Returns the currently tracked previous mask.
    #[must_use]
    pub const fn previous_mask(&self) -> u64 {
        self.previous
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::ButtonTracker;
    use pilotage_protocol::ButtonEdge;

    #[test]
    fn no_change_produces_no_edges() {
        let mut tracker = ButtonTracker::new();
        assert!(tracker.update(0).is_empty());
        tracker.update(0b101);
        assert!(tracker.update(0b101).is_empty());
    }

    #[test]
    fn press_then_release_sequence() {
        let mut tracker = ButtonTracker::new();
        let edges = tracker.update(0b1);
        assert_eq!(edges, vec![(0, ButtonEdge::Pressed)]);
        let edges = tracker.update(0b1);
        assert!(edges.is_empty());
        let edges = tracker.update(0b0);
        assert_eq!(edges, vec![(0, ButtonEdge::Released)]);
    }

    #[test]
    fn multiple_simultaneous_edges_reported_in_index_order() {
        let mut tracker = ButtonTracker::new();
        let edges = tracker.update(0b1010);
        assert_eq!(
            edges,
            vec![(1, ButtonEdge::Pressed), (3, ButtonEdge::Pressed)]
        );
    }

    #[test]
    fn seeded_previous_mask_does_not_emit_spurious_edges() {
        let mut tracker = ButtonTracker::with_previous_mask(0b1);
        let edges = tracker.update(0b1);
        assert!(edges.is_empty());
        assert_eq!(tracker.previous_mask(), 0b1);
    }

    #[test]
    fn held_button_across_many_frames_emits_one_press_and_one_release() {
        let mut tracker = ButtonTracker::new();
        let mut presses = 0usize;
        let mut releases = 0usize;
        for mask in [0b1, 0b1, 0b1, 0b0, 0b0] {
            for (_, edge) in tracker.update(mask) {
                match edge {
                    ButtonEdge::Pressed => presses += 1,
                    ButtonEdge::Released => releases += 1,
                }
            }
        }
        assert_eq!(presses, 1);
        assert_eq!(releases, 1);
    }
}
