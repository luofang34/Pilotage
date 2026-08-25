use crate::identity::harness_build_identity;
use crate::{
    GateEvaluator, Journal, MetricEvaluator, ProposalStrategy, RuntimeIdentities, SimulatorBackend,
    SimulatorSessionReceipt, SimulatorVehicleFactory, TuneError, VehicleBinding,
};

pub(super) fn validate_component_identities<B, F, G, M, P>(
    backend: &B,
    factory: &F,
    gates: &G,
    metric: &M,
    strategy: &P,
) -> Result<(), TuneError>
where
    B: SimulatorBackend,
    F: SimulatorVehicleFactory,
    G: GateEvaluator,
    M: MetricEvaluator,
    P: ProposalStrategy,
{
    for identity in [
        backend.simulator_identity(),
        backend.airframe_identity(),
        factory.vehicle_identity(),
        factory.transition_validator_identity(),
        gates.identity(),
        metric.identity(),
        strategy.identity(),
    ] {
        if identity.digest.is_zero() || identity.id.trim().is_empty() {
            return Err(TuneError::InvalidIdentity {
                detail: "a runtime component identity is incomplete".to_owned(),
            });
        }
    }
    if factory.adjacency_policy_digest().is_zero() {
        return Err(TuneError::InvalidIdentity {
            detail: "the vehicle adjacency-policy identity is incomplete".to_owned(),
        });
    }
    Ok(())
}

pub(super) fn runtime_identities<B, F, G, M, P>(
    backend: &B,
    factory: &F,
    gates: &G,
    metric: &M,
    strategy: &P,
) -> RuntimeIdentities
where
    B: SimulatorBackend,
    F: SimulatorVehicleFactory,
    G: GateEvaluator,
    M: MetricEvaluator,
    P: ProposalStrategy,
{
    RuntimeIdentities {
        harness_build: harness_build_identity(),
        strategy: strategy.identity().clone(),
        metric: metric.identity().clone(),
        hard_gates: gates.identity().clone(),
        simulator: backend.simulator_identity().clone(),
        airframe: backend.airframe_identity().clone(),
        vehicle: factory.vehicle_identity().clone(),
        transition_validator: factory.transition_validator_identity().clone(),
        adjacency_policy_digest: factory.adjacency_policy_digest(),
    }
}

pub(super) fn validate_simulator_receipt(
    journal: &Journal,
    receipt: SimulatorSessionReceipt,
) -> Result<(), TuneError> {
    let runtimes = &journal.session().runtimes;
    if receipt.session_digest != journal.session_digest()?
        || receipt.simulator_digest != runtimes.simulator.digest
        || receipt.airframe_digest != runtimes.airframe.digest
    {
        return Err(TuneError::ReceiptMismatch {
            operation: "open simulator session",
            detail: "simulator session, build, or airframe digest does not match".to_owned(),
        });
    }
    Ok(())
}

pub(super) fn validate_vehicle_binding<V>(
    journal: &Journal,
    vehicle: &VehicleBinding<V>,
) -> Result<(), TuneError> {
    let receipt = vehicle.receipt();
    let transition = vehicle
        .transition_receipt()
        .ok_or_else(|| TuneError::ReceiptMismatch {
            operation: "bind simulator vehicle",
            detail: "the vehicle binding has no transition policy".to_owned(),
        })?;
    if receipt.session_digest != journal.session_digest()?
        || receipt.vehicle_digest != journal.session().runtimes.vehicle.digest
        || transition.session_digest() != receipt.session_digest
        || transition.validator() != &journal.session().runtimes.transition_validator
        || transition.adjacency_policy_digest()
            != journal.session().runtimes.adjacency_policy_digest
    {
        return Err(TuneError::ReceiptMismatch {
            operation: "bind simulator vehicle",
            detail: "vehicle session, build, or transition policy does not match".to_owned(),
        });
    }
    Ok(())
}
