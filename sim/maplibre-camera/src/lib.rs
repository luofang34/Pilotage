//! A simulator camera producer that renders a vehicle's forward and gimbal
//! cameras from the maplibre-rs world model (terrain and imagery) at the
//! vehicle's true pose, and streams the frames to its session host over
//! `pilotage.bridge.v1` (ADR-0008). Simulation only (ADR-0040): no flight
//! build links it.

pub mod args;
pub mod calibration;
pub mod error;
pub mod geometry;
pub mod link;
pub mod prewarm;
pub mod producer;
pub mod render;
pub mod rig;
pub mod style;
