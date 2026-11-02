//! JSBSim's C++ library behind [`Backend`], through the shim in `ffi`.

use std::ffi::{CString, c_int};
use std::path::Path;

use super::{Backend, EngineError, ModelPaths, ffi};
use crate::ground::GroundModel;

fn c_string(what: &'static str, value: &str) -> Result<CString, EngineError> {
    CString::new(value).map_err(|_| EngineError::Nul {
        what,
        value: value.to_owned(),
    })
}

fn c_path(what: &'static str, path: &Path) -> Result<CString, EngineError> {
    c_string(what, &path.display().to_string())
}

/// One shim engine.
pub(super) struct CppBackend {
    raw: ffi::RawEngine,
}

impl CppBackend {
    /// Creates an engine and loads the model.
    pub(super) fn load(paths: &ModelPaths, dt_s: f64) -> Result<Self, EngineError> {
        let root = paths.root.as_deref().ok_or(EngineError::NoRoot)?;
        let args = ffi::CreateArgs {
            root: c_path("JSBSim root", root)?,
            aircraft_dir: c_path("aircraft directory", &paths.aircraft_dir)?,
            engine_dir: c_path("engine directory", &root.join("engine"))?,
            systems_dir: c_path("systems directory", &root.join("systems"))?,
            model: c_string("model", &paths.model)?,
            dt_s,
        };
        let raw = ffi::RawEngine::create(&args).map_err(|message| EngineError::Load {
            model: paths.model.clone(),
            aircraft_dir: paths.aircraft_dir.clone(),
            message,
        })?;
        Ok(Self { raw })
    }
}

impl Backend for CppBackend {
    fn set_ground(&mut self, ground: Box<dyn GroundModel>) -> bool {
        self.raw.set_ground(ground)
    }

    fn replace_ground(&mut self, ground: Box<dyn GroundModel>) -> bool {
        match self.raw.ground_mut() {
            Some(slot) => {
                *slot = ground;
                true
            }
            None => false,
        }
    }

    fn property(&mut self, name: &str, create: bool) -> Result<Option<usize>, EngineError> {
        let name = c_string("property", name)?;
        Ok(self
            .raw
            .property(&name, create)
            .and_then(|index| usize::try_from(index).ok()))
    }

    fn get(&mut self, index: usize) -> f64 {
        c_int::try_from(index).map_or(f64::NAN, |index| self.raw.get(index))
    }

    fn set(&mut self, index: usize, value: f64) -> bool {
        c_int::try_from(index).is_ok_and(|index| self.raw.set(index, value))
    }

    fn run_ic(&mut self) -> bool {
        self.raw.run_ic()
    }

    fn run(&mut self) -> bool {
        self.raw.run()
    }

    fn sim_time_s(&self) -> f64 {
        self.raw.sim_time_s()
    }
}
