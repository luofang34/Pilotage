//! Where the vehicle says it is, read off one telemetry lane.
//!
//! The situation session ingests surveillance, weather and terrain. A vehicle
//! under this operator's own control appears in none of them, so its position
//! reaches the map through this and nothing else.
//!
//! The rules here are the browser's rules, deliberately: both clients draw the
//! same mark from the same measurements, and a mark that means one thing on a
//! tablet and another in a browser is worse than either rule alone.
//!
//! Nothing shared enforces that. There is no corpus these two agree through —
//! the values are copied from `clients/web/situation-ownship.js`, carry the
//! same names, and are held on this side by the tests below and on that side
//! by its own. A change to either is a change to one client's behaviour until
//! somebody moves the other, so the tests state the numbers outright rather
//! than deriving them, and a reader comparing the two files can see in one
//! pass whether they still agree.

use pilotage_protocol::wire;

/// Below this speed a velocity states no direction worth drawing: the track
/// is noise around a stationary vehicle rather than a course.
const TRACK_FLOOR_MPS: f64 = 0.5;

/// The speed a course already drawn must fall below before it is taken away.
///
/// Without a band between the two, a vehicle drifting either side of the floor
/// flickers its course on and off at the telemetry rate.
const TRACK_RELEASE_MPS: f64 = 0.35;

/// How long a direction may go without a new measurement and still be drawn.
///
/// A direction is measured many times a second, and a stale one does not fade
/// — it turns the mark, and keeps pointing confidently at a heading the
/// vehicle left. So a group that has not produced a measurement this recently
/// stops contributing, and the mark is drawn without that direction rather
/// than with a wrong one.
///
/// The position itself is deliberately NOT held to this bound. It is reported
/// far less often, and withdrawing the mark every time a receiver paused would
/// blink it constantly; it keeps the longer staleness bound instead, which the
/// client applies.
const GROUP_COHERENCE_MS: u64 = 300;

/// Bit 0 of the validity mask: the lane states an attitude.
const VALID_ATTITUDE: u32 = 1 << 0;
/// Bit 3: the lane states a velocity.
const VALID_VELOCITY: u32 = 1 << 3;

/// The estimator's own verdict that its solution cannot be used.
const QUALITY_UNUSABLE: u32 = 2;

/// `SourceRole::OPERATIONAL_ESTIMATE`: the flight controller's own solution.
const ROLE_OPERATIONAL_ESTIMATE: i32 = 1;

/// `SourceRole::SIMULATION_TRUTH`: the simulator's oracle.
const ROLE_SIMULATION_TRUTH: i32 = 2;

/// A vehicle's position and the directions that ride with it.
pub(super) struct VehicleFix {
    pub(super) latitude_deg: f64,
    pub(super) longitude_deg: f64,
    pub(super) heading_deg: Option<f64>,
    pub(super) course_deg: Option<f64>,
    pub(super) ground_speed_mps: Option<f64>,
    pub(super) from_simulator: bool,
    /// Whether the position in this fix is a NEW measurement.
    ///
    /// The client withdraws a mark whose position has gone stale, and it can
    /// only time that from when the position was last measured. Arrival will
    /// not do: a host relaying a frozen block delivers samples forever, and a
    /// mark timed from arrival would never go stale no matter how long the
    /// vehicle had stopped reporting.
    pub(super) fix_advanced: bool,
}

/// Which measurement one tracked group belongs to.
#[derive(Clone, Copy)]
enum Group {
    Heading,
    Track,
    Fix,
}

/// What distinguishes one measurement from the one before it.
///
/// The role rides in here with the rest, so a handover from the estimate lane
/// to the truth lane reads as the new measurement it is without the lane
/// having to be remembered separately.
#[derive(PartialEq)]
struct StampIdentity {
    role: i32,
    source_id: u64,
    source_incarnation: Vec<u8>,
    source_epoch: u32,
    sequence: u32,
    acquired_at_ns: u64,
}

