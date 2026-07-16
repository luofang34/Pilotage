#![allow(clippy::expect_used, clippy::panic)]

use std::io;
use std::time::{Duration, Instant};

use aviate_xil_contract::{AttachError, WriterState};
use aviate_xil_shm::{AttachFailure, ModelStateSnapshot, SimWriterSession};

use super::{
    GzStateShm, ShmFreshness, ShmObservation, attach_error, enu_quat_to_ned, enu_to_ned,
    sample_from_snapshot,
};
use crate::error::AviateAdapterError;

fn unique_name(tag: &str) -> String {
    // macOS caps shm names at 31 chars; keep the unique suffix short.
    format!("/plt_t_{tag}_{}", std::process::id())
}

#[test]
fn enu_to_ned_matches_aviates_test_vector() {
    // 1 m east, 2 m north, 3 m up → 2 m north, 1 m east, 3 m down
    // (the test vector in Aviate's plugin.rs).
    assert_eq!(enu_to_ned([1.0, 2.0, 3.0]), [2.0, 1.0, -3.0]);
}

#[test]
fn identity_enu_flu_attitude_is_heading_east_in_ned() {
    // FLU body aligned with ENU world: body forward = +x_ENU = east,
    // so the NED/FRD yaw must be +90°.
    let q = enu_quat_to_ned([1.0, 0.0, 0.0, 0.0]);
    let (w, x, y, z) = (
        f64::from(q[0]),
        f64::from(q[1]),
        f64::from(q[2]),
        f64::from(q[3]),
    );
    let yaw = (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z));
    assert!(
        (yaw - core::f64::consts::FRAC_PI_2).abs() < 1e-5,
        "yaw {yaw}"
    );
}

#[test]
fn ninety_degree_enu_yaw_is_the_ned_identity() {
    // A 90° yaw about ENU up points body forward at +y_ENU = north; in
    // NED/FRD facing north is the identity attitude.
    let half = core::f64::consts::FRAC_1_SQRT_2;
    let q = enu_quat_to_ned([half, 0.0, 0.0, half]);
    let expected = [1.0_f32, 0.0, 0.0, 0.0];
    for (component, want) in q.iter().zip(expected) {
        assert!((component - want).abs() < 1e-6, "quat {q:?}");
    }
}

#[test]
fn snapshot_conversion_translates_frames_and_preserves_identity_fields() {
    let snapshot = ModelStateSnapshot {
        reset_generation: 5,
        sim_step: 777,
        time_us: 42_000_000,
        // pos ENU (east 1, north 2, up 3).
        pos: [1.0, 2.0, 3.0],
        quat: [1.0, 0.0, 0.0, 0.0],
        // vel ENU (0.5 east, 0 north, -1 up = descending).
        vel: [0.5, 0.0, -1.0],
        ang_vel: [0.0; 3],
    };
    let sample = sample_from_snapshot(&snapshot);
    assert_eq!(sample.pos_ned_m, [2.0, 1.0, -3.0]);
    assert_eq!(sample.vel_ned_mps, [0.0, 0.5, 1.0]);
    assert_eq!(sample.quat_wxyz, enu_quat_to_ned([1.0, 0.0, 0.0, 0.0]));
    assert_eq!(sample.time_us, 42_000_000);
    assert_eq!(sample.sim_step, 777);
    assert_eq!(sample.reset_generation, 5);
}

#[test]
fn world_angular_velocity_never_reaches_the_sample() {
    // The contract's ang_vel lane is world-frame and advisory — NOT a
    // body gyro. Two snapshots differing only in that lane must convert
    // identically: no field of the sample may derive from it.
    let still = ModelStateSnapshot {
        reset_generation: 1,
        sim_step: 10,
        time_us: 1_000,
        pos: [1.0, 2.0, 3.0],
        quat: [1.0, 0.0, 0.0, 0.0],
        vel: [0.5, 0.0, -1.0],
        ang_vel: [0.0; 3],
    };
    let spinning = ModelStateSnapshot {
        ang_vel: [7.0, -3.0, 11.0],
        ..still
    };
    assert_eq!(
        sample_from_snapshot(&still),
        sample_from_snapshot(&spinning)
    );
}

