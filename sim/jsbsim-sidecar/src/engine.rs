//! A safe JSBSim engine: load an aircraft, install the ground, read and
//! write properties, and step.

mod ffi;

use std::ffi::{CString, c_int};
use std::path::{Path, PathBuf};

use crate::ground::GroundModel;

/// Why the engine could not do what was asked.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// A path or name holds a NUL byte, which C cannot carry.
    #[error("{what} {value:?} holds a NUL byte")]
    Nul {
        /// Which argument.
        what: &'static str,
        /// Its value.
        value: String,
    },
    /// JSBSim refused to load the model.
    #[error("JSBSim could not load {model} from {}: {message}", aircraft_dir.display())]
    Load {
        /// The model name.
        model: String,
        /// Where it was looked for.
        aircraft_dir: PathBuf,
        /// JSBSim's message.
        message: String,
    },
    /// The model has no property of that name.
    #[error("JSBSim has no property {name}")]
    NoProperty {
        /// The property name.
        name: String,
    },
    /// JSBSim refused the ground callback.
    #[error("JSBSim refused the ground callback")]
    Ground,
    /// JSBSim refused or failed a commanded property.
    #[error("JSBSim failed {name} = {value}")]
    Command {
        /// The property.
        name: &'static str,
        /// The value set.
        value: f64,
    },
    /// JSBSim's initial-condition run failed.
    #[error("JSBSim's initial-condition run failed")]
    InitialConditions,
    /// A JSBSim step failed or asked to stop.
    #[error("JSBSim stopped at {sim_time_s} s")]
    Stopped {
        /// The simulation time of the failed step.
        sim_time_s: f64,
    },
}

/// Where one engine finds its model and JSBSim's shared data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelPaths {
    /// The JSBSim checkout: the root of its engine and systems data.
    pub root: PathBuf,
    /// The directory holding the model's XML file (and its `Engines`).
    pub aircraft_dir: PathBuf,
    /// The model name: its directory and its XML file's stem.
    pub model: String,
}

/// A property of one engine, resolved once so a step reads numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Property(c_int);

/// One JSBSim flight-dynamics instance.
pub struct Engine {
    raw: ffi::RawEngine,
}

fn c_string(what: &'static str, value: &str) -> Result<CString, EngineError> {
    CString::new(value).map_err(|_| EngineError::Nul {
        what,
        value: value.to_owned(),
    })
}

fn c_path(what: &'static str, path: &Path) -> Result<CString, EngineError> {
    c_string(what, &path.display().to_string())
}

impl Engine {
    /// Loads `paths.model` and sets the step to `dt_s`.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Load`] when JSBSim cannot load the model,
    /// or [`EngineError::Nul`] for an unrepresentable path.
    pub fn load(paths: &ModelPaths, dt_s: f64) -> Result<Self, EngineError> {
        let args = ffi::CreateArgs {
            root: c_path("JSBSim root", &paths.root)?,
            aircraft_dir: c_path("aircraft directory", &paths.aircraft_dir)?,
            engine_dir: c_path("engine directory", &paths.root.join("engine"))?,
            systems_dir: c_path("systems directory", &paths.root.join("systems"))?,
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

    /// Makes `ground` the terrain every gear contact stands on.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Ground`] when JSBSim refuses it.
    pub fn set_ground(&mut self, ground: Box<dyn GroundModel>) -> Result<(), EngineError> {
        if self.raw.set_ground(ground) {
            Ok(())
        } else {
            Err(EngineError::Ground)
        }
    }

    /// Replaces the installed ground between steps.
    pub fn replace_ground(&mut self, ground: Box<dyn GroundModel>) -> bool {
        match self.raw.ground_mut() {
            Some(slot) => {
                *slot = ground;
                true
            }
            None => false,
        }
    }

    /// Resolves an existing property.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::NoProperty`] when the model has none of
    /// that name.
    pub fn property(&mut self, name: &str) -> Result<Property, EngineError> {
        self.resolve(name, false)
    }

    /// Resolves a property, creating it when the model has none: an
    /// input a model reads only if it is set.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::NoProperty`] when JSBSim cannot create it.
    pub fn input(&mut self, name: &str) -> Result<Property, EngineError> {
        self.resolve(name, true)
    }

    fn resolve(&mut self, name: &str, create: bool) -> Result<Property, EngineError> {
        let c_name = c_string("property", name)?;
        self.raw
            .property(&c_name, create)
            .map(Property)
            .ok_or_else(|| EngineError::NoProperty {
                name: name.to_owned(),
            })
    }

    /// A property's value.
    pub fn get(&self, property: Property) -> f64 {
        self.raw.get(property.0)
    }

    /// Sets a property. A step's inputs are plain values; a property
    /// that runs JSBSim code when set (a trim) reports through
    /// [`Self::command`] instead.
    pub fn set(&mut self, property: Property, value: f64) {
        // A plain value cannot fail to set; see `command` for the rest.
        self.raw.set(property.0, value);
    }

    /// Sets a property that runs JSBSim code, such as a trim.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Command`] when JSBSim refuses or fails.
    pub fn command(
        &mut self,
        property: Property,
        name: &'static str,
        value: f64,
    ) -> Result<(), EngineError> {
        if self.raw.set(property.0, value) {
            Ok(())
        } else {
            Err(EngineError::Command { name, value })
        }
    }

    /// Applies the initial conditions set through the `ic/` properties.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::InitialConditions`] when JSBSim fails.
    pub fn run_ic(&mut self) -> Result<(), EngineError> {
        if self.raw.run_ic() {
            Ok(())
        } else {
            Err(EngineError::InitialConditions)
        }
    }

    /// Advances one step.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Stopped`] when the step fails.
    pub fn step(&mut self) -> Result<(), EngineError> {
        if self.raw.run() {
            Ok(())
        } else {
            Err(EngineError::Stopped {
                sim_time_s: self.sim_time_s(),
            })
        }
    }

    /// The simulation time, s.
    pub fn sim_time_s(&self) -> f64 {
        self.raw.sim_time_s()
    }
}

#[cfg(test)]
mod tests;
