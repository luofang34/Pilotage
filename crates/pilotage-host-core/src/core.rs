//! The core and its calls.

mod handover;

use std::time::Duration;

use navigate_contract::{GeodeticPosition, MonotonicNanos};
use pilotage_adapter_aviate::AviateAdapter;
use pilotage_mission::{
    MissionEngine, MissionEvent, OnboardSolution, WaypointPlan, WaypointPlanRecord,
};
use pilotage_session::{SessionConfig, SessionEngine};
use pilotage_timing::StalenessPolicy;
use web_time::Instant;

use crate::HOST_VEHICLE;
use crate::automation::{MissionPlan, MissionTask, RunControl, TICK_INTERVAL as MISSION_TICK};
use crate::config::HostCoreConfig;
use crate::engine_actor::{EngineActor, TICK_INTERVAL};
use crate::error::HostCoreError;
use crate::output::{HostEvent, HostStatus, NextWake};

/// Maximum age a control frame may have before the engine rejects it as
/// stale (ADR-0009), as in the session host.
const MAX_CONTROL_AGE: Duration = Duration::from_millis(250);

/// The most commands and replies one call moves between the principal and
/// the engine before it stops: a bound against a loop that never settles.
const MAX_EXCHANGES: usize = 64;

/// One vehicle's Pilotage host, stepped by the embedding host.
///
/// Every time argument is nanoseconds since `origin`, the instant given
/// to [`Self::new`], on the embedding host's clock. Times never go back.
pub struct HostCore {
    config: HostCoreConfig,
    origin: Instant,
    last_now: MonotonicNanos,
    actor: EngineActor<AviateAdapter>,
    principal: Option<Box<MissionTask>>,
    /// A principal over a plan loaded while `principal` flew, until it
    /// takes the motion scope over or gives up.
    incoming: Option<handover::Incoming>,
    next_engine_tick: u64,
    next_mission_tick: u64,
    status: HostStatus,
    events: Vec<HostEvent>,
    /// The mission's counters went to the log when the plan completed.
    counters_logged: bool,
    /// The message ids on the telemetry link the core skips, each logged
    /// once.
    unknown_ids: std::collections::BTreeSet<u32>,
}

impl core::fmt::Debug for HostCore {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("HostCore")
            .field("last_now", &self.last_now)
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

impl HostCore {
    /// A core for one vehicle, at time zero.
    ///
    /// # Errors
    ///
    /// [`HostCoreError::Adapter`] when the adapter does not accept the
    /// airframe, the link configuration or the origin height.
    pub fn new(config: HostCoreConfig, origin: Instant) -> Result<Self, HostCoreError> {
        let mut adapter = AviateAdapter::embedded_for(
            HOST_VEHICLE,
            config.link,
            config.airframe.clone(),
            config.incarnations,
            origin,
        )
        .map_err(|source| HostCoreError::Adapter {
            source: Box::new(source),
        })?;
        // Before the session reads the capabilities the adapter advertises.
        if let Some(height_m) = config.fc_origin_height_m {
            adapter
                .set_local_origin_height(height_m)
                .map_err(|source| HostCoreError::Adapter {
                    source: Box::new(source),
                })?;
        }
        let capabilities = pilotage_adapter_api::VehicleAdapter::capabilities(&adapter);
        let session = SessionEngine::new(
            capabilities,
            StalenessPolicy::new(MAX_CONTROL_AGE),
            SessionConfig::new(
                pilotage_protocol::SESSION_PROTOCOL_VERSION,
                env!("CARGO_PKG_VERSION"),
            ),
        );
        let actor = EngineActor::new(session, adapter, config.magnetic_epoch_unix_s);
        Ok(Self {
            config,
            origin,
            last_now: MonotonicNanos::from_nanos(0),
            actor,
            principal: None,
            incoming: None,
            next_engine_tick: 0,
            next_mission_tick: 0,
            status: HostStatus::default(),
            events: Vec::new(),
            counters_logged: false,
            unknown_ids: std::collections::BTreeSet::new(),
        })
    }

    /// Loads a waypoint plan as the mission to fly. The mission starts
    /// with the next tick: it opens its session, arms the vehicle, climbs,
    /// and flies the plan.
    ///
    /// While another plan holds the motion scope, the new plan takes the
    /// scope over by a transfer, and the flown plan keeps commanding the
    /// vehicle until the transfer commits: the flight controller is never
    /// left without a setpoint. [`HostEvent::NewPlanFlying`] says when the
    /// new plan flies; [`HostEvent::NewPlanRefused`] says when it did not
    /// take over and the flown plan goes on.
    ///
    /// # Errors
    ///
    /// [`HostCoreError::MissionBuild`] when the plan does not build under
    /// the core's mission configuration.
    pub fn load_mission(
        &mut self,
        plan: WaypointPlan,
    ) -> Result<WaypointPlanRecord, HostCoreError> {
        let mut config = self.config.mission.clone();
        config.magnetic_epoch = Some(pilotage_world::MagneticEpoch::from_unix_seconds(
            self.config.magnetic_epoch_unix_s,
        ));
        let (_, record) =
            MissionEngine::from_waypoints(&plan, config.clone()).map_err(|source| {
                HostCoreError::MissionBuild {
                    source: Box::new(source),
                }
            })?;
        // The engine holds the arm until the flight controller reports
        // itself ready, as the principal reads it from the telemetry.
        config.arm_when_vehicle_ready = true;
        // The datum the adapter turns commanded altitudes by, so the climb
        // can capture on the altitude the flight controller flies.
        config.fc_origin_height_m = self.config.fc_origin_height_m;
        let principal = MissionTask::new(MissionPlan::waypoints(plan, config), false);
        self.begin_principal(Box::new(principal));
        Ok(record)
    }

