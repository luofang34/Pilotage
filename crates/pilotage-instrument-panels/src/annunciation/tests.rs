#![allow(clippy::expect_used, clippy::panic)]

use std::string::String;
use std::vec::Vec;

use pilotage_alerts::{
    AlertCondition, AlertContext, AlertEvent, AlertManager, AlertOutput, AlertProfile, AltFault,
    DynFault, ManagerHealth, NavFault,
};
use pilotage_instrument_scene::SceneWriter;
use pilotage_instrument_state::{PanelData, SignalStatus};

use crate::pfd::tests::{flying, texts};
use crate::{PfdConfig, draw_hsi, draw_pfd};

fn stepped(events: &[AlertEvent], healthy: bool) -> AlertOutput {
    let mut manager = AlertManager::new();
    manager.step(
        &AlertProfile::simulator(),
        events,
        AlertContext {
            alerting_path_healthy: healthy,
            ..AlertContext::default()
        },
        1_000,
    )
}

fn saturated() -> AlertOutput {
    let events: Vec<AlertEvent> = (0..30)
        .map(|code| AlertEvent::Assert(AlertCondition::FrameMismatch { code }))
        .collect();
    stepped(&events, true)
}

fn pfd_scene(data: &PanelData, alerts: Option<&AlertOutput>) -> Vec<u8> {
    let mut buf = std::vec![0u8; 32 * 1024];
    let mut w = SceneWriter::new(&mut buf).expect("fits");
    draw_pfd(data, &PfdConfig::default(), alerts, &mut w).expect("pfd fits");
    let len = w.finish();
    buf.truncate(len);
    buf
}

fn hsi_scene(data: &PanelData, alerts: Option<&AlertOutput>) -> Vec<u8> {
    let mut buf = std::vec![0u8; 32 * 1024];
    let mut w = SceneWriter::new(&mut buf).expect("fits");
    draw_hsi(data, alerts, &mut w).expect("hsi fits");
    let len = w.finish();
    buf.truncate(len);
    buf
}

/// The stack tokens of a scene: every text run that is a known alert
/// label or stack marker. Primary flags (ATT/IAS/ALT/HDG/NAV) are
/// two-to-three-character runs that never collide with these.
fn stack_tokens(scene: &[u8]) -> Vec<String> {
    const STACK: &[&str] = &[
        "ALT SRC",
        "NAV SRC",
        "TRN RATE",
        "FRAME",
        "ALRT FAIL",
        "MORE",
        "ALERT",
    ];
    texts(scene)
        .into_iter()
        .filter(|t| STACK.contains(&t.as_str()))
        .collect()
}

fn failed_primaries() -> PanelData {
    let mut data = flying();
    data.roll_rad.status = SignalStatus::Failed;
    data.pitch_rad.status = SignalStatus::Failed;
    data.altitude.value_ft.status = SignalStatus::Failed;
    data
}

#[test]
fn widgets_consume_manager_output() {
    let out = stepped(
        &[
            AlertEvent::Assert(AlertCondition::Altitude(AltFault::Unavailable)),
            AlertEvent::Assert(AlertCondition::TurnSlip(DynFault::TurnRateInvalid)),
        ],
        true,
    );
    let with = stack_tokens(&pfd_scene(&flying(), Some(&out)));
    assert!(with.contains(&String::from("ALT SRC")), "{with:?}");
    assert!(with.contains(&String::from("TRN RATE")), "{with:?}");
    let without = stack_tokens(&pfd_scene(&flying(), None));
    assert!(
        without.is_empty(),
        "no manager output, no stack: {without:?}"
    );
}

#[test]
fn pfd_and_hsi_present_the_same_semantic_alert_state() {
    let out = stepped(
        &[
            AlertEvent::Assert(AlertCondition::Altitude(AltFault::Unavailable)),
            AlertEvent::Assert(AlertCondition::Heading(NavFault::Unavailable)),
        ],
        true,
    );
    let data = flying();
    let pfd = stack_tokens(&pfd_scene(&data, Some(&out)));
    let hsi = stack_tokens(&hsi_scene(&data, Some(&out)));
    assert_eq!(pfd, hsi, "one AlertOutput, one semantic stack, both panels");
    assert!(!pfd.is_empty());
}

#[test]
fn primary_flags_survive_every_manager_failure_mode() {
    let data = failed_primaries();
    let cases: [(&str, Option<AlertOutput>); 4] = [
        ("manager never stepped", None),
        ("manager empty", Some(stepped(&[], true))),
        ("alerting path faulted", Some(stepped(&[], false))),
        ("manager saturated", Some(saturated())),
    ];
    for (name, alerts) in &cases {
        let scene = pfd_scene(&data, alerts.as_ref());
        let all = texts(&scene);
        assert!(
            all.contains(&String::from("ATT")) && all.contains(&String::from("ALT")),
            "{name}: primary-data flags must render regardless of the manager: {all:?}"
        );
    }
}

#[test]
fn degraded_alerting_is_itself_annunciated() {
    let out = stepped(&[], false);
    assert_eq!(out.health(), ManagerHealth::Faulted);
    let tokens = stack_tokens(&pfd_scene(&flying(), Some(&out)));
    assert!(tokens.contains(&String::from("ALRT FAIL")), "{tokens:?}");
}

#[test]
fn saturation_shows_rows_and_the_more_marker() {
    let out = saturated();
    assert!(out.overflow(), "30 asserts past 24 slots must overflow");
    let tokens = stack_tokens(&pfd_scene(&flying(), Some(&out)));
    let frames = tokens.iter().filter(|t| t.as_str() == "FRAME").count();
    assert_eq!(frames, 3, "visible rows cap at the stack limit: {tokens:?}");
    assert!(tokens.contains(&String::from("MORE")), "{tokens:?}");
}

#[test]
fn every_label_is_covered_by_the_glyph_pack() {
    let labels = [
        "ALT REF",
        "BARO CMP",
        "ALT SRC",
        "HDG REF",
        "CRS SRC",
        "NAV SRC",
        "TRN RATE",
        "SLIP",
        "TRN SRC",
        "ATT CMP",
        "IAS CMP",
        "ALT CMP",
        "HDG CMP",
        "DSP STALL",
        "DSP GEN",
        "DSP BUF",
        "DSP LOST",
        "DSP HOLD",
        "DB OLD",
        "MAINT",
        "CONFIG",
        "FRAME",
        "ALERT",
        "ALRT FAIL",
        "MORE",
    ];
    for label in labels {
        for ch in label.chars() {
            assert!(
                pilotage_instrument_glyphs::PANEL_GLYPHS
                    .lookup(ch)
                    .is_some(),
                "glyph pack must cover {ch:?} in {label:?}"
            );
        }
    }
}
