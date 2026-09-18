//! Reply checks and the wire form of the port.

use super::{Frame, ModelReply, ModelRequest, Projection, Refusal, check_reply};
use crate::capability::{FlightEnvelope, NumberRange};
use crate::directive::{Arrival, Directive, DirectiveKind, HoldPoint, TurnDirection};

fn envelope() -> FlightEnvelope {
    FlightEnvelope {
        kinds: vec![
            DirectiveKind::DirectTo,
            DirectiveKind::Heading,
            DirectiveKind::Altitude,
            DirectiveKind::Hold,
            DirectiveKind::JoinProcedure,
        ],
        fixes: vec!["ALPHA".into(), "HOME".into()],
        procedures: vec!["RNAV27".into()],
        height_m: NumberRange {
            min: 2.0,
            max: 30.0,
        },
        speed_mps: NumberRange { min: 0.5, max: 3.0 },
    }
}

fn reply(directive: Directive) -> ModelReply {
    ModelReply {
        directive,
        probabilities: Default::default(),
        model_ms: 0.0,
    }
}

#[test]
fn a_directive_inside_the_envelope_passes() {
    let direct = Directive::DirectTo {
        fix: "ALPHA".into(),
        on_arrival: Arrival::Land,
    };
    assert_eq!(check_reply(&envelope(), &reply(direct.clone())), Ok(direct));
}

#[test]
fn a_kind_that_is_not_offered_is_refused() {
    let refused = check_reply(&envelope(), &reply(Directive::Land {}));
    assert_eq!(refused, Err(Refusal::KindNotOffered(DirectiveKind::Land)));
}

#[test]
fn an_unknown_name_is_refused() {
    let fix = Directive::Hold {
        point: HoldPoint::Fix("ZULU".into()),
    };
    assert_eq!(
        check_reply(&envelope(), &reply(fix)),
        Err(Refusal::UnknownFix("ZULU".into()))
    );
    let procedure = Directive::JoinProcedure {
        procedure: "ILS09".into(),
    };
    assert_eq!(
        check_reply(&envelope(), &reply(procedure)),
        Err(Refusal::UnknownProcedure("ILS09".into()))
    );
}

#[test]
fn a_number_outside_its_range_is_refused_and_not_clamped() {
    let high = Directive::Altitude { height_m: 120.0 };
    assert!(matches!(
        check_reply(&envelope(), &reply(high)),
        Err(Refusal::OutOfRange {
            slot: "height_m",
            ..
        })
    ));
    let not_a_number = Directive::Altitude { height_m: f64::NAN };
    assert!(check_reply(&envelope(), &reply(not_a_number)).is_err());
    let heading = Directive::Heading {
        degrees: 360,
        turn: TurnDirection::Shortest,
    };
    assert!(check_reply(&envelope(), &reply(heading)).is_err());
}

#[test]
fn unable_always_passes_because_it_flies_nothing() {
    let unable = Directive::Unable {
        reason: "not an instruction".into(),
    };
    assert!(check_reply(&envelope(), &reply(unable)).is_ok());
}

#[test]
fn a_request_carries_frames_from_more_than_one_source() {
    let frame = |source: &str, role: &str, projection| Frame {
        source_id: source.into(),
        role: role.into(),
        captured_at_ns: 42,
        media_type: "image/jpeg".into(),
        width: 640,
        height: 480,
        projection,
        data_base64: "AAEC".into(),
    };
    let request = ModelRequest {
        message: "land on the marked pad".into(),
        envelope: envelope(),
        legend: String::new(),
        frames: vec![
            frame("cam.forward", "forward", Projection::Rectilinear),
            frame("cam.pano", "surround", Projection::Equirectangular),
        ],
    };
    let line = serde_json::to_string(&request).unwrap_or_default();
    let back: Result<ModelRequest, _> = serde_json::from_str(&line);
    assert_eq!(back.ok(), Some(request));
    assert!(line.contains(r#""projection":"equirectangular""#));
}

#[test]
fn a_text_request_needs_no_frames_field() {
    let line = r#"{"message":"hold","envelope":{"kinds":["hold"],"fixes":[],"procedures":[],
        "height_m":{"min":1,"max":2},"speed_mps":{"min":1,"max":2}},"legend":""}"#;
    let request: Result<ModelRequest, _> = serde_json::from_str(line);
    assert!(request.is_ok_and(|request| request.frames.is_empty()));
}