    /// Runs everything that is due at `now` and returns when to call
    /// again.
    ///
    /// # Errors
    ///
    /// [`HostCoreError::TimeWentBack`] when `now` is earlier than the time
    /// of an earlier call.
    pub fn tick(&mut self, now: MonotonicNanos) -> Result<NextWake, HostCoreError> {
        self.advance_to(now)?;
        loop {
            let engine_due = self.next_engine_tick <= now.as_nanos();
            let mission_due = self.principal.is_some() && self.next_mission_tick <= now.as_nanos();
            if !engine_due && !mission_due {
                break;
            }
            // At one time the engine steps first, then the mission, so the
            // mission reads the telemetry of the same time.
            let engine_first =
                !mission_due || (engine_due && self.next_engine_tick <= self.next_mission_tick);
            if engine_first {
                self.set_times(self.next_engine_tick);
                self.actor.tick();
                self.next_engine_tick = self.next_engine_tick.saturating_add(nanos(TICK_INTERVAL));
            } else {
                self.set_times(self.next_mission_tick);
                if let Some(principal) = self.principal.as_mut() {
                    principal.on_tick();
                }
                if let Some(incoming) = self.incoming.as_mut() {
                    incoming.task.on_tick();
                }
                self.next_mission_tick = self.next_mission_tick.saturating_add(nanos(MISSION_TICK));
            }
            self.exchange();
        }
        self.set_times(now.as_nanos());
        self.refresh_status();
        let mut at = self.next_engine_tick;
        if self.principal.is_some() {
            at = at.min(self.next_mission_tick);
        }
        Ok(NextWake {
            at: MonotonicNanos::from_nanos(at),
        })
    }

    /// One datagram the flight controller sent on its telemetry link. A
    /// message id the core does not decode is logged once, at debug level.
    pub fn on_downlink(&mut self, datagram: &[u8], now: MonotonicNanos) {
        self.set_times(now.as_nanos().max(self.last_now.as_nanos()));
        let stats = self.actor.adapter_mut().on_downlink(datagram);
        if let Some(id) = stats.last_unknown_id
            && self.unknown_ids.insert(id)
        {
            tracing::debug!(
                message_id = id,
                "the flight controller sends a message the core skips"
            );
        }
    }

    /// One datagram the flight controller sent on its command link.
    pub fn on_uplink_reply(&mut self, datagram: &[u8], now: MonotonicNanos) {
        self.set_times(now.as_nanos().max(self.last_now.as_nanos()));
        self.actor.adapter_mut().on_uplink_reply(datagram);
    }

    /// The MAVLink frames for the flight controller's command port since
    /// the last call, in send order.
    pub fn drain_uplink(&mut self) -> Vec<Vec<u8>> {
        let frames = self.actor.adapter_mut().drain_uplink();
        self.status.uplink_frames = self
            .status
            .uplink_frames
            .wrapping_add(u64::try_from(frames.len()).unwrap_or(u64::MAX));
        frames
    }

    /// One solution of the onboard navigator; the mission flies it when
    /// its ownship source is the onboard navigator. The solution's stamp
    /// is on [`crate::MISSION_CLOCK`] when its age is to count from its
    /// own time. What the navigator says of a position sensor it
    /// monitors travels with the solution
    /// ([`OnboardSolution::with_source_integrity`]).
    pub fn on_onboard_solution(
        &mut self,
        offered: impl Into<OnboardSolution>,
        now: MonotonicNanos,
    ) {
        let offered = offered.into();
        for principal in self.principals_mut() {
            if let Some(mission) = principal.mission_mut() {
                mission.on_onboard_solution(offered, now);
            }
        }
    }

    /// Stops the vehicle in place and pauses the plan's clock, under the
    /// held lease (ADR-0052).
    pub fn hold_mission(&mut self) {
        self.run_control(RunControl::Hold);
    }

    /// Continues a held plan from its active leg.
    pub fn resume_mission(&mut self) {
        self.run_control(RunControl::Resume);
    }

    /// Stops the vehicle in place, then releases the lease and ends the
    /// run.
    pub fn abort_mission(&mut self) {
        self.run_control(RunControl::Abort);
    }

    /// Arms manual exit from an active `HM` (`HoldManual`) leg on the
    /// flying mission.
    pub fn exit_hold(&mut self) {
        self.run_control(RunControl::ExitHold);
    }

