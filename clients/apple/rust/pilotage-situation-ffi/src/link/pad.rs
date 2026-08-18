//! The shared control runtime, driving the pad exactly as the browser
//! does (ADR-0007): the same compiled profile bytes, response curves,
//! gimbal quasimode, edge detection, and lease planning — evaluated
//! in-process instead of behind wasm. The shell samples the pad and
//! executes the returned plan; it holds no mapping of its own.

use pilotage_client_session::{ClientAction, MotionDemand, gimbal_rate_intent, intent_capability};
use pilotage_control_web::{
    AXIS_PITCH, AXIS_ROLL, AXIS_THROTTLE, AXIS_YAW, ArmConfirmed, ArmOrder, ButtonSample,
    ControlPlan, Frame, GIMBAL_SCOPE, LeaseAction, MOTION_SCOPE, Mode, RawSample, SessionState,
    TelegraphPhase,
};
use pilotage_protocol::wire;

use super::driver::Link;
use super::records::LinkEvent;

/// The wire code of the gimbal recenter action.
const ACTION_GIMBAL_RECENTER: i32 = 4;

/// Consecutive gated pad ticks under a held motion lease before the
/// stall is reported: one second at the shell's 20 Hz cadence. The
/// engine holding a lease the runtime refuses to feed is exactly the
/// silence that costs the lease one second later.
const GATED_TICKS_REPORTED: u32 = 20;

impl Link {
    /// Runs one raw pad sample through the shared runtime and executes
    /// the plan. The iPad has no mode picker yet; the pilot scheme is
    /// the camera-drone default the browser also starts from.
    pub(super) fn pad_actions(
        &mut self,
        axes: &[f32],
        values: &[f32],
        pressed: &[bool],
    ) -> Vec<ClientAction> {
        let raw: Vec<ButtonSample> = values
            .iter()
            .zip(pressed)
            .map(|(value, held)| ButtonSample {
                pressed: *held,
                value: *value,
            })
            .collect();
        let mut sample = RawSample::default();
        self.control.pad_sample(axes, &raw, &mut sample);
        self.runtime_actions(&sample)
    }

    /// Runs one tick synthesized from the held keys — the keyboard is
    /// a device layer of the same runtime, so curves, edges, and the
    /// quasimode apply to it unchanged.
    pub(super) fn key_actions(&mut self) -> Vec<ClientAction> {
        let mut sample = RawSample::default();
        self.control.key_sample(&mut sample);
        self.runtime_actions(&sample)
    }

    /// One canonical sample through the shared runtime and out as
    /// engine actions, whatever device produced it.
    fn runtime_actions(&mut self, sample: &RawSample) -> Vec<ClientAction> {
        let session = SessionState {
            now_ms: self.now_ms() as f64,
            mode: Mode::QuadPilot,
            connected: self.engine.admission().is_some(),
            input_lost: false,
        };
        let plan = self.control.evaluate(sample, &session);
        self.announce_device();
        self.execute_plan(&plan)
    }

    /// Announces the resolved device once its transactional swap lands:
    /// the label and the arm/disarm hints then describe the map that is
    /// actually reading the sticks, not the source it replaced.
    fn announce_device(&mut self) {
        let label = self.control.device_label();
        if label.is_empty() || label == self.announced_device {
            return;
        }
        self.announced_device = label.to_owned();
        self.delivery.event(LinkEvent::PadSelected {
            label: self.announced_device.clone(),
            arm_hint: self.control.arm_hint(),
            disarm_hint: self.control.disarm_hint(),
        });
    }

    fn execute_plan(&mut self, plan: &ControlPlan) -> Vec<ClientAction> {
        let Some(vehicle_id) = self
            .engine
            .admission()
            .and_then(|admission| admission.vehicles.first())
            .map(|vehicle| vehicle.vehicle_id)
        else {
            return Vec::new();
        };
        let mut actions = Vec::new();
        if let Some(frame) = plan.motion.as_ref() {
            actions.extend(self.motion_plan_actions(frame));
        }
        self.watch_gating(vehicle_id, plan);
        if let Some(frame) = plan.gimbal.as_ref() {
            actions.extend(self.gimbal_plan_actions(vehicle_id, frame));
        }
        match plan.lease {
            // The quasimode's auxiliary scope follows a HELD motion
            // lease. An admitted observer's ticks must lease nothing:
            // a bystander holding the gimbal is a camera nobody at the
            // sticks can move.
            Some(LeaseAction::Request) if self.engine.holds(vehicle_id, MOTION_SCOPE) => {
                actions.extend(self.engine.request_lease_quiet(vehicle_id, GIMBAL_SCOPE));
            }
            Some(LeaseAction::Release) => {
                actions.extend(self.engine.release_lease(vehicle_id, GIMBAL_SCOPE));
            }
            Some(LeaseAction::Request) | None => {}
        }
        // The runtime's own motion-lease plan stays unexecuted: on this
        // shell control is taken by a person, never by a reconnecting
        // state machine, so acquisition rides the arm press below.
        if plan.arm {
            actions.extend(self.order_actions(true));
        }
        if plan.disarm {
            actions.extend(self.standdown_actions());
        }
        if plan.arm_suppressed {
            actions.extend(self.arm_press_while_gated(vehicle_id));
        }
        if plan.disarm_suppressed {
            self.delivery
                .event(LinkEvent::PressSuppressed { action: 2 });
        }
        if plan.capture_active != self.capture_active {
            self.capture_active = plan.capture_active;
            self.delivery.event(LinkEvent::GimbalCapture {
                active: plan.capture_active,
            });
        }
        actions
    }

