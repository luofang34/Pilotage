use thiserror::Error;

/// An error in a metric input or phase selection.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum MetricError {
    /// A series has fewer samples than the metric needs.
    #[error("the series needs {needed} samples but has {actual}")]
    TooFewSamples {
        /// The minimum sample count.
        needed: usize,
        /// The supplied sample count.
        actual: usize,
    },
    /// A sample time is not finite.
    #[error("sample {index} has a non-finite time")]
    NonFiniteTime {
        /// The sample index.
        index: usize,
    },
    /// A sample value is not finite.
    #[error("sample {index} has a non-finite {field}")]
    NonFiniteValue {
        /// The sample index.
        index: usize,
        /// The value field.
        field: &'static str,
    },
    /// A calculated metric value is not finite.
    #[error("the calculated {field} metric is not finite")]
    NonFiniteResult {
        /// The metric field.
        field: &'static str,
    },
    /// A sample time does not increase.
    #[error("sample {index} has time {current_s}, which does not follow {previous_s}")]
    NonMonotonicTime {
        /// The sample index.
        index: usize,
        /// The time of the prior sample, in seconds.
        previous_s: f64,
        /// The time of this sample, in seconds.
        current_s: f64,
    },
    /// An event time is outside the supplied series.
    #[error("{field} time {time_s} is outside the series")]
    EventOutsideSeries {
        /// The event field.
        field: &'static str,
        /// The event time, in seconds.
        time_s: f64,
    },
    /// A step has no change in value.
    #[error("the step has zero amplitude")]
    ZeroStep,
    /// The vehicle has no clear direction at release.
    #[error("the release velocity has no clear direction")]
    NoReleaseDirection,
    /// A numeric parameter is outside its fixed metric domain.
    #[error("{field} is not valid")]
    InvalidParameter {
        /// The parameter field.
        field: &'static str,
    },
}