    fn run_control(&mut self, control: RunControl) {
        if let Some(principal) = self.principal.as_mut() {
            principal.on_control(control);
        }
        self.exchange();
    }

    /// The position the mission declares as its start: the mission
    /// anchor, where the embedding host places the vehicle.
    #[must_use]
    pub fn declared_start(&self) -> GeodeticPosition {
        self.config.mission.anchor
    }

    /// What the operator stated about the vehicle's start (position,
    /// height, true heading, speed, each with its uncertainty), validated
    /// when the mission was built: the initial state the embedding host
    /// hands its onboard navigator. `None` before a mission is loaded or
    /// when the mission states none.
    #[must_use]
    pub fn navigator_initial_state(&self) -> Option<pilotage_mission::DeclaredStart> {
        self.principal
            .as_ref()
            .and_then(|principal| principal.mission())
            .and_then(MissionEngine::declared_start)
            .copied()
    }

    /// Where the core stands after its last call.
    #[must_use]
    pub fn status(&self) -> &HostStatus {
        &self.status
    }

    /// The events since the last call, in order.
    pub fn drain_events(&mut self) -> Vec<HostEvent> {
        core::mem::take(&mut self.events)
    }

    /// The instant all core times count from.
    #[must_use]
    pub fn origin(&self) -> Instant {
        self.origin
    }

    /// The principals the core steps: the flying one, and one over a
    /// newly loaded plan while it takes over.
    fn principals_mut(&mut self) -> impl Iterator<Item = &mut MissionTask> {
        self.principal
            .iter_mut()
            .chain(self.incoming.iter_mut().map(|incoming| &mut incoming.task))
            .map(|principal| &mut **principal)
    }

    fn advance_to(&mut self, now: MonotonicNanos) -> Result<(), HostCoreError> {
        if now < self.last_now {
            return Err(HostCoreError::TimeWentBack {
                now_ns: now.as_nanos(),
                previous_ns: self.last_now.as_nanos(),
            });
        }
        self.last_now = now;
        Ok(())
    }

    /// Sets the time of every part to `now_ns` since the origin.
    fn set_times(&mut self, now_ns: u64) {
        let now = Duration::from_nanos(now_ns);
        self.actor.set_now(now);
        self.actor.adapter_mut().set_now(self.origin + now);
        for principal in self.principals_mut() {
            principal.set_now(now_ns);
        }
    }

    /// Moves the principals' commands to the engine and the engine's
    /// replies to the principals until neither has more, then settles a
    /// handover between them.
    fn exchange(&mut self) {
        if let Some(principal) = self.principal.as_mut() {
            exchange_with(&mut self.actor, &mut self.events, principal);
        }
        if let Some(incoming) = self.incoming.as_mut() {
            exchange_with(&mut self.actor, &mut self.events, &mut incoming.task);
        }
        self.settle_handover();
    }

    fn refresh_status(&mut self) {
        let Some(principal) = self.principal.as_ref() else {
            return;
        };
        let automation = principal.status();
        let rejected_before = self.status.frames_rejected;
        self.status.mission_state = automation.mission_state;
        self.status.estimate = principal
            .mission()
            .and_then(MissionEngine::solution)
            .cloned();
        self.status.plan_complete = automation.plan_complete;
        self.status.navigation = automation.navigation.clone();
        self.status.frames_sent = automation.frames_sent;
        self.status.frames_rejected = automation.frames_rejected;
        self.status.frame_digest = automation.frame_digest;
        self.status.armed = automation.arm_accepted;
        if automation.frames_rejected != rejected_before {
            self.events.push(HostEvent::FrameRejected);
        }
        if automation.plan_complete && !self.counters_logged {
            self.counters_logged = true;
            principal.log_counters();
        }
    }
}

/// Moves `principal`'s commands to the engine and the engine's replies to
/// `principal` until neither has more, and its events to `events`.
fn exchange_with(
    actor: &mut EngineActor<AviateAdapter>,
    events: &mut Vec<HostEvent>,
    principal: &mut MissionTask,
) {
    for _ in 0..MAX_EXCHANGES {
        let commands = principal.take_outbox();
        for command in commands {
            actor.handle_command(command);
        }
        let replies = actor.take_messages(principal.client());
        if replies.is_empty() && principal_idle(principal) {
            break;
        }
        for reply in replies {
            principal.on_connection_message(reply);
        }
    }
    for event in principal.take_events() {
        // The engine's own answer to its arm: accepted only once the
        // flight controller confirmed it (arm receipt), refused when the
        // controller refused it or did not answer in time.
        match event {
            MissionEvent::ArmAccepted { .. } => events.push(HostEvent::ArmConfirmed),
            MissionEvent::ArmRejected { .. } => events.push(HostEvent::ArmRefused),
            _ => {}
        }
        events.push(HostEvent::Mission(event));
    }
}

/// Whether the principal has no command waiting.
fn principal_idle(principal: &mut MissionTask) -> bool {
    principal.outbox_is_empty()
}

fn nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}
