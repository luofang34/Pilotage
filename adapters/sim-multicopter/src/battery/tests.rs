use super::{Battery, Draw};
use crate::config::BatteryConfig;

#[test]
fn a_hover_drains_the_hover_power() {
    let config = BatteryConfig::default();
    let mut battery = Battery::full(config);
    for _ in 0..6_000 {
        battery.step(Draw::Flying, 0.0, 0.01);
    }
    // One minute at 28 W is 1680 J of the 65160 J pack.
    assert!((battery.consumed_j() - 1_680.0).abs() < 1e-6);
    let expected = 1.0 - 1_680.0 / (18.1 * 3_600.0);
    assert!((battery.remaining_fraction() - expected).abs() < 1e-9);
    assert!(battery.voltage_v() < config.full_voltage_v);
    assert!(battery.current_a() > 3.0);
}

#[test]
fn cruise_draws_more_than_hover() {
    let mut hover = Battery::full(BatteryConfig::default());
    let mut cruise = Battery::full(BatteryConfig::default());
    hover.step(Draw::Flying, 0.0, 10.0);
    cruise.step(Draw::Flying, 10.0, 10.0);
    assert!((cruise.consumed_j() - 360.0).abs() < 1e-9);
    assert!(cruise.consumed_j() > hover.consumed_j());
}

#[test]
fn stopped_motors_draw_nothing_and_the_pack_empties_at_its_capacity() {
    let mut battery = Battery::full(BatteryConfig {
        capacity_wh: 0.01,
        ..BatteryConfig::default()
    });
    battery.step(Draw::Off, 0.0, 100.0);
    assert_eq!(battery.consumed_j(), 0.0);
    assert_eq!(battery.time_remaining_s(), None);
    battery.step(Draw::Flying, 0.0, 100.0);
    assert!(battery.is_empty());
    assert_eq!(battery.remaining_fraction(), 0.0);
}
