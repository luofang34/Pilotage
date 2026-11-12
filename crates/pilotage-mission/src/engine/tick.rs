//! One operational tick through the shared sequencing core.

use core::mem::discriminant;

use navigate_contract::{
    GeodeticPosition, GuidanceSetpoint, LateralReference, MonotonicNanos, NavigationSolution,
    NedVelocity, Waypoint,
};
use navigate_guidance::guide_velocity;
use navigate_route::{SequenceEvent, SequenceReason};
use pilotage_mission_core::{
    EngineEvent as CoreEvent, EngineStart, MissionEngine as CoreMissionEngine,
    MissionObservation as CoreObservation, MissionTerminal as CoreTerminal, NavigationObservation,
    TickInput, TickOutput as CoreTickOutput, VehicleObservation, WallDeadline,
};
use pilotage_protocol::ControlIntent;

use crate::body_frame::{project_guidance_setpoint, project_ned_velocity};
use crate::policy::OPERATIONAL_WALL_DEADLINE_NS;

use super::document::ARM_PHASE_ID;
use super::{MissionAction, MissionEngine, MissionEvent, MissionOutput, MissionState};

mod climb;
mod directives;
pub(super) mod outputs;
mod overflight;
mod speed;
mod terminal;
mod wind_turn;

use outputs::{failed_output, terminal_output};

