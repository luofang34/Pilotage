//! One operational tick through the shared sequencing core.

use core::mem::discriminant;

use navigate_contract::{
    GeodeticPosition, GuidanceSetpoint, MonotonicNanos, NavigationSolution, NedVelocity, Waypoint,
};
use navigate_fpl::SequenceEvent;
use navigate_guidance::guide_velocity;
use pilotage_mission_core::{
    EngineEvent as CoreEvent, EngineStart, FlightAction as CoreFlightAction,
    MissionEngine as CoreMissionEngine, MissionObservation as CoreObservation,
    MissionTerminal as CoreTerminal, NavigationObservation, TickInput,
    TickOutput as CoreTickOutput, VehicleObservation, WallDeadline,
};
use pilotage_protocol::{ControlIntent, ReferenceFrame, VelocityIntent};

use crate::body_frame::{bearing_rad, cap_horizontal, clamp_symmetric, ned_to_body, wrap_to_pi};
use crate::policy::OPERATIONAL_WALL_DEADLINE_NS;

use super::document::{ARM_PHASE_ID, CLIMB_CAPTURE_MARGIN_M};
use super::{MissionAction, MissionEngine, MissionEvent, MissionOutput, MissionState};

/// Commanded horizontal speed below which course has no useful direction.
const YAW_COURSE_FLOOR_MPS: f64 = 0.05;

mod directives;
mod outputs;
mod overflight;

use outputs::{failed_output, terminal_output, zero_velocity_intent};

impl MissionEngine {
    /// Advances the operational mission with one caller clock value.
    ///
    /// The wrapper supplies the same monotonic value as simulator time and
    /// wall time. It performs no clock read and no input or output operation.
    pub fn tick(&mut self, now: MonotonicNanos) -> MissionOutput {
        match self.state() {
            MissionState::Complete => return terminal_output(MissionState::Complete),
            MissionState::Failed => return terminal_output(MissionState::Failed),
            MissionState::Held => return self.held_output(now),
            MissionState::AwaitSolution
            | MissionState::Arming
            | MissionState::Climb
            | MissionState::Enroute => {}
        }
        let mut events = core::mem::take(&mut self.hold_events);
        let solution = self.publish_solution(now);
        let mut plan_advanced = false;
        if self.state() == MissionState::Enroute {
            plan_advanced = self.advance_active_plan(solution.as_ref(), &mut events);
        }
        let observation = self.core_observation(solution.as_ref());
        let Some(core_output) = self.tick_core(now, observation, &mut events) else {
            return failed_output(events);
        };
        let mut action = self.apply_core_output(&core_output, &mut events);
        let mut state = self.state();
        if state == MissionState::Climb && self.climb_is_captured(solution.as_ref()) {
            let Some(extra_action) = self.continue_core(now, solution.as_ref(), &mut events) else {
                return failed_output(events);
            };
            if action.is_none() {
                action = extra_action;
            }
            state = self.state();
        }
        if state == MissionState::Enroute && !plan_advanced {
            self.advance_active_plan(solution.as_ref(), &mut events);
        }
        if self.plan_complete && state == MissionState::Enroute {
            let Some(extra_action) = self.continue_core(now, solution.as_ref(), &mut events) else {
                return failed_output(events);
            };
            if action.is_none() {
                action = extra_action;
            }
            state = self.state();
        }
        let intent = self.intent_for_state(state, solution.as_ref(), now, &mut events);
        MissionOutput {
            intent,
            action,
            state,
            events,
        }
    }

    fn continue_core(
        &mut self,
        now: MonotonicNanos,
        solution: Option<&NavigationSolution>,
        events: &mut Vec<MissionEvent>,
    ) -> Option<Option<MissionAction>> {
        let observation = self.core_observation(solution);
        let output = self.tick_core(now, observation, events)?;
        Some(self.apply_core_output(&output, events))
    }

