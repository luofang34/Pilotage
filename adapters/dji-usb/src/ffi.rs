// SAFETY: IOKit pointers stay on their creating worker thread. Native calls
// receive bounded slices and complete before those slices are released.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::io;
use std::ptr::NonNull;
use std::time::Instant;

use crate::error::Failure;
use crate::worker::{CHUNK_BYTES, Device};
use crate::{UpperUsbCandidate, UsbError};

#[repr(C)]
#[derive(Default)]
struct NativeError {
    stage: u32,
    status: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct NativeCandidate {
    registry_id: u64,
    location_id: u32,
    reserved: u32,
}

unsafe extern "C" {
    fn dji_usb_discover(
        output: *mut NativeCandidate,
        capacity: u32,
        count: *mut u32,
        error: *mut NativeError,
    ) -> i32;
    fn dji_usb_open(identity: u64, output: *mut *mut c_void, error: *mut NativeError) -> i32;
    fn dji_usb_poll(
        handle: *mut c_void,
        output: *mut u8,
        capacity: u32,
        count: *mut u32,
        error: *mut NativeError,
    ) -> i32;
    fn dji_usb_write(
        handle: *mut c_void,
        bytes: *const u8,
        count: u32,
        deadline_ns: u64,
        error: *mut NativeError,
    ) -> i32;
    fn dji_usb_close(handle: *mut c_void, error: *mut NativeError) -> i32;
    fn dji_usb_monotonic_ns() -> u64;
    #[cfg(test)]
    fn dji_usb_profile_test() -> i32;
    #[cfg(test)]
    fn dji_usb_lifecycle_test() -> i32;
    #[cfg(test)]
    fn dji_usb_deadline_test() -> i32;
    #[cfg(test)]
    fn dji_usb_discovery_test() -> i32;
}

pub(crate) fn discover() -> Result<Vec<UpperUsbCandidate>, UsbError> {
    let mut candidates = [NativeCandidate::default(); 32];
    let mut count = 0;
    let mut error = NativeError::default();
    // SAFETY: The native call only fills this bounded array and retains no pointers.
    let status = unsafe {
        dji_usb_discover(
            candidates.as_mut_ptr(),
            candidates.len() as u32,
            &mut count,
            &mut error,
        )
    };
    if status != 0 {
        return Err(error.into());
    }
    let Some(candidates) = candidates.get(..count as usize) else {
        return Err(UsbError::Native {
            stage: 23,
            status: 0xe00002bd,
        });
    };
    Ok(candidates
        .iter()
        .map(|candidate| UpperUsbCandidate {
            registry_id: candidate.registry_id,
            location_id: candidate.location_id,
        })
        .collect())
}

pub(crate) struct Native {
    handle: NonNull<c_void>,
}

impl Native {
    pub(crate) fn open(identity: u64) -> Result<Self, UsbError> {
        let mut handle = std::ptr::null_mut();
        let mut error = NativeError::default();
        // SAFETY: Both output pointers are valid for this synchronous call.
        let status = unsafe { dji_usb_open(identity, &mut handle, &mut error) };
        match (status, NonNull::new(handle)) {
            (0, Some(handle)) => Ok(Self { handle }),
            _ => Err(error.into()),
        }
    }
}

impl Device for Native {
    fn poll(&mut self) -> Result<Option<Vec<u8>>, Failure> {
        let mut bytes = vec![0; CHUNK_BYTES];
        let mut count = 0;
        let mut error = NativeError::default();
        // SAFETY: The native buffer fits this slice; no pointer is retained.
        let status = unsafe {
            dji_usb_poll(
                self.handle.as_ptr(),
                bytes.as_mut_ptr(),
                CHUNK_BYTES as u32,
                &mut count,
                &mut error,
            )
        };
        if status == 1 {
            return Ok(None);
        }
        if status != 0 || count as usize > bytes.len() {
            return Err(error.failure());
        }
        bytes.truncate(count as usize);
        Ok(Some(bytes))
    }

    fn write(&mut self, bytes: &[u8], deadline: Instant) -> Result<(), Failure> {
        // Sample the native clock first so translation cannot extend the deadline.
        // SAFETY: This reads a process-independent monotonic clock without pointers.
        let native_now = unsafe { dji_usb_monotonic_ns() };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.as_millis() == 0 {
            return Err(Failure::new(
                io::ErrorKind::TimedOut,
                "DJI USB write expired before IOKit",
            ));
        }
        let deadline_ns = native_now.saturating_add(remaining.as_nanos().min(50_000_000) as u64);
        let mut error = NativeError::default();
        // SAFETY: The worker keeps this bounded slice alive until WritePipeTO returns.
        let status = unsafe {
            dji_usb_write(
                self.handle.as_ptr(),
                bytes.as_ptr(),
                bytes.len() as u32,
                deadline_ns,
                &mut error,
            )
        };
        if status == 0 {
            Ok(())
        } else {
            Err(error.failure())
        }
    }

    fn close(self) -> Result<(), UsbError> {
        let mut error = NativeError::default();
        // SAFETY: This consumes the sole handle. Native retains its context only
        // when a pending callback could still use it and reports cleanup failure.
        let status = unsafe { dji_usb_close(self.handle.as_ptr(), &mut error) };
        if status == 0 {
            Ok(())
        } else {
            Err(error.into())
        }
    }
}

impl NativeError {
    fn failure(self) -> Failure {
        Failure::native(self.stage, self.status)
    }
}

impl From<NativeError> for UsbError {
    fn from(error: NativeError) -> Self {
        Self::Native {
            stage: error.stage,
            status: error.status,
        }
    }
}

#[cfg(test)]
mod tests;
