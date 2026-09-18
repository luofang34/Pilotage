#![allow(clippy::expect_used, clippy::panic)]

use pilotage_agent::{FlightEnvelope, ModelRequest, NumberRange};

use super::{ModelProcess, ModelProcessError};

/// A shell adapter: the declaration, then one reply line for each request
/// with the request number echoed. The reply is chosen by the message.
const ADAPTER: &str = r#"printf '%s\n' '{"ready":true,"adapter":"shell/1","model":"fixed","kinds":["land"]}'
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *stale-first*)
      printf '%s\n' '{"id":0,"directive":{"kind":"takeoff"},"probabilities":{},"model_ms":1.0}'
      printf '{"id":%s,"directive":{"kind":"land"},"probabilities":{},"model_ms":1.0}\n' "$id" ;;
    *from-the-future*)
      printf '{"id":%s,"directive":{"kind":"land"},"probabilities":{},"model_ms":1.0}\n' "$((id + 7))" ;;
    *) printf '{"id":%s,"directive":{"kind":"land"},"probabilities":{},"model_ms":1.0}\n' "$id" ;;
  esac
done"#;

fn request(message: &str) -> ModelRequest {
    ModelRequest {
        id: 0,
        message: message.to_owned(),
        envelope: FlightEnvelope {
            kinds: Vec::new(),
            fixes: Vec::new(),
            procedures: Vec::new(),
            height_m: NumberRange {
                min: 2.0,
                max: 30.0,
            },
            speed_mps: NumberRange { min: 0.3, max: 5.0 },
        },
        legend: String::new(),
        frames: Vec::new(),
    }
}

#[tokio::test]
async fn the_port_numbers_each_request_and_skips_a_reply_to_an_earlier_one() {
    let mut model = ModelProcess::spawn(ADAPTER)
        .await
        .expect("the shell adapter starts");
    let (first, _) = model
        .ask(&request("Cleared to land."))
        .await
        .expect("first reply");
    assert_eq!(first.id, 1);
    let (second, _) = model
        .ask(&request("stale-first"))
        .await
        .expect("second reply");
    assert_eq!(second.id, 2, "the stale line with id 0 is skipped");
    assert_eq!(
        serde_json::to_value(&second.directive).expect("encode")["kind"],
        "land"
    );
    model.stop().await;
}

#[tokio::test]
async fn a_reply_to_a_request_that_was_never_sent_is_a_fault() {
    let mut model = ModelProcess::spawn(ADAPTER)
        .await
        .expect("the shell adapter starts");
    let result = model.ask(&request("from-the-future")).await;
    assert!(
        matches!(
            result,
            Err(ModelProcessError::Correlation {
                expected: 1,
                actual: 8
            })
        ),
        "{result:?}"
    );
    model.stop().await;
}