impl StampIdentity {
    fn of(stamp: &wire::MeasurementStamp) -> Self {
        Self {
            role: stamp.role,
            source_id: stamp.source_id,
            source_incarnation: stamp.source_incarnation.clone(),
            source_epoch: stamp.source_epoch,
            sequence: stamp.sequence,
            acquired_at_ns: stamp.acquired_at_ns,
        }
    }
}

/// What this reader carries between samples.
///
/// When each group last stated something it had not already seen, and whether
/// a course is currently on screen.
///
/// Kept across samples because that is the only place the answer lives: one
/// sample cannot say whether its stamp is the same one the last sample
/// carried, and a lane that keeps repeating a measurement is exactly the case
/// this exists to notice.
#[derive(Default)]
pub(super) struct MarkMemory {
    identity: [Option<StampIdentity>; 3],
    advanced_at_ms: [u64; 3],
    /// Whether a course is on screen now, which decides which side of the
    /// track band this sample is judged against.
    course_drawn: bool,
}

impl MarkMemory {
    /// When this group last advanced, and whether it advanced just now.
    ///
    /// Advance is identity INEQUALITY rather than a sequence comparison. A
    /// sequence that wrapped, or one a source restarted, still states a new
    /// measurement, and ordering it against the last would read the newest
    /// measurement there is as the oldest.
    fn advanced(
        &mut self,
        group: Group,
        stamp: Option<&wire::MeasurementStamp>,
        now_ms: u64,
    ) -> Option<(u64, bool)> {
        let identity = StampIdentity::of(stamp?);
        let slot = group as usize;
        if self.identity[slot].as_ref() != Some(&identity) {
            self.identity[slot] = Some(identity);
            self.advanced_at_ms[slot] = now_ms;
            return Some((now_ms, true));
        }
        Some((self.advanced_at_ms[slot], false))
    }

    /// Whether this group has stated a measurement recently enough to draw.
    ///
    /// A group that states no stamp at all cannot be shown to be current, and
    /// what cannot be shown current is not drawn.
    fn is_current(
        &mut self,
        group: Group,
        stamp: Option<&wire::MeasurementStamp>,
        now_ms: u64,
    ) -> bool {
        self.advanced(group, stamp, now_ms)
            .is_some_and(|(when, _)| now_ms.saturating_sub(when) <= GROUP_COHERENCE_MS)
    }
}

