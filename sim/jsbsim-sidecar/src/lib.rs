//! A JSBSim flight-dynamics sidecar for PX4 SITL.
//!
//! JSBSim integrates the vehicle's rigid body on the shared world's
//! ground ([`pilotage_world`]); the sidecar synthesizes the vehicle's
//! sensors from the true state and the world's magnetic field, and
//! speaks PX4's MAVLink simulator protocol in lockstep. A simulation
//! tool only: no flight build carries it.

pub mod aircraft;
pub mod args;
pub mod engine;
pub mod gimbal;
pub mod ground;
pub mod hil;
pub mod motor;
pub mod pacer;
pub mod rotation;
pub mod run;
pub mod sensors;
pub mod session;
pub mod simulation;
pub mod truth;
