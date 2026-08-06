//! Dependency-free SHA-256 (FIPS 180-4) for `no_std` consumers.
//!
//! [`sha256`] is a `const fn` over a complete byte slice, so a content hash
//! can be evaluated at compile time and pinned without a build script or a
//! checked-in magic literal — the reason this is written here rather than
//! pulled from a dependency. [`Sha256Ctx`] streams the same digest over
//! input produced incrementally (for example a sequence of length-prefixed
//! scene buffers), without materializing the concatenation.
//!
//! Both front ends share one compression function, and correctness is
//! anchored by the published FIPS 180-4 test vectors plus split-point
//! equivalence tests in the accompanying test modules.

#![no_std]

#[cfg(test)]
extern crate std;

mod compress;
mod oneshot;
mod streaming;

pub use oneshot::sha256;
pub use streaming::Sha256Ctx;