impl MissionEngine {
    /// Advances the operational mission with one caller clock value.
    ///
    /// The wrapper supplies the same monotonic value as simulator time and
    /// wall time. It performs no clock read and no input or output operation.
    pub fn tick(&mut self, now: MonotonicNanos) -> MissionOutput {
        match self.state() {
            MissionState::Complete => return self.complete_output(now),
            MissionState::Failed => return terminal_output(MissionState::Failed),
            MissionState::Held => return self.held_output(now),
            MissionState::AwaitSolution
            | MissionState::Arming
            | MissionState::Climb
            | MissionState::Enroute => {}
        }
        let mut events = core::mem::take(&mut self.hold_events);
        let solution = self.publish_solution(now);
        self.report_dead_reckoning(solution.is_some(), &mut events);
        let mut plan_advanced = false;
        if self.state() == MissionState::Enroute {
            plan_advanced = self.advance_active_plan(solution.as_ref(), &mut events);
        }
        let observation = self.core_observation(solution.as_ref(), now);
        let Some(core_output) = self.tick_core(now, observation, &mut events) else {
            return failed_output(events);
        };
        let mut action = self.apply_core_output(&core_output, &mut events);
        let mut state = self.state();
        if state == MissionState::Climb && self.climb_is_captured(solution.as_ref(), now) {
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
        let withheld = if state == MissionState::AwaitSolution {
            self.start_withheld_reason(solution.as_ref(), now)
        } else {
            None
        };
        self.announce_start(withheld, &mut events);
        let action = match action {
            Some(arm) => self.gate_arm(arm, now, &mut events),
            None => self.release_held_arm(now, &mut events),
        };
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
        let observation = self.core_observation(solution, now);
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
        self.drop_held_arm();
        events.push(MissionEvent::MissionEngineRefused { detail });
    }

    fn core_observation(
        &self,
        solution: Option<&NavigationSolution>,
        now: MonotonicNanos,
    ) -> CoreObservation {
        CoreObservation {
            navigation: NavigationObservation {
                guidance_valid: Some(solution.is_some()),
                plan_complete: Some(self.plan_complete),
                waypoints_captured: u32::try_from(self.waypoints_captured()).ok(),
                altitude_m: self.observed_altitude_m(solution, now),
            },
            vehicle: VehicleObservation {
                lifecycle: self.vehicle_lifecycle(now),
                ..VehicleObservation::default()
            },
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
                CoreEvent::PhaseEntered { phase_id, .. } => {
                    events.push(MissionEvent::TaskStarted {
                        task: phase_id.clone(),
                    })
                }
                CoreEvent::PhaseCompleted { phase_id, .. } => {
                    self.active_action = None;
                    events.push(MissionEvent::TaskFinished {
                        task: phase_id.clone(),
                    });
                }
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
                    self.drop_held_arm();
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
        if self.steers_by_heading() {
            return self.heading_intent_for_state(state, solution, now, events);
        }
        if matches!(state, MissionState::Climb | MissionState::Enroute) && solution.is_none() {
            return self.hold_position(now, events);
        }
        match state {
            MissionState::Climb => self.climb_intent(solution),
            MissionState::Enroute => self.follow_plan_intent(solution, now, events),
            MissionState::Complete => self.end_of_plan_intent(solution, now, events),
            MissionState::Arming if self.arm_request_sent() => self.arming_intent(solution),
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

    pub(super) fn follow_plan_intent(
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
            return self.end_of_plan_intent(Some(solution), now, events);
        }
        if self.route.approaching() {
            let last = *self.route.last_waypoint()?;
            return self.guide_to(
                solution,
                LateralReference::PresentPosition,
                &last,
                now,
                events,
            );
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
        if self.route.approaching() {
            self.plan_complete = self.route.arrive(&solution.position);
            return;
        }
        if let Some((to_index, cross_track_m)) = self.route.pass_start(&solution.position) {
            events.push(MissionEvent::LegAdvanced {
                to_index,
                reason: SequenceReason::Passed { cross_track_m },
            });
            return;
        }
        match self
            .route
            .advance(&solution.position, self.route_groundspeed_mps(solution))
        {
            SequenceEvent::LegAdvanced {
                to_index, reason, ..
            } => events.push(MissionEvent::LegAdvanced { to_index, reason }),
            SequenceEvent::PlanComplete { reason, .. } => {
                self.last_waypoint_sequenced(&solution.position, reason, events);
            }
            _ => {}
        }
    }

    /// Publishes the configured source's solution for `now`. A tick with
    /// no solution clears the published one, so display data cannot go
    /// stale.
    fn publish_solution(&mut self, now: MonotonicNanos) -> Option<NavigationSolution> {
        self.last_solution = self.source_solution(now);
        self.last_solution
    }

    fn guide_active_leg(
        &mut self,
        solution: &NavigationSolution,
        now: MonotonicNanos,
        events: &mut Vec<MissionEvent>,
    ) -> Option<ControlIntent> {
        let (leg_from, leg_to): (Option<GeodeticPosition>, Waypoint) = {
            let leg = self.route.active_leg()?;
            (leg.from.map(|waypoint| waypoint.position), *leg.to)
        };
        let leg_from = self.track_start(&solution.position, leg_from, &leg_to, events);
        let reference = leg_from.map_or(LateralReference::PresentPosition, LateralReference::track);
        self.guide_to(solution, reference, &leg_to, now, events)
    }

    /// Guides along `reference` to `to`.
    pub(super) fn guide_to(
        &mut self,
        solution: &NavigationSolution,
        reference: LateralReference,
        to: &Waypoint,
        now: MonotonicNanos,
        events: &mut Vec<MissionEvent>,
    ) -> Option<ControlIntent> {
        let guided = guide_velocity(
            solution,
            reference,
            to,
            now,
            self.config.clock,
            &self.guidance,
        );
        let guided = guided.map(|mut command| {
            let intercept = self.intercept_setpoint(command.setpoint, &reference, &to.position);
            command.setpoint =
                self.trimmed_setpoint(intercept, &reference, &to.position, solution, now);
            command
        });
        match guided {
            Ok(command) if self.steers_by_heading() => {
                self.last_refusal = None;
                self.steer(
                    solution,
                    &command.setpoint,
                    to.position.altitude_m,
                    now,
                    events,
                )
            }
            Ok(command) => {
                if !self.config.can_stop() {
                    self.update_wind(solution, now);
                }
                self.guided_intent(command.setpoint, solution.position.altitude_m)
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

    fn guided_intent(
        &mut self,
        setpoint: GuidanceSetpoint,
        altitude_m: f64,
    ) -> Option<ControlIntent> {
        self.last_refusal = None;
        if !matches!(
            setpoint,
            GuidanceSetpoint::Velocity { .. } | GuidanceSetpoint::DeviationTracking { .. }
        ) {
            self.counters.guidance_refused = self.counters.guidance_refused.wrapping_add(1);
            return None;
        }
        let Some(yaw) = self.last_yaw_rad else {
            self.counters.missing_yaw = self.counters.missing_yaw.wrapping_add(1);
            return None;
        };
        let intent = project_guidance_setpoint(
            &setpoint,
            yaw,
            &self.config.limits,
            self.config.yaw_gain_per_s,
        )?;
        if self.config.can_stop() {
            return Some(intent);
        }
        let intent = speed::at_most_cruise(intent, self.commanded_cruise_mps());
        Some(self.airspeed_for_track(intent, yaw, altitude_m))
    }

    #[allow(dead_code)]
    fn compose_intent(&self, velocity: &NedVelocity, yaw_rad: f64) -> ControlIntent {
        project_ned_velocity(
            velocity,
            yaw_rad,
            &self.config.limits,
            self.config.yaw_gain_per_s,
        )
    }
}
