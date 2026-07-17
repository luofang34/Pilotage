#![allow(clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use aviate_xil_contract::AttachError;
use aviate_xil_shm::{AttachFailure, ModelStateSnapshot, SimWriterSession};
use pilotage_adapter_api::SourceIncarnation;
use pilotage_protocol::VehicleId;

use super::{REATTACH_INTERVAL, ShmSource, TRUTH_VALID_FLAGS, batch_from_sample};
use crate::error::AviateAdapterError;
use crate::shm::{GzStateSample, attach_error};

fn unique_name(tag: &str) -> String {
    // macOS caps shm names at 31 chars; keep the unique suffix short.
    format!("/plt_s_{tag}_{}", std::process::id())
}

fn incarnation() -> SourceIncarnation {
    SourceIncarnation::new([0x5a; 16])
}

fn snapshot(generation: u32, sim_step: u64, time_us: u64) -> ModelStateSnapshot {
    ModelStateSnapshot {
        reset_generation: generation,
        sim_step,
        time_us,
        pos: [1.0, 2.0, 3.0],
        quat: [1.0, 0.0, 0.0, 0.0],
        vel: [0.5, 0.0, -1.0],
        // Advisory world-frame lane: deliberately non-zero in every test
        // so any leak into published telemetry becomes visible.
        ang_vel: [7.0, -3.0, 11.0],
    }
}

#[test]
fn coherent_snapshot_flows_with_truth_stamping_and_no_body_rates() {
    let name = unique_name("flow");
    let writer = SimWriterSession::create(&name).expect("create writer");
    writer.write_model_state(&snapshot(writer.reset_generation(), 40, 1_000));

    let mut source = ShmSource::open_named(&name, 0, incarnation()).expect("attach");
    let batch = source.sample(VehicleId::new(1), 0);
    let avionics = batch.samples[0].avionics.expect("avionics");
    let attitude = avionics.attitude.expect("attitude");
    let kinematics = avionics.kinematics.expect("kinematics");

    assert_eq!(kinematics.pos_ned_m, [2.0, 1.0, -3.0]);
    assert_eq!(kinematics.vel_ned_mps, [0.0, 0.5, 1.0]);
    assert_eq!(attitude.stamp.sequence, 40, "sim_step is the sequence");
    assert_eq!(attitude.stamp.source_epoch, 1);
    assert_eq!(attitude.stamp.acquired_at_ns, 1_000_000);

    // The producer publishes non-zero world-frame angular velocity; it
    // must not surface as authoritative body rates.
    assert_eq!(avionics.valid_flags & 0b10, 0, "rates bit must stay clear");
    assert_eq!(avionics.valid_flags, TRUTH_VALID_FLAGS);
    assert_eq!(attitude.rates_rps, [0.0; 3]);
}

#[test]
fn world_reset_starts_a_new_epoch_and_accepts_the_time_rewind() {
    let name = unique_name("rst");
    let writer = SimWriterSession::create(&name).expect("create writer");
    writer.write_model_state(&snapshot(writer.reset_generation(), 100, 5_000_000));

    let mut source = ShmSource::open_named(&name, 0, incarnation()).expect("attach");
    let now = Instant::now();
    let first = source.usable_sample(now).expect("pre-reset sample");
    assert_eq!(first.time_us, 5_000_000);
    assert_eq!(source.epoch, 1);

    // Between the reset and the new world's first published step there is
    // nothing to serve — and nothing is replayed.
    let new_generation = writer.bump_reset_generation();
    assert_eq!(source.usable_sample(now + Duration::from_millis(1)), None);

    // Sim time rewinds to zero by design; the step counter stays
    // monotonic. The sampler re-keys on the new generation and accepts
    // the rewind in a new source epoch.
    writer.write_model_state(&snapshot(new_generation, 101, 1_000));
    let resumed = source
        .usable_sample(now + Duration::from_millis(2))
        .expect("post-reset sample");
    assert_eq!(resumed.reset_generation, new_generation);
    assert_eq!(resumed.time_us, 1_000);
    assert_eq!(source.epoch, 2);
}

#[test]
fn writer_disappearance_stops_output_without_frozen_replay() {
    let name = unique_name("gone");
    let writer = SimWriterSession::create(&name).expect("create writer");
    writer.write_model_state(&snapshot(writer.reset_generation(), 7, 1_000));

    let mut source = ShmSource::open_named(&name, 0, incarnation()).expect("attach");
    let now = Instant::now();
    assert!(source.usable_sample(now).is_some());

    // The dead mapping still holds the final snapshot; not one more
    // sample may be published from it.
    drop(writer);
    assert_eq!(source.usable_sample(now + Duration::from_millis(1)), None);
    assert!(source.session.is_none(), "detached from the dead world");
    assert!(source.fault.is_none(), "a vanished writer is not a fault");
}

