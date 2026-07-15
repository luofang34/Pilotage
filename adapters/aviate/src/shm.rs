//! Pure-Rust reader for Aviate's Gazebo-bridge shared-memory state block
//! (ADR-0019: the co-located SITL vehicle link binds shared memory).
//!
//! Aviate's gz-sim plugin publishes `AviateSharedState` (its
//! `shared_state.h`, a standalone-C contract) into POSIX shm
//! `/aviate_gz_bridge[_<instance>]` every physics tick: ENU pose,
//! ENU/FLU quaternion, velocities, sim time, and a sequence counter.
//! This module attaches read-only as a second consumer beside the FC and
//! converts to the NED/FRD convention the telemetry plane carries,
//! mirroring Aviate's own conversion math exactly.
//!
//! The shared block is coherent simulator ground truth. Its sampler publishes
//! an explicit synthetic authorization stamp so consumers can distinguish
//! this source from FC estimator telemetry.

use std::ffi::CString;
use std::time::Instant;

use crate::error::AviateAdapterError;

// libc::dev_t differs in signedness and width across supported Unix targets.
#[allow(clippy::unnecessary_cast)]
const fn device_identity(device: libc::dev_t) -> u64 {
    device as u64
}

/// Byte count the reader maps and reads from `AviateSharedState`. This size
/// and the `OFF_*` offsets below hand-mirror the producer's `shared_state.h`
/// and are not validated against a magic/version/declared-size header, so a
/// producer layout change is not detected here. Capacity does not establish
/// layout compatibility: the check below only proves the object is large
/// enough to map, never that these offsets are still correct.
const SHM_SIZE: usize = 216;

/// Whether a POSIX shm object whose kernel-reported `st_size` is `st_size`
/// bytes can back a `required`-byte read-only mapping, returning the admitted
/// capacity. This is a CAPACITY decision only: the kernel may page-round the
/// object up (a 216-byte `ftruncate` reports `st_size = 16384` on a 16 KiB
/// page), so any capacity at least `required` is admitted and only `required`
/// bytes are ever mapped. A negative `st_size` (which must never be coerced to
/// a huge unsigned value) or one smaller than `required` is refused. It makes
/// no claim about field layout or version.
fn admissible_capacity(st_size: i64, required: usize) -> Option<u64> {
    let capacity = u64::try_from(st_size).ok()?;
    (capacity >= required as u64).then_some(capacity)
}

// Field offsets in `AviateSharedState` (shared_state.h) — provisional, see
// `SHM_SIZE`.
const OFF_POS: usize = 0; // double[3]
const OFF_QUAT: usize = 24; // double[4] (w, x, y, z), ENU/FLU
const OFF_VEL: usize = 56; // double[3], ENU world
const OFF_ANG_VEL: usize = 80; // double[3], FLU body
const OFF_TIME_US: usize = 104; // u64 sim time
const OFF_SEQ: usize = 112; // u32 update counter
const OFF_VALID: usize = 116; // u32 non-zero = valid
const OFF_PLUGIN_READY: usize = 192; // u32 set once plugin is up

/// One coherent ground-truth sample, already in NED/FRD.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GzStateSample {
    /// Attitude quaternion (w, x, y, z), body FRD → world NED.
    pub quat_wxyz: [f32; 4],
    /// Body rates (p, q, r) rad/s, FRD.
    pub rates_rps: [f32; 3],
    /// Position NED, meters.
    pub pos_ned_m: [f32; 3],
    /// Velocity NED, m/s.
    pub vel_ned_mps: [f32; 3],
    /// Simulation time in microseconds.
    pub time_us: u64,
    /// The block's update counter.
    pub seq: u32,
}

/// A read-only mapping of the Aviate gz-bridge block.
#[derive(Debug)]
pub struct GzStateShm {
    base: *const u8,
    identity: ShmIdentity,
}

/// Stable operating-system identity of one POSIX shared-memory object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShmIdentity {
    device: u64,
    inode: u64,
    size: u64,
}

/// Progress classification for one coherent SHM observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShmObservation {
    /// Sequence and acquisition time advanced in the same incarnation.
    Advancing,
    /// The same coherent sample remains mapped, with its frozen duration.
    Unchanged(std::time::Duration),
    /// Sequence or acquisition time rolled back on the same SHM object.
    Quarantined,
}

// SAFETY: the mapping is process-private, read-only, and lives for the
// adapter's lifetime; the raw pointer is only dereferenced through
// volatile byte copies in `raw_snapshot`.
#[allow(unsafe_code)]
unsafe impl Send for GzStateShm {}

