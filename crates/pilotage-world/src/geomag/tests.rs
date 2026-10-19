//! The magnetic epoch.

use super::MagneticEpoch;

#[test]
fn an_epoch_from_unix_time_is_the_decimal_year() {
    // 2026-01-01T00:00:00Z.
    let epoch = MagneticEpoch::from_unix_seconds(1_767_225_600);
    assert!((epoch.decimal_year() - 2026.0).abs() < 1e-9);
    assert_eq!(
        MagneticEpoch::from_decimal_year(2027.25).decimal_year(),
        2027.25
    );
}
