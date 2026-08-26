use flight_tune::{ConfidenceInterval, PromotionPairedStatistics, RunRecord, ScoreAggregate};

use crate::{FeedbackError, error::invalid};

pub(super) fn aggregate(runs: &[RunRecord]) -> Result<ScoreAggregate, FeedbackError> {
    if runs.len() < 2 {
        return Err(invalid("an evaluation aggregate needs at least two runs"));
    }
    let loss = OnlineStats::from_values(runs.iter().map(|run| run.loss))?;
    let effort = OnlineStats::from_values(runs.iter().map(|run| run.control_effort))?;
    let variance = loss.sample_variance()?;
    let count = runs.len();
    let half_width = checked(student_t_95(count - 1) * (variance / count as f64).sqrt())?;
    let mut losses = runs.iter().map(|run| run.loss).collect::<Vec<_>>();
    losses.sort_by(f64::total_cmp);
    let percentile_index = ((count * 95).div_ceil(100)).saturating_sub(1);
    Ok(ScoreAggregate {
        run_count: u32::try_from(count)
            .map_err(|_| invalid("the evaluation run count exceeds u32"))?,
        mean_loss: loss.mean,
        p95_loss: losses[percentile_index],
        loss_variance: variance,
        loss_confidence_95: ConfidenceInterval {
            lower: checked(loss.mean - half_width)?,
            upper: checked(loss.mean + half_width)?,
        },
        mean_control_effort: effort.mean,
    })
}

pub(super) fn paired(
    values: impl Iterator<Item = f64>,
) -> Result<PromotionPairedStatistics, FeedbackError> {
    let stats = OnlineStats::from_values(values)?;
    if stats.count < 2 {
        return Err(invalid("a promotion comparison needs at least two pairs"));
    }
    let variance = stats.sample_variance()?;
    let half_width =
        checked(student_t_95(stats.count - 1) * (variance / stats.count as f64).sqrt())?;
    Ok(PromotionPairedStatistics {
        mean: stats.mean,
        upper_95: checked(stats.mean + half_width)?,
    })
}

pub(super) fn mean(values: impl Iterator<Item = f64>) -> Result<f64, FeedbackError> {
    let stats = OnlineStats::from_values(values)?;
    if stats.count < 2 {
        return Err(invalid("a promotion mean needs at least two runs"));
    }
    Ok(stats.mean)
}

struct OnlineStats {
    count: usize,
    mean: f64,
    m2: f64,
}

impl OnlineStats {
    fn from_values(values: impl Iterator<Item = f64>) -> Result<Self, FeedbackError> {
        let mut stats = Self {
            count: 0,
            mean: 0.0,
            m2: 0.0,
        };
        for value in values {
            if !value.is_finite() {
                return Err(invalid("a score input is not finite"));
            }
            stats.count = stats.count.wrapping_add(1);
            let delta = checked(value - stats.mean)?;
            stats.mean = checked(stats.mean + delta / stats.count as f64)?;
            let next_delta = checked(value - stats.mean)?;
            stats.m2 = checked(stats.m2 + checked(delta * next_delta)?)?;
        }
        Ok(stats)
    }

    fn sample_variance(&self) -> Result<f64, FeedbackError> {
        if self.count < 2 {
            return Err(invalid("sample variance needs at least two values"));
        }
        checked(self.m2 / (self.count - 1) as f64)
    }
}

pub(super) fn student_t_95(df: usize) -> f64 {
    const SMALL: [f64; 30] = [
        12.706, 4.303, 3.182, 2.776, 2.571, 2.447, 2.365, 2.306, 2.262, 2.228, 2.201, 2.179, 2.160,
        2.145, 2.131, 2.120, 2.110, 2.101, 2.093, 2.086, 2.080, 2.074, 2.069, 2.064, 2.060, 2.056,
        2.052, 2.048, 2.045, 2.042,
    ];
    match df {
        0 => f64::INFINITY,
        1..=30 => SMALL[df - 1],
        31..=40 => 2.042,
        41..=60 => 2.021,
        61..=80 => 2.000,
        81..=100 => 1.990,
        101..=120 => 1.984,
        _ => 1.980,
    }
}

fn checked(value: f64) -> Result<f64, FeedbackError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(invalid("score arithmetic produced a non-finite value"))
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::{paired, student_t_95};

    #[test]
    fn student_t_breakpoints_are_fixed() {
        assert_eq!(student_t_95(1), 12.706);
        assert_eq!(student_t_95(30), 2.042);
        assert_eq!(student_t_95(31), 2.042);
        assert_eq!(student_t_95(41), 2.021);
        assert_eq!(student_t_95(61), 2.000);
        assert_eq!(student_t_95(81), 1.990);
        assert_eq!(student_t_95(101), 1.984);
        assert_eq!(student_t_95(121), 1.980);
    }

    #[test]
    fn nonzero_variance_paired_vector_is_fixed() {
        let result = paired([0.7, 0.8, 0.9].into_iter().map(|value| value - 1.0))
            .expect("calculate paired statistics");
        assert_eq!(result.mean.to_bits(), 0xbfc9_9999_9999_9999);
        assert_eq!(result.upper_95.to_bits(), 0x3fa8_cc51_58fd_753c);
    }
}