    /// An arm edge the runtime consumed while motion output was gated.
    /// Without the lease this press is the operator reaching for
    /// control, so it becomes the cooperative ask itself — one ask per
    /// answer, later presses wait. Holding the lease while gated
    /// (recovery in progress) keeps the loud suppression report: a
    /// swallowed safety press that stays silent is indistinguishable
    /// from a dead control.
    fn arm_press_while_gated(&mut self, vehicle_id: u64) -> Vec<ClientAction> {
        if self.engine.holds(vehicle_id, MOTION_SCOPE) || self.motion_request_pending {
            self.delivery
                .event(LinkEvent::PressSuppressed { action: 1 });
            return Vec::new();
        }
        let actions = self.engine.request_lease(vehicle_id, MOTION_SCOPE);
        if actions.is_empty() {
            // Not admitted: the press cannot become an ask, so it must
            // still be surfaced rather than vanish.
            self.delivery
                .event(LinkEvent::PressSuppressed { action: 1 });
            return actions;
        }
        self.motion_request_pending = true;
        self.delivery.event(LinkEvent::Notice {
            text: "asking for vehicle.motion".to_owned(),
        });
        actions
    }

    /// A live disarm edge. With the lever already on SAFE and the
    /// vehicle confirmed disarmed there is nothing left to stop, so
    /// the press means "stand down": the held scopes go back to the
    /// host. The screen lever never lands here — a redundant tap on
    /// SAFE is a no-op there — so only a deliberate control press
    /// releases.
    fn standdown_actions(&mut self) -> Vec<ClientAction> {
        let vehicle_id = self
            .engine
            .admission()
            .and_then(|admission| admission.vehicles.first())
            .map(|vehicle| vehicle.vehicle_id);
        let settled_safe = self.telegraph.order() == ArmOrder::Safe
            && self.telegraph.confirmed() == ArmConfirmed::Disarmed;
        if settled_safe
            && let Some(vehicle_id) = vehicle_id
            && self.engine.holds(vehicle_id, MOTION_SCOPE)
        {
            return self.release_held_actions();
        }
        self.order_actions(false)
    }

    /// Releasing control gives back everything held: the motion lane
    /// and the gimbal lane the runtime leased alongside it.
    pub(super) fn release_held_actions(&mut self) -> Vec<ClientAction> {
        match self
            .engine
            .admission()
            .and_then(|admission| admission.vehicles.first())
            .map(|vehicle| vehicle.vehicle_id)
        {
            Some(vehicle_id) => {
                let mut actions = self.engine.release_lease(vehicle_id, MOTION_SCOPE);
                actions.extend(self.engine.release_lease(vehicle_id, GIMBAL_SCOPE));
                actions
            }
            None => Vec::new(),
        }
    }

    /// The plan's motion frame through the typed velocity path — the
    /// same construction, envelope scaling, and body-frame rule the
    /// timer-driven neutral path uses.
    fn motion_plan_actions(&mut self, frame: &Frame) -> Vec<ClientAction> {
        let axis = |id: u16| {
            frame
                .axes()
                .iter()
                .find(|(axis_id, _)| *axis_id == id)
                .map_or(0.0, |(_, value)| *value)
        };
        self.motion_actions(MotionDemand {
            roll: axis(AXIS_ROLL),
            pitch: axis(AXIS_PITCH),
            throttle: axis(AXIS_THROTTLE),
            yaw: axis(AXIS_YAW),
        })
    }

