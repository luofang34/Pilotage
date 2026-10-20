use super::{Px4Model, complete_scaffold, context, plan_with_px4_dir, value};

#[test]
fn simultaneous_plans_keep_their_generated_configs_inside_their_fixtures() {
    let (first_root, first_px4) = complete_scaffold("isolated-first");
    let (second_root, second_px4) = complete_scaffold("isolated-second");
    let source = "Tools/simulation/gz/server.config";
    for (px4, marker) in [(&first_px4, "first"), (&second_px4, "second")] {
        std::fs::write(
            px4.join(source),
            format!("<!-- {marker} --><plugin name=\"gz::sim::systems::Magnetometer\"/>"),
        )
        .expect("distinct source");
    }
    let barrier = std::sync::Barrier::new(2);
    let (first, second) = std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            barrier.wait();
            plan_with_px4_dir(
                &context(first_root.clone()),
                &first_px4,
                Px4Model::X500Gimbal,
            )
            .expect("first plan")
        });
        let second = scope.spawn(|| {
            barrier.wait();
            plan_with_px4_dir(
                &context(second_root.clone()),
                &second_px4,
                Px4Model::X500Gimbal,
            )
            .expect("second plan")
        });
        (first.join().expect("first"), second.join().expect("second"))
    });
    let first_config = value(&first[0].spec.env, "GZ_SIM_SERVER_CONFIG_PATH").expect("first");
    let second_config = value(&second[0].spec.env, "GZ_SIM_SERVER_CONFIG_PATH").expect("second");
    assert_ne!(first_config, second_config);
    for (config, root, marker) in [
        (first_config, &first_root, "first"),
        (second_config, &second_root, "second"),
    ] {
        assert!(std::path::Path::new(config).starts_with(root));
        let contents = std::fs::read_to_string(config).expect("generated config");
        assert!(contents.contains(&format!("<!-- {marker} -->")));
        assert!(contents.contains("<use_units_gauss>false</use_units_gauss>"));
    }
    std::fs::remove_dir_all(first_root).expect("remove first fixture");
    assert!(
        std::fs::read_to_string(second_config)
            .expect("surviving second config")
            .contains("<!-- second -->")
    );
    std::fs::remove_dir_all(second_root).expect("remove second fixture");
}
