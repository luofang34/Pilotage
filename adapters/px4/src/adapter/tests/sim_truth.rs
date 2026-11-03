#![allow(clippy::expect_used, clippy::panic)]

//! The simulator's truth report reaches telemetry in a simulation profile
//! only, and never as an estimate.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use pilotage_adapter_api::VehicleAdapter;
use pilotage_mavlink::LinkState;
use pilotage_mavlink::codec::FcMessage;
use pilotage_mavlink::link::apply_messages_at;
use pilotage_protocol::VehicleId;

use super::{SOURCE, live_state};
use crate::adapter::Px4Adapter;

fn feed_truth(state: &Arc<Mutex<LinkState>>) {
    let truth = FcMessage::SimTruth {
        time_usec: 1_000,
        quat_wxyz: [1.0, 0.0, 0.0, 0.0],
        vel_ned_mps: [0.0; 3],
        lat_lon_alt: [474_581_590, 85_480_040, 419_200],
    };
    apply_messages_at(state, &[(SOURCE, truth)], 0, 0, Instant::now());
}

#[test]
fn a_simulation_publishes_the_truth_report_under_its_own_role() {
    let state = live_state();
    feed_truth(&state);
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), state);
    adapter.publishes_sim_truth = true;
    let batch = adapter.sample_telemetry();
    let truth = batch.samples.first().and_then(|sample| sample.sim_truth);
    let truth = truth.expect("the truth report reaches the sample");
    assert_eq!(
        truth.stamp.role,
        pilotage_adapter_api::SourceRole::SimulationTruth
    );
}

#[test]
fn a_physical_profile_withholds_a_truth_report_it_receives() {
    let state = live_state();
    feed_truth(&state);
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), state);
    let batch = adapter.sample_telemetry();
    assert!(!batch.samples.is_empty(), "the estimate still publishes");
    assert!(
        batch
            .samples
            .iter()
            .all(|sample| sample.sim_truth.is_none())
    );
}
