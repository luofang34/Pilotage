//! The tick: one caller-supplied `now` becomes at most one intent, at
//! most one action, and the events the phase machine surfaced.

use core::mem::discriminant;

use navigate_contract::{
    GeodeticPosition, GuidanceSetpoint, MonotonicNanos, NavigationSolution, NedVelocity, Waypoint,
};
use navigate_fpl::SequenceEvent;
use navigate_guidance::guide_velocity;
use pilotage_protocol::{ControlAction, ControlIntent, ReferenceFrame, VelocityIntent};

use crate::body_frame::{bearing_rad, cap_horizontal, clamp_symmetric, ned_to_body, wrap_to_pi};

use super::{MissionAction, MissionEngine, MissionEvent, MissionOutput, MissionState};

/// Altitude margin below the cruise target at which the climb phase
/// hands over to enroute guidance, meters.
const CLIMB_CAPTURE_MARGIN_M: f64 = 1.0;

/// Commanded horizontal speed below which the course is numerically
/// meaningless and the yaw rate is held at zero, m/s.
const YAW_COURSE_FLOOR_MPS: f64 = 0.05;

impl MissionEngine {
    /// Advances the mission one step at `now` (engine clock domain).
    ///
    /// A tick that emits no intent is deliberate — a guidance refusal or
    /// a not-yet-initialized filter — and the host's holder-silence
    /// watchdog is the backstop for a stretch of them.
    pub fn tick(&mut self, now: MonotonicNanos) -> MissionOutput {
        let mut events = core::mem::take(&mut self.pending_events);
        let mut intent = None;
        let mut action = None;
        match self.state {
            MissionState::AwaitSolution => {
                if self.publish_solution(now).is_some() {
                    self.state = MissionState::Arming;
                    action = Some(self.send_arm(&mut events));
                }
            }
            MissionState::Arming => {
                if self.arm_needs_send {
                    action = Some(self.send_arm(&mut events));
                }
            }
            MissionState::Climb => intent = self.climb_tick(now, &mut events),
            MissionState::Enroute => intent = self.enroute_tick(now, &mut events),
            MissionState::Complete => intent = Some(zero_velocity_intent()),
        }
        MissionOutput {
            intent,
            action,
            state: self.state,
            events,
        }
    }

    /// Publishes the filter's solution for `now` and remembers it as the
    /// basis of the display-facing guidance view. A tick that publishes
    /// nothing forgets the previous solution: guidance that cannot be
    /// recomputed must disappear rather than age on an instrument.
    fn publish_solution(&mut self, now: MonotonicNanos) -> Option<NavigationSolution> {
        self.last_solution = self.filter.tick(now);
        self.last_solution
    }

    /// Emits an arm action under a fresh nonzero wrapping id. The action
    /// goes out once; only a rejected result schedules another send.
    fn send_arm(&mut self, events: &mut Vec<MissionEvent>) -> MissionAction {
        self.next_action_id = self.next_action_id.wrapping_add(1);
        if self.next_action_id == 0 {
            // The session wire reserves zero; skip it on wrap.
            self.next_action_id = 1;
        }
        let action_id = self.next_action_id;
        self.outstanding_arm = Some(action_id);
        self.arm_needs_send = false;
        events.push(MissionEvent::ArmRequested { action_id });
        MissionAction {
            action: ControlAction::Arm,
            action_id,
        }
    }

    /// Climb straight up until the solution altitude reaches the cruise
    /// target (within [`CLIMB_CAPTURE_MARGIN_M`]), then hand over to
    /// enroute guidance on the same tick so no tick goes intent-less at
    /// the seam.
    fn climb_tick(
        &mut self,
        now: MonotonicNanos,
        events: &mut Vec<MissionEvent>,
    ) -> Option<ControlIntent> {
        let solution = self.publish_solution(now)?;
        let target_m = self.config.anchor.altitude_m + self.config.cruise_height_m;
        if solution.position.altitude_m >= target_m - CLIMB_CAPTURE_MARGIN_M {
            self.state = MissionState::Enroute;
            events.push(MissionEvent::EnrouteStarted);
            return self.guide(&solution, now, events);
        }
        let up_mps = clamp_symmetric(
            self.config.climb_rate_mps,
            self.config.limits.max_vertical_mps,
        );
        Some(ControlIntent::Velocity(VelocityIntent {
            frame: ReferenceFrame::BodyFrd,
            vx: 0.0,
            vy: 0.0,
            vz: -up_mps as f32,
            yaw_rate: 0.0,
        }))
    }

