use super::*;

const STEP_S: f64 = 0.1;

fn shape() -> CircuitShape {
    CircuitShape {
        inbound_rad: 1.0,
        right_turns: true,
        airspeed_mps: 60.0,
        turn_radius_m: 60.0 / (3.0_f64.to_radians()),
        leg_time_s: 60.0,
    }
}

/// Flies the circuit from the inbound leg's start for `seconds`, holding
/// each commanded heading exactly in `wind`: where each inbound leg
/// started, and the most it turned in a step against the pattern's
/// direction.
fn fly(wind: [f64; 2], seconds: f64) -> (Vec<[f64; 2]>, f64) {
    let shape = shape();
    let mut circuit = TimedCircuit::take_over(
        shape,
        HoldingLeg::Inbound,
        0.0,
        60.0,
        shape.inbound_rad,
        0.0,
        wind,
    );
    let (mut position, mut heading, mut against) = ([0.0_f64; 2], shape.inbound_rad, 0.0_f64);
    let mut starts = vec![position];
    let mut t = 0.0;
    while t < seconds {
        t += STEP_S;
        let was_inbound = circuit.on_inbound();
        let next = circuit.step(t, wind);
        if circuit.on_inbound() && !was_inbound {
            starts.push(position);
        }
        let turned = (next - heading).sin().atan2((next - heading).cos());
        against = against.max(-turned);
        heading = next;
        position = [
            position[0] + (60.0 * heading.cos() + wind[0]) * STEP_S,
            position[1] + (60.0 * heading.sin() + wind[1]) * STEP_S,
        ];
    }
    (starts, against)
}

/// One circuit's time: two legs of a minute and two half turns at 3°/s.
const CIRCUIT_S: f64 = 240.0;

/// The farthest any inbound leg started from the first, metres.
fn spread_m(starts: &[[f64; 2]]) -> f64 {
    starts.iter().map(|p| p[0].hypot(p[1])).fold(0.0, f64::max)
}

#[test]
fn in_still_air_a_timed_circuit_closes_on_itself() {
    let (starts, against) = fly([0.0, 0.0], 5.5 * CIRCUIT_S);
    assert!(starts.len() >= 6, "{starts:?}");
    assert!(spread_m(&starts) < 50.0, "{starts:?}");
    assert!(against < 1e-9, "a turn went the wrong way by {against} rad");
}

#[test]
fn in_a_wind_the_outbound_leg_takes_the_circuit_s_drift_back() {
    // Uncorrected, the turns alone drift 12 m/s for two minutes a circuit:
    // 1.4 km a circuit.
    for wind in [[12.0, 0.0], [0.0, 12.0], [-8.5, 8.5], [0.0, -12.0]] {
        let (starts, _) = fly(wind, 6.0 * CIRCUIT_S);
        assert!(starts.len() >= 5, "{wind:?}: {starts:?}");
        assert!(spread_m(&starts) < 50.0, "{wind:?}: {starts:?}");
    }
}