    fn tick_core(
        &mut self,
        now: MonotonicNanos,
        observation: CoreObservation,
        events: &mut Vec<MissionEvent>,
    ) -> Option<CoreTickOutput> {
        if self.core.is_none() && !self.start_core(now, events) {
            return None;
        }
        let input = TickInput {
            mission_time_ns: self.sequencing_time(now).as_nanos(),
            // An operational mission runs on its own clock; no simulator supplies one.
            simulator_time_ns: None,
            wall_time_ns: now.as_nanos(),
            observation,
            receipts: self.pending_receipt.take().into_iter().collect(),
        };
        let result = self.core.as_mut()?.tick(input);
        match result {
            Ok(output) => Some(output),
            Err(error) => {
                self.refuse_core(error.to_string(), events);
                None
            }
        }
    }

    fn start_core(&mut self, now: MonotonicNanos, events: &mut Vec<MissionEvent>) -> bool {
        let now_ns = now.as_nanos();
        let expires_at_ns = now_ns.saturating_add(OPERATIONAL_WALL_DEADLINE_NS);
        let start = EngineStart {
            host_target: self.config.execution_target,
            mission_time_ns: self.sequencing_time(now).as_nanos(),
            simulator_time_ns: None,
            wall_time_ns: now_ns,
            wall_deadline: WallDeadline {
                mission_content_digest: self.document.identity.content_digest,
                expires_at_ns,
            },
        };
        match CoreMissionEngine::start(self.document.clone(), start) {
            Ok(core) => {
                self.core = Some(core);
                true
            }
            Err(error) => {
                self.refuse_core(error.to_string(), events);
                false
            }
        }
    }

    fn refuse_core(&mut self, detail: String, events: &mut Vec<MissionEvent>) {
        self.core_failed = true;
        self.active_action = None;
        self.outstanding_arm = None;
        events.push(MissionEvent::MissionEngineRefused { detail });
    }

    fn core_observation(&self, solution: Option<&NavigationSolution>) -> CoreObservation {
        CoreObservation {
            navigation: NavigationObservation {
                guidance_valid: Some(solution.is_some()),
                plan_complete: Some(self.plan_complete),
                waypoints_captured: u32::try_from(self.waypoints_captured()).ok(),
                altitude_m: solution.map(|value| value.position.altitude_m),
            },
            vehicle: VehicleObservation::default(),
            signals: Vec::new(),
        }
    }

    fn apply_core_output(
        &mut self,
        output: &CoreTickOutput,
        events: &mut Vec<MissionEvent>,
    ) -> Option<MissionAction> {
        let mut action = None;
        for event in &output.events {
            match event {
                CoreEvent::PhaseCompleted { .. } => self.active_action = None,
                CoreEvent::ReceiptAccepted {
                    context, result, ..
                } if context.phase_id == ARM_PHASE_ID => {
                    self.arm_receipt_event(context.action_id.get(), result, events);
                }
                CoreEvent::DirectiveEmitted { directive } => {
                    action = self.interpret_directive(directive, events);
                }
                CoreEvent::Terminal { result } => {
                    self.active_action = None;
                    self.outstanding_arm = None;
                    match result {
                        CoreTerminal::Complete { .. } => {
                            events.push(MissionEvent::MissionComplete);
                        }
                        other => events.push(MissionEvent::MissionFailed {
                            result: other.clone(),
                        }),
                    }
                }
                _ => {}
            }
        }
        action
    }

    fn intent_for_state(
        &mut self,
        state: MissionState,
        solution: Option<&NavigationSolution>,
        now: MonotonicNanos,
        events: &mut Vec<MissionEvent>,
    ) -> Option<ControlIntent> {
        match state {
            MissionState::Climb => self.climb_intent(solution),
            MissionState::Enroute => self.follow_plan_intent(solution, now, events),
            MissionState::Complete => Some(zero_velocity_intent()),
            MissionState::AwaitSolution
            | MissionState::Arming
            | MissionState::Held
            | MissionState::Failed => None,
        }
    }

    /// A held tick: the ownship stays fresh for the first resumed tick,
    /// nothing is commanded, and the hold is announced once.
    fn held_output(&mut self, now: MonotonicNanos) -> MissionOutput {
        self.publish_solution(now);
        MissionOutput {
            intent: None,
            action: None,
            state: MissionState::Held,
            events: core::mem::take(&mut self.hold_events),
        }
    }

