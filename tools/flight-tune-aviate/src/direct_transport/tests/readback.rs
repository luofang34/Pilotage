//! The causal maximum-skew bound on the raw direct readback.

use flight_tune::ControlChannel;

use super::super::{
    CausalReadbackBound, DirectEnactment, DirectSetpoint, DirectTransportError,
    EffectiveSetpointReport, ReadbackSelection,
};
use super::sender::RecordingSender;
use super::{
    SAMPLE_PERIOD_NS, authorize, authorize_without_skew, baseline_request, readback_bound,
    step_request,
};

const MICROSECOND_NS: u64 = 1_000;

fn report_at(sample_index: u64) -> EffectiveSetpointReport {
    EffectiveSetpointReport {
        setpoint: DirectSetpoint {
            roll_rad: 0.0,
            pitch_rad: 0.0,
            yaw_rad: 0.0,
            collective_force: 0.72,
        },
        sample_sequence: sample_index,
        sample_time_ns: sample_index * SAMPLE_PERIOD_NS,
        estimate_time_ns: sample_index * SAMPLE_PERIOD_NS,
        simulator_truth_time_ns: sample_index * SAMPLE_PERIOD_NS,
    }
}

#[test]
fn a_sample_at_the_inclusive_bound_is_the_exact_source() {
    let bound = readback_bound();
    let report = report_at(4);
    let query = report.sample_time_ns + bound.max_skew_ns();

    assert_eq!(
        bound.select(query, &report).expect("selection"),
        ReadbackSelection::Exact
    );
}

#[test]
fn a_sample_one_microsecond_past_the_bound_has_no_exact_source() {
    let bound = readback_bound();
    let report = report_at(4);
    let query = report.sample_time_ns + bound.max_skew_ns() + MICROSECOND_NS;

    assert_eq!(
        bound.select(query, &report).expect("selection"),
        ReadbackSelection::Absent
    );
}

#[test]
fn a_future_sample_waits() {
    let bound = readback_bound();
    let report = report_at(4);
    let query = report.sample_time_ns - MICROSECOND_NS;

    assert_eq!(
        bound.select(query, &report).expect("selection"),
        ReadbackSelection::Pending
    );
}

#[test]
fn an_invalid_alignment_fails_closed() {
    let bound = readback_bound();
    let mut report = report_at(4);
    report.sample_time_ns += MICROSECOND_NS;

    let result = bound.select(report.sample_time_ns, &report);

    assert!(matches!(
        result,
        Err(DirectTransportError::InvalidReadbackAlignment { .. })
    ));
}

#[test]
fn a_zero_sample_period_is_not_a_usable_bound() {
    assert!(matches!(
        CausalReadbackBound::new(0, 0),
        Err(DirectTransportError::InvalidReadbackBound { .. })
    ));
}

#[test]
fn the_exact_next_sample_after_a_transmit_is_one_sample_later() {
    let bound = readback_bound();

    assert_eq!(
        bound.next_sample_after(0).expect("next sample"),
        SAMPLE_PERIOD_NS
    );
    assert_eq!(
        bound.next_sample_after(SAMPLE_PERIOD_NS).expect("next"),
        2 * SAMPLE_PERIOD_NS,
        "a transmit exactly on a sample still waits for the NEXT one"
    );
    assert_eq!(
        bound.next_sample_after(SAMPLE_PERIOD_NS + 1).expect("next"),
        2 * SAMPLE_PERIOD_NS
    );
}

#[test]
fn a_valid_delayed_source_with_no_exact_source_sends_nothing_and_records_nothing() {
    let mut sender = RecordingSender::new();
    let mut transport = authorize(&sender);
    transport
        .freeze_baseline_blocking(&mut sender, &baseline_request())
        .expect("frozen baseline");
    let prepared = transport
        .prepare_step(&step_request(ControlChannel::Roll, 1.0))
        .expect("prepared step");
    sender.clear_transmitted();
    // The source stays valid but stops advancing, so the clock leaves it
    // further behind than the causal bound allows.
    let mut delayed = RecordingSender::new()
        .reporting(report_at(1))
        .holding_sample();
    delayed.advance(8);

    let outcome = transport
        .enact_blocking(&mut delayed, &prepared)
        .expect("a delayed source is not an error");

    assert_eq!(outcome, DirectEnactment::NoExactSource);
    assert!(
        delayed.transmitted().is_empty(),
        "no exact source means no direct demand"
    );
}

#[test]
fn a_raw_source_that_has_not_reached_the_command_time_waits() {
    let mut sender = RecordingSender::new();
    let mut transport = authorize(&sender);
    transport
        .freeze_baseline_blocking(&mut sender, &baseline_request())
        .expect("frozen baseline");
    let prepared = transport
        .prepare_step(&step_request(ControlChannel::Roll, 1.0))
        .expect("prepared step");
    // The only sample the source carries is later than the command time.
    let mut ahead = RecordingSender::new()
        .reporting(report_at(4))
        .holding_sample();

    let outcome = transport
        .enact_blocking(&mut ahead, &prepared)
        .expect("waiting is not an error");

    assert_eq!(outcome, DirectEnactment::Pending);
    assert!(
        ahead.transmitted().is_empty(),
        "a future sample waits; it does not command the vehicle"
    );
}

#[test]
fn a_transmitted_command_without_an_exact_readback_quarantines_the_run() {
    let mut sender = RecordingSender::new();
    let mut transport = authorize_without_skew(&sender);
    transport
        .freeze_baseline_blocking(&mut sender, &baseline_request())
        .expect("frozen baseline");
    let prepared = transport
        .prepare_step(&step_request(ControlChannel::Roll, 1.0))
        .expect("prepared step");
    // The source is exact for the pre-send check and then stops advancing,
    // so the command leaves the process and never gets its readback.
    sender.hold_sample_from_now();
    sender.clear_transmitted();

    let result = transport.enact_blocking(&mut sender, &prepared);

    assert!(matches!(
        result,
        Err(DirectTransportError::NoEffectiveReadback)
    ));
    assert_eq!(
        sender.transmitted().len(),
        1,
        "the command was already on the link when the readback failed"
    );
}

#[test]
fn a_silent_raw_source_sends_nothing_and_records_nothing() {
    let mut sender = RecordingSender::new();
    let mut transport = authorize(&sender);
    transport
        .freeze_baseline_blocking(&mut sender, &baseline_request())
        .expect("frozen baseline");
    let prepared = transport
        .prepare_step(&step_request(ControlChannel::Roll, 1.0))
        .expect("prepared step");
    let mut silent = RecordingSender::new().silent();

    let outcome = transport
        .enact_blocking(&mut silent, &prepared)
        .expect("a silent source is not an error");

    assert_eq!(outcome, DirectEnactment::NoExactSource);
    assert!(silent.transmitted().is_empty());
}
