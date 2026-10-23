use serde_json::json;

use crate::survey::{FrameContext, frame_request, read_frame};
use crate::{Answer, DecisionError, Provider};

fn request() -> crate::DecisionRequest {
    frame_request(
        vec![0xff, 0xd8, 0xff],
        &FrameContext {
            task_id: "line-01".to_owned(),
            height_m: 110.0,
            nadir: true,
        },
    )
}

#[test]
fn the_request_follows_the_system_one_shape() {
    let body = request().to_system_one(true);
    assert_eq!(body["questions"]["usable"]["type"], "noul");
    assert_eq!(body["questions"]["cover"]["type"], "choice");
    assert_eq!(
        body["questions"]["cover"]["criteria"]["trees"],
        "woodland or tree canopy"
    );
    assert_eq!(body["state"]["height_above_ground_m"], 110.0);
    let image = body["images"][0].as_str().unwrap_or_default();
    assert_eq!(image, "data:image/jpeg;base64,/9j/");
}

#[test]
fn text_is_normalized_to_composed_form() {
    let mut request = request();
    request.state = json!({"name": "Cafe\u{301}"});
    let body = request.to_system_one(true);
    assert_eq!(body["state"]["name"], "Caf\u{e9}");
    assert_eq!(request.to_system_one(false)["state"]["name"], "Cafe\u{301}");
}

#[test]
fn an_answer_is_read_and_the_floor_applies_to_yes_no_only() {
    let body = json!({"model": "m", "answers": {
        "usable": {"type": "noul", "noul": 0.62},
        "hazard": {"type": "noul", "noul": 0.05},
        "cover": {"type": "choice", "choice": "open", "probabilities": {"open": 0.7, "built": 0.3}, "confidence": 0.4}
    }});
    let bytes = body.to_string().into_bytes();
    let plain = Provider::system_one("http://127.0.0.1:1/v1/systemone");
    let response = plain.read(&request(), 200, &bytes).expect("an answer");
    assert_eq!(response.yes_no("usable"), Some(0.62));
    let floored = Provider::from_spec("http://127.0.0.1:1/v1/systemone;floor=0.801").expect("spec");
    let response = floored.read(&request(), 200, &bytes).expect("an answer");
    assert_eq!(response.yes_no("usable"), Some(0.801));
    assert_eq!(response.yes_no("hazard"), Some(0.05));
    let judgment = read_frame(&response).expect("a judgment");
    assert_eq!(judgment.cover.as_deref(), Some("open"));
    assert!(matches!(
        response.answers.get("cover"),
        Some(Answer::Choice { .. })
    ));
}

#[test]
fn a_missing_answer_or_a_failed_status_is_no_judgment() {
    let provider = Provider::system_one("http://127.0.0.1:1/v1/systemone");
    let partial = json!({"answers": {"usable": {"type": "noul", "noul": 0.9}}}).to_string();
    assert!(matches!(
        provider.read(&request(), 200, partial.as_bytes()),
        Err(DecisionError::Malformed(_))
    ));
    assert!(matches!(
        provider.read(&request(), 503, b"busy"),
        Err(DecisionError::Status { .. })
    ));
    assert!(Provider::from_spec("not-a-url").is_err());
    assert!(Provider::from_spec("http://x/v1/systemone;floor=2").is_err());
    let bearer = Provider::from_spec("https://x/v1/decide;bearer=t;nfc=off").expect("spec");
    assert!(
        bearer
            .call(&request())
            .headers
            .iter()
            .any(|(k, v)| k == "Authorization" && v == "Bearer t")
    );
    assert!(!bearer.normalize_nfc);
}