    fn climb_intent(&self, solution: Option<&NavigationSolution>) -> Option<ControlIntent> {
        let solution = solution?;
        let Some(CoreFlightAction::Climb { target_altitude_m }) = self.active_action.as_ref()
        else {
            return None;
        };
        if solution.position.altitude_m >= *target_altitude_m - CLIMB_CAPTURE_MARGIN_M {
            return None;
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

    fn climb_is_captured(&self, solution: Option<&NavigationSolution>) -> bool {
        let Some(solution) = solution else {
            return false;
        };
        let Some(CoreFlightAction::Climb { target_altitude_m }) = self.active_action.as_ref()
        else {
            return false;
        };
        solution.position.altitude_m >= *target_altitude_m - CLIMB_CAPTURE_MARGIN_M
    }

    fn follow_plan_intent(
        &mut self,
        solution: Option<&NavigationSolution>,
        now: MonotonicNanos,
        events: &mut Vec<MissionEvent>,
    ) -> Option<ControlIntent> {
        let solution = solution?;
        if !self.following() {
            return None;
        }
        if self.plan_complete {
            return Some(zero_velocity_intent());
        }
        self.guide_active_leg(solution, now, events)
    }

    fn advance_active_plan(
        &mut self,
        solution: Option<&NavigationSolution>,
        events: &mut Vec<MissionEvent>,
    ) -> bool {
        let Some(solution) = solution else {
            return false;
        };
        if !self.following() {
            return false;
        }
        self.advance_plan(solution, events);
        true
    }

    fn advance_plan(&mut self, solution: &NavigationSolution, events: &mut Vec<MissionEvent>) {
        if self.plan_complete {
            return;
        }
        match self
            .execution
            .advance(&solution.position, self.commanded_groundspeed_mps())
        {
            SequenceEvent::LegAdvanced {
                to_index, reason, ..
            } => events.push(MissionEvent::LegAdvanced { to_index, reason }),
            SequenceEvent::PlanComplete { .. } => self.plan_complete = true,
            _ => {}
        }
    }

    /// Publishes the solution for `now` and clears stale display data: the
    /// filter's solution under simulation truth, the latest fresh vehicle
    /// estimate otherwise.
    fn publish_solution(&mut self, now: MonotonicNanos) -> Option<NavigationSolution> {
        self.last_solution = match self.estimate_budget() {
            None => self.filter.tick(now),
            Some(budget) => {
                let solution = self
                    .estimate
                    .solution(now, &self.plane, self.config.clock, budget);
                if solution.is_none() && self.estimate.has_sample() {
                    self.counters.estimate_stale = self.counters.estimate_stale.wrapping_add(1);
                }
                solution
            }
        };
        self.last_solution
    }

    fn guide_active_leg(
        &mut self,
        solution: &NavigationSolution,
        now: MonotonicNanos,
        events: &mut Vec<MissionEvent>,
    ) -> Option<ControlIntent> {
        let (leg_from, leg_to): (Option<GeodeticPosition>, Waypoint) = {
            let leg = self.execution.active_leg()?;
            (leg.from.map(|waypoint| waypoint.position), leg.to.clone())
        };
        let leg_from = self.track_start(&solution.position, leg_from, &leg_to, events);
        let guided = guide_velocity(
            solution,
            leg_from.as_ref(),
            &leg_to,
            now,
            self.config.clock,
            &self.guidance,
        );
        match guided {
            Ok(command) => self.guided_intent(command.setpoint),
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

    fn guided_intent(&mut self, setpoint: GuidanceSetpoint) -> Option<ControlIntent> {
        self.last_refusal = None;
        let GuidanceSetpoint::Velocity { velocity } = setpoint else {
            self.counters.guidance_refused = self.counters.guidance_refused.wrapping_add(1);
            return None;
        };
        let Some(yaw) = self.last_yaw_rad else {
            self.counters.missing_yaw = self.counters.missing_yaw.wrapping_add(1);
            return None;
        };
        Some(self.compose_intent(&velocity, yaw))
    }

    fn commanded_groundspeed_mps(&self) -> f64 {
        self.config
            .cruise_mps
            .min(self.config.limits.max_horizontal_mps)
    }

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
