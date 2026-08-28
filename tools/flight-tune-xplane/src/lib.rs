//! X-Plane projection for the simulator-neutral tuning runtime.
//!
//! SIM / NOT FOR FLIGHT.

#![forbid(unsafe_code)]

mod runtime;

pub use runtime::{
    VehicleFrameValues, XPlaneFrameProjection, XPlaneProjectionError, XPlaneScenarioRuntime,
    XPlaneSimulatorAction, XPlaneSimulatorActionDriver,
};
