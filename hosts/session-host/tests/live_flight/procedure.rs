//! Live simulation procedure through the public client-session API.

use std::time::{Duration, Instant};

use pilotage_client_session::{ActionRequestIdentity, ModuleEvent};
use pilotage_protocol::wire;

use super::Client;

const MOTION: &str = "vehicle.motion";

impl Client {
    pub(crate) async fn takeoff_exercise(&mut self) -> Result<(), String> {
        self.lease_with_recovery().await?;
        self.arm_on_ground().await?;
        let baseline = self
            .flight
            .height_snapshot()
            .ok_or("no fresh initial height")?;
        let (identity, result) = self.action_observed(wire::ControlAction::Takeoff).await?;
        if !result.accepted {
            return Err(format!("takeoff rejected: {}", result.detail));
        }
        tracing::info!(
            ?identity,
            baseline_m = baseline.0,
            "native takeoff admitted through WebTransport"
        );
        let deadline = Instant::now() + Duration::from_secs(35);
        loop {
            self.events.clear();
            self.progress(deadline).await?;
            if self.flight.invalidated {
                return Err("source identity changed during takeoff".to_owned());
            }
            let Some(state) = self.flight.matching_procedure(&identity) else {
                continue;
            };
            let phase = wire::FlightProcedurePhase::try_from(state.phase)
                .map_err(|_| "unknown procedure phase")?;
            if matches!(
                phase,
                wire::FlightProcedurePhase::Failed
                    | wire::FlightProcedurePhase::TimedOut
                    | wire::FlightProcedurePhase::Cancelled
            ) {
                return Err(format!("takeoff ended without completion: {state:?}"));
            }
            if phase == wire::FlightProcedurePhase::Completed
                && self.flight.landed() == Some(wire::LandedState::InAir)
                && self
                    .flight
                    .climb_from(&baseline)
                    .is_some_and(|climb| climb >= 2.0)
                && self.flight.armed() == Some(true)
            {
                tracing::info!(
                    ?identity,
                    revision = state.revision,
                    height_m = self.flight.height(),
                    "takeoff observed complete with independent InAir and altitude"
                );
                return Ok(());
            }
        }
    }