/// Reads one sample into a fix, or nothing when no lane states a position.
///
/// The oracle wins where a session has one, because a session that has one is
/// being judged against it. A session without one is the normal case, not a
/// failure, and it is the only case a physical vehicle has.
///
/// A truth lane that cannot be read falls through to the estimate lane rather
/// than taking the mark away. That is not leniency: the browser's decoder
/// drops an unreadable group before the map is offered the sample, so the map
/// there sees a session with no oracle and reads the estimate. Refusing the
/// sample outright here would blank a mark the other client still draws.
pub(super) fn from_sample(
    memory: &mut MarkMemory,
    sample: &wire::TelemetrySample,
    now_ms: u64,
) -> Option<VehicleFix> {
    if let Some(truth) = sample.sim_truth.as_ref()
        && let Some(geodetic) = truth.geodetic.as_ref()
        // A position that cannot be shown to be this vehicle's, this boot's
        // and this moment's is not drawn.
        //
        // Nor one stamped with another lane's role. Role travels in
        // provenance precisely so a group cannot be read as something it is
        // not, and an oracle's position read as the estimator's own solution
        // is the substitution the roles exist to stop.
        && stamp_states_role(truth.stamp.as_ref(), ROLE_SIMULATION_TRUTH)
        && let Some(position) = position_of(geodetic)
    {
        // The truth lane states one observation and every group rides it, so
        // all three are tracked against the same stamp. They are still tracked
        // separately: the lane can hand over to the estimate lane, which
        // stamps its groups apart.
        let stamp = truth.stamp.as_ref();
        let heading_current = memory.is_current(Group::Heading, stamp, now_ms);
        let track_current = memory.is_current(Group::Track, stamp, now_ms);
        let fix_advanced = memory
            .advanced(Group::Fix, stamp, now_ms)
            .is_some_and(|(_, now)| now);
        let velocity = track_from(
            [truth.vel_n_mps, truth.vel_e_mps],
            truth.valid_flags,
            memory.course_drawn,
        );
        let course = track_current.then_some(velocity).flatten();
        memory.course_drawn = course.is_some();
        return Some(VehicleFix {
            latitude_deg: position.0,
            longitude_deg: position.1,
            heading_deg: heading_current
                .then(|| {
                    heading_from(
                        [truth.quat_w, truth.quat_x, truth.quat_y, truth.quat_z],
                        truth.valid_flags,
                    )
                })
                .flatten(),
            course_deg: course.map(|(bearing, _)| bearing),
            ground_speed_mps: course.map(|(_, speed)| speed),
            from_simulator: true,
            fix_advanced,
        });
    }

    let avionics = sample.avionics.as_ref()?;
    let geodetic = avionics.geodetic.as_ref()?;
    // The fix carries its own stamp and its own role, and is refused on both:
    // a truth-stamped position in this slot would be drawn as the estimator's
    // answer, which is the one thing the roles are there to prevent.
    if !stamp_states_role(avionics.geodetic_stamp.as_ref(), ROLE_OPERATIONAL_ESTIMATE) {
        return None;
    }
    let position = position_of(geodetic)?;
    // On the estimate lane the mask is a latched authorization from the
    // estimator, and it means nothing without the status observation behind
    // it — nor when that observation says the solution is unusable. An
    // estimator calling its own answer unusable is the clearest refusal there
    // is, and reading its directions anyway would turn the mark by a number
    // its author disowned.
    let authorized =
        avionics.estimator_status_stamp.is_some() && avionics.quality != QUALITY_UNUSABLE;
    let flags = if authorized { avionics.valid_flags } else { 0 };
    // This lane stamps its groups apart, so each is asked about its own.
    let heading_current =
        memory.is_current(Group::Heading, avionics.attitude_stamp.as_ref(), now_ms);
    let track_current = memory.is_current(Group::Track, avionics.kinematics_stamp.as_ref(), now_ms);
    let fix_advanced = memory
        .advanced(Group::Fix, avionics.geodetic_stamp.as_ref(), now_ms)
        .is_some_and(|(_, now)| now);
    let velocity = track_from(
        [avionics.vel_n_mps, avionics.vel_e_mps],
        flags,
        memory.course_drawn,
    );
    let course = track_current.then_some(velocity).flatten();
    memory.course_drawn = course.is_some();
    Some(VehicleFix {
        latitude_deg: position.0,
        longitude_deg: position.1,
        heading_deg: heading_current
            .then(|| {
                heading_from(
                    [
                        avionics.quat_w,
                        avionics.quat_x,
                        avionics.quat_y,
                        avionics.quat_z,
                    ],
                    flags,
                )
            })
            .flatten(),
        course_deg: course.map(|(bearing, _)| bearing),
        ground_speed_mps: course.map(|(_, speed)| speed),
        from_simulator: false,
        fix_advanced,
    })
}

/// Whether a stamp states the role its lane requires.
///
/// A group is only what its provenance says it is. The browser settles this in
/// its decoder, which drops a mislabelled group before any consumer sees it.
/// This client decodes the sample without that step, so the lane that draws
/// the mark has to make the refusal itself or not make it at all.
fn stamp_states_role(stamp: Option<&wire::MeasurementStamp>, role: i32) -> bool {
    stamp.is_some_and(|stamp| stamp.role == role)
}

