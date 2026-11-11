#![allow(clippy::expect_used)]

use std::sync::{Arc, Mutex};
use web_time::Instant;

use crate::codec::{
    FcMessage, FrameSource, MAV_CMD_COMPONENT_ARM_DISARM, MAV_CMD_SET_MESSAGE_INTERVAL,
    STATUS_TEXT_LEN,
};
use crate::link::{LinkState, STATUS_TEXT_HISTORY, apply_messages_at};

const SOURCE: FrameSource = FrameSource {
    system_id: 1,
    component_id: 1,
    frame_sequence: 0,
};

fn apply(state: &Arc<Mutex<LinkState>>, message: FcMessage) {
    apply_messages_at(state, &[(SOURCE, message)], 0, 0, Instant::now());
}

fn ack(command: u16, result: u8) -> FcMessage {
    FcMessage::CommandAck {
        command,
        result,
        target_system: 0,
        target_component: 0,
    }
}

fn text(body: &str) -> FcMessage {
    let mut raw = [0_u8; STATUS_TEXT_LEN];
    for (slot, byte) in raw.iter_mut().zip(body.bytes()) {
        *slot = byte;
    }
    FcMessage::StatusText {
        severity: 4,
        text: raw,
    }
}

#[test]
fn a_periodic_interval_ack_does_not_bury_the_arm_verdict() {
    let state = Arc::new(Mutex::new(LinkState::default()));
    apply(&state, ack(MAV_CMD_COMPONENT_ARM_DISARM, 4));
    apply(&state, ack(MAV_CMD_SET_MESSAGE_INTERVAL, 0));

    let latest = state.lock().expect("lock");
    assert_eq!(
        latest.last_command_ack.expect("an ack").command,
        MAV_CMD_SET_MESSAGE_INTERVAL
    );
    let arm = latest
        .fc_reports
        .command_ack(MAV_CMD_COMPONENT_ARM_DISARM)
        .expect("the arm verdict stays");
    assert_eq!(arm.result, 4);
}

#[test]
fn status_texts_keep_a_bounded_ordered_history() {
    let state = Arc::new(Mutex::new(LinkState::default()));
    for index in 0..STATUS_TEXT_HISTORY + 3 {
        apply(&state, text(&format!("PreArm: reason {index}")));
    }

    let latest = state.lock().expect("lock");
    let texts = &latest.fc_reports.status_texts;
    assert_eq!(texts.len(), STATUS_TEXT_HISTORY);
    assert_eq!(texts.front().expect("oldest").text, "PreArm: reason 3");
    let newest = texts.back().expect("newest");
    assert_eq!(
        newest.text,
        format!("PreArm: reason {}", STATUS_TEXT_HISTORY + 2)
    );
    assert_eq!(newest.sequence, (STATUS_TEXT_HISTORY + 2) as u32);
}

#[test]
fn heartbeat_records_the_mode_and_autopilot() {
    let state = Arc::new(Mutex::new(LinkState::default()));
    apply(
        &state,
        FcMessage::Heartbeat {
            armed: true,
            custom_mode: 4,
            autopilot: 3,
            mav_type: 1,
        },
    );

    let latest = state.lock().expect("lock");
    assert_eq!(latest.fc_reports.custom_mode, Some(4));
    assert_eq!(latest.fc_reports.autopilot, Some(3));
    assert_eq!(latest.fc_reports.mav_type, Some(1));
    assert_eq!(latest.heartbeat_armed, Some(true));
}

#[test]
fn parameter_values_are_kept_by_name() {
    let state = Arc::new(Mutex::new(LinkState::default()));
    apply(
        &state,
        FcMessage::ParamValue {
            param_id: crate::codec::param_id("MOT_THST_HOVER"),
            value: crate::codec::ParamValue::Real32(0.39),
        },
    );
    let latest = state.lock().expect("lock");
    let report = latest.fc_reports.param("MOT_THST_HOVER").expect("kept");
    assert_eq!(report.value, crate::codec::ParamValue::Real32(0.39));
    assert!(latest.fc_reports.param("MOT_THST").is_none());
}

#[test]
fn an_ack_reaches_this_gcs_by_its_system_and_component_or_by_broadcast() {
    let addressed = |target_system, target_component| {
        let state = Arc::new(Mutex::new(LinkState::default()));
        apply(
            &state,
            FcMessage::CommandAck {
                command: MAV_CMD_COMPONENT_ARM_DISARM,
                result: 0,
                target_system,
                target_component,
            },
        );
        state.lock().expect("lock").last_command_ack.is_some()
    };
    // This GCS is system 255, component 190. Zero addresses every system
    // or every component of a system.
    for (system, component) in [(0, 0), (255, 190), (255, 0), (0, 190)] {
        assert!(addressed(system, component), "({system}, {component})");
    }
    for (system, component) in [(1, 0), (255, 1), (254, 190), (1, 1)] {
        assert!(!addressed(system, component), "({system}, {component})");
    }
}