    async fn lease_with_recovery(&mut self) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.engine.admission().is_none() {
            self.progress(deadline).await?;
        }
        let admission = self.engine.admission().ok_or("admission disappeared")?;
        if admission.physically_embodied {
            return Err("live takeoff test refuses a physical vehicle host".to_owned());
        }
        let vehicle = admission
            .vehicles
            .iter()
            .find(|vehicle| vehicle.scopes.iter().any(|scope| scope.scope == MOTION))
            .ok_or("no advertised vehicle.motion scope")?
            .vehicle_id;
        self.flight.vehicle = Some(vehicle);
        self.lease_motion(vehicle, deadline).await?;
        let actions = self.engine.release_lease(vehicle, MOTION);
        self.execute_checked(actions).await?;
        loop {
            if std::mem::take(&mut self.events).iter().any(|event| {
                matches!(event, ModuleEvent::LeaseReleased(result) if result.released
                    && result.vehicle.as_ref().is_some_and(|value| value.value == vehicle)
                    && result.scope.as_ref().is_some_and(|value| value.value == MOTION))
            }) {
                break;
            }
            self.progress(deadline).await?;
        }
        let generation = self.lease_motion(vehicle, deadline).await?;
        loop {
            if std::mem::take(&mut self.events).iter().any(|event| {
                matches!(event, ModuleEvent::LinkLossCleared(result)
                    if result.vehicle.as_ref().is_some_and(|value| value.value == vehicle)
                    && result.scope.as_ref().is_some_and(|value| value.value == MOTION)
                    && result.generation.as_ref().is_some_and(|value| value.value == generation))
            }) {
                tracing::info!(
                    vehicle,
                    generation,
                    "motion lease recovered on neutral input"
                );
                return Ok(());
            }
            self.progress(deadline).await?;
        }
    }

    async fn lease_motion(&mut self, vehicle: u64, deadline: Instant) -> Result<u64, String> {
        self.events.clear();
        let actions = self.engine.request_lease(vehicle, MOTION);
        self.execute_checked(actions).await?;
        loop {
            for event in std::mem::take(&mut self.events) {
                if let ModuleEvent::Lease(result) = event
                    && result
                        .vehicle
                        .as_ref()
                        .is_some_and(|value| value.value == vehicle)
                    && result
                        .scope
                        .as_ref()
                        .is_some_and(|value| value.value == MOTION)
                {
                    if !result.granted {
                        return Err(format!("motion lease refused: {result:?}"));
                    }
                    return result
                        .generation
                        .map(|value| value.value)
                        .ok_or_else(|| "motion lease has no generation".to_owned());
                }
            }
            self.progress(deadline).await?;
        }
    }

    async fn arm_on_ground(&mut self) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            let (_, result) = self.action_observed(wire::ControlAction::Arm).await?;
            tracing::info!(
                accepted = result.accepted,
                detail = result.detail,
                "arm verdict"
            );
            let observe_until = deadline.min(Instant::now() + Duration::from_secs(3));
            while Instant::now() < observe_until {
                if self.flight.armed() == Some(true)
                    && self.flight.landed() == Some(wire::LandedState::OnGround)
                    && self.flight.height().is_some()
                {
                    return Ok(());
                }
                self.events.clear();
                self.progress(observe_until).await?;
            }
        }
        Err("SITL did not report armed and on-ground before the deadline".to_owned())
    }

    pub(crate) async fn disarm_to_ground(&mut self) -> Result<(), String> {
        if self.flight.armed() == Some(false)
            && self.flight.landed() == Some(wire::LandedState::OnGround)
        {
            return Ok(());
        }
        if !self.engine.holds_control() {
            return Err("cannot send cleanup disarm without the motion lease".to_owned());
        }
        let deadline = Instant::now() + Duration::from_secs(45);
        while Instant::now() < deadline {
            let (_, result) = self.action_observed(wire::ControlAction::Disarm).await?;
            tracing::info!(
                accepted = result.accepted,
                detail = result.detail,
                "cleanup disarm"
            );
            let observe_until = deadline.min(Instant::now() + Duration::from_secs(3));
            while Instant::now() < observe_until {
                if self.flight.armed() == Some(false)
                    && self.flight.landed() == Some(wire::LandedState::OnGround)
                {
                    tracing::info!("SITL is observed disarmed and on-ground");
                    return Ok(());
                }
                self.events.clear();
                self.progress(observe_until).await?;
            }
        }
        Err("SITL disarm was not observed before the cleanup deadline".to_owned())
    }

    async fn action_observed(
        &mut self,
        action: wire::ControlAction,
    ) -> Result<(ActionRequestIdentity, wire::ControlActionResult), String> {
        let (vehicle, scope) = self.engine.control_target().ok_or("motion lease lost")?;
        let request = wire::ControlActionRequest {
            action: action as i32,
            ..wire::ControlActionRequest::default()
        };
        let prepared = self
            .engine
            .prepare_control_action_at(vehicle, &scope, request, self.sampled_at())
            .ok_or("action could not bind the active motion lease")?;
        if action == wire::ControlAction::Takeoff {
            self.flight.begin_takeoff(&prepared.identity);
        }
        self.events.clear();
        self.execute_checked(prepared.actions).await?;
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            for event in std::mem::take(&mut self.events) {
                if let ModuleEvent::ActionResult(result) = event
                    && result.action_id == prepared.identity.action_id
                    && result
                        .vehicle
                        .as_ref()
                        .is_some_and(|value| value.value == vehicle)
                    && result
                        .scope
                        .as_ref()
                        .is_some_and(|value| value.value == scope)
                    && result
                        .generation
                        .as_ref()
                        .is_some_and(|value| value.value == prepared.identity.generation)
                    && result
                        .sequence
                        .as_ref()
                        .is_some_and(|value| value.value == prepared.identity.sequence)
                {
                    return Ok((prepared.identity, result));
                }
            }
            self.progress(deadline).await?;
        }
    }
}
