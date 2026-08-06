//! The fail-closed authorization fold: which regime's declared
//! estimator state vouches for each admitted numeric group.
//!
//! A transport validation fault cannot mint a source acquisition time,
//! so a publication backed by the current status stamp may change trust
//! only in the fail-closed direction (bitwise-and of flags, max of
//! quality) and never refreshes that stamp's age. The CURRENT regime is
//! additionally a monotonic ceiling: even a numeric legitimately judged
//! by a still-good previous regime is capped by the estimator's most
//! recent declared state, so a duplicate-status downgrade cannot be
//! reversed by an interleaved numeric.

use super::{
    ATTITUDE_VALID_FLAGS, AvionicsIngress, AvionicsSample, KINEMATICS_VALID_FLAGS,
    KNOWN_VALID_FLAGS, QUALITY_UNUSABLE, StatusRegime,
};
use crate::stamp::RawStamp;

fn stamps_equal(left: Option<RawStamp>, right: Option<RawStamp>) -> bool {
    match (left, right) {
        (Some(a), Some(b)) => {
            a.source_id == b.source_id
                && a.incarnation == b.incarnation
                && a.epoch == b.epoch
                && a.sequence == b.sequence
                && a.acquired_at_ns == b.acquired_at_ns
                && a.clock == b.clock
        }
        _ => false,
    }
}

impl AvionicsIngress {
    pub(super) fn update_authorization(
        &mut self,
        sample: &AvionicsSample,
        accepted_attitude: bool,
        accepted_kinematics: bool,
    ) {
        let status_matches = stamps_equal(
            sample.estimator_status_stamp,
            self.estimator_status.map(|group| group.stamp),
        );
        if status_matches {
            self.apply_status_downgrade(sample);
        }
        if accepted_attitude || accepted_kinematics {
            self.update_from_numeric(sample, accepted_attitude, accepted_kinematics);
        }
    }

    /// A duplicate status stamp may only tighten authorization, never
    /// restore it — folded into the regime itself so a later numeric
    /// bearing this same status stamp cannot consult the pre-downgrade
    /// regime and reverse a fail-closed decision. The fold is monotone,
    /// so re-applying the status that opened the regime is a no-op.
    fn apply_status_downgrade(&mut self, sample: &AvionicsSample) {
        if let Some(regime) = self.regime.as_mut() {
            regime.valid_flags &= sample.valid_flags;
            regime.quality = regime.quality.max(sample.quality);
        }
        if !self.has_established_authorization() {
            self.fail_closed_authorization();
            return;
        }
        self.valid_flags &= sample.valid_flags;
        self.quality = self.quality.max(sample.quality);
    }

    fn has_established_authorization(&self) -> bool {
        (self.attitude.is_some() && self.attitude_paired)
            || (self.kinematics.is_some() && self.kinematics_paired)
    }

    /// The regime whose declared estimator state governs a numeric
    /// group acquired at `numeric`: the current status when acquired at
    /// or after its instant, else the previous status when acquired
    /// within its reign. The skew budget against the current status
    /// keeps a stale numeric from borrowing authority across a stream
    /// gap; identity and clock must match in every case, so nothing
    /// authorizes across a source reset. `None` means no status can
    /// vouch for this acquisition — fail closed.
    fn regime_for(&self, numeric: &RawStamp) -> Option<StatusRegime> {
        let current = self.regime?;
        if !current.pairs(numeric, self.config.maximum_skew_nanos) {
            return None;
        }
        if numeric.acquired_at_ns >= current.acquired_at_ns {
            return Some(current);
        }
        let previous = self.previous_regime?;
        if previous.pairs(numeric, self.config.maximum_skew_nanos)
            && numeric.acquired_at_ns >= previous.acquired_at_ns
        {
            return Some(previous);
        }
        None
    }

    fn update_from_numeric(
        &mut self,
        sample: &AvionicsSample,
        accepted_attitude: bool,
        accepted_kinematics: bool,
    ) {
        let status_matches = stamps_equal(
            sample.estimator_status_stamp,
            self.estimator_status.map(|group| group.stamp),
        );
        let mut paired_quality: Option<u32> = None;
        if accepted_attitude {
            let stamp = sample.attitude_stamp.unwrap_or_else(zero_stamp);
            let regime = if status_matches {
                self.regime_for(&stamp)
            } else {
                None
            };
            self.attitude_paired = regime.is_some();
            let flags = self.masked_flags(regime, sample.valid_flags, ATTITUDE_VALID_FLAGS);
            self.valid_flags = (self.valid_flags & !ATTITUDE_VALID_FLAGS) | flags;
            if let Some(regime) = regime {
                paired_quality = Some(self.capped_quality(regime, sample.quality, 0));
            }
        }
        if accepted_kinematics {
            let stamp = sample.kinematics_stamp.unwrap_or_else(zero_stamp);
            let regime = if status_matches {
                self.regime_for(&stamp)
            } else {
                None
            };
            self.kinematics_paired = regime.is_some();
            let flags = self.masked_flags(regime, sample.valid_flags, KINEMATICS_VALID_FLAGS);
            self.valid_flags = (self.valid_flags & !KINEMATICS_VALID_FLAGS) | flags;
            if let Some(regime) = regime {
                let floor = paired_quality.unwrap_or(0);
                paired_quality = Some(self.capped_quality(regime, sample.quality, floor));
            }
        }
        if self.valid_flags & KNOWN_VALID_FLAGS == 0 {
            self.quality = QUALITY_UNUSABLE;
            return;
        }
        if let Some(quality) = paired_quality {
            self.quality = quality;
        }
    }

    fn masked_flags(&self, regime: Option<StatusRegime>, incoming: u32, group_mask: u32) -> u32 {
        match (regime, self.regime) {
            (Some(regime), Some(current)) => {
                regime.valid_flags & current.valid_flags & incoming & group_mask
            }
            _ => 0,
        }
    }

    fn capped_quality(&self, regime: StatusRegime, incoming: u32, floor: u32) -> u32 {
        let current = self.regime.map_or(0, |current| current.quality);
        floor.max(regime.quality).max(current).max(incoming)
    }
}

fn zero_stamp() -> RawStamp {
    RawStamp {
        role: 0,
        integrity: 0,
        source_id: 0,
        incarnation: [0; 16],
        epoch: 0,
        sequence: 0,
        acquired_at_ns: 0,
        clock: 0,
    }
}
