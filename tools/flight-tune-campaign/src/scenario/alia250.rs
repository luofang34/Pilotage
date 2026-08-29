//! The Alia 250 matrix declaration, mirrored from its generator.
//!
//! The generator writes the corpus and this states what the corpus has to be.
//! Two independent statements of one declaration is the point: a generator
//! that drifted would produce artifacts this rejects, and a declaration that
//! drifted would reject the artifacts the generator produced.

use pilotage_trial::{ControlChannel, ControlFamily};

use super::{MatrixCondition, MatrixStimulus, ScenarioMatrix, UncertaintyFactor};

/// One degree in radians.
const DEGREE: f64 = 0.017_453_292_519_943_295;
/// The angular envelope every direct attitude stimulus spans.
const ATTITUDE_ENDPOINT_RAD: f64 = 20.0 * DEGREE;

const STIMULI: [MatrixStimulus; 15] = [
    direct(
        "roll-step-5deg",
        ControlChannel::Roll,
        "alia.direct.roll",
        0.25,
    ),
    direct(
        "roll-step-10deg",
        ControlChannel::Roll,
        "alia.direct.roll",
        0.5,
    ),
    direct(
        "roll-step-20deg",
        ControlChannel::Roll,
        "alia.direct.roll",
        1.0,
    ),
    direct(
        "pitch-step-5deg",
        ControlChannel::Pitch,
        "alia.direct.pitch",
        0.25,
    ),
    direct(
        "pitch-step-10deg",
        ControlChannel::Pitch,
        "alia.direct.pitch",
        0.5,
    ),
    direct(
        "pitch-step-20deg",
        ControlChannel::Pitch,
        "alia.direct.pitch",
        1.0,
    ),
    direct(
        "yaw-step-10deg",
        ControlChannel::Yaw,
        "alia.direct.yaw",
        0.5,
    ),
    direct(
        "roll-return-zero",
        ControlChannel::Roll,
        "alia.direct.roll",
        0.5,
    ),
    direct(
        "pitch-return-zero",
        ControlChannel::Pitch,
        "alia.direct.pitch",
        0.5,
    ),
    collective("collective-step-up", 0.5),
    collective("collective-step-down", -0.5),
    operator(
        "operator-roll-velocity",
        ControlChannel::Roll,
        "alia.operator.horizontal",
        5.0,
    ),
    operator(
        "operator-pitch-velocity",
        ControlChannel::Pitch,
        "alia.operator.horizontal",
        5.0,
    ),
    operator(
        "operator-vertical-velocity",
        ControlChannel::Vertical,
        "alia.operator.vertical",
        3.0,
    ),
    operator(
        "operator-yaw-rate",
        ControlChannel::Yaw,
        "alia.operator.yaw",
        1.5,
    ),
];

const CONDITIONS: [MatrixCondition; 12] = [
    MatrixCondition {
        id: "calm",
        factor: UncertaintyFactor::Calm,
    },
    MatrixCondition {
        id: "crosswind",
        factor: UncertaintyFactor::SteadyWind {
            speed_mps: 5.0,
            direction_deg: 270.0,
        },
    },
    MatrixCondition {
        id: "headwind",
        factor: UncertaintyFactor::SteadyWind {
            speed_mps: 5.0,
            direction_deg: 0.0,
        },
    },
    MatrixCondition {
        id: "gust-release",
        factor: UncertaintyFactor::Gust {
            speed_mps: 5.0,
            hold_ns: 1_000_000_000,
        },
    },
    MatrixCondition {
        id: "authority-high",
        factor: UncertaintyFactor::ActuatorAuthority {
            basis_points: 12_000,
        },
    },
    MatrixCondition {
        id: "authority-low",
        factor: UncertaintyFactor::ActuatorAuthority {
            basis_points: 8_000,
        },
    },
    MatrixCondition {
        id: "hover-trim-high",
        factor: UncertaintyFactor::HoverTrim {
            basis_points: 11_000,
        },
    },
    MatrixCondition {
        id: "hover-trim-low",
        factor: UncertaintyFactor::HoverTrim {
            basis_points: 9_000,
        },
    },
    MatrixCondition {
        id: "sensor-noise",
        factor: UncertaintyFactor::SensorNoise { lanes: 6 },
    },
    MatrixCondition {
        id: "timing-jitter",
        factor: UncertaintyFactor::TimingJitter {
            maximum_delay_ns: 4_000_000,
            interval_ns: 250_000_000,
        },
    },
    MatrixCondition {
        id: "added-delay",
        factor: UncertaintyFactor::AddedDelay {
            estimate_delay_ns: 30_000_000,
        },
    },
    MatrixCondition {
        id: "command-loss",
        factor: UncertaintyFactor::CommandLoss {
            fraction_basis_points: 100,
            decision_interval_samples: 100,
        },
    },
];

const REPRESENTATIVES: [&str; 2] = ["roll-step-10deg", "operator-roll-velocity"];

/// The complete Alia 250 scenario matrix.
pub const ALIA250_MATRIX: ScenarioMatrix = ScenarioMatrix {
    id: "alia250-xplane",
    stimuli: &STIMULI,
    conditions: &CONDITIONS,
    family_representatives: &REPRESENTATIVES,
};

const fn direct(
    id: &'static str,
    channel: ControlChannel,
    envelope_id: &'static str,
    normalized_value: f64,
) -> MatrixStimulus {
    MatrixStimulus {
        id,
        family: ControlFamily::DirectAttitudeThrust,
        channel,
        envelope_id,
        positive_endpoint: ATTITUDE_ENDPOINT_RAD,
        normalized_value,
    }
}

const fn collective(id: &'static str, normalized_value: f64) -> MatrixStimulus {
    MatrixStimulus {
        id,
        family: ControlFamily::DirectAttitudeThrust,
        channel: ControlChannel::Vertical,
        envelope_id: "alia.direct.collective",
        positive_endpoint: 0.3,
        normalized_value,
    }
}

const fn operator(
    id: &'static str,
    channel: ControlChannel,
    envelope_id: &'static str,
    positive_endpoint: f64,
) -> MatrixStimulus {
    MatrixStimulus {
        id,
        family: ControlFamily::OperatorVelocity,
        channel,
        envelope_id,
        positive_endpoint,
        normalized_value: 0.85,
    }
}
