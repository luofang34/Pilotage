//! The only module of this crate with unsafe code: the declarations of
//! the C shim over JSBSim (`shim/jsbsim_shim.cpp`), the owner of one
//! shim engine, and the ground trampoline JSBSim calls into Rust through.
//!
//! The shim catches every C++ exception and bounds-checks every property
//! index, so its functions are total over the values this module passes:
//! a live engine pointer and NUL-terminated strings.

// SAFETY: this module is the crate's one FFI boundary; every unsafe
// block below states why its call is sound.
#![allow(unsafe_code)]

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::ptr::NonNull;

use crate::ground::GroundModel;

/// The shim's opaque engine.
#[repr(C)]
struct PjsEngine {
    _private: [u8; 0],
}

type GroundFn = extern "C" fn(
    ctx: *mut c_void,
    lat_rad: f64,
    lon_rad: f64,
    height_m: *mut f64,
    normal_ecef: *mut f64,
) -> c_int;

// SAFETY: these declarations match `shim/jsbsim_shim.cpp` exactly: the
// same names, argument types, and return types, with C linkage.
unsafe extern "C" {
    fn pjs_create(
        root: *const c_char,
        aircraft_dir: *const c_char,
        engine_dir: *const c_char,
        systems_dir: *const c_char,
        model: *const c_char,
        dt: f64,
        err: *mut c_char,
        err_len: usize,
    ) -> *mut PjsEngine;
    fn pjs_destroy(engine: *mut PjsEngine);
    fn pjs_set_ground(engine: *mut PjsEngine, f: GroundFn, ctx: *mut c_void) -> c_int;
    fn pjs_property(engine: *mut PjsEngine, name: *const c_char, create: c_int) -> c_int;
    fn pjs_get(engine: *const PjsEngine, index: c_int) -> f64;
    fn pjs_set(engine: *mut PjsEngine, index: c_int, value: f64) -> c_int;
    fn pjs_run_ic(engine: *mut PjsEngine) -> c_int;
    fn pjs_run(engine: *mut PjsEngine) -> c_int;
    fn pjs_sim_time(engine: *const PjsEngine) -> f64;
}

/// The ground model JSBSim asks through the trampoline. A raw pointer,
/// not a `Box` field: JSBSim reads it during a step while the engine is
/// mutably borrowed, and a `Box` would claim that borrow covered it.
type GroundSlot = Box<dyn GroundModel>;

/// One owned shim engine, and the ground model installed in it.
pub(super) struct RawEngine {
    engine: NonNull<PjsEngine>,
    ground: Option<NonNull<GroundSlot>>,
}

/// The paths and model of one engine, as C strings.
pub(super) struct CreateArgs {
    pub(super) root: CString,
    pub(super) aircraft_dir: CString,
    pub(super) engine_dir: CString,
    pub(super) systems_dir: CString,
    pub(super) model: CString,
    pub(super) dt_s: f64,
}

extern "C" fn ground_trampoline(
    ctx: *mut c_void,
    lat_rad: f64,
    lon_rad: f64,
    height_m: *mut f64,
    normal_ecef: *mut f64,
) -> c_int {
    // SAFETY: `ctx` is the `GroundSlot` pointer `set_ground` installed;
    // it stays valid until the engine is destroyed or the slot replaced,
    // and both happen only outside a JSBSim call, so no `&mut` to it
    // exists while JSBSim runs this callback.
    let ground = unsafe { &*ctx.cast::<GroundSlot>() };
    let Some(answer) = ground.height(lat_rad, lon_rad) else {
        return 0;
    };
    // SAFETY: the shim passes a valid, aligned, writable double and an
    // array of three of them.
    unsafe {
        *height_m = answer.height_m;
        let [x, y, z] = answer.normal_ecef;
        *normal_ecef = x;
        *normal_ecef.add(1) = y;
        *normal_ecef.add(2) = z;
    }
    1
}

