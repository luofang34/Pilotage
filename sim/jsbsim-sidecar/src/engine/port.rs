//! The pure-Rust engine: the `jsbsim` crate behind [`Backend`].
//!
//! The crate reads through a [`Resolver`] and asks the ground through a
//! [`GroundCallback`]; this module answers both as the C++ engine's shim
//! does, so a model and a terrain behave the same on either engine.

use std::cell::Cell;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use jsbsim::math::{ColumnVector3, Location};
use jsbsim::{Contact, Fdm, GroundCallback, NodeId, Resolver};

use super::{Backend, EngineError, ModelPaths};
use crate::ground::GroundModel;

/// The Earth's axes, ft: JSBSim's own, which no bundled model overrides
/// with a `<planet>`.
const SEMIMAJOR_FT: f64 = 20_925_646.325_46;
const SEMIMINOR_FT: f64 = 20_855_486.595_1;
const FEET_PER_METRE: f64 = 1.0 / 0.3048;

/// Reads model files by absolute path: a model's directory and JSBSim's
/// shared data need not share a root.
struct AbsolutePaths;

impl Resolver for AbsolutePaths {
    fn resolve(&self, path: &str) -> Result<Vec<u8>, jsbsim::Error> {
        std::fs::read(path).map_err(|_| jsbsim::Error::ResourceNotFound {
            path: path.to_owned(),
        })
    }

    fn exists(&self, path: &str) -> bool {
        Path::new(path).is_file()
    }
}

type SharedGround = Arc<Mutex<Box<dyn GroundModel>>>;

fn lock(ground: &SharedGround) -> MutexGuard<'_, Box<dyn GroundModel>> {
    // A panic in a ground model leaves no invariant here to repair.
    ground.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The height and normal to keep when the model has no answer: a missing
/// tile must not drop the ground from under the vehicle.
#[derive(Clone, Copy)]
struct LastAnswer {
    height_m: f64,
    normal_ecef: [f64; 3],
}

/// The ground as the crate asks for it. Shared with the backend, so a
/// replaced ground keeps the last answer.
struct WorldGround {
    ground: SharedGround,
    last: Cell<LastAnswer>,
}

impl GroundCallback for WorldGround {
    fn ag_level(&self, _time: f64, location: &Location) -> Contact {
        let mut position = *location;
        position.set_ellipse(SEMIMAJOR_FT, SEMIMINOR_FT);
        position.cache_derived();
        let latitude_rad = position.geod_latitude_rad();
        let longitude_rad = position.longitude();
        let answer = lock(&self.ground).height(latitude_rad, longitude_rad);
        let last = answer.map_or(self.last.get(), |answer| LastAnswer {
            height_m: answer.height_m,
            normal_ecef: answer.normal_ecef,
        });
        self.last.set(last);
        let height_ft = last.height_m * FEET_PER_METRE;
        let [x, y, z] = last.normal_ecef;
        let mut point = Location::default();
        point.set_ellipse(SEMIMAJOR_FT, SEMIMINOR_FT);
        point.set_position_geodetic(longitude_rad, latitude_rad, height_ft);
        Contact::new(
            position.geod_altitude() - height_ft,
            point,
            ColumnVector3::new(x, y, z),
        )
    }
}

/// One `jsbsim::Fdm`, and the ground installed in it.
pub(super) struct PortBackend {
    fdm: Fdm,
    nodes: Vec<NodeId>,
    ground: Option<SharedGround>,
}

impl PortBackend {
    /// Creates an executive and loads the model.
    pub(super) fn load(paths: &ModelPaths, dt_s: f64) -> Result<Self, EngineError> {
        let failed = |message: String| EngineError::Load {
            model: paths.model.clone(),
            aircraft_dir: paths.aircraft_dir.clone(),
            message,
        };
        let mut fdm = Fdm::new().map_err(|error| failed(error.to_string()))?;
        // Without a checkout a model finds nothing but its own files.
        let shared = paths.root.as_deref().unwrap_or(&paths.aircraft_dir);
        fdm.set_engine_path(&shared.join("engine").display().to_string());
        fdm.set_systems_path(&shared.join("systems").display().to_string());
        let file = paths.aircraft_dir.join(&paths.model);
        fdm.load_model_file(&AbsolutePaths, &file.display().to_string())
            .map_err(|error| failed(error.to_string()))?;
        fdm.set_dt(dt_s);
        Ok(Self {
            fdm,
            nodes: Vec::new(),
            ground: None,
        })
    }
}

impl Backend for PortBackend {
    fn set_ground(&mut self, ground: Box<dyn GroundModel>) -> bool {
        let shared = Arc::new(Mutex::new(ground));
        self.fdm.set_ground_callback(Box::new(WorldGround {
            ground: Arc::clone(&shared),
            last: Cell::new(LastAnswer {
                height_m: 0.0,
                normal_ecef: [0.0, 0.0, 1.0],
            }),
        }));
        self.ground = Some(shared);
        true
    }

    fn replace_ground(&mut self, ground: Box<dyn GroundModel>) -> bool {
        match &self.ground {
            Some(shared) => {
                *lock(shared) = ground;
                true
            }
            None => false,
        }
    }

    fn property(&mut self, name: &str, create: bool) -> Result<Option<usize>, EngineError> {
        let refused = |_| EngineError::NoProperty {
            name: name.to_owned(),
        };
        let mut node = self.fdm.node(name).map_err(refused)?;
        // A write creates a node, and writing an existing one could run
        // its code (a trim), so only a missing node is written.
        if node.is_none() && create {
            self.fdm.set_property(name, 0.0).map_err(refused)?;
            node = self.fdm.node(name).map_err(refused)?;
        }
        Ok(node.map(|node| {
            self.nodes.push(node);
            self.nodes.len() - 1
        }))
    }

    fn get(&mut self, index: usize) -> f64 {
        match self.nodes.get(index) {
            Some(&node) => self.fdm.read_node(node).unwrap_or(f64::NAN),
            None => f64::NAN,
        }
    }

    fn set(&mut self, index: usize, value: f64) -> bool {
        let Some(&node) = self.nodes.get(index) else {
            return false;
        };
        match self.fdm.write_node(node, value) {
            Ok(written) => written,
            Err(error) => {
                tracing::warn!(%error, "JSBSim refused a property write");
                false
            }
        }
    }

    fn run_ic(&mut self) -> bool {
        match self.fdm.run_ic() {
            Ok(()) => true,
            Err(error) => {
                tracing::error!(%error, "JSBSim's initial-condition run failed");
                false
            }
        }
    }

    fn run(&mut self) -> bool {
        match self.fdm.run() {
            Ok(running) => running,
            Err(error) => {
                tracing::error!(%error, "JSBSim's step failed");
                false
            }
        }
    }

    fn sim_time_s(&self) -> f64 {
        self.fdm.sim_time()
    }
}