#[test]
fn writer_replacement_reattaches_into_a_new_source_epoch() {
    let name = unique_name("repl");
    let writer = SimWriterSession::create(&name).expect("create writer");
    writer.write_model_state(&snapshot(writer.reset_generation(), 10, 2_000));

    let mut source = ShmSource::open_named(&name, 0, incarnation()).expect("attach");
    let now = Instant::now();
    assert_eq!(source.usable_sample(now).expect("live sample").sim_step, 10);
    assert_eq!(source.epoch, 1);

    drop(writer);
    let replacement = SimWriterSession::create(&name).expect("replacement writer");
    replacement.write_model_state(&snapshot(replacement.reset_generation(), 3, 500));

    // One poll observes the replacement, detaches, re-attaches, and
    // serves the new world's first coherent sample in a new source epoch.
    let resumed = source
        .usable_sample(now + Duration::from_millis(1))
        .expect("post-replacement sample");
    assert_eq!(resumed.sim_step, 3);
    assert_eq!(source.epoch, 2);
}

#[test]
fn reattachment_is_bounded_by_the_retry_interval() {
    let name = unique_name("bnd");
    let writer = SimWriterSession::create(&name).expect("create writer");
    writer.write_model_state(&snapshot(writer.reset_generation(), 1, 100));

    let mut source = ShmSource::open_named(&name, 0, incarnation()).expect("attach");
    let now = Instant::now();
    assert!(source.usable_sample(now).is_some());

    // Detach and burn the immediate attempt while no writer exists.
    drop(writer);
    assert_eq!(source.usable_sample(now), None);

    let replacement = SimWriterSession::create(&name).expect("replacement writer");
    replacement.write_model_state(&snapshot(replacement.reset_generation(), 2, 200));

    // Inside the interval no new attempt is made: still no output.
    assert_eq!(source.usable_sample(now + REATTACH_INTERVAL / 2), None);
    // At the interval the bounded retry lands and a new epoch begins.
    assert!(source.usable_sample(now + REATTACH_INTERVAL).is_some());
    assert_eq!(source.epoch, 2);
}

#[test]
fn unavailable_attachment_fails_closed() {
    let refusal = ShmSource::open_named(&unique_name("no"), 0, incarnation());
    assert!(
        matches!(refusal, Err(AviateAdapterError::ShmAttachIo { .. })),
        "got {refusal:?}"
    );
}

#[test]
fn a_contract_mismatch_is_a_sticky_fail_closed_fault() {
    let name = unique_name("flt");
    let writer = SimWriterSession::create(&name).expect("create writer");
    writer.write_model_state(&snapshot(writer.reset_generation(), 1, 100));

    let mut source = ShmSource::open_named(&name, 0, incarnation()).expect("attach");
    let now = Instant::now();
    assert!(source.usable_sample(now).is_some());

    // An incompatible-layout refusal (detected upstream on attach) fails
    // the machine closed with the violation preserved verbatim.
    source.session = None;
    source.absorb_reattach(
        Err(attach_error(
            &name,
            AttachFailure::Contract(AttachError::VersionMismatch { found: 2 }),
        )),
        now,
    );
    assert!(
        matches!(
            source.fault(),
            Some(AviateAdapterError::ShmContractMismatch {
                violation: AttachError::VersionMismatch { found: 2 },
                ..
            })
        ),
        "got {:?}",
        source.fault()
    );

    // Even with a healthy, publishing writer behind the name, a
    // fail-closed source publishes nothing and never re-attaches.
    assert_eq!(source.usable_sample(now + Duration::from_millis(1)), None);
    assert_eq!(
        source.usable_sample(now + REATTACH_INTERVAL + Duration::from_millis(1)),
        None
    );
    assert!(source.session.is_none(), "no session behind a fault");
    assert!(source.fault.is_some(), "the fault is sticky");
}

#[test]
fn simulator_batch_is_an_estimator_shaped_projection_with_shared_stamps() {
    let batch = batch_from_sample(
        VehicleId::new(1),
        0,
        GzStateSample {
            quat_wxyz: [1.0, 0.0, 0.0, 0.0],
            pos_ned_m: [0.0; 3],
            vel_ned_mps: [0.0; 3],
            time_us: 42,
            // Verifies the u64 step truncates to the wrapping u32 wire
            // sequence by its low bits.
            sim_step: (1 << 32) | 7,
            reset_generation: 1,
        },
        0,
        3,
        incarnation(),
    );

    let avionics = batch.samples[0].avionics.expect("avionics");
    let status = avionics.estimator_status_stamp.expect("status stamp");
    assert_eq!(status, avionics.attitude.expect("attitude").stamp);
    assert_eq!(status, avionics.kinematics.expect("kinematics").stamp);
    assert_eq!(status.sequence, 7);
    assert_eq!(status.source_epoch, 3);
    assert_eq!(avionics.valid_flags, TRUTH_VALID_FLAGS);
    assert_eq!(avionics.quality, 0);
}
