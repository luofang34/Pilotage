//! Deterministic condition evaluation for caller observations.

use crate::{
    Comparison, MissionCondition, MissionObservation, NavigationCondition, SignalCondition,
    SimulatorCondition, VehicleCondition,
};

pub(crate) fn all_match(
    conditions: &[MissionCondition],
    simulator_time_ns: u64,
    observation: &MissionObservation,
) -> bool {
    conditions
        .iter()
        .all(|condition| matches(condition, simulator_time_ns, observation))
}

pub(crate) fn first_match(
    conditions: &[MissionCondition],
    simulator_time_ns: u64,
    observation: &MissionObservation,
) -> Option<usize> {
    conditions
        .iter()
        .position(|condition| matches(condition, simulator_time_ns, observation))
}

fn matches(
    condition: &MissionCondition,
    simulator_time_ns: u64,
    observation: &MissionObservation,
) -> bool {
    match condition {
        MissionCondition::Always {} => true,
        MissionCondition::Navigation(condition) => navigation(condition, observation),
        MissionCondition::Vehicle(condition) => vehicle(condition, observation),
        MissionCondition::Simulator(condition) => simulator(condition, simulator_time_ns),
        MissionCondition::Signal(condition) => signal(condition, observation),
    }
}

fn navigation(condition: &NavigationCondition, observation: &MissionObservation) -> bool {
    match condition {
        NavigationCondition::GuidanceValid { expected } => {
            observation.navigation.guidance_valid == Some(*expected)
        }
        NavigationCondition::PlanComplete { expected } => {
            observation.navigation.plan_complete == Some(*expected)
        }
        NavigationCondition::Altitude {
            comparison,
            value_m,
        } => observation
            .navigation
            .altitude_m
            .is_some_and(|actual| compare(*comparison, actual, *value_m)),
    }
}

fn vehicle(condition: &VehicleCondition, observation: &MissionObservation) -> bool {
    match condition {
        VehicleCondition::Lifecycle { state } => observation.vehicle.lifecycle == Some(*state),
        VehicleCondition::GroundContact { expected } => {
            observation.vehicle.ground_contact == Some(*expected)
        }
        VehicleCondition::Crashed { expected } => observation.vehicle.crashed == Some(*expected),
        VehicleCondition::LinkValid { expected } => {
            observation.vehicle.link_valid == Some(*expected)
        }
        VehicleCondition::EstimatorValid { expected } => {
            observation.vehicle.estimator_valid == Some(*expected)
        }
    }
}

fn simulator(condition: &SimulatorCondition, simulator_time_ns: u64) -> bool {
    let SimulatorCondition::Time {
        comparison,
        value_ns,
    } = condition;
    compare_u64(*comparison, simulator_time_ns, *value_ns)
}

fn signal(condition: &SignalCondition, observation: &MissionObservation) -> bool {
    let SignalCondition::Value {
        selector,
        comparison,
        value,
    } = condition;
    observation
        .signal(selector)
        .is_some_and(|actual| compare(*comparison, actual, *value))
}

fn compare(comparison: Comparison, actual: f64, expected: f64) -> bool {
    match comparison {
        Comparison::LessThan => actual < expected,
        Comparison::LessOrEqual => actual <= expected,
        Comparison::GreaterThan => actual > expected,
        Comparison::GreaterOrEqual => actual >= expected,
        Comparison::AbsoluteLessOrEqual => actual.abs() <= expected,
        Comparison::AbsoluteGreaterOrEqual => actual.abs() >= expected,
    }
}

fn compare_u64(comparison: Comparison, actual: u64, expected: u64) -> bool {
    match comparison {
        Comparison::LessThan => actual < expected,
        Comparison::LessOrEqual | Comparison::AbsoluteLessOrEqual => actual <= expected,
        Comparison::GreaterThan => actual > expected,
        Comparison::GreaterOrEqual | Comparison::AbsoluteGreaterOrEqual => actual >= expected,
    }
}
