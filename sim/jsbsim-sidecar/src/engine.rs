//! A safe JSBSim engine: load an aircraft, install the ground, read and
//! write properties, and step. The pure-Rust port flies by default; the
//! `cpp` feature adds JSBSim's C++ library as a second choice with the
//! same surface.

#[cfg(feature = "cpp")]
mod cpp;
#[cfg(feature = "cpp")]
mod ffi;
mod port;

use std::path::PathBuf;

use crate::ground::GroundModel;

/// Why the engine could not do what was asked.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// This build has no C++ engine.
    #[error("this sidecar has no C++ engine: build it with the cpp feature")]
    CppNotBuilt,
    /// The C++ engine needs a JSBSim checkout and has none.
    #[error("the C++ engine needs --jsbsim-root")]
    NoRoot,
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
    /// The JSBSim checkout, the root of its engine and systems data. A
    /// self-contained model needs none on the Rust engine.
    pub root: Option<PathBuf>,
    /// The directory holding the model's XML file (and its `Engines`).
    pub aircraft_dir: PathBuf,
    /// The model name: its directory and its XML file's stem.
    pub model: String,
}

/// Which implementation of JSBSim flies.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EngineKind {
    /// The pure-Rust port, the `jsbsim` crate.
    #[default]
    Rust,
    /// JSBSim's C++ library, in a build with the `cpp` feature.
    Cpp,
}

impl EngineKind {
    /// The kind a command line names.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "rust" => Some(Self::Rust),
            "cpp" => Some(Self::Cpp),
            _ => None,
        }
    }
}

/// A property of one engine, resolved once so a step reads numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Property(usize);

/// What one implementation of JSBSim provides. Properties are indices
/// into a table the backend keeps; a failure is reported the same way by
/// both, so everything above [`Engine`] flies either unchanged.
trait Backend {
    /// Makes `ground` the terrain every gear contact stands on.
    fn set_ground(&mut self, ground: Box<dyn GroundModel>) -> bool;
    /// Replaces the installed ground, keeping what it remembers.
    fn replace_ground(&mut self, ground: Box<dyn GroundModel>) -> bool;
    /// The table index of a property, created on request.
    fn property(&mut self, name: &str, create: bool) -> Result<Option<usize>, EngineError>;
    /// A property's value; NaN when it cannot be read.
    fn get(&mut self, index: usize) -> f64;
    /// Sets a property; false when it was refused or its code failed.
    fn set(&mut self, index: usize, value: f64) -> bool;
    /// Applies the initial conditions.
    fn run_ic(&mut self) -> bool;
    /// Advances one step; false when it failed or asked to stop.
    fn run(&mut self) -> bool;
    /// The simulation time, s.
    fn sim_time_s(&self) -> f64;
}

/// One JSBSim flight-dynamics instance.
pub struct Engine {
    raw: Box<dyn Backend>,
}

impl Engine {
    /// Loads `paths.model` on the `kind` engine and sets the step to
    /// `dt_s`.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Load`] when JSBSim cannot load the model,
    /// [`EngineError::Nul`] for an unrepresentable path, and
    /// [`EngineError::CppNotBuilt`] or [`EngineError::NoRoot`] when the
    /// C++ engine is not available.
    pub fn load(kind: EngineKind, paths: &ModelPaths, dt_s: f64) -> Result<Self, EngineError> {
        let raw: Box<dyn Backend> = match kind {
            EngineKind::Rust => Box::new(port::PortBackend::load(paths, dt_s)?),
            #[cfg(feature = "cpp")]
            EngineKind::Cpp => Box::new(cpp::CppBackend::load(paths, dt_s)?),
            #[cfg(not(feature = "cpp"))]
            EngineKind::Cpp => return Err(EngineError::CppNotBuilt),
        };
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
        self.raw.replace_ground(ground)
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
        self.raw
            .property(name, create)?
            .map(Property)
            .ok_or_else(|| EngineError::NoProperty {
                name: name.to_owned(),
            })
    }

    /// A property's value.
    pub fn get(&mut self, property: Property) -> f64 {
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