impl RawEngine {
    /// Creates and loads an engine, or returns the shim's message.
    pub(super) fn create(args: &CreateArgs) -> Result<Self, String> {
        let mut err = [0 as c_char; 512];
        // SAFETY: every string is a live NUL-terminated `CString`, and the
        // error buffer's length is passed with it.
        let engine = unsafe {
            pjs_create(
                args.root.as_ptr(),
                args.aircraft_dir.as_ptr(),
                args.engine_dir.as_ptr(),
                args.systems_dir.as_ptr(),
                args.model.as_ptr(),
                args.dt_s,
                err.as_mut_ptr(),
                err.len(),
            )
        };
        match NonNull::new(engine) {
            Some(engine) => Ok(Self {
                engine,
                ground: None,
            }),
            // SAFETY: the shim always NUL-terminates within the buffer.
            None => Err(unsafe { CStr::from_ptr(err.as_ptr()) }
                .to_string_lossy()
                .into_owned()),
        }
    }

    /// Installs `ground` as the engine's terrain, replacing any before.
    pub(super) fn set_ground(&mut self, ground: GroundSlot) -> bool {
        let slot = NonNull::from(Box::leak(Box::new(ground)));
        // SAFETY: the engine is live; the slot outlives every call JSBSim
        // makes through it (it is freed only after a replacement is
        // installed, or after the engine is destroyed).
        let installed = unsafe {
            pjs_set_ground(
                self.engine.as_ptr(),
                ground_trampoline,
                slot.as_ptr().cast(),
            )
        };
        if installed == 0 {
            // SAFETY: the slot was leaked above and never handed over.
            drop(unsafe { Box::from_raw(slot.as_ptr()) });
            return false;
        }
        if let Some(old) = self.ground.replace(slot) {
            // SAFETY: JSBSim dropped its callback holding `old` when the
            // new one was installed, so nothing refers to it any more.
            drop(unsafe { Box::from_raw(old.as_ptr()) });
        }
        true
    }

    /// The installed ground model, to update between steps.
    pub(super) fn ground_mut(&mut self) -> Option<&mut GroundSlot> {
        // SAFETY: `&mut self` means no JSBSim call is running, so the
        // trampoline holds no reference to the slot.
        self.ground.map(|slot| unsafe { &mut *slot.as_ptr() })
    }

    /// The table index of a property, or `None` when it does not exist.
    pub(super) fn property(&mut self, name: &CStr, create: bool) -> Option<c_int> {
        let create = c_int::from(create);
        // SAFETY: the engine is live and `name` is NUL-terminated.
        let index = unsafe { pjs_property(self.engine.as_ptr(), name.as_ptr(), create) };
        (index >= 0).then_some(index)
    }

    pub(super) fn get(&self, index: c_int) -> f64 {
        // SAFETY: the engine is live; the shim bounds-checks the index.
        unsafe { pjs_get(self.engine.as_ptr(), index) }
    }

    pub(super) fn set(&mut self, index: c_int, value: f64) -> bool {
        // SAFETY: the engine is live; the shim bounds-checks the index.
        unsafe { pjs_set(self.engine.as_ptr(), index, value) != 0 }
    }

    pub(super) fn run_ic(&mut self) -> bool {
        // SAFETY: the engine is live.
        unsafe { pjs_run_ic(self.engine.as_ptr()) != 0 }
    }

    pub(super) fn run(&mut self) -> bool {
        // SAFETY: the engine is live.
        unsafe { pjs_run(self.engine.as_ptr()) != 0 }
    }

    pub(super) fn sim_time_s(&self) -> f64 {
        // SAFETY: the engine is live.
        unsafe { pjs_sim_time(self.engine.as_ptr()) }
    }
}

impl Drop for RawEngine {
    fn drop(&mut self) {
        // SAFETY: the engine is live and destroyed exactly once, here.
        unsafe { pjs_destroy(self.engine.as_ptr()) };
        if let Some(slot) = self.ground.take() {
            // SAFETY: the engine that called through the slot is gone.
            drop(unsafe { Box::from_raw(slot.as_ptr()) });
        }
    }
}