#[test]
fn attach_failures_map_to_typed_errors_preserving_context() {
    let io = attach_error(
        "/t",
        AttachFailure::Io(io::Error::from(io::ErrorKind::NotFound)),
    );
    assert!(
        matches!(
            io,
            AviateAdapterError::ShmAttachIo { ref name, ref source }
                if name == "/t" && source.kind() == io::ErrorKind::NotFound
        ),
        "got {io:?}"
    );

    let contract = attach_error(
        "/t",
        AttachFailure::Contract(AttachError::VersionMismatch { found: 2 }),
    );
    assert!(
        matches!(
            contract,
            AviateAdapterError::ShmContractMismatch {
                ref name,
                violation: AttachError::VersionMismatch { found: 2 },
            } if name == "/t"
        ),
        "got {contract:?}"
    );

    let not_ready = attach_error("/t", AttachFailure::NotReady);
    assert!(
        matches!(
            not_ready,
            AviateAdapterError::ShmWriterNotReady { ref name } if name == "/t"
        ),
        "got {not_ready:?}"
    );
}

#[test]
fn attach_to_an_absent_object_fails_closed_with_io_context() {
    let refusal = GzStateShm::open_named(&unique_name("no"));
    assert!(
        matches!(refusal, Err(AviateAdapterError::ShmAttachIo { .. })),
        "got {refusal:?}"
    );
}

#[test]
fn attach_reads_the_writers_published_snapshot_until_it_exits() {
    let name = unique_name("att");
    let writer = SimWriterSession::create(&name).expect("create writer");
    let generation = writer.reset_generation();
    writer.write_model_state(&ModelStateSnapshot {
        reset_generation: generation,
        sim_step: 2,
        time_us: 1_000,
        pos: [1.0, 2.0, 3.0],
        quat: [1.0, 0.0, 0.0, 0.0],
        vel: [0.0; 3],
        ang_vel: [0.0; 3],
    });

    let reader = GzStateShm::open_named(&name).expect("attach accepts the block");
    assert_eq!(reader.writer_state(), WriterState::Current);
    let sample = reader.read().expect("coherent sample");
    assert_eq!(sample.sim_step, 2);
    assert_eq!(sample.reset_generation, generation);
    assert_eq!(sample.time_us, 1_000);
    assert_eq!(sample.pos_ned_m, [2.0, 1.0, -3.0]);

    // The writer's exit is announced by the writer-state machine, not by
    // the mapping (which still holds the dead world's final snapshot).
    drop(writer);
    assert_eq!(reader.writer_state(), WriterState::Gone);
}

#[test]
fn frozen_sample_never_revives_without_new_progress() {
    let start = Instant::now();
    let mut freshness = ShmFreshness::new_at(start);
    assert_eq!(
        freshness.observe_at(8, 42_000, start),
        ShmObservation::Advancing
    );
    assert_eq!(
        freshness.observe_at(8, 42_000, start + Duration::from_secs(4)),
        ShmObservation::Unchanged(Duration::from_secs(4))
    );
    assert_eq!(
        freshness.observe_at(8, 42_000, start + Duration::from_secs(8)),
        ShmObservation::Unchanged(Duration::from_secs(8))
    );
}

#[test]
fn same_epoch_rollback_is_quarantined_but_step_wrap_is_valid() {
    let start = Instant::now();
    let mut wrapped = ShmFreshness::new_at(start);
    assert_eq!(
        wrapped.observe_at(u64::MAX, 100, start),
        ShmObservation::Advancing
    );
    assert_eq!(
        wrapped.observe_at(0, 101, start + Duration::from_millis(1)),
        ShmObservation::Advancing
    );

    let mut reset = ShmFreshness::new_at(start);
    assert_eq!(reset.observe_at(100, 100, start), ShmObservation::Advancing);
    assert_eq!(
        reset.observe_at(1, 1, start + Duration::from_millis(1)),
        ShmObservation::Quarantined
    );
    assert_eq!(
        reset.observe_at(101, 101, start + Duration::from_secs(1)),
        ShmObservation::Quarantined
    );

    let mut unchanged_clock = ShmFreshness::new_at(start);
    assert_eq!(
        unchanged_clock.observe_at(10, 500, start),
        ShmObservation::Advancing
    );
    assert_eq!(
        unchanged_clock.observe_at(11, 500, start + Duration::from_millis(1)),
        ShmObservation::Quarantined
    );
}