    /// The plan's gimbal frame onto the gimbal lane: normalized LOS
    /// rates scaled by the advertised envelope, and the recenter edge
    /// as the reliable-stream action it is — an edge on the droppable
    /// datagram channel would be refused.
    fn gimbal_plan_actions(&mut self, vehicle_id: u64, frame: &Frame) -> Vec<ClientAction> {
        let Some(admission) = self.engine.admission().cloned() else {
            return Vec::new();
        };
        let axis = |id: u16| {
            frame
                .axes()
                .iter()
                .find(|(axis_id, _)| *axis_id == id)
                .map_or(0.0, |(_, value)| *value)
        };
        let capability = intent_capability(
            &admission,
            vehicle_id,
            GIMBAL_SCOPE,
            wire::IntentFamily::GimbalRate,
        );
        let Some(intent) = gimbal_rate_intent(axis(AXIS_PITCH), axis(AXIS_YAW), capability) else {
            return Vec::new();
        };
        let mut actions = Vec::new();
        if !frame.edges().is_empty() {
            actions.extend(self.engine.control_action(
                vehicle_id,
                GIMBAL_SCOPE,
                wire::ControlActionRequest {
                    action: ACTION_GIMBAL_RECENTER,
                    mode_target: 0,
                    action_id: 0,
                },
            ));
        }
        let sampled_at_nanos = self.now_ms().saturating_mul(1_000_000);
        self.stats.control_frames = self.stats.control_frames.wrapping_add(1);
        actions.extend(self.engine.control_frame(
            vehicle_id,
            GIMBAL_SCOPE,
            pilotage_client_session::ControlCommand::Intent(intent),
            sampled_at_nanos,
        ));
        actions
    }

    /// The loud check against silent gating: the engine holds the
    /// motion lease, yet the runtime's plan carried no motion frame.
    /// Left unnoticed this is the holder-silence revocation with a new
    /// face, so a sustained stall is reported instead of endured.
    fn watch_gating(&mut self, vehicle_id: u64, plan: &ControlPlan) {
        if plan.motion.is_none() && self.engine.holds(vehicle_id, MOTION_SCOPE) {
            self.gated_ticks = self.gated_ticks.wrapping_add(1);
            if self.gated_ticks == GATED_TICKS_REPORTED {
                self.delivery.event(LinkEvent::Notice {
                    text: "pad output gated while the motion lease is held; \
                           the silence watchdog will revoke it"
                        .to_owned(),
                });
            }
        } else {
            self.gated_ticks = 0;
        }
    }

    /// Moves the arm order lever and sends the one command the move
    /// asks for. Button edge and screen control land here alike: the
    /// telegraph is the only writer of arm intents.
    pub(super) fn order_actions(&mut self, armed: bool) -> Vec<ClientAction> {
        let order = if armed {
            ArmOrder::Armed
        } else {
            ArmOrder::Safe
        };
        let sent = self.telegraph.set_order(order);
        let actions = match sent {
            Some(order_action) => {
                self.action_actions(i32::try_from(order_action.action).unwrap_or(0))
            }
            None => Vec::new(),
        };
        self.publish_telegraph();
        actions
    }

    /// Tells the shell where order and answer stand, when that changed.
    pub(super) fn publish_telegraph(&mut self) {
        let confirmed = match self.telegraph.confirmed() {
            ArmConfirmed::Unknown => 0,
            ArmConfirmed::Disarmed => 1,
            ArmConfirmed::Armed => 2,
        };
        let (phase, detail) = match self.telegraph.phase() {
            TelegraphPhase::InSync => (0, String::new()),
            TelegraphPhase::AwaitingAnswer => (1, String::new()),
            TelegraphPhase::Refused(reason) => (2, reason.clone()),
            TelegraphPhase::Dropped => (3, String::new()),
        };
        let ordered_armed = self.telegraph.order() == ArmOrder::Armed;
        let picture = (ordered_armed, confirmed, phase, detail);
        if self.telegraph_shown.as_ref() == Some(&picture) {
            return;
        }
        self.telegraph_shown = Some(picture.clone());
        self.delivery.event(LinkEvent::ArmTelegraph {
            ordered_armed,
            confirmed,
            phase,
            detail: picture.3,
        });
    }

    /// Mirrors one engine authority fact into the runtime, so its plan
    /// gates on the same reality the wire enforces. The scope string
    /// picks the slot; unknown scopes are not the runtime's business.
    pub(super) fn mirror_authority(
        &mut self,
        scope: &str,
        event: pilotage_control_web::AuthorityEvent,
    ) {
        let slot = match scope {
            MOTION_SCOPE => pilotage_control_web::AuthorityScope::Motion,
            GIMBAL_SCOPE => pilotage_control_web::AuthorityScope::Gimbal,
            _ => return,
        };
        self.control.authority_event(slot, event);
    }
}
