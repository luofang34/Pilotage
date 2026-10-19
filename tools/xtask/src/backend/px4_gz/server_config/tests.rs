//! Server-config tests: the Magnetometer system gets the corrected
//! output, the rest of PX4's configuration stays as it is, and a
//! configuration without one Magnetometer system is refused.

#![allow(clippy::expect_used, clippy::panic)]

use super::{CORRECTED_OPTIONS, corrected, write_corrected};

const MAGNETOMETER_LINE: &str = "    <plugin entity_name=\"*\" entity_type=\"world\" \
    filename=\"gz-sim-magnetometer-system\" name=\"gz::sim::systems::Magnetometer\"/>";

fn config(magnetometer: &str) -> String {
    format!(
        "<server_config>\n  <plugins>\n    <plugin entity_name=\"*\" entity_type=\"world\" \
         filename=\"gz-sim-navsat-system\" name=\"gz::sim::systems::NavSat\"/>\n\
         {magnetometer}\n  </plugins>\n</server_config>\n"
    )
}

#[test]
fn a_self_closed_magnetometer_gets_the_corrected_output() {
    let fixed = corrected(&config(MAGNETOMETER_LINE)).expect("corrects");
    assert!(fixed.contains(&format!(
        "name=\"gz::sim::systems::Magnetometer\">{CORRECTED_OPTIONS}</plugin>"
    )));
    assert_eq!(
        fixed.replace(&format!(">{CORRECTED_OPTIONS}</plugin>"), "/>"),
        config(MAGNETOMETER_LINE),
        "nothing else changes"
    );
}

#[test]
fn an_open_magnetometer_gets_the_options_inside_it() {
    let open = "<plugin filename=\"gz-sim-magnetometer-system\" \
                name=\"gz::sim::systems::Magnetometer\"></plugin>";
    let fixed = corrected(&config(open)).expect("corrects");
    assert!(fixed.contains(&format!(
        "name=\"gz::sim::systems::Magnetometer\">{CORRECTED_OPTIONS}</plugin>"
    )));
}

#[test]
fn a_config_that_states_an_option_is_kept() {
    let stated = "<plugin name=\"gz::sim::systems::Magnetometer\">\
                  <use_units_gauss>true</use_units_gauss></plugin>";
    assert_eq!(corrected(&config(stated)), Ok(config(stated)));
}

#[test]
fn a_config_without_one_magnetometer_is_refused() {
    assert!(corrected(&config("")).is_err());
    let twice = format!("{MAGNETOMETER_LINE}\n{MAGNETOMETER_LINE}");
    assert!(corrected(&config(&twice)).is_err());
}

/// The PX4 checkout's own configuration, when one is present, corrects:
/// the session never runs with the mirrored field.
#[test]
fn px4s_shipped_config_corrects_when_the_checkout_is_present() {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("repo root");
    let shipped = super::super::px4_dir(repo_root).join("Tools/simulation/gz/server.config");
    let Ok(text) = std::fs::read_to_string(&shipped) else {
        return;
    };
    let fixed = corrected(&text).expect("PX4's config has one Magnetometer system");
    assert!(fixed.contains("<use_units_gauss>false</use_units_gauss>"));
}

#[test]
fn the_corrected_copy_lands_in_the_run_directory() {
    let dir = std::env::temp_dir().join(format!("px4-server-config-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let source = dir.join("source.config");
    std::fs::write(&source, config(MAGNETOMETER_LINE)).expect("source");
    let run_dir = dir.join("run");
    let target = write_corrected(&source, &run_dir).expect("writes");
    assert_eq!(target, run_dir.join("server.config"));
    let written = std::fs::read_to_string(&target).expect("copy");
    assert!(written.contains(CORRECTED_OPTIONS));
    assert_eq!(
        std::fs::read_to_string(&source).expect("source"),
        config(MAGNETOMETER_LINE),
        "PX4's file stays as it is"
    );
    std::fs::remove_dir_all(&dir).ok();
}