impl GzStateShm {
    /// Attaches read-only to instance `instance`'s block
    /// (`/aviate_gz_bridge` for instance 0).
    ///
    /// # Errors
    ///
    /// Returns a typed [`AviateAdapterError`] when the region does not
    /// exist, is too small to back the mapping, or cannot be mapped.
    pub fn open(instance: u8) -> Result<Self, AviateAdapterError> {
        let name = if instance == 0 {
            "/aviate_gz_bridge".to_owned()
        } else {
            format!("/aviate_gz_bridge_{instance}")
        };
        let c_name = CString::new(name.clone()).map_err(|source| AviateAdapterError::ShmName {
            name: name.clone(),
            source,
        })?;
        // SAFETY: shm_open/mmap with a valid, NUL-terminated name;
        // read-only PROT_READ/O_RDONLY so no writer state can be
        // corrupted; fd is closed after mapping (POSIX keeps the mapping
        // alive); MAP_FAILED and negative fds are checked before use.
        #[allow(unsafe_code)]
        let (base, identity) = unsafe {
            let fd = libc::shm_open(c_name.as_ptr(), libc::O_RDONLY, 0);
            if fd < 0 {
                return Err(AviateAdapterError::ShmIo {
                    name,
                    operation: "shm_open",
                    source: std::io::Error::last_os_error(),
                });
            }
            let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
            if libc::fstat(fd, metadata.as_mut_ptr()) != 0 {
                let source = std::io::Error::last_os_error();
                libc::close(fd);
                return Err(AviateAdapterError::ShmIo {
                    name,
                    operation: "fstat",
                    source,
                });
            }
            let metadata = metadata.assume_init();
            let Some(capacity) = admissible_capacity(metadata.st_size, SHM_SIZE) else {
                libc::close(fd);
                return Err(AviateAdapterError::ShmCapacityTooSmall {
                    name,
                    required: SHM_SIZE,
                    observed: metadata.st_size,
                });
            };
            let ptr = libc::mmap(
                std::ptr::null_mut(),
                SHM_SIZE,
                libc::PROT_READ,
                libc::MAP_SHARED,
                fd,
                0,
            );
            libc::close(fd);
            if ptr == libc::MAP_FAILED {
                return Err(AviateAdapterError::ShmIo {
                    name,
                    operation: "mmap",
                    source: std::io::Error::last_os_error(),
                });
            }
            (
                ptr.cast::<u8>().cast_const(),
                ShmIdentity {
                    device: device_identity(metadata.st_dev),
                    inode: metadata.st_ino,
                    size: capacity,
                },
            )
        };
        Ok(Self { base, identity })
    }

    /// Returns the POSIX object identity captured before the mapping opened.
    #[must_use]
    pub const fn identity(&self) -> ShmIdentity {
        self.identity
    }

    /// Copies the block byte-by-byte (volatile: the plugin writes
    /// concurrently; the `seq` double-read in [`Self::read`] rejects
    /// torn copies).
    fn raw_snapshot(&self) -> [u8; SHM_SIZE] {
        let mut out = [0u8; SHM_SIZE];
        for (i, slot) in out.iter_mut().enumerate() {
            // SAFETY: `base` is a live PROT_READ mapping of exactly
            // SHM_SIZE bytes established in `open`; `i < SHM_SIZE`.
            #[allow(unsafe_code)]
            {
                *slot = unsafe { self.base.add(i).read_volatile() };
            }
        }
        out
    }

    /// Reads one coherent sample, or `None` when the plugin is not
    /// ready, the block is flagged invalid, or two attempts kept
    /// tearing.
    pub fn read(&self) -> Option<GzStateSample> {
        for _ in 0..4 {
            let first = self.raw_snapshot();
            let seq = u32_at(&first, OFF_SEQ);
            let second_seq = u32_at(&self.raw_snapshot(), OFF_SEQ);
            if seq != second_seq {
                continue; // torn mid-update; retry
            }
            if u32_at(&first, OFF_PLUGIN_READY) == 0 || u32_at(&first, OFF_VALID) == 0 {
                return None;
            }
            return Some(decode_sample(&first, seq));
        }
        None
    }
}

impl Drop for GzStateShm {
    fn drop(&mut self) {
        // SAFETY: unmapping the exact region mapped in `open`; `base`
        // is never used again (drop consumes the sole owner).
        #[allow(unsafe_code)]
        unsafe {
            libc::munmap(self.base.cast_mut().cast(), SHM_SIZE);
        }
    }
}

fn f64_at(buf: &[u8; SHM_SIZE], off: usize) -> f64 {
    let mut b = [0u8; 8];
    if let Some(src) = buf.get(off..off + 8) {
        b.copy_from_slice(src);
    }
    f64::from_ne_bytes(b)
}

fn u64_at(buf: &[u8; SHM_SIZE], off: usize) -> u64 {
    let mut b = [0u8; 8];
    if let Some(src) = buf.get(off..off + 8) {
        b.copy_from_slice(src);
    }
    u64::from_ne_bytes(b)
}