/// The latitude and longitude a lane states, or nothing when what it states
/// cannot be placed on the Earth.
///
/// Only what places the mark is examined, and the browser's decoder refuses
/// more than this does. It drops the whole fix when the VERTICAL datum lacks
/// the identity it requires — a geoid model for MSL, a terrain reference for
/// AGL, a setting for baro-indicated, an origin for local-relative — or when
/// no vertical datum is named at all, which is what proto3 gives a field
/// nobody set. It also refuses a realization too large to be one, whatever the
/// horizontal datum, so the divergence is not confined to height.
///
/// A sample malformed in any of those ways draws a mark here and none there.
/// The mark that appears is in the right place; the defect is that the two
/// clients disagree about whether to draw it, and closing that means consuming
/// the geo contract rather than keeping a second copy of its rules here.
///
/// None of it is reachable from a conforming host, which re-normalizes a
/// position before it reaches the wire. That is worth stating and worth
/// distrusting: "the producer would not send that" is the argument this lane
/// already declined to accept for role and for datum.
fn position_of(fix: &wire::GeodeticFix) -> Option<(f64, f64)> {
    if !fix.latitude_deg.is_finite() || !(-90.0..=90.0).contains(&fix.latitude_deg) {
        return None;
    }
    // The wire says the producer sends a normalized longitude, so one needing
    // a wrap is a producer that did not keep the contract. Wrapping it here
    // would draw the vehicle a full turn of the Earth from where the other
    // client draws nothing at all.
    if !fix.longitude_deg.is_finite() || !(-180.0..180.0).contains(&fix.longitude_deg) {
        return None;
    }
    // `pilotage_geo::HorizontalDatum`: 0 is Unknown, and 4 upward is a datum
    // this build does not know. The schema states that unknown is refused at
    // the receiver and never guessed — two datums put the same degrees a
    // couple of metres apart, and which was meant is not recoverable later.
    if fix.horizontal_datum == 0 || fix.horizontal_datum > 3 {
        return None;
    }
    // A datum that is a realization of a frame is uninterpretable without
    // saying which realization.
    if matches!(fix.horizontal_datum, 2 | 3) && fix.realization == 0 {
        return None;
    }
    Some((fix.latitude_deg, fix.longitude_deg))
}

/// Where the nose points, from the attitude quaternion.
///
/// The full form rather than `1 - 2(y² + z²)`: it is exact at any scale, and a
/// quaternion that arrived slightly off unit length still yields the angle it
/// represents instead of a silently wrong one.
fn heading_from(quat: [f32; 4], valid_flags: u32) -> Option<f64> {
    if valid_flags & VALID_ATTITUDE == 0 {
        return None;
    }
    let [w, x, y, z] = quat.map(f64::from);
    let norm_squared = w * w + x * x + y * y + z * z;
    // A quaternion nowhere near unit length is not an attitude that was
    // measured; it is a field nobody filled in.
    if !(0.9..=1.1).contains(&norm_squared) {
        return None;
    }
    let yaw = f64::atan2(
        2.0 * z.mul_add(w, x * y),
        z.mul_add(-z, y.mul_add(-y, x.mul_add(x, w * w))),
    );
    Some(wrap_bearing(yaw.to_degrees()))
}

/// Track over the ground and the speed along it.
///
/// `drawn` says whether a course is on screen now. A course already drawn is
/// held to the lower bound, so a vehicle hovering either side of the floor
/// keeps the one it has instead of flickering at the telemetry rate.
fn track_from(vel_ne: [f32; 2], valid_flags: u32, drawn: bool) -> Option<(f64, f64)> {
    if valid_flags & VALID_VELOCITY == 0 {
        return None;
    }
    let [north, east] = vel_ne.map(f64::from);
    if !north.is_finite() || !east.is_finite() {
        return None;
    }
    let speed = north.hypot(east);
    if speed
        < if drawn {
            TRACK_RELEASE_MPS
        } else {
            TRACK_FLOOR_MPS
        }
    {
        return None;
    }
    Some((wrap_bearing(f64::atan2(east, north).to_degrees()), speed))
}

/// A bearing in `[0, 360)`.
fn wrap_bearing(degrees: f64) -> f64 {
    degrees.rem_euclid(360.0)
}

#[cfg(test)]
mod tests;
