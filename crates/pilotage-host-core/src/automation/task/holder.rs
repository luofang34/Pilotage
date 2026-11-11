//! The lease holder's frames keep flowing while the mission flies. From
//! the arm on, a tick whose engine gives no intent (it waits for the arm's
//! receipt, or for its next phase) frames zero velocity: a holder that
//! sends nothing is a silent holder, and the session engages link loss on
//! it, and a flight controller that hears no command after it armed
//! enters its own command-loss behaviour.

use pilotage_mission::MissionState;
use pilotage_protocol::ControlIntent;
use pilotage_timing::MonoTimestamp;

use super::MissionTask;
use super::frames::neutral_intent;

impl MissionTask {
    /// Frames this tick's intent, or zero velocity in a flown phase that
    /// gave none. False when the engine actor is gone.
    pub(super) fn frame_intent(
        &mut self,
        intent: Option<ControlIntent>,
        state: MissionState,
        nanos: u64,
    ) -> bool {
        let flown = matches!(
            state,
            MissionState::Arming | MissionState::Climb | MissionState::Enroute
        );
        let intent = match intent {
            Some(intent) => intent,
            None if flown => neutral_intent(),
            None => return true,
        };
        self.send_intent(intent, MonoTimestamp::from_nanos(nanos))
    }
}