    fn enroute_tick(
        &mut self,
        now: MonotonicNanos,
        events: &mut Vec<MissionEvent>,
    ) -> Option<ControlIntent> {
        let solution = self.publish_solution(now)?;
        self.guide(&solution, now, events)
    }

    /// Sequences the plan on the solution position, then derives one
    /// velocity intent toward the active leg.
    fn guide(
        &mut self,
        solution: &NavigationSolution,
        now: MonotonicNanos,
        events: &mut Vec<MissionEvent>,
    ) -> Option<ControlIntent> {
        match self.execution.advance(&solution.position) {
            SequenceEvent::LegAdvanced { to_index } => {
                events.push(MissionEvent::LegAdvanced { to_index });
            }
            SequenceEvent::PlanComplete => {
                self.state = MissionState::Complete;
                events.push(MissionEvent::MissionComplete);
                return Some(zero_velocity_intent());
            }
            // Non-exhaustive upstream: any event that neither advances
            // nor completes leaves the active leg unchanged.
            _ => {}
        }
        // Owned copies release the borrow of the execution before the
        // counters and refusal latch are touched below.
        let (leg_from, leg_to): (Option<GeodeticPosition>, Waypoint) = {
            let leg = self.execution.active_leg()?;
            (leg.from.map(|wp| wp.position), leg.to.clone())
        };
        let guided = guide_velocity(
            solution,
            leg_from.as_ref(),
            &leg_to,
            now,
            self.config.clock,
            &self.guidance,
        );
        match guided {
            Ok(command) => {
                self.last_refusal = None;
                let GuidanceSetpoint::Velocity { velocity } = command.setpoint else {
                    // guide_velocity only issues velocity setpoints; a
                    // foreign shape is refused rather than misread.
                    self.counters.guidance_refused = self.counters.guidance_refused.wrapping_add(1);
                    return None;
                };
                let Some(yaw) = self.last_yaw_rad else {
                    // Without a known heading the NED command cannot be
                    // rotated into the body frame honestly; a guessed
                    // zero would steer every command toward due north.
                    self.counters.missing_yaw = self.counters.missing_yaw.wrapping_add(1);
                    return None;
                };
                Some(self.compose_intent(&velocity, yaw))
            }
            Err(refusal) => {
                self.counters.guidance_refused = self.counters.guidance_refused.wrapping_add(1);
                let kind = discriminant(&refusal);
                if self.last_refusal != Some(kind) {
                    self.last_refusal = Some(kind);
                    events.push(MissionEvent::GuidanceRefused { reason: refusal });
                }
                None
            }
        }
    }

    /// Clamps the commanded NED velocity to the mission limits, rotates
    /// it into body FRD at the latest sample yaw, and commands a yaw
    /// rate toward the course of the commanded horizontal velocity —
    /// chosen over the leg course so the nose also follows cross-track
    /// corrections and stays defined on direct-to legs.
    fn compose_intent(&self, velocity: &NedVelocity, yaw_rad: f64) -> ControlIntent {
        let limits = &self.config.limits;
        let (vn, ve) = cap_horizontal(
            velocity.north_mps,
            velocity.east_mps,
            limits.max_horizontal_mps,
        );
        let vd = clamp_symmetric(velocity.down_mps, limits.max_vertical_mps);
        let (vx, vy) = ned_to_body(vn, ve, yaw_rad);
        let horizontal_mps = (vn * vn + ve * ve).sqrt();
        let yaw_rate = if horizontal_mps < YAW_COURSE_FLOOR_MPS {
            0.0
        } else {
            let course = bearing_rad(vn, ve);
            clamp_symmetric(
                self.config.yaw_gain_per_s * wrap_to_pi(course - yaw_rad),
                limits.max_yaw_rate_rps,
            )
        };
        ControlIntent::Velocity(VelocityIntent {
            frame: ReferenceFrame::BodyFrd,
            vx: vx as f32,
            vy: vy as f32,
            vz: vd as f32,
            yaw_rate: yaw_rate as f32,
        })
    }
}

/// The zero-velocity intent completion holds: the adapter's
/// brake-then-hold takes over from an explicit stationary command.
fn zero_velocity_intent() -> ControlIntent {
    ControlIntent::Velocity(VelocityIntent {
        frame: ReferenceFrame::BodyFrd,
        vx: 0.0,
        vy: 0.0,
        vz: 0.0,
        yaw_rate: 0.0,
    })
}
