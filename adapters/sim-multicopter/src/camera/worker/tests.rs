use pilotage_adapter_api::{GIMBAL_CAMERA, MeasurementClock, SourceIncarnation};

use super::CameraWorker;
use crate::camera::fault::parse_faults;
use crate::camera::render::CameraPose;
use crate::config::{CameraConfig, TileSourceConfig};
use crate::geodesy::LocalFrame;

fn config(faults: &str) -> CameraConfig {
    CameraConfig {
        width: 64,
        height: 36,
        tiles: TileSourceConfig::Procedural,
        faults: parse_faults(faults).expect("faults parse"),
        ..CameraConfig::default()
    }
}

fn pose(time_s: u64) -> CameraPose {
    CameraPose {
        time_ns: time_s * 1_000_000_000,
        pos_ned_m: [0.0, 0.0, -40.0],
        yaw_rad: 0.0,
        pitch_rad: -core::f64::consts::FRAC_PI_2,
        lat_deg: 40.3317,
        lon_deg: -74.6745,
    }
}

fn mean(rgb: &[u8]) -> u32 {
    rgb.iter().map(|&value| u32::from(value)).sum::<u32>() / rgb.len() as u32
}

#[test]
fn a_submitted_pose_becomes_a_stamped_gimbal_frame() {
    let frame = LocalFrame::anchored_at(40.3317, -74.6745);
    let (worker, mut frames) = CameraWorker::spawn(
        config("dark@10-20s"),
        frame,
        7,
        SourceIncarnation::new([3; 16]),
    )
    .expect("the worker starts");
    worker.submit(pose(1));
    let clear = frames.blocking_recv().expect("a frame arrives");
    assert_eq!(clear.source_id, GIMBAL_CAMERA);
    assert_eq!((clear.width, clear.height), (64, 36));
    assert_eq!(clear.pixel_format, "RGB_INT8");
    assert_eq!(clear.rgb.len(), 64 * 36 * 3);
    assert_eq!(clear.tick.as_u64(), 1_000_000_000);
    assert_eq!(clear.capture.stamp.clock, MeasurementClock::Simulation);

    worker.submit(pose(15));
    let dark = frames.blocking_recv().expect("a frame arrives");
    assert!(
        mean(&dark.rgb) * 4 < mean(&clear.rgb),
        "{} vs {}",
        mean(&dark.rgb),
        mean(&clear.rgb)
    );
    assert!(dark.capture.stamp.sequence > clear.capture.stamp.sequence);
    drop(worker);
    assert!(frames.blocking_recv().is_none(), "the worker stops");
}
