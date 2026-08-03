//! The ingress admission gate (ADR-0018).

use crate::stamp::MeasurementStamp;

/// Why a measurement group was refused.
///
/// A refusal is counted and never replaces display state or refreshes its age;
/// a stale value that keeps its true age is honest, a stale value refreshed by
/// a rejected frame is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    /// The attachment changed to an incarnation the receiver has not
    /// authorized at a lifecycle boundary.
    UnauthorizedIncarnation,
    /// The epoch went backwards within one incarnation.
    OlderEpoch,
    /// The sequence repeats one already seen at this epoch.
    DuplicateSequence,
    /// The sequence went backwards under wrap-safe serial comparison.
    ReorderedSequence,
    /// Acquisition time regressed within one attachment and epoch.
    AcquisitionRegressed,
    /// The clock domain changed without a new attachment, so no ordering
    /// between the old and new timestamps can be established.
    ClockDomainChanged,
}

/// The gate's decision for one measurement group.
///
/// Dropping this verdict silently applies nothing while looking like the
/// group was ingested, which is the one failure the gate exists to prevent —
/// so it must be honored, not discarded.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    /// The group may replace display state.
    Accepted {
        /// The attachment or epoch changed, so every other group carried by
        /// the previous identity must be cleared before this one is applied.
        /// One display generation must never mix values across a source reset.
        identity_changed: bool,
    },
    /// The group was refused for this reason and must not be applied.
    Rejected(RejectReason),
}

/// Admits or refuses one measurement group's stream from one source.
///
/// A caller holds one gate per (source, role, group): the stamp carries a
/// single sequence, and ADR-0018 advances attitude, kinematics, and estimator
/// status independently.
#[derive(Debug, Default)]
pub struct SourceGate {
    seen: Option<MeasurementStamp>,
    accepted: u32,
    rejected: u32,
}

impl SourceGate {
    /// Creates a gate that has seen nothing.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            seen: None,
            accepted: 0,
            rejected: 0,
        }
    }

    /// Groups admitted so far. Wraps rather than saturating; the value is a
    /// diagnostic counter, not a limit.
    #[must_use]
    pub const fn accepted(&self) -> u32 {
        self.accepted
    }

    /// Groups refused so far.
    #[must_use]
    pub const fn rejected(&self) -> u32 {
        self.rejected
    }

    /// The stamp of the last admitted group.
    #[must_use]
    pub const fn last_admitted(&self) -> Option<&MeasurementStamp> {
        self.seen.as_ref()
    }

    /// Decides whether `stamp`'s group may replace display state.
    ///
    /// `authorize_attachment` is consulted only when the attachment changes.
    /// Deployment profiles differ here: an aircraft profile pins a
    /// source-issued incarnation during authenticated bootstrap, while a
    /// simulator profile may accept a bounded number of unseen incarnations.
    /// The policy is the caller's; the ordering rules are this gate's.
    pub fn admit(
        &mut self,
        stamp: &MeasurementStamp,
        authorize_attachment: impl FnOnce(&MeasurementStamp) -> bool,
    ) -> Admission {
        let Some(seen) = self.seen else {
            return self.settle(stamp, first_attachment(stamp, authorize_attachment));
        };
        if !seen.same_attachment(stamp) {
            return self.settle(stamp, first_attachment(stamp, authorize_attachment));
        }
        self.settle(stamp, continue_attachment(&seen, stamp))
    }

    fn settle(&mut self, stamp: &MeasurementStamp, outcome: Admission) -> Admission {
        match outcome {
            Admission::Accepted { .. } => {
                self.accepted = self.accepted.wrapping_add(1);
                self.seen = Some(*stamp);
            }
            Admission::Rejected(_) => {
                self.rejected = self.rejected.wrapping_add(1);
            }
        }
        outcome
    }
}

fn first_attachment(
    stamp: &MeasurementStamp,
    authorize: impl FnOnce(&MeasurementStamp) -> bool,
) -> Admission {
    if authorize(stamp) {
        Admission::Accepted {
            identity_changed: true,
        }
    } else {
        Admission::Rejected(RejectReason::UnauthorizedIncarnation)
    }
}

fn continue_attachment(seen: &MeasurementStamp, stamp: &MeasurementStamp) -> Admission {
    if stamp.source_epoch != seen.source_epoch {
        return if serial_gt(stamp.source_epoch, seen.source_epoch) {
            // A new epoch is a source reset inside one attachment: ordering
            // established under the old epoch no longer applies.
            Admission::Accepted {
                identity_changed: true,
            }
        } else {
            Admission::Rejected(RejectReason::OlderEpoch)
        };
    }
    if stamp.clock != seen.clock {
        return Admission::Rejected(RejectReason::ClockDomainChanged);
    }
    if stamp.sequence == seen.sequence {
        return Admission::Rejected(RejectReason::DuplicateSequence);
    }
    if !serial_gt(stamp.sequence, seen.sequence) {
        return Admission::Rejected(RejectReason::ReorderedSequence);
    }
    if stamp.acquired_at_ns < seen.acquired_at_ns {
        return Admission::Rejected(RejectReason::AcquisitionRegressed);
    }
    Admission::Accepted {
        identity_changed: false,
    }
}

/// Wrap-safe serial comparison (RFC 1982): whether `a` is newer than `b`.
///
/// A counter that wraps must not make the next value look ancient, so
/// "newer" is defined as being within the forward half of the number space.
fn serial_gt(a: u32, b: u32) -> bool {
    a != b && a.wrapping_sub(b) < (1 << 31)
}

#[cfg(test)]
mod tests;
