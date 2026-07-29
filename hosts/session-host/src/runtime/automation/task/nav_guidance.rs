//! Mission guidance onto the telemetry plane (ADR-0031): the executor's
//! plan-relative geometry becomes one stamped wire group per publication.
//!
//! The stamp is the host's own — this principal is the source, so the
//! acquisition time is host-monotonic (the shared origin every host
//! stamp derives from, ADR-0009) and the integrity is that of an
//! in-process observation, not an authenticated link. The sequence
//! advances only for a NEW sample, so a re-published cached group stays
//! byte-identical, and guidance that ends is cleared rather than frozen
//! or centered.

use pilotage_mission::{NavGuidance, NavQuality};
use pilotage_protocol::{SessionId, wire};

#[cfg(test)]
mod tests;

/// Source identity of the mission principal's guidance group. Role
/// travels in the stamp, so this id may collide with another role's
/// without ambiguity.
const NAV_GUIDANCE_SOURCE_ID: u64 = 1;

/// Epoch of that source. The principal publishes guidance for exactly one
/// mission per session, so nothing restarts under a live incarnation.
const NAV_GUIDANCE_EPOCH: u32 = 0;

/// Label half of the guidance incarnation token, so an incarnation from
/// this group cannot be confused with another group's that happens to
/// share a session.
const INCARNATION_TAG: &[u8; 8] = b"pmisnav\0";

/// What one mission tick owes the telemetry assembly.
pub(super) enum NavPublication {
    /// A new guidance sample under an advanced sequence.
    Sample(wire::NavGuidanceState),
    /// Guidance ended: the stored state must be dropped so the field goes
    /// absent on the wire.
    Clear,
}

/// Stamps the mission executor's guidance for one session.
pub(super) struct NavGuidancePublisher {
    sequence: u32,
    incarnation: [u8; 16],
    published: bool,
}

impl NavGuidancePublisher {
    /// Binds a publisher to `session`, deriving the equality-only
    /// incarnation token once: receivers compare it for equality, so it
    /// must stay fixed for as long as this principal flies, while a
    /// later session is visibly a different attachment.
    pub(super) fn for_session(session: SessionId) -> Self {
        let mut incarnation = [0u8; 16];
        let (tag, id) = incarnation.split_at_mut(INCARNATION_TAG.len());
        tag.copy_from_slice(INCARNATION_TAG);
        id.copy_from_slice(&session.as_u64().to_le_bytes());
        Self {
            sequence: 0,
            incarnation,
            published: false,
        }
    }

    /// What to send for this tick's `guidance`, or `None` when the wire
    /// already says the right thing: a clear is owed once, not on every
    /// tick a plan is not being flown.
    pub(super) fn publication(
        &mut self,
        guidance: Option<&NavGuidance>,
        acquired_at_ns: u64,
    ) -> Option<NavPublication> {
        match guidance {
            Some(guidance) => {
                self.sequence = self.sequence.wrapping_add(1);
                self.published = true;
                Some(NavPublication::Sample(to_wire(
                    guidance,
                    self.stamp(acquired_at_ns),
                )))
            }
            None if self.published => {
                self.published = false;
                Some(NavPublication::Clear)
            }
            None => None,
        }
    }

    fn stamp(&self, acquired_at_ns: u64) -> wire::MeasurementStamp {
        wire::MeasurementStamp {
            source_id: NAV_GUIDANCE_SOURCE_ID,
            source_epoch: NAV_GUIDANCE_EPOCH,
            sequence: self.sequence,
            acquired_at_ns,
            clock: wire::MeasurementClock::HostMonotonic as i32,
            source_incarnation: self.incarnation.to_vec(),
            role: wire::SourceRole::NavigationSolution as i32,
            integrity: wire::SourceIntegrity::Unprotected as i32,
        }
    }
}

/// The wire shape of one guidance sample. A deviation the executor is not
/// tracking travels as NaN — the schema's "no reading" encoding — because
/// zero would read as on-course or on-profile.
fn to_wire(guidance: &NavGuidance, stamp: wire::MeasurementStamp) -> wire::NavGuidanceState {
    wire::NavGuidanceState {
        stamp: Some(stamp),
        to_ident: guidance.to_ident.clone(),
        from_ident: guidance.from_ident.clone().unwrap_or_default(),
        course_rad: guidance.course_rad as f32,
        lateral_deviation_m: deviation_to_wire(guidance.lateral_deviation_m),
        vertical_deviation_m: deviation_to_wire(guidance.vertical_deviation_m),
        distance_to_waypoint_m: guidance.distance_to_waypoint_m as f32,
        leg_index: guidance.leg_index,
        waypoint_count: guidance.waypoint_count,
        solution_quality: match guidance.quality {
            NavQuality::Good => 0,
            NavQuality::Degraded => 1,
            NavQuality::Unusable => 2,
        },
    }
}

fn deviation_to_wire(deviation_m: Option<f64>) -> f32 {
    deviation_m.map_or(f32::NAN, |value| value as f32)
}
