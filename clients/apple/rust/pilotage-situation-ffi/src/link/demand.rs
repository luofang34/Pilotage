//! Demand routing onto NAMED lanes: the flight axes and the arm and
//! disarm pair belong to the motion scope, whichever auxiliary lanes
//! are also held. Inferring "the" lane from iteration order routed
//! flight into the gimbal's fencing the moment that lease landed, and
//! the silence watchdog took the motion lease while the gimbal stream
//! flowed on.

use pilotage_client_session::{
    ClientAction, ControlCommand, MotionDemand, intent_capability, velocity_intent,
};
use pilotage_control_web::{GIMBAL_SCOPE, MOTION_SCOPE};

/// How long the shell may stay silent before the link speaks a neutral
/// frame for it. The host's silence watchdog revokes a holder after one
/// quiet second; an interface thread stalled by a busy frame must never
/// cost the operator the lease, so the link — whose loop no interface
/// stall can touch — fills the gap well inside the window.
const DEMAND_SILENCE_FILL_MS: u64 = 150;
use pilotage_protocol::wire;

use super::driver::Link;
use super::records::LinkEvent;

impl Link {
    /// Builds and sends one fenced motion frame, or nothing without a
    /// lease and an advertised envelope. The motion scope is named, not
    /// inferred: with a gimbal lane also open, "whichever lane comes
    /// first" silently routes flight demands into the gimbal's fencing
    /// and the silence watchdog takes the motion lease one second later.
    pub(super) fn motion_actions(&mut self, demand: MotionDemand) -> Vec<ClientAction> {
        self.last_demand_ms = self.now_ms();
        self.send_motion(demand)
    }

    /// Speaks a neutral frame when the shell has gone quiet while the
    /// motion lease is held: holder liveness belongs to the link, not
    /// to the interface thread's fortunes.
    pub(super) fn keepalive_actions(&mut self) -> Vec<ClientAction> {
        let Some(vehicle_id) = self
            .engine
            .admission()
            .and_then(|admission| admission.vehicles.first())
            .map(|vehicle| vehicle.vehicle_id)
        else {
            return Vec::new();
        };
        if !self.engine.holds(vehicle_id, MOTION_SCOPE) {
            return Vec::new();
        }
        if self.now_ms().saturating_sub(self.last_demand_ms) < DEMAND_SILENCE_FILL_MS {
            return Vec::new();
        }
        self.send_motion(MotionDemand {
            roll: 0.0,
            pitch: 0.0,
            throttle: 0.0,
            yaw: 0.0,
        })
    }

    fn send_motion(&mut self, demand: MotionDemand) -> Vec<ClientAction> {
        let Some(admission) = self.engine.admission().cloned() else {
            return Vec::new();
        };
        let Some(vehicle_id) = admission.vehicles.first().map(|v| v.vehicle_id) else {
            return Vec::new();
        };
        let scope = MOTION_SCOPE.to_owned();
        let capability =
            intent_capability(&admission, vehicle_id, &scope, wire::IntentFamily::Velocity);
        let Some(intent) = velocity_intent(demand, capability) else {
            return Vec::new();
        };
        self.stats.control_frames = self.stats.control_frames.wrapping_add(1);
        let sampled_at_nanos = self.now_ms().saturating_mul(1_000_000);
        self.engine.control_frame(
            vehicle_id,
            &scope,
            ControlCommand::Intent(intent),
            sampled_at_nanos,
        )
    }

    /// The wire `ControlAction` code for a gimbal recenter — the
    /// discrete press that AIMS the payload (a held lease alone is not
    /// an aim), which is what makes the producer render its view.
    const ACTION_GIMBAL_RECENTER: i32 = 4;
    /// The video source ids the shells bind, the same table the
    /// browser pins: 0 forward (FPV), 2 payload (gimbal).
    const SOURCE_FPV: u8 = 0;
    const SOURCE_GIMBAL: u8 = 2;

