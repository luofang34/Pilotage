#![allow(clippy::expect_used, clippy::panic)]

use pilotage_agent::Phase;

use super::{AgentModule, Decision, Telemetry};
use crate::error::ModuleError;

const CHART: &str = include_str!("../../../web/agent-chart.json");

fn module() -> AgentModule {
    AgentModule::new(CHART, 5.0, true).expect("the shipped chart is valid")
}

fn on_the_pad() -> Telemetry {
    Telemetry {
        position_ned_m: [0.0, 0.0, 0.0],
        velocity_ned_mps: [0.0; 3],
        quaternion_wxyz: [1.0, 0.0, 0.0, 0.0],
        arm_state: 1,
    }
}

#[test]
fn the_shipped_chart_gives_a_request_with_its_fixes_and_procedures() {
    let request = module()
        .request_json("Cleared RNAV 27 approach.")
        .expect("the request encodes");
    let value: serde_json::Value = serde_json::from_str(&request).expect("one JSON line");
    assert_eq!(value["message"], "Cleared RNAV 27 approach.");
    assert_eq!(value["frames"], serde_json::json!([]));
    let fixes = value["envelope"]["fixes"].to_string();
    assert!(fixes.contains("ALPHA") && fixes.contains("HOME"), "{fixes}");
    assert!(
        value["envelope"]["procedures"]
            .to_string()
            .contains("RNAV27")
    );
}

#[test]
fn a_good_reply_is_flown_and_the_agent_asks_for_an_arm() {
    let mut module = module();
    module.observe(on_the_pad());
    let decision = module.take_reply_json(
        "Cleared for takeoff.",
        r#"{"directive":{"kind":"takeoff"},"probabilities":{},"model_ms":3.0}"#,
        0.0,
    );
    assert!(matches!(decision, Decision::Flown { .. }), "{decision:?}");
    let entered = module.step(0.05);
    assert_eq!(entered.phase, Phase::Arming);
    assert!(!module.is_down());
    let tick = module.step(0.10);
    assert!(tick.arm, "a takeoff on the ground asks for an arm");
    assert!(!tick.disarm);
}

#[test]
fn a_reply_that_the_message_does_not_ground_is_refused() {
    let mut module = module();
    module.observe(on_the_pad());
    let decision = module.take_reply_json(
        "Proceed direct ZULU and land.",
        r#"{"directive":{"kind":"direct_to","fix":"ALPHA","on_arrival":"land"},"probabilities":{},"model_ms":3.0}"#,
        0.0,
    );
    assert!(matches!(decision, Decision::Refused { .. }), "{decision:?}");
    assert!(module.is_down(), "a refused reply changes nothing");
}

#[test]
fn an_adapter_fault_and_a_broken_line_are_faults_and_change_nothing() {
    let mut module = module();
    for line in [r#"{"error":"the model is not loaded"}"#, "not json", "{}"] {
        let decision = module.take_reply_json("Cleared for takeoff.", line, 0.0);
        assert!(
            matches!(decision, Decision::Fault { .. }),
            "{line}: {decision:?}"
        );
    }
    assert!(module.is_down());
}

#[test]
fn the_decision_encodes_with_a_result_tag() {
    let mut module = module();
    let decision = module.take_reply_json("hello", r#"{"error":"x"}"#, 0.0);
    let text = serde_json::to_string(&decision).expect("encode");
    assert_eq!(text, r#"{"result":"fault","detail":"x"}"#);
}

#[test]
fn a_tick_before_telemetry_is_neutral() {
    let mut module = module();
    let tick = module.step(0.0);
    assert_eq!(tick.demand, pilotage_agent::Demand::default());
    assert!(!tick.arm && !tick.disarm);
}

#[test]
fn a_chart_that_is_not_valid_and_a_speed_that_is_not_usable_are_refused() {
    assert!(matches!(
        AgentModule::new("{}", 5.0, true),
        Err(ModuleError::Chart(_))
    ));
    for speed in [0.0, -1.0, f64::NAN] {
        assert!(matches!(
            AgentModule::new(CHART, speed, true),
            Err(ModuleError::SpeedLimit { .. })
        ));
    }
}
