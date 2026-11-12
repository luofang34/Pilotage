use crate::engine::condition::all_match;
use crate::{MissionCondition, MissionObservation, NavigationCondition, NavigationObservation};

fn captured(state: Option<bool>) -> MissionObservation {
    MissionObservation {
        navigation: NavigationObservation {
            altitude_captured: state,
            ..NavigationObservation::default()
        },
        ..MissionObservation::default()
    }
}

#[test]
fn an_altitude_capture_is_met_only_by_the_state_the_executor_reports() {
    let wants = |expected| {
        [MissionCondition::Navigation(
            NavigationCondition::AltitudeCaptured { expected },
        )]
    };
    let cases = [
        (true, Some(true), true),
        (true, Some(false), false),
        (true, None, false),
        (false, Some(false), true),
        (false, Some(true), false),
        (false, None, false),
    ];
    for (expected, reported, met) in cases {
        assert_eq!(
            all_match(&wants(expected), None, &captured(reported)),
            met,
            "wants {expected}, reported {reported:?}"
        );
    }
}

#[test]
fn an_observation_without_the_capture_reads_as_none_reported() {
    let observation: MissionObservation = serde_json::from_str(
        r#"{"navigation":{"guidance_valid":true,"plan_complete":false,"altitude_m":12.0},
            "vehicle":{},"signals":[]}"#,
    )
    .expect("an observation that states no capture still reads");
    assert_eq!(observation.navigation.altitude_captured, None);
}
