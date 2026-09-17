//! Scenario validation and the legend.

use super::Scenario;
use crate::capability::NumberRange;

const GOOD: &str = r#"{
  "id": "s",
  "fixes": {"ALPHA": {"north_m": 15, "east_m": 0}, "DELTA": {"north_m": -10, "east_m": 10}},
  "procedures": {"RNAV27": {"fixes": ["DELTA", "ALPHA"], "then": "land"}},
  "cruise_height_m": 5, "arrival_radius_m": 1.5, "max_range_m": 40,
  "height_m": {"min": 2, "max": 30},
  "messages": [{"text": "proceed direct ALPHA", "trigger": {"when": "start"},
                "means": {"kind": "direct_to", "fix": "ALPHA", "on_arrival": "hold"}}],
  "expect": {"checkpoints": [{"check": "approach", "fix": "ALPHA", "within_m": 5}],
             "end": {"state": "holding_at", "fix": "ALPHA"}, "timeout_s": 60}
}"#;

#[test]
fn a_complete_scenario_parses_and_gives_an_envelope_and_a_legend() {
    let scenario = Scenario::parse(GOOD);
    let Ok(scenario) = scenario else {
        unreachable!("the scenario parses: {scenario:?}");
    };
    let envelope = scenario.envelope(NumberRange { min: 0.5, max: 3.0 });
    assert_eq!(envelope.fixes, ["ALPHA", "DELTA", "HOME"]);
    assert_eq!(envelope.procedures, ["RNAV27"]);
    let legend = scenario.legend();
    assert!(legend.contains("ALPHA is 15 m north of HOME."), "{legend}");
    assert!(
        legend.contains("DELTA is 14 m south-east of HOME."),
        "{legend}"
    );
    assert!(
        legend.contains("RNAV27 goes DELTA, ALPHA, then land."),
        "{legend}"
    );
}

#[test]
fn a_scenario_that_names_an_unknown_fix_is_refused() {
    let bad = GOOD.replace(
        r#""fix": "ALPHA"}, "timeout_s""#,
        r#""fix": "ZULU"}, "timeout_s""#,
    );
    assert!(Scenario::parse(&bad).is_err());
    let bad_procedure = GOOD.replace(r#"["DELTA", "ALPHA"]"#, r#"["DELTA", "ZULU"]"#);
    assert!(Scenario::parse(&bad_procedure).is_err());
}

#[test]
fn home_is_implicit_and_a_cruise_height_outside_the_range_is_refused() {
    let listed = GOOD.replace(r#""ALPHA": {"north_m": 15"#, r#""HOME": {"north_m": 15"#);
    assert!(Scenario::parse(&listed).is_err());
    let low = GOOD.replace(r#""cruise_height_m": 5"#, r#""cruise_height_m": 1"#);
    assert!(Scenario::parse(&low).is_err());
}
