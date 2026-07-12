//! Typed reference frames and six-DoF vehicle state (FRAME-01).
//!
//! A vehicle-neutral, frame-explicit state contract: every attitude,
//! position, velocity, angular velocity, and acceleration carries its
//! frame, epoch, clock domain, and time scale, and unit quaternions are
//! the single SO(3) rotation kernel. Composition and inversion are
//! checked operations — incompatible frames, epochs, clocks, or scales
//! fail with typed errors, and unknown wire frame ids fail closed.
//! Producer provenance (source identity, acquisition time, integrity,
//! authorization, coherence) rides [`Tagged::meta`] through every
//! accepted transform untouched.
//!
//! Aircraft horizon presentation is one adapter ([`ned_attitude`]) that
//! explicitly selects a NED reference; canonical state is never reduced
//! to pitch/bank/yaw or a down vector. The crate is `no_std` and
//! allocation-free, and contains no propagation: transforms between
//! frames are always supplied by a producer.
//!
//! Architecture rule for the later relativistic layer: a rotation may
//! embed exactly into a Lorentz transform, but a transform containing a
//! boost may never convert silently back into a rotation — that
//! embedding is one-way by construction and lives outside this crate.

#![no_std]

#[cfg(test)]
extern crate std;

mod error;
mod frame;
mod ned;
mod rotation;
mod tagged;
mod time;
mod transform;

pub use error::FrameError;
pub use frame::FrameId;
pub use ned::ned_attitude;
pub use rotation::{NotARotation, Quat};
pub use tagged::{
    Acceleration, AngularVelocity, Attitude, Pose, Position, Tagged, Twist, Velocity,
    transform_attitude, transform_position, transform_vector,
};
pub use time::{ClockDomain, Epoch, TimeScale};
pub use transform::{FrameTransform, ROTATION_NORM_TOLERANCE};
