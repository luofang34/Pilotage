#![allow(clippy::expect_used, clippy::panic)]

use super::{ArgsError, DEFAULT_IMAGERY_URL, parse};

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|arg| (*arg).to_owned()).collect()
}

#[test]
fn only_the_host_port_is_required_and_the_imagery_is_open_by_default() {
    let config = parse(&args(&["--host-port", "45991"])).expect("parses");
    assert_eq!(config.host_port, 45991);
    assert_eq!(config.imagery_url, DEFAULT_IMAGERY_URL);
    assert!(config.imagery_url.contains("nationalmap.gov"));
    assert!(config.imagery_url.contains("{bbox-epsg-3857}"));
    assert_eq!(config.imagery_max_zoom, 19);
    assert_eq!(
        (config.fpv_size, config.gimbal_size),
        ((640, 480), (640, 480))
    );
    assert_eq!(parse(&[]), Err(ArgsError::NoHostPort));
}

#[test]
fn every_option_is_read() {
    let config = parse(&args(&[
        "--host-port",
        "1",
        "--imagery-url",
        "https://tiles.example/{z}/{x}/{y}.jpg",
        "--imagery-max-zoom",
        "18",
        "--dem-url",
        "https://dem.example/{z}/{x}/{y}.png",
        "--style",
        "style.json",
        "--cache-dir",
        "/tmp/cache",
        "--calibration-dir",
        "/tmp/cal",
        "--fps",
        "30",
        "--fpv-size",
        "1280x720",
        "--gimbal-size",
        "320x240",
    ]))
    .expect("parses");
    assert_eq!(config.imagery_url, "https://tiles.example/{z}/{x}/{y}.jpg");
    assert_eq!(config.imagery_max_zoom, 18);
    assert_eq!(config.dem_url, "https://dem.example/{z}/{x}/{y}.png");
    assert_eq!(
        config.style.as_deref(),
        Some(std::path::Path::new("style.json"))
    );
    assert_eq!(config.fps, 30.0);
    assert_eq!(config.fpv_size, (1280, 720));
    assert_eq!(config.gimbal_size, (320, 240));
    assert!(config.cache_dir.is_some() && config.calibration_dir.is_some());
}

#[test]
fn values_a_camera_cannot_use_are_refused() {
    for bad in [
        ["--host-port", "0"],
        ["--host-port", "70000"],
        ["--imagery-url", "https://tiles.example/{z}/{x}.jpg"],
        ["--imagery-max-zoom", "23"],
        ["--fps", "0"],
        ["--fps", "NaN"],
        ["--fpv-size", "8x8"],
        ["--gimbal-size", "640"],
    ] {
        let mut list = vec!["--host-port", "5"];
        list.extend(bad);
        assert!(
            matches!(parse(&args(&list)), Err(ArgsError::Invalid { .. })),
            "{bad:?} must be refused"
        );
    }
    assert_eq!(
        parse(&args(&["--host-port"])),
        Err(ArgsError::MissingValue("--host-port"))
    );
    assert_eq!(
        parse(&args(&["--host-port", "5", "--zoom"])),
        Err(ArgsError::Unknown("--zoom".to_owned()))
    );
}
