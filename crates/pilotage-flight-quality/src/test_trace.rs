use crate::TimedValue;

pub(crate) fn sample_value(
    rate_hz: u32,
    duration_s: f64,
    value: impl Fn(f64) -> f64,
) -> Vec<TimedValue> {
    let count = (duration_s * f64::from(rate_hz)).round() as u32;
    (0..=count)
        .map(|index| {
            let time_s = f64::from(index) / f64::from(rate_hz);
            TimedValue {
                time_s,
                value: value(time_s),
            }
        })
        .collect()
}