fn u32_at(buf: &[u8; SHM_SIZE], off: usize) -> u32 {
    let mut b = [0u8; 4];
    if let Some(src) = buf.get(off..off + 4) {
        b.copy_from_slice(src);
    }
    u32::from_ne_bytes(b)
}

fn vec3_at(buf: &[u8; SHM_SIZE], off: usize) -> [f64; 3] {
    [
        f64_at(buf, off),
        f64_at(buf, off + 8),
        f64_at(buf, off + 16),
    ]
}

fn decode_sample(buf: &[u8; SHM_SIZE], seq: u32) -> GzStateSample {
    let quat_enu_flu = [
        f64_at(buf, OFF_QUAT),
        f64_at(buf, OFF_QUAT + 8),
        f64_at(buf, OFF_QUAT + 16),
        f64_at(buf, OFF_QUAT + 24),
    ];
    GzStateSample {
        quat_wxyz: enu_quat_to_ned(quat_enu_flu),
        rates_rps: flu_to_frd(vec3_at(buf, OFF_ANG_VEL)),
        pos_ned_m: enu_to_ned(vec3_at(buf, OFF_POS)),
        vel_ned_mps: enu_to_ned(vec3_at(buf, OFF_VEL)),
        time_us: u64_at(buf, OFF_TIME_US),
        seq,
    }
}

/// ENU world vector → NED (swap x/y, negate z), matching Aviate's
/// `enu_to_ned_f32`.
fn enu_to_ned(enu: [f64; 3]) -> [f32; 3] {
    [enu[1] as f32, enu[0] as f32, -enu[2] as f32]
}

/// FLU body vector → FRD (negate y/z), matching Aviate's `flu_to_frd_f32`.
fn flu_to_frd(flu: [f64; 3]) -> [f32; 3] {
    [flu[0] as f32, -flu[1] as f32, -flu[2] as f32]
}

/// Body→world quaternion, ENU/FLU convention → NED/FRD, matching
/// Aviate's `enu_quat_to_ned_f32`:
/// `q_NED_FRD = q_ENU→NED · q_ENU_FLU · q_FRD→FLU`.
fn enu_quat_to_ned(q: [f64; 4]) -> [f32; 4] {
    let s = core::f32::consts::FRAC_1_SQRT_2;
    let (w, x, y, z) = (q[0] as f32, q[1] as f32, q[2] as f32, q[3] as f32);
    [s * (w + z), s * (x + y), s * (x - y), s * (w - z)]
}

/// Wall-clock progress tracking so a frozen block (paused sim, dead
/// plugin) ages into withheld telemetry instead of replaying forever.
#[derive(Debug)]
pub struct ShmFreshness {
    last_seq: Option<u32>,
    last_time_us: Option<u64>,
    last_progress: Instant,
    quarantined: bool,
}

impl ShmFreshness {
    /// Starts tracking.
    pub fn new() -> Self {
        Self::new_at(Instant::now())
    }

    pub(crate) fn new_at(now: Instant) -> Self {
        Self {
            last_seq: None,
            last_time_us: None,
            last_progress: now,
            quarantined: false,
        }
    }

    /// Feeds the latest observed `seq`; returns how long the block has
    /// been frozen (zero while it keeps advancing).
    pub fn observe(&mut self, seq: u32, time_us: u64) -> ShmObservation {
        self.observe_at(seq, time_us, Instant::now())
    }

    pub(crate) fn observe_at(&mut self, seq: u32, time_us: u64, now: Instant) -> ShmObservation {
        if self.quarantined {
            return ShmObservation::Quarantined;
        }
        let observation = match (self.last_seq, self.last_time_us) {
            (Some(previous_seq), Some(previous_time))
                if seq == previous_seq && time_us == previous_time =>
            {
                ShmObservation::Unchanged(
                    now.checked_duration_since(self.last_progress)
                        .unwrap_or_default(),
                )
            }
            (Some(previous_seq), Some(previous_time))
                if serial_is_newer(seq, previous_seq) && time_us > previous_time =>
            {
                ShmObservation::Advancing
            }
            (None, None) => ShmObservation::Advancing,
            _ => {
                self.quarantined = true;
                ShmObservation::Quarantined
            }
        };
        if observation == ShmObservation::Advancing {
            self.last_seq = Some(seq);
            self.last_time_us = Some(time_us);
            self.last_progress = now;
        }
        observation
    }

    /// Marks an unreadable/invalid block; returns how long since the
    /// last good progress.
    pub fn observe_absent(&mut self) -> std::time::Duration {
        self.last_progress.elapsed()
    }

    pub(crate) fn observe_absent_at(&self, now: Instant) -> std::time::Duration {
        now.checked_duration_since(self.last_progress)
            .unwrap_or_default()
    }
}

fn serial_is_newer(candidate: u32, current: u32) -> bool {
    let distance = candidate.wrapping_sub(current);
    distance != 0 && distance < (1_u32 << 31)
}

impl Default for ShmFreshness {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
