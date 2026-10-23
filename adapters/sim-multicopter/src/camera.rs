//! The simulated gimbal camera.
//!
//! The adapter hands the latest camera pose to a worker thread. The worker
//! renders the view from aerial imagery, applies any injected fault,
//! stamps the frame, and offers it to a bounded channel. A pose that
//! arrives while the worker is busy replaces the waiting one, and a frame
//! that finds the channel full is dropped, so rendering never slows the
//! control and telemetry path.

pub mod fault;
pub mod render;
mod worker;

pub use worker::CameraWorker;
