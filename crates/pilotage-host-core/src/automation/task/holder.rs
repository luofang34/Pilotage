//! The lease holder's frames keep flowing from the lease on. A tick whose
//! engine gives no intent (it waits for a solution, for the vehicle to be
//! ready, for the arm's receipt, or for its next phase) frames the stop
//! the airframe can fly: zero velocity for a vehicle that can stop, the
//! present track and altitude for a fixed wing. Before the vehicle is
//! armed, a fixed wing with no solution yet frames zero velocity: a
//! vehicle that is not armed flies nothing. The session counts only a
//! frame with an intent as the holder's input, so a frame without one
//! cannot keep the lease. The engine's own intent, when it gives one, is
//! framed instead. A holder that sends nothing is a silent holder: the session
//! engages link loss on it and fences the mission, and a flight
//! controller that hears no command after it armed enters its own
//! command-loss behaviour.

use pilotage_mission::MissionState;
use pilotage_protocol::{AltitudeReference, ControlIntent, HeadingAltitudeIntent};
use pilotage_timing::MonoTimestamp;

use super::MissionTask;
use super::frames::neutral_intent;
use super::session_flow::AIRBORNE_HEIGHT_M;

impl MissionTask {
    /// Frames this tick's intent, or the airframe's stop in a phase before
    /// the end that gave none. False when the engine actor is gone.
    pub(super) fn frame_intent(
        &mut self,
        intent: Option<ControlIntent>,
        state: MissionState,
        nanos: u64,
    ) -> bool {
        let flown = matches!(
            state,
            MissionState::AwaitSolution
                | MissionState::Arming
                | MissionState::Climb
                | MissionState::Enroute
        );
        let intent = match intent {
            Some(intent) => intent,
            None if flown => {
                let unarmed = (!self.status.arm_accepted).then(neutral_intent);
                let Some(stop) = self.stop_intent().or(unarmed) else {
                    return true;
                };
                self.filler_sequence = Some(self.sequence.wrapping_add(1));
                stop
            }
            None => return true,
        };
        self.send_intent(Some(intent), MonoTimestamp::from_nanos(nanos))
    }

    /// What a held or stopping run frames: zero velocity for a vehicle
    /// that can stop in place; for a fixed wing, its present track and
    /// altitude held at its own cruise airspeed, or nothing when it has no
    /// solution to read them from.
    pub(super) fn stop_intent(&self) -> Option<ControlIntent> {
        if self.can_stop {
            return Some(neutral_intent());
        }
        let solution = self.mission.as_ref()?.solution()?;
        let track = solution
            .velocity
            .east_mps
            .atan2(solution.velocity.north_mps);
        Some(ControlIntent::HeadingAltitude(HeadingAltitudeIntent {
            heading_rad: track as f32,
            altitude_m: solution.position.altitude_m as f32,
            reference: AltitudeReference::Ellipsoid,
            airspeed_mps: None,
        }))
    }

    /// Notes the launch of a real vehicle whose mission waits for it: the
    /// engine's launch detection (the flight controller's height or
    /// airspeed), or the ownship's height above the launch point when the
    /// mission has one.
    pub(super) fn check_launch(&mut self, ownship_height_m: Option<f64>) {
        if !self.awaiting_launch {
            return;
        }
        let now = self.mission_now();
        let detected = self
            .mission
            .as_ref()
            .is_some_and(|mission| mission.launched(now));
        let height_m = ownship_height_m.unwrap_or(0.0);
        if detected || height_m >= AIRBORNE_HEIGHT_M {
            self.on_airborne(height_m);
        }
    }
}
