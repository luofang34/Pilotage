//! Independent source observations used by the live takeoff test.

use std::time::{Duration, Instant};

use pilotage_client_session::{ActionRequestIdentity, ModuleEvent};
use pilotage_protocol::wire;

const FRESH_FOR: Duration = Duration::from_secs(3);

#[derive(Default)]
pub(crate) struct FlightObservation {
    pub(crate) vehicle: Option<u64>,
    arm: Option<Observation<u32>>,
    landed: Option<Observation<i32>>,
    height: Option<Observation<f64>>,
    active_request: Option<ActionRequestIdentity>,
    procedure: Option<(wire::FlightProcedureState, Instant)>,
    pub(crate) invalidated: bool,
}

struct Observation<T> {
    value: Option<T>,
    stamp: wire::MeasurementStamp,
    received: Instant,
}

impl FlightObservation {
    pub(super) fn observe(&mut self, event: &ModuleEvent) {
        let ModuleEvent::Telemetry(sample) = event else {
            return;
        };
        if sample.vehicle.as_ref().map(|vehicle| vehicle.value) != self.vehicle {
            return;
        }
        let mut changed = false;
        if let Some(fc) = sample.fc_state.as_ref() {
            changed |= update(
                &mut self.arm,
                fc.arm_state,
                fc.stamp.as_ref(),
                wire::SourceRole::FcState,
                wire::MeasurementClock::HostMonotonic,
            );
            if let Some(landed) = fc.landed_state.as_ref() {
                changed |= update(
                    &mut self.landed,
                    landed.state,
                    landed.stamp.as_ref(),
                    wire::SourceRole::FcState,
                    wire::MeasurementClock::HostMonotonic,
                );
            }
        }
        if let Some(avionics) = sample.avionics.as_ref()
            && avionics.valid_flags & 4 != 0
            && avionics.quality <= 1
            && avionics.pos_d_m.is_finite()
        {
            changed |= update(
                &mut self.height,
                -f64::from(avionics.pos_d_m),
                avionics.kinematics_stamp.as_ref(),
                wire::SourceRole::OperationalEstimate,
                wire::MeasurementClock::VehicleBoot,
            );
        }
        if changed {
            clear(&mut self.arm);
            clear(&mut self.landed);
            clear(&mut self.height);
            self.procedure = None;
            self.invalidated = self.active_request.is_some();
        }
        if let Some(procedure) = sample.flight_procedure.as_ref() {
            self.observe_procedure(procedure);
        }
    }

    pub(crate) fn begin_takeoff(&mut self, identity: &ActionRequestIdentity) {
        self.active_request = Some(identity.clone());
        self.procedure = None;
        self.invalidated = false;
    }

    fn observe_procedure(&mut self, next: &wire::FlightProcedureState) {
        let Some(identity) = self.active_request.as_ref() else {
            return;
        };
        if self.invalidated || !matches_request(next, identity) {
            return;
        }
        if let Some((current, _)) = self.procedure.as_ref() {
            if current.adapter_incarnation != next.adapter_incarnation {
                self.invalidated = true;
                self.procedure = None;
                return;
            }
            let distance = next.revision.wrapping_sub(current.revision);
            if distance == 0 || distance >= (1_u64 << 63) {
                return;
            }
        }
        self.procedure = Some((next.clone(), Instant::now()));
    }

    pub(crate) fn armed(&self) -> Option<bool> {
        match fresh(self.arm.as_ref())? {
            1 => Some(false),
            2 => Some(true),
            _ => None,
        }
    }

    pub(crate) fn landed(&self) -> Option<wire::LandedState> {
        wire::LandedState::try_from(fresh(self.landed.as_ref())?).ok()
    }

    pub(crate) fn height(&self) -> Option<f64> {
        fresh(self.height.as_ref())
    }

    pub(crate) fn height_snapshot(&self) -> Option<(f64, wire::MeasurementStamp)> {
        Some((self.height()?, self.height.as_ref()?.stamp.clone()))
    }

    pub(crate) fn climb_from(&self, baseline: &(f64, wire::MeasurementStamp)) -> Option<f64> {
        let (height, stamp) = self.height_snapshot()?;
        let reference = &baseline.1;
        (stamp.source_id == reference.source_id
            && stamp.source_incarnation == reference.source_incarnation
            && stamp.source_epoch == reference.source_epoch
            && stamp.clock == reference.clock
            && stamp.acquired_at_ns > reference.acquired_at_ns)
            .then_some(height - baseline.0)
    }

    pub(crate) fn matching_procedure(
        &self,
        identity: &ActionRequestIdentity,
    ) -> Option<&wire::FlightProcedureState> {
        let (state, received) = self.procedure.as_ref()?;
        (!self.invalidated
            && received.elapsed() < FRESH_FOR
            && self.vehicle == Some(identity.vehicle_id)
            && matches_request(state, identity))
        .then_some(state)
    }
}

fn matches_request(state: &wire::FlightProcedureState, identity: &ActionRequestIdentity) -> bool {
    let Some(request) = state.request.as_ref() else {
        return false;
    };
    request
        .session
        .as_ref()
        .is_some_and(|value| value.value == identity.session_id)
        && request
            .scope
            .as_ref()
            .is_some_and(|value| value.value == identity.scope)
        && request
            .generation
            .as_ref()
            .is_some_and(|value| value.value == identity.generation)
        && request
            .sequence
            .as_ref()
            .is_some_and(|value| value.value == identity.sequence)
        && request.action_id == identity.action_id
        && state.adapter_incarnation.len() == 16
        && state.kind == wire::FlightProcedureKind::Takeoff as i32
}

fn fresh<T: Copy>(value: Option<&Observation<T>>) -> Option<T> {
    let value = value?;
    (value.received.elapsed() < FRESH_FOR)
        .then_some(value.value)
        .flatten()
}

fn clear<T>(target: &mut Option<Observation<T>>) {
    if let Some(current) = target.as_mut() {
        current.value = None;
    }
}

fn update<T>(
    target: &mut Option<Observation<T>>,
    value: T,
    stamp: Option<&wire::MeasurementStamp>,
    role: wire::SourceRole,
    clock: wire::MeasurementClock,
) -> bool {
    let Some(stamp) = stamp.filter(|stamp| {
        stamp.source_incarnation.len() == 16
            && stamp.role == role as i32
            && stamp.clock == clock as i32
    }) else {
        return false;
    };
    let mut changed = false;
    if let Some(current) = target.as_ref() {
        let previous = &current.stamp;
        let same_attachment = previous.source_id == stamp.source_id
            && previous.source_incarnation == stamp.source_incarnation;
        changed = !same_attachment || previous.source_epoch != stamp.source_epoch;
        if same_attachment {
            if previous.source_epoch != stamp.source_epoch {
                if !serial_gt(stamp.source_epoch, previous.source_epoch) {
                    return false;
                }
            } else if !serial_gt(stamp.sequence, previous.sequence)
                || stamp.acquired_at_ns < previous.acquired_at_ns
            {
                return false;
            }
        }
    }
    *target = Some(Observation {
        value: Some(value),
        stamp: stamp.clone(),
        received: Instant::now(),
    });
    changed
}

fn serial_gt(next: u32, previous: u32) -> bool {
    next != previous && next.wrapping_sub(previous) < (1_u32 << 31)
}
