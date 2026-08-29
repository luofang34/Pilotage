//! What each waveform commands, and when it is finished.

use flight_tune::{SineComponent, Waveform};

use crate::runtime::phase::waveform::{WaveformSample, sample};

fn active(waveform: &Waveform, elapsed_ns: u64) -> f64 {
    match sample(waveform, elapsed_ns).expect("a resolvable waveform") {
        WaveformSample::Active(value) => value,
        WaveformSample::Complete => panic!("the waveform ended early at {elapsed_ns} ns"),
    }
}

fn is_complete(waveform: &Waveform, elapsed_ns: u64) -> bool {
    matches!(
        sample(waveform, elapsed_ns).expect("a resolvable waveform"),
        WaveformSample::Complete
    )
}

#[test]
fn a_step_holds_its_value_and_states_no_window_of_its_own() {
    let step = Waveform::Step { value: 0.6 };
    assert!((active(&step, 0) - 0.6).abs() < f64::EPSILON);
    assert!((active(&step, 10_000_000_000) - 0.6).abs() < f64::EPSILON);
}

#[test]
fn a_ramp_moves_between_its_endpoints_and_then_completes() {
    let ramp = Waveform::Ramp {
        from: 0.0,
        to: 1.0,
        duration_ns: 1_000_000_000,
    };
    assert!(active(&ramp, 0).abs() < f64::EPSILON);
    assert!((active(&ramp, 500_000_000) - 0.5).abs() < 1e-9);
    assert!(is_complete(&ramp, 1_000_000_000));
}

#[test]
fn a_pulse_holds_for_exactly_its_declared_duration() {
    let pulse = Waveform::Pulse {
        value: -0.4,
        duration_ns: 200_000_000,
    };
    assert!((active(&pulse, 199_999_999) + 0.4).abs() < f64::EPSILON);
    assert!(is_complete(&pulse, 200_000_000));
}

#[test]
fn a_reversal_dwells_on_each_value_before_it_completes() {
    let reversal = Waveform::Reversal {
        first: 0.5,
        second: -0.5,
        dwell_ns: 100_000_000,
    };
    assert!((active(&reversal, 0) - 0.5).abs() < f64::EPSILON);
    assert!((active(&reversal, 99_999_999) - 0.5).abs() < f64::EPSILON);
    assert!((active(&reversal, 100_000_000) + 0.5).abs() < f64::EPSILON);
    assert!(is_complete(&reversal, 200_000_000));
}

#[test]
fn a_multisine_stays_inside_the_normalized_range() {
    let multisine = Waveform::Multisine {
        bias: 0.0,
        components: vec![
            SineComponent {
                amplitude: 0.5,
                frequency_hz: 1.0,
                phase_rad: 0.0,
            },
            SineComponent {
                amplitude: 0.4,
                frequency_hz: 3.0,
                phase_rad: 0.5,
            },
        ],
        duration_ns: 2_000_000_000,
    };
    for step in 0..200_u64 {
        let value = active(&multisine, step * 10_000_000);
        assert!((-1.0..=1.0).contains(&value), "{value}");
    }
    assert!(is_complete(&multisine, 2_000_000_000));
}

#[test]
fn a_recorded_waveform_is_refused_rather_than_approximated() {
    let recorded = Waveform::Recorded {
        source: flight_tune::MissionArtifactIdentity {
            id: "recorded-control".to_owned(),
            revision: "recorded-v1".to_owned(),
            digest: flight_tune::MissionDigest::from_bytes([7; 32]),
        },
    };
    let detail = sample(&recorded, 0)
        .expect_err("a recorded artifact must be refused")
        .to_string();
    assert!(detail.contains("recorded"), "{detail}");
}