    /// Steers the producer toward one source. The simulator host
    /// renders one camera at a time and the payload view follows the
    /// gimbal scope's engagement, so this drives THAT: gimbal =
    /// acquire-and-aim, forward = release. Any other source is a local
    /// display choice with nothing to ask of the host.
    pub(super) fn select_video_source_actions(&mut self, source: u8) -> Vec<ClientAction> {
        let Some(vehicle_id) = self
            .engine
            .admission()
            .and_then(|admission| admission.vehicles.first())
            .map(|vehicle| vehicle.vehicle_id)
        else {
            return Vec::new();
        };
        match source {
            Self::SOURCE_GIMBAL => {
                if self.engine.holds(vehicle_id, GIMBAL_SCOPE) {
                    self.pending_gimbal_engage = false;
                    return self.gimbal_engage_command(vehicle_id);
                }
                self.pending_gimbal_engage = true;
                self.engine.request_lease(vehicle_id, GIMBAL_SCOPE)
            }
            Self::SOURCE_FPV => {
                self.pending_gimbal_engage = false;
                if self.engine.holds(vehicle_id, GIMBAL_SCOPE) {
                    // Releasing the scope clears the payload view; the
                    // host's fail-closed clear returns the forward
                    // picture without a second command.
                    return self.engine.release_lease(vehicle_id, GIMBAL_SCOPE);
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Completes a pending payload engagement once the gimbal grant
    /// lands; the tick calls this alongside the reset counterpart.
    pub(super) fn pending_gimbal_engage_actions(&mut self) -> Vec<ClientAction> {
        if !self.pending_gimbal_engage {
            return Vec::new();
        }
        let Some(vehicle_id) = self
            .engine
            .admission()
            .and_then(|admission| admission.vehicles.first())
            .map(|vehicle| vehicle.vehicle_id)
        else {
            return Vec::new();
        };
        if !self.engine.holds(vehicle_id, GIMBAL_SCOPE) {
            return Vec::new();
        }
        self.pending_gimbal_engage = false;
        self.gimbal_engage_command(vehicle_id)
    }

    fn gimbal_engage_command(&mut self, vehicle_id: u64) -> Vec<ClientAction> {
        self.engine.control_action(
            vehicle_id,
            GIMBAL_SCOPE,
            wire::ControlActionRequest {
                action: Self::ACTION_GIMBAL_RECENTER,
                mode_target: 0,
                action_id: 0,
            },
        )
    }

    /// The auxiliary scope carrying simulator lifecycle actions — the
    /// same identity the browser's SIM_LIFECYCLE_SCOPE names. Only a
    /// simulator host advertises it.
    const LIFECYCLE_SCOPE: &'static str = "sim.lifecycle";
    /// The wire `ControlAction` code for a simulation reset.
    const ACTION_SIM_RESET: i32 = 5;

    /// Requests a simulation reset: sends the action when the
    /// lifecycle scope's authority is already held, otherwise asks for
    /// that authority and leaves the press pending for the grant. A
    /// host that does not advertise the action gets nothing.
    pub(super) fn sim_reset_actions(&mut self) -> Vec<ClientAction> {
        let Some(vehicle_id) = self.lifecycle_vehicle() else {
            self.delivery.event(LinkEvent::Notice {
                text: "sim reset not advertised by this host; not sent".to_owned(),
            });
            return Vec::new();
        };
        if self.engine.holds(vehicle_id, Self::LIFECYCLE_SCOPE) {
            self.pending_sim_reset = false;
            return self.sim_reset_command(vehicle_id);
        }
        self.pending_sim_reset = true;
        self.engine.request_lease(vehicle_id, Self::LIFECYCLE_SCOPE)
    }

    /// Completes a pending reset once the lifecycle grant lands; the
    /// tick calls this, so the press survives the authority round trip.
    pub(super) fn pending_sim_reset_actions(&mut self) -> Vec<ClientAction> {
        if !self.pending_sim_reset {
            return Vec::new();
        }
        let Some(vehicle_id) = self.lifecycle_vehicle() else {
            return Vec::new();
        };
        if !self.engine.holds(vehicle_id, Self::LIFECYCLE_SCOPE) {
            return Vec::new();
        }
        self.pending_sim_reset = false;
        self.sim_reset_command(vehicle_id)
    }

    /// The vehicle whose catalog advertises the reset action, if any.
    fn lifecycle_vehicle(&self) -> Option<u64> {
        self.engine.admission().and_then(|admission| {
            admission.vehicles.iter().find_map(|vehicle| {
                vehicle
                    .scopes
                    .iter()
                    .any(|scope| {
                        scope.scope == Self::LIFECYCLE_SCOPE
                            && scope
                                .actions
                                .iter()
                                .any(|action| action.action == Self::ACTION_SIM_RESET)
                    })
                    .then_some(vehicle.vehicle_id)
            })
        })
    }

    fn sim_reset_command(&mut self, vehicle_id: u64) -> Vec<ClientAction> {
        self.delivery.event(LinkEvent::Notice {
            text: "simulation reset requested".to_owned(),
        });
        self.engine.control_action(
            vehicle_id,
            Self::LIFECYCLE_SCOPE,
            wire::ControlActionRequest {
                action: Self::ACTION_SIM_RESET,
                mode_target: 0,
                action_id: 0,
            },
        )
    }

    /// Builds a discrete action command on the MOTION lane: arm and
    /// disarm belong to flight authority, never to whichever auxiliary
    /// lane happens to sort first.
    pub(super) fn action_actions(&mut self, code: i32) -> Vec<ClientAction> {
        let Some(vehicle_id) = self
            .engine
            .admission()
            .and_then(|admission| admission.vehicles.first())
            .map(|vehicle| vehicle.vehicle_id)
        else {
            return Vec::new();
        };
        self.engine.control_action(
            vehicle_id,
            MOTION_SCOPE,
            wire::ControlActionRequest {
                action: code,
                mode_target: 0,
                action_id: 0,
            },
        )
    }
}
