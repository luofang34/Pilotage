//! Model files and shared data must load on each engine.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::{ENGINES, Engine, EngineError, ModelPaths, start};
use crate::aircraft::Aircraft;

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "pilotage model {} {}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn paths(&self) -> ModelPaths {
        ModelPaths {
            root: Some(self.0.clone()),
            aircraft_dir: self.0.join("aircraft"),
            model: "rascal".to_owned(),
        }
    }

    fn rascal(&self) -> ModelPaths {
        let paths = self.paths();
        std::fs::create_dir_all(&paths.aircraft_dir).unwrap();
        std::fs::copy(
            Aircraft::Rascal.bundled_dir().join("rascal.xml"),
            paths.aircraft_dir.join("rascal.xml"),
        )
        .unwrap();
        paths
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            tracing::warn!(%error, "test model cleanup failed");
        }
    }
}

#[test]
fn malformed_xml_and_missing_engine_files_are_load_errors() {
    let fixture = Fixture::new();
    let paths = fixture.rascal();
    for &kind in ENGINES {
        assert!(
            matches!(
                Engine::load(kind, &paths, 0.001),
                Err(EngineError::Load { .. })
            ),
            "{kind:?}: the missing engine file must fail"
        );
    }
    std::fs::write(paths.aircraft_dir.join("rascal.xml"), "<fdm_config>").unwrap();
    for &kind in ENGINES {
        assert!(
            matches!(
                Engine::load(kind, &paths, 0.001),
                Err(EngineError::Load { .. })
            ),
            "{kind:?}: malformed XML must fail"
        );
    }
}

#[test]
fn a_model_can_load_engines_and_systems_from_a_separate_shared_root() {
    let fixture = Fixture::new();
    let paths = fixture.rascal();
    let shared = fixture.0.join("engine");
    std::fs::create_dir_all(&shared).unwrap();
    for file in ["electrical_engine_1050W.xml", "18x8.xml"] {
        std::fs::copy(
            Aircraft::Rascal.bundled_dir().join("Engines").join(file),
            shared.join(file),
        )
        .unwrap();
    }
    let systems = fixture.0.join("systems");
    std::fs::create_dir_all(&systems).unwrap();
    std::fs::write(systems.join("review-system.xml"),
        r#"<system name="review"><property value="4.25">pilotage/shared-system</property></system>"#).unwrap();
    let model_file = paths.aircraft_dir.join("rascal.xml");
    let model = std::fs::read_to_string(&model_file).unwrap().replace(
        "</fdm_config>",
        "<system file=\"review-system\"/></fdm_config>",
    );
    std::fs::write(model_file, model).unwrap();
    for &kind in ENGINES {
        let mut engine = Engine::load(kind, &paths, 0.001).unwrap();
        let shared_system = engine.property("pilotage/shared-system").unwrap();
        assert_eq!(engine.get(shared_system), 4.25, "{kind:?}");
        start(&mut engine, 120.0);
        for _ in 0..100 {
            engine.step().unwrap();
        }
        let altitude = engine.property("position/h-sl-meters").unwrap();
        assert!(engine.get(altitude).is_finite(), "{kind:?}");
    }
}
