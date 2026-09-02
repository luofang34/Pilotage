//! Fixtures the executed-uncertainty tests share.

use pilotage_trial::ConditionSet;

use crate::Digest;

/// The run seed the cross-repository contract pins.
pub(super) const GOLDEN_RUN_SEED: u64 = 0x1112_1314_1516_1718;

/// The condition the executor and this repository both pin, byte for byte.
const GOLDEN_CONDITION: &str = concat!(
    r#"{"schema_version":4,"id":"condition-v4-golden","revision":7,"#,
    r#""seed":72623859790382856,"#,
    r#""wind":{"steady":{"speed_mps":0.0,"direction_deg":0.0},"gusts":[],"#,
    r#""turbulence":{"kind":"none"}},"#,
    r#""timing":{"estimate_delay_ns":30000000,"#,
    r#""update_jitter":{"kind":"sample_and_hold","maximum_delay_ns":20000000,"#,
    r#""interval_ns":100000000}},"#,
    r#""sensor":{"kind":"bounded_noise","lanes":["#,
    r#"{"sensor":"accelerometer","axis":"x","peak_amplitude_mps2":0.05,"#,
    r#""update_interval_samples":10},"#,
    r#"{"sensor":"gyroscope","axis":"y","peak_amplitude_rad_s":0.01,"#,
    r#""update_interval_samples":20},"#,
    r#"{"sensor":"magnetometer","axis":"z","peak_amplitude_gauss":0.02,"#,
    r#""update_interval_samples":30},"#,
    r#"{"sensor":"absolute_pressure","peak_amplitude_hpa":1.0,"#,
    r#""update_interval_samples":40},"#,
    r#"{"sensor":"differential_pressure","peak_amplitude_hpa":0.5,"#,
    r#""update_interval_samples":50},"#,
    r#"{"sensor":"pressure_altitude","peak_amplitude_m":2.0,"#,
    r#""update_interval_samples":60}]},"#,
    r#""actuator":{"authority_scale_basis_points":12000,"#,
    r#""command_loss":{"kind":"seeded_zero_order_hold","fraction_basis_points":100,"#,
    r#""decision_interval_samples":100}},"#,
    r#""controller_initialization":{"hover_thrust_force":{"kind":"scale_baseline","#,
    r#""scale_basis_points":9000}},"#,
    r#""plant":{"payload_mass_delta_kg":0.0,"longitudinal_cg_offset_m":0.0,"#,
    r#""lateral_cg_offset_m":0.0,"hover_thrust_expectation":{"kind":"measured_weight_ratio"}}}"#,
);

/// Reads the pinned condition every executed-uncertainty test starts from.
pub(super) fn golden_condition() -> ConditionSet {
    ConditionSet::from_json(GOLDEN_CONDITION.as_bytes()).expect("golden condition")
}

/// The condition identity the cross-repository derivation goldens use.
pub(super) fn cross_repository_digest() -> Digest {
    let mut bytes = [0_u8; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::try_from(index).expect("lane index fits one byte");
    }
    Digest::from_bytes(bytes)
}

/// A short condition whose command hold completes inside a test stream.
const STREAM_CONDITION: &str = concat!(
    r#"{"schema_version":4,"id":"executed-uncertainty-stream","revision":1,"seed":11,"#,
    r#""wind":{"steady":{"speed_mps":0.0,"direction_deg":0.0},"gusts":[],"#,
    r#""turbulence":{"kind":"none"}},"#,
    r#""timing":{"estimate_delay_ns":0,"update_jitter":{"kind":"none"}},"#,
    r#""sensor":{"kind":"bounded_noise","lanes":["#,
    r#"{"sensor":"accelerometer","axis":"x","peak_amplitude_mps2":2.0,"#,
    r#""update_interval_samples":2}]},"#,
    r#""actuator":{"authority_scale_basis_points":12000,"#,
    r#""command_loss":{"kind":"seeded_zero_order_hold","fraction_basis_points":1000,"#,
    r#""decision_interval_samples":10}},"#,
    r#""controller_initialization":{"hover_thrust_force":{"kind":"scale_baseline","#,
    r#""scale_basis_points":9000}},"#,
    r#""plant":{"payload_mass_delta_kg":0.0,"longitudinal_cg_offset_m":0.0,"#,
    r#""lateral_cg_offset_m":0.0,"hover_thrust_expectation":{"kind":"measured_weight_ratio"}}}"#,
);

/// Reads the condition the stream tests fly.
pub(super) fn stream_condition() -> ConditionSet {
    ConditionSet::from_json(STREAM_CONDITION.as_bytes()).expect("stream condition")
}
